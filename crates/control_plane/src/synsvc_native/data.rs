use std::mem;

use syneroym_data_db::{auth, traits::ServiceStore};
use syneroym_fdae::Mode;
use syneroym_rpc::{
    AbacError, Ability, CandidateRow, NativeInvocation, NativeResponse, PERMISSION_DENIED_CODE,
    ResourceUri, RpcError, RpcResult, apply_stage4, union_masked_fields,
};
use syneroym_wit_interfaces::host::syneroym::data_layer::store::{
    CollectionSchema, DataLayerError, IndexDefinition, IndexType, Mutation, PatchMutation,
    QueryOptions, RawQueryResult, RecordReadValue, RecordWriteValue, SqlValue,
};

use super::*;

/// Maps `DataLayerError` the way `blob_error` does for `BlobError`, so a
/// caller (in particular the HTTP bridge, via
/// `status_for_rpc_error_code` in `crates/router/src/route_handler/http.rs`)
/// can distinguish "collection not found"/"schema violation"/"quota
/// exceeded" from a generic internal failure instead of every case
/// collapsing into `RpcError::InternalError`.
///
/// `PermissionDenied` is not reachable through the HTTP bridge's own
/// `get`/`query`/`put`/`patch` routes -- `execute-ddl` (unconditionally
/// denied to native callers, see the `execute-ddl` match arm below) and
/// `resolve-relation`'s stage-4 deny (ADR-0017 §7) are the two real
/// producers, and neither is bridged by any of those routes.
pub(crate) fn data_layer_error(e: DataLayerError) -> RpcError {
    match e {
        DataLayerError::PermissionDenied => {
            RpcError::Custom(PERMISSION_DENIED_CODE, "permission denied".to_string(), None)
        }
        DataLayerError::CollectionNotFound => {
            RpcError::Custom(-32011, "collection not found".to_string(), None)
        }
        DataLayerError::SchemaViolation(msg) => RpcError::Custom(-32012, msg, None),
        DataLayerError::QuotaExceeded => {
            RpcError::Custom(-32013, "data-layer quota exceeded".to_string(), None)
        }
        DataLayerError::Internal(msg) => internal(msg),
    }
}

/// Maps a stage-4 after-step failure to a `DataLayerError`, mirroring
/// `sandbox_wasm::host_capabilities`'s identical helper for the ingress
/// direction: `Unavailable` (includes the after-step's own
/// pool-exhaustion case)/`BudgetExceeded`/the batch-size caps are
/// resource pressure, reported as `QuotaExceeded` the same way `data_db`'s
/// watchdog timeout already is; the rest are the guest's own after-step
/// misbehaving, reported as `Internal`. Both ingresses fail closed either
/// way -- this only makes *why* distinguishable from "the after-step ran and
/// found nothing".
fn abac_error_to_data_layer_error(e: AbacError) -> DataLayerError {
    match e {
        AbacError::Unavailable(_)
        | AbacError::BudgetExceeded { .. }
        | AbacError::BatchTooLarge(_)
        | AbacError::PayloadTooLarge { .. } => DataLayerError::QuotaExceeded,
        // `MissingExport`/`ArityMismatch` carry only a service id / row
        // counts -- safe to echo in full.
        AbacError::MissingExport(_) | AbacError::ArityMismatch { .. } => {
            DataLayerError::Internal(e.to_string())
        }
        // `Trap`/`Malformed` can carry guest-authored (and, for a malformed
        // decision, potentially row-derived) text -- review residual R3,
        // same reasoning as `sandbox_wasm::host_capabilities::map_abac_error`:
        // a generic message keeps the caller-visible signal without putting
        // that text on the wire to the calling client; the detail still
        // reaches `AbacTrace::emit`'s (truncated, B4-06) log line.
        AbacError::Trap { .. } => {
            DataLayerError::Internal("stage-4 after-step trapped".to_string())
        }
        AbacError::Malformed(_) => {
            DataLayerError::Internal("stage-4 after-step returned a malformed decision".to_string())
        }
    }
}

/// Applies the host-side CLS field-mask projection to a single read record
/// (ADR-0017 §4). `query_auth` builds a real `QueryAuth` from `fdae_policy` +
/// the invocation's verified caller session, so
/// `outcome.masked_fields` is live here exactly as it is on the WASM host
/// path -- this is no longer a no-op for a policy-carrying service reached by
/// a router-verified external caller (`dispatch.rs`'s native arm). It stays
/// a no-op for a service deployed without a policy (`fdae_policy: None`),
/// unchanged from before.
fn strip_record(
    mut record: RecordReadValue,
    masked_fields: &[String],
) -> Result<RecordReadValue, DataLayerError> {
    record.payload = auth::strip_masked_fields(record.payload, masked_fields)?;
    Ok(record)
}

/// Converts a store-returned record into the stage-4 after-step's candidate
/// shape (ADR-0017 §7) -- mirrors
/// `sandbox_wasm::host_capabilities::to_candidate_row`; each ingress
/// converts its own `RecordReadValue` rather than sharing a type across the
/// WASM/native boundary (`syneroym-rpc` doesn't depend on
/// `syneroym-wit-interfaces`).
fn to_candidate_row(record: &RecordReadValue) -> CandidateRow {
    CandidateRow {
        id: record.id.clone(),
        payload: record.payload.clone(),
        creator_id: record.creator_id.clone(),
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

/// The inverse of [`to_candidate_row`] -- see that function's sibling in
/// `host_capabilities.rs` for why `apply_stage4`'s output can't be
/// positionally re-aligned against the original row list.
fn from_candidate_row(row: CandidateRow) -> RecordReadValue {
    RecordReadValue {
        id: row.id,
        payload: row.payload,
        creator_id: row.creator_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Hand-rolled DTO: the bindgen `SqlValue` variant derives serde's default
/// PascalCase externally-tagged form; this API is snake_case tagged JSON.
/// Used symmetrically for both `query-raw`'s request `params` and both
/// `query-raw`'s and `aggregate`'s response `rows` -- a caller must be able
/// to feed a returned cell straight back into a subsequent `query-raw`
/// call's `params` without re-encoding it.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
enum SqlValueDto {
    Text(String),
    Integer(i64),
    Real(f64),
    Boolean(bool),
    Null,
}

#[derive(serde::Serialize)]
struct RawQueryResultDto {
    columns: Vec<String>,
    rows: Vec<Vec<SqlValueDto>>,
}

fn raw_query_result_payload(result: RawQueryResult) -> RpcResult<NativeResponse> {
    let dto = RawQueryResultDto {
        columns: result.columns,
        rows: result
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|v| match v {
                        SqlValue::Text(s) => SqlValueDto::Text(s),
                        SqlValue::Integer(i) => SqlValueDto::Integer(i),
                        SqlValue::Real(f) => SqlValueDto::Real(f),
                        SqlValue::Boolean(b) => SqlValueDto::Boolean(b),
                        SqlValue::Null => SqlValueDto::Null,
                    })
                    .collect()
            })
            .collect(),
    };
    to_payload(&dto)
}

impl SynSvcNativeService {
    pub(super) async fn open_store(&self) -> Result<Box<dyn ServiceStore>, DataLayerError> {
        self.storage_provider
            .open_service_db(&self.service_id, &self.key_store)
            .await
            .map_err(|e| DataLayerError::Internal(e.to_string()))
    }

    /// Builds the `QueryAuth` for the current invocation from `fdae_policy` +
    /// the invocation's `caller.session`, mirroring `HostState::query_auth`.
    ///
    /// **No `AuthLevel` carve-out.** This deliberately does not branch on
    /// `AuthLevel::System` (or a `"system:"`-prefixed `caller_did`) to fall
    /// back to `auth = None`. Doing so would make a guest's self-proxy route
    /// (`ProxyRouter::invoke_local`'s `NativeHostChannel` branch, which
    /// synthesizes `CallerContext::service_system` for a guest calling its
    /// own service) *more* permissive than its direct WIT `store::Host`
    /// route under the same policy -- i.e. a guest under a policy could
    /// proxy to itself to escape it. The synthesized-identity ingress
    /// returning empty is over-restriction, which is correct; a carve-out
    /// here would be a bypass. Do not "simplify" this away -- it is a
    /// deliberate decision.
    /// Runs `syneroym_fdae::plan_read` itself (rather than
    /// letting `data_db` call the local-only `compile_read` internally),
    /// and when the policy's selected paths need a remote relationship
    /// fetch (pipeline stage 2), resolves it via `syneroym_rpc::
    /// resolve_fetches` + `syneroym_fdae::finalize` before ever reaching
    /// the store. Mirrors `sandbox_wasm::host_capabilities::HostState::
    /// resolve_query_auth`; see that doc comment for the fail-closed
    /// contract (a fetch error maps to `DataLayerError::PermissionDenied`).
    ///
    /// **No `AuthLevel` carve-out.** This deliberately does not branch on
    /// `AuthLevel::System` (or a `"system:"`-prefixed `caller_did`) to fall
    /// back to `auth = None`. Doing so would make a guest's self-proxy route
    /// (`ProxyRouter::invoke_local`'s `NativeHostChannel` branch, which
    /// synthesizes `CallerContext::service_system` for a guest calling its
    /// own service) *more* permissive than its direct WIT `store::Host`
    /// route under the same policy -- i.e. a guest under a policy could
    /// proxy to itself to escape it. The synthesized-identity ingress
    /// returning empty is over-restriction, which is correct; a carve-out
    /// here would be a bypass. Do not "simplify" this away -- it is a
    pub(super) async fn dispatch_data_layer(
        &self,
        invocation: NativeInvocation,
    ) -> RpcResult<NativeResponse> {
        let store = self.open_store().await.map_err(|e| internal(e.to_string()))?;
        match invocation.method.as_str() {
            "create-collection" | "create_collection" => {
                // Hand-rolled DTO: the bindgen-generated `IndexDefinition`
                // escapes the WIT `type` field as `type_` (a reserved
                // keyword), which doesn't match the plain `type` a JSON
                // caller would naturally send.
                #[derive(serde::Deserialize)]
                struct IndexDefinitionDto {
                    field_name: String,
                    #[serde(rename = "type")]
                    index_type: IndexType,
                }
                #[derive(serde::Deserialize)]
                struct Req {
                    name: String,
                    #[serde(default)]
                    indexes: Vec<IndexDefinitionDto>,
                }
                let req: Req = parse_params(&invocation)?;
                let schema = CollectionSchema {
                    name: req.name,
                    indexes: req
                        .indexes
                        .into_iter()
                        .map(|i| IndexDefinition { field_name: i.field_name, type_: i.index_type })
                        .collect(),
                };
                store.create_collection(&schema).await.map_err(|e| internal(e.to_string()))?;
                to_payload(&())
            }
            "drop-collection" | "drop_collection" => {
                // Admin-capability gate -- identical to `execute-ddl` below:
                // dropping an *existing* collection bypasses any per-row
                // policy on it entirely (every row a write-capable-but-
                // otherwise-unreachable caller could not delete individually
                // goes with it), so it must not be reachable through an
                // ordinary write capability. `create-collection` stays
                // ungated deliberately: a never-yet-existing collection has
                // no policy-protected rows to destroy, and an owner-rooted
                // UCAN chain can never carry `data-layer/admin` at all
                // (`router/src/route_handler/io.rs`'s `is_root` excludes
                // admin-entailing capabilities from per-service owner-rooting
                // on purpose) -- gating creation too would make it
                // impossible for a service on an unowned substrate to
                // provision its own schema at all.
                let resource = ResourceUri::service(
                    invocation.caller.app_instance.as_deref().unwrap_or(&self.service_id),
                    &self.service_id,
                );
                if !invocation
                    .caller
                    .has_capability(&resource, &Ability(Ability::DATA_LAYER_ADMIN.to_string()))
                {
                    return Err(data_layer_error(DataLayerError::PermissionDenied));
                }
                #[derive(serde::Deserialize)]
                struct Req {
                    name: String,
                }
                let req: Req = parse_params(&invocation)?;
                store.drop_collection(&req.name).await.map_err(|e| internal(e.to_string()))?;
                to_payload(&())
            }
            "put" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    value: RecordWriteValue,
                }
                let req: Req = parse_params(&invocation)?;
                let creator = invocation.caller.write_attribution(&self.service_id);
                let auth = self
                    .resolve_query_auth(
                        &invocation,
                        &req.collection,
                        &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                        Mode::Filter,
                    )
                    .await
                    .map_err(data_layer_error)?;
                store
                    .put(&req.collection, &req.value, &creator, auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;
                to_payload(&())
            }
            "patch" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    id: String,
                    patch_json: Vec<u8>,
                }
                let req: Req = parse_params(&invocation)?;
                let auth = self
                    .resolve_query_auth(
                        &invocation,
                        &req.collection,
                        &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                        Mode::Filter,
                    )
                    .await
                    .map_err(data_layer_error)?;
                store
                    .patch(&req.collection, &req.id, &req.patch_json, auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;
                to_payload(&())
            }
            "get" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    id: String,
                }
                let req: Req = parse_params(&invocation)?;
                let auth = self
                    .resolve_query_auth(
                        &invocation,
                        &req.collection,
                        &Ability(Ability::DATA_LAYER_READ.to_string()),
                        Mode::PointInTime { id: req.id.clone() },
                    )
                    .await
                    .map_err(data_layer_error)?;
                let outcome = store
                    .get(&req.collection, &req.id, auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;

                let result: Option<RecordReadValue> = match outcome.value {
                    None => None,
                    Some(record) => {
                        let sieve = auth.as_ref().and_then(|a| a.resolved_sieve.as_ref());
                        match sieve {
                            Some(sieve) if !sieve.abac_permissions.is_empty() => {
                                let session = &invocation.caller.session;
                                let candidate = to_candidate_row(&record);
                                // Fail-closed, but distinguishably (B4-04):
                                // see `abac_error_to_data_layer_error`.
                                let kept = apply_stage4(
                                    sieve,
                                    session,
                                    &self.service_id,
                                    &req.collection,
                                    self.row_authorizer.upgrade(),
                                    vec![candidate],
                                )
                                .await
                                .map_err(|e| data_layer_error(abac_error_to_data_layer_error(e)))?;
                                match kept.into_iter().next() {
                                    Some((_, extra)) => {
                                        let masked =
                                            union_masked_fields(&outcome.masked_fields, extra);
                                        Some(
                                            strip_record(record, &masked)
                                                .map_err(data_layer_error)?,
                                        )
                                    }
                                    None => None,
                                }
                            }
                            _ => Some(
                                strip_record(record, &outcome.masked_fields)
                                    .map_err(data_layer_error)?,
                            ),
                        }
                    }
                };
                to_payload(&result)
            }
            "query" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    opts: QueryOptions,
                }
                let req: Req = parse_params(&invocation)?;
                let auth = self
                    .resolve_query_auth(
                        &invocation,
                        &req.collection,
                        &Ability(Ability::DATA_LAYER_READ.to_string()),
                        Mode::Filter,
                    )
                    .await
                    .map_err(data_layer_error)?;
                let mut outcome = store
                    .query(&req.collection, &req.opts, auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;
                let rows = mem::take(&mut outcome.value.records);

                let sieve = auth.as_ref().and_then(|a| a.resolved_sieve.as_ref());
                let kept: Vec<(RecordReadValue, Vec<String>)> = match sieve {
                    Some(sieve) if !sieve.abac_permissions.is_empty() => {
                        let session = &invocation.caller.session;
                        let candidates: Vec<CandidateRow> =
                            rows.iter().map(to_candidate_row).collect();
                        match apply_stage4(
                            sieve,
                            session,
                            &self.service_id,
                            &req.collection,
                            self.row_authorizer.upgrade(),
                            candidates,
                        )
                        .await
                        {
                            Ok(kept) => kept
                                .into_iter()
                                .map(|(row, extra)| (from_candidate_row(row), extra))
                                .collect(),
                            // Fail-closed, but as a distinguishable error,
                            // not a silent empty-and-successful page
                            // (B4-04): resetting `next_cursor` to `None`
                            // here is exactly "no more pages", which is the
                            // wrong signal for an after-step that couldn't
                            // run at all, as opposed to one that ran and
                            // denied every row.
                            Err(e) => {
                                return Err(data_layer_error(abac_error_to_data_layer_error(e)));
                            }
                        }
                    }
                    _ => rows.into_iter().map(|r| (r, Vec::new())).collect(),
                };

                let records = kept
                    .into_iter()
                    .map(|(record, extra)| {
                        let masked = union_masked_fields(&outcome.masked_fields, extra);
                        strip_record(record, &masked)
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(data_layer_error)?;
                outcome.value.records = records;
                to_payload(&outcome.value)
            }
            "delete" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    id: String,
                }
                let req: Req = parse_params(&invocation)?;
                let auth = self
                    .resolve_query_auth(
                        &invocation,
                        &req.collection,
                        &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                        Mode::Filter,
                    )
                    .await
                    .map_err(data_layer_error)?;
                // `data_layer_error`, not `internal` -- a stage-4/watchdog
                // `PermissionDenied` (and an FDAE write denial) must surface
                // as a permission denial, not an opaque internal error
                // (pre-existing bug, fixed in passing).
                store
                    .delete(&req.collection, &req.id, auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;
                to_payload(&())
            }
            "delete-many" | "delete_many" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    filter: Option<String>,
                }
                let req: Req = parse_params(&invocation)?;
                let auth = self
                    .resolve_query_auth(
                        &invocation,
                        &req.collection,
                        &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                        Mode::Filter,
                    )
                    .await
                    .map_err(data_layer_error)?;
                // `data_layer_error`, not `internal` -- `delete_many`'s
                // stage-4 `PermissionDenied` (`sqlite.rs`) was already
                // surfacing as an opaque internal error rather than a
                // permission denial before this fix (pre-existing bug,
                // fixed in passing, same class as `delete`/`batch-mutate`
                // above).
                let affected = store
                    .delete_many(&req.collection, req.filter.as_deref(), auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;
                to_payload(&affected)
            }
            "batch-mutate" | "batch_mutate" => {
                // Hand-rolled DTO: the bindgen-generated `Mutation` variant
                // derives serde's default externally-tagged representation
                // (e.g. `{"Put": {...}}`, PascalCase), which doesn't match
                // this API's snake_case JSON convention.
                #[derive(serde::Deserialize)]
                #[serde(tag = "type", content = "value", rename_all = "snake_case")]
                enum MutationDto {
                    Put(RecordWriteValue),
                    Patch(PatchMutation),
                    Delete(String),
                }
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    mutations: Vec<MutationDto>,
                }
                let req: Req = parse_params(&invocation)?;
                let mutations: Vec<Mutation> = req
                    .mutations
                    .into_iter()
                    .map(|m| match m {
                        MutationDto::Put(v) => Mutation::Put(v),
                        MutationDto::Patch(v) => Mutation::Patch(v),
                        MutationDto::Delete(v) => Mutation::Delete(v),
                    })
                    .collect();
                let creator = invocation.caller.write_attribution(&self.service_id);
                let auth = self
                    .resolve_query_auth(
                        &invocation,
                        &req.collection,
                        &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                        Mode::Filter,
                    )
                    .await
                    .map_err(data_layer_error)?;
                // `data_layer_error`, not `internal` -- same fix as `delete`
                // above (pre-existing bug, fixed in passing).
                store
                    .batch_mutate(&req.collection, &mutations, &creator, auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;
                to_payload(&())
            }
            "execute-ddl" | "execute_ddl" => {
                // Admin-capability gate (ADR-0015/0016, replaces the former
                // `is_init_context` scaffold): only a caller holding
                // `data-layer/admin` on this service's resource may run DDL.
                // Lifecycle init/migrate runs as `AuthLevel::LocalElevated`
                // (`CallerContext::local_elevated`), which carries it; an
                // ordinary caller does not.
                let resource = ResourceUri::service(
                    invocation.caller.app_instance.as_deref().unwrap_or(&self.service_id),
                    &self.service_id,
                );
                if !invocation
                    .caller
                    .has_capability(&resource, &Ability(Ability::DATA_LAYER_ADMIN.to_string()))
                {
                    return Err(data_layer_error(DataLayerError::PermissionDenied));
                }
                #[derive(serde::Deserialize)]
                struct Req {
                    sql: String,
                }
                let req: Req = parse_params(&invocation)?;
                store.execute_ddl(&req.sql).await.map_err(data_layer_error)?;
                to_payload(&())
            }
            "query-raw" | "query_raw" => {
                // Admin-capability gate -- identical to `execute-ddl` above.
                let resource = ResourceUri::service(
                    invocation.caller.app_instance.as_deref().unwrap_or(&self.service_id),
                    &self.service_id,
                );
                if !invocation
                    .caller
                    .has_capability(&resource, &Ability(Ability::DATA_LAYER_ADMIN.to_string()))
                {
                    return Err(data_layer_error(DataLayerError::PermissionDenied));
                }

                #[derive(serde::Deserialize)]
                struct Req {
                    sql: String,
                    #[serde(default)]
                    params: Vec<SqlValueDto>,
                }
                let req: Req = parse_params(&invocation)?;
                let params: Vec<SqlValue> = req
                    .params
                    .into_iter()
                    .map(|p| match p {
                        SqlValueDto::Text(s) => SqlValue::Text(s),
                        SqlValueDto::Integer(i) => SqlValue::Integer(i),
                        SqlValueDto::Real(f) => SqlValue::Real(f),
                        SqlValueDto::Boolean(b) => SqlValue::Boolean(b),
                        SqlValueDto::Null => SqlValue::Null,
                    })
                    .collect();
                let result = store.query_raw(&req.sql, &params).await.map_err(data_layer_error)?;
                raw_query_result_payload(result)
            }
            "aggregate" => {
                // No capability gate -- unlike `execute-ddl`/`query-raw`,
                // `aggregate` compiles a whitelisted operator document, the
                // same trust level as `query`.
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    pipeline: String,
                }
                let req: Req = parse_params(&invocation)?;
                let auth = self
                    .resolve_query_auth(
                        &invocation,
                        &req.collection,
                        &Ability(Ability::DATA_LAYER_READ.to_string()),
                        Mode::Filter,
                    )
                    .await
                    .map_err(data_layer_error)?;
                let result = store
                    .aggregate(&req.collection, &req.pipeline, auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;
                raw_query_result_payload(result)
            }
            "resolve-relation" | "resolve_relation" => {
                self.resolve_relation(&invocation, store.as_ref()).await
            }
            other => Err(RpcError::MethodNotFound(format!("data-layer/{other}"))),
        }
    }

    // -- vault ------------------------------------------------------------
}

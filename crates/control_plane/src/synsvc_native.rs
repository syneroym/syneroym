//! Native (non-WASM) JSON-RPC dispatch for a deployed `SynSvc`'s
//! data-layer/vault/app-config/blob-store capabilities.
//!
//! One instance is registered per deployed `service_id` in
//! `ControlPlaneService::deploy` (`crates/control_plane/src/service.rs`),
//! mirroring the same host-provided capabilities the WASM `Host` trait
//! impls in `crates/sandbox_wasm/src/engine.rs` expose to guests -- this is
//! the second, independent adapter over the same underlying
//! `StorageProvider`/`ServiceStore`/`BlobProvider` traits, not a
//! reimplementation of their logic. Does **not** depend on
//! `syneroym-sandbox-wasm`: that crate is an optional, feature-gated
//! dependency of `control_plane` (see `crate::dummy_sandbox`), and native
//! data-layer/blob-store access must work even in builds without the WASM
//! sandbox feature enabled.

use std::{collections::HashMap, fmt, mem, sync::Arc};

use serde_json::Value;
use syneroym_data_blob::{
    BlobError, BlobProvider,
    native_types::{
        CloseDownloadRequest, FinishUploadResponse, OpenDownloadRequest, OpenDownloadResponse,
        OpenUploadResponse, ReadChunkRequest, ReadChunkResponse, SessionIdRequest,
        WriteChunkRequest,
    },
    traits::{DownloadSession, UploadSession},
};
use syneroym_data_db::{
    auth,
    auth::QueryAuth,
    traits::{ServiceStore, StorageProvider},
};
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::Policy;
use syneroym_mqtt_broker::{MqttBroker, namespace_topic_for_publish};
use syneroym_rpc::{
    Ability, NativeInvocation, NativeResponse, NativeService, ResourceUri, RpcError, RpcResult,
};
use syneroym_wit_interfaces::host::syneroym::{
    app_config::app_config::ConfigError,
    data_layer::store::{
        CollectionSchema, DataLayerError, IndexDefinition, IndexType, Mutation, PatchMutation,
        QueryOptions, RawQueryResult, RecordReadValue, RecordWriteValue, SqlValue,
    },
    vault::vault::VaultError,
};
use tokio::sync::Mutex;
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct SynSvcNativeService {
    service_id: String,
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
    blob_provider: Arc<dyn BlobProvider>,
    messaging_broker: Arc<MqttBroker>,
    upload_sessions: Mutex<HashMap<String, Box<dyn UploadSession>>>,
    download_sessions: Mutex<HashMap<String, Box<dyn DownloadSession>>>,
    /// `None` = unfiltered (today's behavior for a service deployed without
    /// a policy). Set once at construction from the `Arc<Policy>` `deploy`
    /// already parsed/validated (ADR-0017) -- no load, no cache, no parse on
    /// this hot path. A re-deploy reconstructs the service, so a policy edit
    /// takes effect with the deploy that carries it.
    fdae_policy: Option<Arc<Policy>>,
}

impl fmt::Debug for SynSvcNativeService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SynSvcNativeService")
            .field("service_id", &self.service_id)
            .finish_non_exhaustive()
    }
}

fn internal(msg: impl fmt::Display) -> RpcError {
    RpcError::InternalError(msg.to_string())
}

fn invalid_params(msg: impl fmt::Display) -> RpcError {
    RpcError::InvalidParams(msg.to_string())
}

/// Maps `BlobError` the way `engine.rs`'s `map_blob_error` does for the WASM
/// path, but into `RpcError::Custom` codes (there's no shared WIT
/// `blob-error` variant on this native-dispatch path to map onto), so a
/// caller can distinguish "not found"/"quota exceeded" from a generic
/// internal failure instead of every case collapsing into
/// `RpcError::InternalError`.
fn blob_error(e: BlobError) -> RpcError {
    match e {
        BlobError::NotFound => RpcError::Custom(-32001, "blob not found".to_string(), None),
        BlobError::QuotaExceeded => {
            RpcError::Custom(-32002, "blob quota exceeded".to_string(), None)
        }
        BlobError::Internal(msg) => internal(msg),
    }
}

/// Maps `DataLayerError` the way `blob_error` does for `BlobError`, so a
/// caller (in particular Slice 7's HTTP bridge, via
/// `status_for_rpc_error_code` in `crates/router/src/route_handler/http.rs`)
/// can distinguish "collection not found"/"schema violation"/"quota
/// exceeded" from a generic internal failure instead of every case
/// collapsing into `RpcError::InternalError`.
///
/// `PermissionDenied` is mapped for completeness, but note it is not
/// reachable through any of Slice 7's own bridged `get`/`query`/`put`/
/// `patch` routes -- the only real producer is `execute-ddl`, which is
/// unconditionally denied to native callers (see the `execute-ddl` match
/// arm below) and is not bridged by any Slice 7 route.
fn data_layer_error(e: DataLayerError) -> RpcError {
    match e {
        DataLayerError::PermissionDenied => {
            RpcError::Custom(-32010, "permission denied".to_string(), None)
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

fn parse_params<T: serde::de::DeserializeOwned>(invocation: &NativeInvocation) -> RpcResult<T> {
    serde_json::from_value(invocation.params.clone())
        .map_err(|e| invalid_params(format!("invalid params for {}: {e}", invocation.method)))
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

fn to_payload<T: serde::Serialize>(value: &T) -> RpcResult<NativeResponse> {
    serde_json::to_value(value)
        .map(|payload| NativeResponse { payload })
        .map_err(|e| internal(format!("failed to serialize response: {e}")))
}

impl SynSvcNativeService {
    #[must_use]
    pub fn new(
        service_id: String,
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
        blob_provider: Arc<dyn BlobProvider>,
        messaging_broker: Arc<MqttBroker>,
        fdae_policy: Option<Arc<Policy>>,
    ) -> Self {
        Self {
            service_id,
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            upload_sessions: Mutex::new(HashMap::new()),
            download_sessions: Mutex::new(HashMap::new()),
            fdae_policy,
        }
    }

    async fn open_store(&self) -> Result<Box<dyn ServiceStore>, DataLayerError> {
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
    /// here would be a bypass. Do not "simplify" this away -- see
    /// D-04-02-h in `task.md`'s Decision Register.
    fn query_auth<'a>(&'a self, invocation: &'a NativeInvocation) -> Option<QueryAuth<'a>> {
        self.fdae_policy.as_ref().map(|policy| QueryAuth {
            policy,
            session: &invocation.caller.session,
            service_id: &self.service_id,
        })
    }

    async fn resolve_blob_dek(&self) -> RpcResult<Option<Zeroizing<[u8; 32]>>> {
        self.storage_provider
            .load_service_dek(&self.service_id, &self.key_store)
            .await
            .map_err(internal)
    }

    // -- data-layer -----------------------------------------------------

    async fn dispatch_data_layer(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
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
                let creator = invocation
                    .caller
                    .app_instance
                    .as_deref()
                    .unwrap_or(&invocation.caller.caller_did);
                store.put(&req.collection, &req.value, creator).await.map_err(data_layer_error)?;
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
                store
                    .patch(&req.collection, &req.id, &req.patch_json)
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
                let auth = self.query_auth(&invocation);
                let outcome = store
                    .get(&req.collection, &req.id, auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;
                let result = outcome
                    .value
                    .map(|record| strip_record(record, &outcome.masked_fields))
                    .transpose()
                    .map_err(data_layer_error)?;
                to_payload(&result)
            }
            "query" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    opts: QueryOptions,
                }
                let req: Req = parse_params(&invocation)?;
                let auth = self.query_auth(&invocation);
                let mut outcome = store
                    .query(&req.collection, &req.opts, auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;
                let records = mem::take(&mut outcome.value.records)
                    .into_iter()
                    .map(|record| strip_record(record, &outcome.masked_fields))
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
                store
                    .delete(&req.collection, &req.id)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
                to_payload(&())
            }
            "delete-many" | "delete_many" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    collection: String,
                    filter: Option<String>,
                }
                let req: Req = parse_params(&invocation)?;
                let auth = self.query_auth(&invocation);
                let affected = store
                    .delete_many(&req.collection, req.filter.as_deref(), auth.as_ref())
                    .await
                    .map_err(|e| internal(e.to_string()))?;
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
                let creator = invocation
                    .caller
                    .app_instance
                    .as_deref()
                    .unwrap_or(&invocation.caller.caller_did);
                store
                    .batch_mutate(&req.collection, &mutations, creator)
                    .await
                    .map_err(|e| internal(e.to_string()))?;
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
                let auth = self.query_auth(&invocation);
                let result = store
                    .aggregate(&req.collection, &req.pipeline, auth.as_ref())
                    .await
                    .map_err(data_layer_error)?;
                raw_query_result_payload(result)
            }
            other => Err(RpcError::MethodNotFound(format!("data-layer/{other}"))),
        }
    }

    // -- vault ------------------------------------------------------------

    async fn dispatch_vault(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        match invocation.method.as_str() {
            "reveal" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    key: String,
                }
                let req: Req = parse_params(&invocation)?;
                let store = self
                    .storage_provider
                    .open_service_db(&self.service_id, &self.key_store)
                    .await
                    .map_err(internal)?;
                match store.reveal_secret(&req.key).await.map_err(internal)? {
                    Some(bytes) => to_payload(&bytes),
                    None => Err(internal(VaultError::NotFound.to_string())),
                }
            }
            other => Err(RpcError::MethodNotFound(format!("vault/{other}"))),
        }
    }

    // -- app-config ---------------------------------------------------------

    async fn dispatch_app_config(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        // Generation is resolved fresh per call, the native-dispatch
        // equivalent of "pinned at invocation start" (ADR-0008) -- each RPC
        // call *is* its own invocation here, there's no longer-lived Store
        // to pin a generation on ahead of time the way a WASM guest's does.
        let generation = self
            .storage_provider
            .get_latest_config_generation(&self.service_id)
            .await
            .map_err(internal)?;

        match invocation.method.as_str() {
            "get" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    key: String,
                }
                let req: Req = parse_params(&invocation)?;
                let Some((_, blob)) = generation else {
                    return to_payload(&Option::<String>::None);
                };
                let json: Value = serde_json::from_str(&blob)
                    .map_err(|e| internal(ConfigError::Internal(e.to_string()).to_string()))?;
                let val = json.get(&req.key).and_then(|v| v.as_str()).map(str::to_string);
                to_payload(&val)
            }
            "get-section" | "get_section" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    prefix: String,
                }
                let req: Req = parse_params(&invocation)?;
                let Some((_, blob)) = generation else {
                    return to_payload(&Vec::<(String, String)>::new());
                };
                let json: Value = serde_json::from_str(&blob)
                    .map_err(|e| internal(ConfigError::Internal(e.to_string()).to_string()))?;
                let mut results = Vec::new();
                if let Value::Object(map) = json {
                    for (k, v) in map {
                        if (k == req.prefix || k.starts_with(&format!("{}.", req.prefix)))
                            && let Some(s) = v.as_str()
                        {
                            results.push((k, s.to_string()));
                        }
                    }
                }
                to_payload(&results)
            }
            other => Err(RpcError::MethodNotFound(format!("app-config/{other}"))),
        }
    }

    // -- blob-store -----------------------------------------------------

    async fn dispatch_blob_store(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        // DEK resolution is a keystore/DB round trip -- only resolved for
        // the methods that actually pass it to `blob_provider` below.
        // `write-chunk`/`read-chunk`/`finish-upload`/`abort-upload`/
        // `close-download` operate on an already-open session (the DEK was
        // already used, once, at `open-upload`/`open-download` time) and
        // must not re-resolve it on every chunk -- a per-chunk resolve
        // here would mean one DB/keystore query per 64KB streamed.
        match invocation.method.as_str() {
            "put-blob" | "put_blob" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    data: Vec<u8>,
                }
                let req: Req = parse_params(&invocation)?;
                let dek = self.resolve_blob_dek().await?;
                let hash = self
                    .blob_provider
                    .put_blob(&self.service_id, req.data, dek)
                    .await
                    .map_err(blob_error)?;
                to_payload(&hash)
            }
            "get-blob" | "get_blob" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    hash: String,
                }
                let req: Req = parse_params(&invocation)?;
                let dek = self.resolve_blob_dek().await?;
                let data = self
                    .blob_provider
                    .get_blob(&self.service_id, &req.hash, dek)
                    .await
                    .map_err(blob_error)?;
                to_payload(&data)
            }
            "delete-blob" | "delete_blob" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    hash: String,
                }
                let req: Req = parse_params(&invocation)?;
                self.blob_provider
                    .delete_blob(&self.service_id, &req.hash)
                    .await
                    .map_err(blob_error)?;
                to_payload(&())
            }
            "signed-url" | "signed_url" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    hash: String,
                    ttl_secs: u32,
                }
                let req: Req = parse_params(&invocation)?;
                let dek = self.resolve_blob_dek().await?;
                let url = self
                    .blob_provider
                    .signed_url(&self.service_id, &req.hash, req.ttl_secs, dek)
                    .await
                    .map_err(blob_error)?;
                to_payload(&url)
            }
            "open-upload" | "open_upload" => {
                let dek = self.resolve_blob_dek().await?;
                let session = self
                    .blob_provider
                    .open_upload(&self.service_id, dek)
                    .await
                    .map_err(blob_error)?;
                let upload_id = Uuid::new_v4().to_string();
                self.upload_sessions.lock().await.insert(upload_id.clone(), session);
                to_payload(&OpenUploadResponse { upload_id })
            }
            "write-chunk" | "write_chunk" => {
                let req: WriteChunkRequest = parse_params(&invocation)?;
                // Held only for the lookup/reinsert, not across the I/O
                // `.await` below, so concurrent uploads for other sessions
                // aren't serialized on this one.
                let mut session = self
                    .upload_sessions
                    .lock()
                    .await
                    .remove(&req.upload_id)
                    .ok_or_else(|| invalid_params("unknown upload_id"))?;
                let result = session.write(req.chunk).await;
                self.upload_sessions.lock().await.insert(req.upload_id, session);
                result.map_err(blob_error)?;
                to_payload(&())
            }
            "finish-upload" | "finish_upload" => {
                let req: SessionIdRequest = parse_params(&invocation)?;
                let session = self
                    .upload_sessions
                    .lock()
                    .await
                    .remove(&req.upload_id)
                    .ok_or_else(|| invalid_params("unknown upload_id"))?;
                let hash = session.finish().await.map_err(blob_error)?;
                to_payload(&FinishUploadResponse { hash })
            }
            "abort-upload" | "abort_upload" => {
                let req: SessionIdRequest = parse_params(&invocation)?;
                let session = self.upload_sessions.lock().await.remove(&req.upload_id);
                if let Some(session) = session {
                    session.abort().await;
                }
                to_payload(&())
            }
            "open-download" | "open_download" => {
                let req: OpenDownloadRequest = parse_params(&invocation)?;
                let dek = self.resolve_blob_dek().await?;
                let session = self
                    .blob_provider
                    .open_download(&self.service_id, &req.hash, req.offset, dek)
                    .await
                    .map_err(blob_error)?;
                let download_id = Uuid::new_v4().to_string();
                self.download_sessions.lock().await.insert(download_id.clone(), session);
                to_payload(&OpenDownloadResponse { download_id })
            }
            "read-chunk" | "read_chunk" => {
                let req: ReadChunkRequest = parse_params(&invocation)?;
                // Held only for the lookup/reinsert, not across the I/O
                // `.await` below, so concurrent downloads for other
                // sessions aren't serialized on this one.
                let mut session = self
                    .download_sessions
                    .lock()
                    .await
                    .remove(&req.download_id)
                    .ok_or_else(|| invalid_params("unknown download_id"))?;
                let chunk = match session.read(req.max_bytes).await {
                    Ok(chunk) => chunk,
                    Err(e) => {
                        self.download_sessions.lock().await.insert(req.download_id, session);
                        return Err(blob_error(e));
                    }
                };
                let eof = chunk.is_empty();
                if !eof {
                    self.download_sessions.lock().await.insert(req.download_id, session);
                }
                to_payload(&ReadChunkResponse { chunk, eof })
            }
            "close-download" | "close_download" => {
                // Best-effort session release for a download that never
                // reaches EOF (e.g. an HTTP client disconnecting mid-
                // stream) -- see `BlobDownloadState`'s `Drop` impl in
                // `crates/router/src/route_handler/http.rs`. Removing an
                // unknown/already-EOF'd `download_id` is not an error.
                let req: CloseDownloadRequest = parse_params(&invocation)?;
                self.download_sessions.lock().await.remove(&req.download_id);
                to_payload(&())
            }
            other => Err(RpcError::MethodNotFound(format!("blob-store/{other}"))),
        }
    }

    // -- messaging --------------------------------------------------------

    /// Only `publish` is dispatched here: `subscribe`/`unsubscribe` need a
    /// long-lived push channel back to the caller, which `NativeService`'s
    /// one-request-one-response shape can't express -- see the router-level
    /// `messaging/subscribe` special-casing instead (ADR-0010 Finding A2).
    async fn dispatch_messaging(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        match invocation.method.as_str() {
            "publish" => {
                #[derive(serde::Deserialize)]
                struct Req {
                    topic: String,
                    payload: Vec<u8>,
                }
                let req: Req = parse_params(&invocation)?;
                let namespaced = namespace_topic_for_publish(&self.service_id, &req.topic);
                self.messaging_broker.publish(namespaced, req.payload).await.map_err(internal)?;
                to_payload(&())
            }
            other => Err(RpcError::MethodNotFound(format!("messaging/{other}"))),
        }
    }
}

#[async_trait::async_trait]
impl NativeService for SynSvcNativeService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        match invocation.interface.as_str() {
            "data-layer" => self.dispatch_data_layer(invocation).await,
            "vault" => self.dispatch_vault(invocation).await,
            "app-config" => self.dispatch_app_config(invocation).await,
            "blob-store" => self.dispatch_blob_store(invocation).await,
            "messaging" => self.dispatch_messaging(invocation).await,
            other => Err(RpcError::MethodNotFound(format!("unknown interface: {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `permission-denied`/`quota-exceeded` are not reachable end to end
    /// through any of Slice 7's own bridged `get`/`query`/`put`/`patch`
    /// HTTP routes (see the module doc on `data_layer_error`) -- unit-tested
    /// directly here instead, matching the honesty precedent Slice 6B used
    /// for its own untestable-end-to-end coverage gaps.
    #[test]
    fn data_layer_error_maps_every_variant_to_a_distinguishable_code() {
        assert!(matches!(
            data_layer_error(DataLayerError::PermissionDenied),
            RpcError::Custom(-32010, _, _)
        ));
        assert!(matches!(
            data_layer_error(DataLayerError::CollectionNotFound),
            RpcError::Custom(-32011, _, _)
        ));
        let RpcError::Custom(-32012, msg, _) =
            data_layer_error(DataLayerError::SchemaViolation("bad field".to_string()))
        else {
            panic!("schema-violation must map to Custom(-32012, ..)");
        };
        assert_eq!(msg, "bad field");
        assert!(matches!(
            data_layer_error(DataLayerError::QuotaExceeded),
            RpcError::Custom(-32013, _, _)
        ));
        assert!(matches!(
            data_layer_error(DataLayerError::Internal("boom".to_string())),
            RpcError::InternalError(_)
        ));
    }
}

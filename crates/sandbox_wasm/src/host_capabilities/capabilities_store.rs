use super::*;

/// Opens the calling component's isolated `ServiceStore`, mapping any
/// storage-level failure into an `Internal` data-layer error.
///
/// Takes owned/cloned pieces rather than `&HostState`: `HostState` embeds a
/// `WasiCtx`, which is not `Sync`, so holding a `&HostState` across an
/// `.await` would make the enclosing future non-`Send` (required by the
/// generated `Host` trait). Callers must clone what they need out of `self`
/// before awaiting, exactly as the pre-existing `vault::reveal` impl below
/// already does.
async fn open_store(
    component_id: String,
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
) -> Result<Box<dyn ServiceStore>, DataLayerError> {
    storage_provider
        .open_service_db(&component_id, &key_store)
        .await
        .map_err(|e| DataLayerError::Internal(e.to_string()))
}

/// Applies the host-side CLS field-mask projection to a single read record
/// (ADR-0017 §4, Phase 3). A fail-closed `Err` from `strip_masked_fields`
/// propagates, never a leaked payload.
fn strip_record(
    mut record: RecordReadValue,
    masked_fields: &[String],
) -> Result<RecordReadValue, DataLayerError> {
    record.payload = auth::strip_masked_fields(record.payload, masked_fields)?;
    Ok(record)
}

/// Converts a store-returned record into the stage-4 after-step's candidate
/// shape (ADR-0017 §7) -- the two types share the same fields (both mirror
/// the physical row), so this is a plain field copy.
fn to_candidate_row(record: &RecordReadValue) -> CandidateRow {
    CandidateRow {
        id: record.id.clone(),
        payload: record.payload.clone(),
        creator_id: record.creator_id.clone(),
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

/// The inverse of [`to_candidate_row`] -- reconstructs the WIT record shape
/// from a stage-4-surviving candidate, since `apply_stage4`'s output
/// (`kept`) is no longer positionally aligned with the original row list
/// (denied rows are dropped, not carried as `None`s).
fn from_candidate_row(row: CandidateRow) -> RecordReadValue {
    RecordReadValue {
        id: row.id,
        payload: row.payload,
        creator_id: row.creator_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Maps a stage-4 after-step failure to the `DataLayerError` `get`/`query`
/// actually return. Both fail closed either way (no
/// row data reaches the caller), but collapsing every `AbacError` into an
/// empty-but-successful page made "the after-step said no" indistinguishable
/// from "the after-step couldn't run at all" -- and the ADR-0007 "no result
/// is a valid outcome" principle this leaned on covers authorization
/// denials, not infrastructure failures. `Unavailable` (includes the
/// after-step's own pool-exhaustion case) and `BudgetExceeded` are
/// resource pressure, reported the same way `data_db`'s watchdog timeout
/// already reports one (`QuotaExceeded`); the rest are the guest's own
/// after-step misbehaving (a missing export, a trap, a malformed or
/// arity-mismatched decision list), reported as `Internal`.
fn map_abac_error(e: AbacError) -> DataLayerError {
    match e {
        AbacError::Unavailable(_)
        | AbacError::BudgetExceeded { .. }
        | AbacError::BatchTooLarge(_)
        | AbacError::PayloadTooLarge { .. } => DataLayerError::QuotaExceeded,
        // `MissingExport`/`ArityMismatch` carry only a service id / row
        // counts -- safe to echo in full, and useful for diagnosing a
        // deploy-time misconfiguration.
        AbacError::MissingExport(_) | AbacError::ArityMismatch { .. } => {
            DataLayerError::Internal(e.to_string())
        }
        // `Trap`/`Malformed` can carry guest-authored (and, for a malformed
        // decision, potentially row-derived) text: echoing it via
        // `DataLayerError::Internal` puts it on the wire to the calling
        // client, a channel that did not exist when an after-step failure
        // never reached the caller at all. A generic message keeps the
        // caller-visible signal to "the after-step failed" without the
        // detail; the detail itself still reaches `AbacTrace::emit`'s
        // (truncated) log line, which is the audience it's actually useful
        // to.
        AbacError::Trap { .. } => {
            DataLayerError::Internal("stage-4 after-step trapped".to_string())
        }
        AbacError::Malformed(_) => {
            DataLayerError::Internal("stage-4 after-step returned a malformed decision".to_string())
        }
    }
}

impl store::Host for HostState {
    async fn create_collection(&mut self, schema: CollectionSchema) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.create_collection(&schema).await
    }

    async fn drop_collection(&mut self, name: String) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        // Admin-capability gate, identical to `execute_ddl`'s: dropping an
        // *existing* collection bypasses any per-row policy on it entirely
        // (every row a write-capable-but-otherwise-unreachable caller could
        // not delete individually goes with it), so it must not be
        // reachable through an ordinary write capability. `create_collection`
        // stays ungated deliberately: a never-yet-existing collection has no
        // policy-protected rows to destroy, and an owner-rooted UCAN chain
        // can never carry `data-layer/admin` at all (`router/src/route_
        // handler/io.rs`'s `is_root` excludes admin-entailing capabilities
        // from per-service owner-rooting on purpose) -- gating creation too
        // would make it impossible for a service on an unowned substrate to
        // provision its own schema at all.
        let resource = ResourceUri::service(&self.component_id, &self.component_id);
        if !self.caller.has_capability(&resource, &Ability(Ability::DATA_LAYER_ADMIN.to_string())) {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.drop_collection(&name).await
    }

    async fn put(
        &mut self,
        collection: String,
        value: RecordWriteValue,
    ) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        // Owned locals first -- `resolve_query_auth` takes `&mut self` and
        // the returned `QueryAuth<'_>` borrows it, so nothing may touch
        // `self` afterwards. Same discipline `get`/`query` already document
        // above.
        let creator_id = self.caller.write_attribution(&self.component_id);
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.put(&collection, &value, &creator_id, query_auth.as_ref()).await
    }

    async fn patch(
        &mut self,
        collection: String,
        id: String,
        patch_json: Vec<u8>,
    ) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.patch(&collection, &id, &patch_json, query_auth.as_ref()).await
    }

    async fn get(
        &mut self,
        collection: String,
        id: String,
    ) -> Result<Option<RecordReadValue>, DataLayerError> {
        // Copied into owned locals up front, before `resolve_query_auth`'s
        // `&mut self` borrow starts (its returned `QueryAuth<'_>` ties to
        // that borrow, so `self` cannot be touched again while it's held) --
        // same `Send`-future discipline `resolve_query_auth`'s own doc
        // comment describes, applied one step earlier.
        let session = self.caller.session.clone();
        let service_id = self.component_id.clone();
        let authorizer = self.row_authorizer.upgrade();

        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_READ.to_string()),
                Mode::PointInTime { id: id.clone() },
            )
            .await?;
        let outcome = store.get(&collection, &id, query_auth.as_ref()).await?;
        let Some(record) = outcome.value else { return Ok(None) };

        let Some(sieve) = query_auth.as_ref().and_then(|a| a.resolved_sieve.as_ref()) else {
            return strip_record(record, &outcome.masked_fields).map(Some);
        };
        if sieve.abac_permissions.is_empty() {
            return strip_record(record, &outcome.masked_fields).map(Some);
        }
        let candidate = to_candidate_row(&record);
        // Fail-closed, but distinguishably: an after-step error
        // (pool exhaustion, a trap, a budget overrun) is not the same claim
        // as "the after-step ran and denied this row" -- only the latter is
        // `Ok(None)`.
        let kept =
            apply_stage4(sieve, &session, &service_id, &collection, authorizer, vec![candidate])
                .await
                .map_err(map_abac_error)?;
        let Some((_, extra)) = kept.into_iter().next() else { return Ok(None) };
        let masked = union_masked_fields(&outcome.masked_fields, extra);
        strip_record(record, &masked).map(Some)
    }

    async fn query(
        &mut self,
        collection: String,
        opts: QueryOptions,
    ) -> Result<QueryResult, DataLayerError> {
        // See `get`'s identical comment on why these are captured before
        // `resolve_query_auth` runs.
        let session = self.caller.session.clone();
        let service_id = self.component_id.clone();
        let authorizer = self.row_authorizer.upgrade();

        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_READ.to_string()),
                Mode::Filter,
            )
            .await?;
        let mut outcome = store.query(&collection, &opts, query_auth.as_ref()).await?;
        let rows = mem::take(&mut outcome.value.records);

        let sieve = query_auth.as_ref().and_then(|a| a.resolved_sieve.as_ref());
        let kept: Vec<(RecordReadValue, Vec<String>)> = match sieve {
            Some(sieve) if !sieve.abac_permissions.is_empty() => {
                let candidates: Vec<CandidateRow> = rows.iter().map(to_candidate_row).collect();
                match apply_stage4(
                    sieve,
                    &session,
                    &service_id,
                    &collection,
                    authorizer,
                    candidates,
                )
                .await
                {
                    // `kept` already excludes denied rows -- rebuild each
                    // surviving `RecordReadValue` from its candidate rather
                    // than trying to re-align against the original `rows`.
                    Ok(kept) => kept
                        .into_iter()
                        .map(|(row, extra)| (from_candidate_row(row), extra))
                        .collect(),
                    // Fail-closed, but as a distinguishable error, not a
                    // silent empty-and-successful page: clearing
                    // `next_cursor` here would make `records: []` +
                    // `next_cursor: None` read as "no more pages", which is
                    // exactly the wrong signal for an after-step that
                    // couldn't run, as opposed to one that ran and denied
                    // every row.
                    Err(e) => {
                        return Err(map_abac_error(e));
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
            .collect::<Result<Vec<_>, _>>()?;
        outcome.value.records = records;
        Ok(outcome.value)
    }

    async fn aggregate(
        &mut self,
        collection: String,
        pipeline: String,
    ) -> Result<RawQueryResult, DataLayerError> {
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_READ.to_string()),
                Mode::Filter,
            )
            .await?;
        store.aggregate(&collection, &pipeline, query_auth.as_ref()).await
    }

    async fn delete(&mut self, collection: String, id: String) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.delete(&collection, &id, query_auth.as_ref()).await
    }

    async fn delete_many(
        &mut self,
        collection: String,
        filter: String,
    ) -> Result<u64, DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.delete_many(&collection, Some(filter.as_str()), query_auth.as_ref()).await
    }

    /// Mode A point-in-time authorization check (ADR-0017 §4). No capability
    /// gate, unlike `execute_ddl`/`query_raw`: `check-access` *is* the
    /// authorization primitive, reveals only the caller's own access, and is
    /// fail-closed to `false` inside the store -- gating it would be
    /// circular.
    async fn check_access(
        &mut self,
        collection: String,
        id: String,
        operation: String,
    ) -> Result<bool, DataLayerError> {
        // See `get`'s comment on why these are captured before
        // `resolve_query_auth`'s `&mut self` borrow starts.
        let session = self.caller.session.clone();
        let service_id = self.component_id.clone();
        let authorizer = self.row_authorizer.upgrade();

        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        // Fail-closed to `Ok(false)` on any resolution error (including a
        // cross-service fetch failure), matching Mode A's existing
        // `PolicyError`-compile-failure convention -- unlike Mode B's
        // `get`/`query`/`aggregate`/`delete_many`, a broken/undecidable
        // policy read is "no access", not a hard error.
        let query_auth = match self
            .resolve_query_auth(
                &collection,
                &Ability(operation.clone()),
                Mode::PointInTime { id: id.clone() },
            )
            .await
        {
            Ok(auth) => auth,
            Err(_) => return Ok(false),
        };
        let sieve = query_auth.as_ref().and_then(|a| a.resolved_sieve.as_ref());
        match sieve {
            Some(sieve) if !sieve.abac_permissions.is_empty() => {
                // A `get` under this `Mode::PointInTime` sieve runs exactly
                // the predicate `check_access` would have run, and
                // additionally hands back the row -- required to ask the
                // after-step (ADR-0017 §7) the same question `check-access`
                // does: "may this caller reach the row".
                let outcome = match store.get(&collection, &id, query_auth.as_ref()).await {
                    Ok(o) => o,
                    Err(_) => return Ok(false),
                };
                let Some(record) = outcome.value else { return Ok(false) };
                let candidate = to_candidate_row(&record);
                let kept = apply_stage4(
                    sieve,
                    &session,
                    &service_id,
                    &collection,
                    authorizer,
                    vec![candidate],
                )
                .await;
                // A `redact` decision counts as reachable: the question is
                // "may this caller reach the row", and a redacted row was
                // reached. `Err` or an empty `kept` (a `deny` decision) is
                // `false`.
                Ok(matches!(kept, Ok(k) if !k.is_empty()))
            }
            _ => store.check_access(&collection, &id, &operation, query_auth.as_ref()).await,
        }
    }

    async fn batch_mutate(
        &mut self,
        collection: String,
        mutations: Vec<Mutation>,
    ) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        // Owned locals first -- see `put`'s identical comment.
        let creator_id = self.caller.write_attribution(&self.component_id);
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.batch_mutate(&collection, &mutations, &creator_id, query_auth.as_ref()).await
    }

    async fn create(
        &mut self,
        collection: String,
        values: Vec<RecordWriteValue>,
    ) -> Result<Option<String>, DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        let creator_id = self.caller.write_attribution(&self.component_id);
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        let query_auth = self
            .resolve_query_auth(
                &collection,
                &Ability(Ability::DATA_LAYER_WRITE.to_string()),
                Mode::Filter,
            )
            .await?;
        store.create(&collection, &values, &creator_id, query_auth.as_ref()).await
    }

    async fn execute_ddl(&mut self, sql: String) -> Result<(), DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        // Admin-capability gate (ADR-0015/0016, replaces the former
        // `is_init_context` scaffold): only a caller holding
        // `data-layer/admin` on this component's own resource may run DDL.
        // Lifecycle init/migrate runs as `AuthLevel::LocalElevated`
        // (`CallerContext::local_elevated`), which carries it.
        let resource = ResourceUri::service(&self.component_id, &self.component_id);
        if !self.caller.has_capability(&resource, &Ability(Ability::DATA_LAYER_ADMIN.to_string())) {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.execute_ddl(&sql).await
    }

    async fn query_raw(
        &mut self,
        sql: String,
        params: Vec<SqlValue>,
    ) -> Result<RawQueryResult, DataLayerError> {
        if self.read_only {
            return Err(DataLayerError::PermissionDenied);
        }
        // Admin-capability gate (ADR-0015/0016), identical to execute_ddl: only
        // a caller holding `data-layer/admin` on this component's own resource
        // may run raw SQL. Lifecycle init/migrate runs as
        // `AuthLevel::LocalElevated`, which carries it.
        let resource = ResourceUri::service(&self.component_id, &self.component_id);
        if !self.caller.has_capability(&resource, &Ability(Ability::DATA_LAYER_ADMIN.to_string())) {
            return Err(DataLayerError::PermissionDenied);
        }
        let store = open_store(
            self.component_id.clone(),
            self.key_store.clone(),
            self.storage_provider.clone(),
        )
        .await?;
        store.query_raw(&sql, &params).await
    }
}

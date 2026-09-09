use super::*;

impl SupervisorService {
    /// Tier 2 (ADR-0022 §3): signs and serves this app instance's topology
    /// document for one logical service.
    ///
    /// - looked up by the app's **master DID**, not its human name -- Tier 1
    ///   answers with a DID;
    /// - an unknown app and an unauthorized caller are refused identically, so
    ///   a caller with no grant cannot probe for an app's existence;
    /// - a refusal carries no member DIDs at all: the document is built whole
    ///   or not at all, and the authorization check runs before it is built;
    /// - answers for a paused instance, refuses for a retired one -- pause
    ///   stops the resident loop touching an instance, not its members, which
    ///   stay worth routing to;
    /// - signs once per `(service, epoch)` and serves the cached copy
    ///   afterwards, re-signing once less than half its validity remains.
    pub(super) async fn handle_resolve(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        let (app_did_str, service_name_str): (String, String) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse resolve params: {e}")))?;
        let app_did = AppDid::try_new(&app_did_str)
            .map_err(|e| RpcError::InvalidParams(format!("invalid app DID: {e}")))?;
        let service_name = LogicalServiceName::try_new(&service_name_str)
            .map_err(|e| RpcError::InvalidParams(format!("invalid service name: {e}")))?;

        // Look up first, authorize second, and report both failures the
        // same way: the lookup is a local read that tells the caller
        // nothing, and returning a distinguishable "no such app" would let
        // an ungranted caller enumerate this node's apps.
        let denied = || {
            RpcError::Custom(
                PERMISSION_DENIED_CODE,
                format!(
                    "no app instance '{app_did}' is resolvable by caller {} on this supervisor",
                    caller.caller_did
                ),
                None,
            )
        };
        let mut state = self
            .store
            .instance_by_app_master_did(app_did.as_str())
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(denied)?;

        let mut open_to_all = false;
        if let Ok(plan) = DeploymentPlan::from_json(&state.plan_json)
            && let Ok(name) = topology::resolve_service_name(&plan, &service_name)
        {
            open_to_all = topology::service_topology_visibility(&plan, &name)
                .map(|v| v == TopologyVisibility::Open)
                .unwrap_or(false);
        }

        // `synapp:<app-did>`, not `substrate:<node>/app/<id>`: the
        // latter's `app/` slot already holds a `service_id`, and it dies on
        // the handover ADR-0022 §5 explicitly worried about. A bare
        // `substrate:<node>` `substrate/admin` grant still covers this,
        // because `Capability::grants` short-circuits on
        // `is_substrate_scope`.
        if !open_to_all
            && !caller.has_capability(
                &ResourceUri(format!("synapp:{app_did}")),
                &Ability(Ability::SUPERVISOR_RESOLVE.to_string()),
            )
        {
            return Err(denied());
        }
        // A retired instance answers nothing -- the same denial as an
        // unknown app. `paused` is deliberately NOT checked: pause stops
        // the resident loop touching an instance, not its members, which
        // stay worth routing to.
        if state.retired {
            return Err(denied());
        }

        // The plan read, the epoch, and the signature must describe one
        // plan. This call holds no instance lock, so a `submit` can land
        // between the read and the sign -- retry once on the lock-free
        // insert-only path, then fall back to a locked repair below rather
        // than sign a mismatched pair. `NoSuchService` is caller input (an
        // authorized caller asking for a service this app does not have),
        // unlike `InconsistentPlan`, which is a compiler defect.
        let map_topology_err = |e: TopologyBuildError| match e {
            TopologyBuildError::NoSuchService(_) | TopologyBuildError::AmbiguousHash(_) => {
                RpcError::InvalidParams(e.to_string())
            }
            TopologyBuildError::InconsistentPlan(_) => RpcError::InternalError(e.to_string()),
        };
        // The supplied name is canonicalised (`resolve` accepts a logical
        // service name *or* its `short_hash`) **inside** the
        // two-attempt loop, since each attempt re-reads `state.plan_json`
        // and a `submit` landing between attempts can change the declared
        // names. `resolved_name` then replaces `service_name` at every
        // later use in this function -- the epoch key, the cache key, and
        // `TopologyDocument.service_name` -- so the document always names
        // the real service name, never the hash a caller sent. That
        // property is what lets the gateway's own check
        // (`short_hash(doc.service_name) == s_hash`) be meaningful rather
        // than tautological.
        let mut topo = None;
        let mut resolved_name = None;
        let mut epoch = 0u64;
        for _attempt in 0..2 {
            let plan = DeploymentPlan::from_json(&state.plan_json)
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
            let name =
                topology::resolve_service_name(&plan, &service_name).map_err(map_topology_err)?;
            let t = topology::service_topology(&plan, &name).map_err(map_topology_err)?;
            let fp = topology_fingerprint(t.mode, &t.members, t.sharding_strategy.as_ref());
            let (e, stored_fp) = self
                .store
                .initialise_topology_epoch(&state.app_instance_id, name.as_str(), &fp)
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
            if stored_fp == fp {
                topo = Some(t);
                resolved_name = Some(name);
                epoch = e;
                break;
            }
            // A submit landed under us. Re-read and try once more.
            state = self
                .store
                .instance_by_app_master_did(app_did.as_str())
                .map_err(|e| RpcError::InternalError(e.to_string()))?
                .ok_or_else(denied)?;
        }
        let (resolved_name, topo, epoch) = match topo {
            Some(t) => (resolved_name.unwrap_or_else(|| service_name.clone()), t, epoch),
            None => {
                // Two lock-free attempts still disagreed with the stored
                // fingerprint: either a `submit` is genuinely still in
                // flight, or an earlier `submit`'s fingerprint write never
                // landed (that write is best-effort against a durable
                // write already made, so a failure there only ever
                // surfaces as a `tracing::warn!` in `handle_submit`),
                // leaving a permanently stale row the insert-only form can
                // never correct on its own. The instance lock is an async
                // per-instance mutex never held across the store's own
                // (synchronous, short) critical section, so taking it here
                // can only ever wait behind an in-flight submit/adopt/
                // retire/force-reconcile finishing -- not deadlock one --
                // and the advancing form is safe once this call is the
                // sole writer for the instance.
                let lock = self.instance_lock(&state.app_instance_id);
                let _guard = lock.lock().await;
                state = self
                    .store
                    .instance_by_app_master_did(app_did.as_str())
                    .map_err(|e| RpcError::InternalError(e.to_string()))?
                    .ok_or_else(denied)?;
                // The lock hand-off makes this the expected ordering, not
                // a narrow race: a `retire` holding the same instance lock
                // can finish while this call waits for it, and the
                // pre-lock `retired` check above read a state from before
                // that happened.
                if state.retired {
                    return Err(denied());
                }
                let plan = DeploymentPlan::from_json(&state.plan_json)
                    .map_err(|e| RpcError::InternalError(e.to_string()))?;
                let name = topology::resolve_service_name(&plan, &service_name)
                    .map_err(map_topology_err)?;
                let t = topology::service_topology(&plan, &name).map_err(map_topology_err)?;
                let fp = topology_fingerprint(t.mode, &t.members, t.sharding_strategy.as_ref());
                let e = self
                    .store
                    .record_topology_fingerprint(&state.app_instance_id, name.as_str(), &fp)
                    .map_err(|e| RpcError::InternalError(e.to_string()))?;
                (name, t, e)
            }
        };
        let service_name = resolved_name;

        // One signature per (service, epoch), re-signed when less than
        // half the document's own validity remains. Keyed only by
        // (app_instance_id, service_name), which a handover can leave
        // pointing at a document signed by a *different* master -- the
        // instance id survives `import-master`/`adopt`, the app DID does
        // not -- so the hit condition, not the key, has to bind to the DID
        // actually being resolved. `generation` is checked for the same
        // reason: `adopt` can advance it with no membership change, so the
        // epoch alone does not prove a cached document's `generation` is
        // still current.
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let cache_key = (state.app_instance_id.clone(), service_name.to_string());
        if let Some(cached) = self.signed_documents.get(&cache_key)
            && cached.signed.document.app_did == app_did
            && cached.signed.document.generation == state.generation
            && cached.epoch == epoch
            && cached.signed.document.not_after.saturating_sub(now)
                > self.topology_document_not_after_secs / 2
        {
            return Ok(NativeResponse {
                payload: serde_json::to_value(&cached.signed).unwrap_or(Value::Null),
            });
        }

        let instance_id = AppInstanceId::try_new(state.app_instance_id.clone())
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let master = match keys::existing_app_master(&self.vault, &state.app_instance_id).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                return Err(RpcError::InternalError(format!(
                    "app instance '{}' has no app master; run `adopt`",
                    state.app_instance_id
                )));
            }
            Err(keys::VaultError::Locked) => {
                let _ = self.store.alerts.raise(
                    &instance_id,
                    None,
                    None,
                    &self.node_did,
                    AlertKind::VaultLocked,
                    &format!(
                        "'{}' cannot sign a Tier-2 topology document because this supervisor's \
                         vault is locked. Run: roymctl --substrate {} security inject-kek \
                         --kek-hex <...>",
                        state.app_instance_id, self.node_did
                    ),
                );
                return Err(RpcError::InternalError(format!(
                    "this supervisor's vault is locked; run `inject-kek` before {} can be resolved",
                    state.app_instance_id
                )));
            }
            Err(e) => return Err(RpcError::InternalError(e.to_string())),
        };
        let actual_did = syneroym_identity::substrate::derive_did_key(&master.public_key());
        if actual_did != app_did.as_str() {
            let _ = self.store.alerts.raise(
                &instance_id,
                None,
                None,
                &self.node_did,
                AlertKind::AppIdentityMismatch,
                &format!(
                    "this instance's row records app master {app_did}, but the vault's app-<id> \
                     key derives {actual_did} -- run `import-master` for the correct key, then \
                     `adopt`, before this app instance can be resolved"
                ),
            );
            return Err(RpcError::InternalError(
                "this instance's vault key does not match its recorded app master DID".to_string(),
            ));
        }
        let _ = self.store.alerts.clear(&instance_id, None, &self.node_did, AlertKind::VaultLocked);
        let _ = self.store.alerts.clear(
            &instance_id,
            None,
            &self.node_did,
            AlertKind::AppIdentityMismatch,
        );

        let document = TopologyDocument {
            app_instance_id: instance_id,
            app_did: app_did.clone(),
            service_name,
            mode: topo.mode,
            members: topo.members,
            sharding_strategy: topo.sharding_strategy,
            epoch: TopologyEpoch(epoch),
            generation: state.generation,
            issued_at: now,
            not_after: now.saturating_add(self.topology_document_not_after_secs),
            cache_ttl_ms: self.topology_document_cache_ttl_secs.saturating_mul(1_000),
        };
        let signed = document.sign(&master).map_err(|e| RpcError::InternalError(e.to_string()))?;
        self.signed_documents.insert(cache_key, CachedDocument { signed: signed.clone(), epoch });

        Ok(NativeResponse { payload: serde_json::to_value(&signed).unwrap_or(Value::Null) })
    }
}

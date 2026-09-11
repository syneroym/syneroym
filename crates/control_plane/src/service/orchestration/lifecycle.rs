use super::*;

impl ControlPlaneService {
    /// Epoch-guarded binding write (ADR-0021 §3): the only path
    /// that changes a dependent's resolution without redeploying it.
    /// Touches the binding tables and the resolver and nothing else -- no
    /// artifact work, no restart, no lifecycle hook.
    pub(super) async fn write_bindings_impl(
        &self,
        write: BindingWrite,
        caller: &CallerContext,
    ) -> Result<Vec<BindingWriteOutcomeWire>, String> {
        // Same gate `deploy_with_context` applies, for the same reason: a
        // binding write changes what a service calls, which is a
        // deploy-class change to that service, not a read.
        let deploy_resource =
            ResourceUri(format!("substrate:{}/app/{}", self.node_did, write.service_id));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{}' on this substrate",
                caller.caller_did, write.service_id
            ));
        }

        // The service must be deployed here and its recorded app context
        // must match -- without this an authorized caller could write
        // bindings into an app instance its service does not belong to,
        // the same hole `deploy`'s `binding.app_instance_id != ctx.
        // app_instance_id` check closes at deploy time.
        match self.registry.app_context_of(&write.service_id) {
            None => {
                return Err(format!("'{}' has no app context on this substrate", write.service_id));
            }
            Some((instance, _)) if instance != write.app_instance_id => {
                return Err(format!(
                    "'{}' belongs to app instance '{instance}', not '{}'",
                    write.service_id, write.app_instance_id
                ));
            }
            Some(_) => {}
        }

        // The same app-instance-owner gate `deploy_with_context` applies
        // (orchestration.rs's deploy ownership check), for the same reason:
        // `write.service_id` genuinely belonging to `write.app_instance_id`
        // (just checked above) proves the write targets its own service's
        // app, not that the caller may manage that app instance as a
        // whole. Without this, an app-scoped `orchestrator/deploy` grant on
        // one service of an instance -- not its owner, not node-wide --
        // could push a binding change that, through the shared resolver
        // entry `write-bindings` writes into, affects every other service
        // of that instance too. `check_generation` below is not a
        // substitute: it is a tiebreaker among already-authorized writers,
        // not an authorization check, and an unmanaged instance's
        // generation-0 gate now correctly accepts any authorized writer --
        // "authorized" has to be decided here, same as `deploy`.
        if let Some(existing) =
            self.registry.app_instance_management_of(&write.app_instance_id).map(|m| m.owner_did)
            && existing != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "app instance '{}' is owned by {existing}; a binding write into it must come from \
                 its owner or a substrate owner",
                write.app_instance_id
            ));
        }

        // Persisted immediately, before any binding is examined
        // -- the same rule every other gate site follows (deploy, restart,
        // undeploy, claim). A mid-validation refusal below must not leave
        // the accepting generation unrecorded.
        let management = self.check_generation(&write.app_instance_id, caller, write.generation)?;
        self.registry
            .set_app_instance_management(write.app_instance_id.clone(), management)
            .await
            .map_err(|e| e.to_string())?;

        // Validate every binding before applying any of it: `prepare_binding`
        // and the `binding_of` existence check are both pure reads, so the
        // whole list can be checked up front. Without this, a refusal partway
        // through (a malformed member DID, an undeclared dependency) would
        // leave earlier bindings already applied with no way for the caller
        // to know which ones landed -- the WIT contract's "one outcome per
        // binding, in the order sent" reads as all-or-nothing.
        let mut prepared = Vec::with_capacity(write.bindings.len());
        for binding in &write.bindings {
            let (dependency_name, entry) = prepare_binding(binding, &write.app_instance_id)?;

            // Update-only: a push may not introduce a dependency the
            // guest never declared at deploy -- a new dependency changes
            // the guest's contract and needs a redeploy, not a push.
            let held_json = self
                .registry
                .binding_of(&write.service_id, &binding.dependency_name)
                .await
                .map_err(|e| e.to_string())?;
            let Some(held_json) = held_json else {
                return Err(format!(
                    "'{}' declares no dependency '{}'; a new dependency needs a redeploy, not a \
                     binding push",
                    write.service_id, binding.dependency_name
                ));
            };
            let held: TopologyEntry = serde_json::from_str(&held_json).map_err(|e| {
                format!(
                    "stored binding for '{}' dependency '{}' is corrupt: {e}",
                    write.service_id, binding.dependency_name
                )
            })?;

            let outcome = classify_binding_write(Some(&held), &entry);
            prepared.push((binding, dependency_name, entry, outcome));
        }

        let mut outcomes = Vec::with_capacity(prepared.len());
        let mut any_applied = false;
        for (binding, dependency_name, entry, outcome) in prepared {
            if outcome == BindingWriteOutcome::Applied {
                any_applied = true;
                let entry_json = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
                self.registry
                    .save_binding(
                        &write.service_id,
                        &write.app_instance_id,
                        &binding.dependency_name,
                        &entry_json,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                // `NoOp`/`Stale`/`Conflict` write nothing. `NoOp` in
                // particular must not re-register: re-registering evicts
                // the resolver cache for an unchanged entry, turning the
                // ordinary retry into cache churn on the hot path.
                //
                // `try_new`, not `new`: `write.app_instance_id` is
                // caller-supplied wire input. Not reachable today -- the
                // `app_context_of` equality check above already guarantees
                // it equals a stored id validated at deploy -- but that is
                // a non-local invariant to depend on for a panic, and
                // `prepare_binding`'s own doc calls out this exact hazard.
                let app_instance_id =
                    AppInstanceId::try_new(&write.app_instance_id).map_err(|e| e.to_string())?;
                self.logical_resolver
                    .register(TopologyKey::local(app_instance_id, dependency_name), entry);
            }
            outcomes.push(wire_binding_outcome(&outcome));
        }

        // The deploy dedup key hashes what a deploy *sends*, not what is
        // currently installed, so it cannot see a push that happened since
        // the last deploy. Without this, a repair redeploy of byte-identical
        // content after a push would match the stale hash and take the
        // no-op path, silently leaving the pushed (not the redeployed)
        // bindings in place -- exactly the "restart is the cheap path,
        // deploy is the repair path" case. Clearing the hash here forces
        // that redeploy through the full reinstall instead.
        if any_applied
            && let Some((service_type, health_check_json, _, visibility)) =
                self.registry.deploy_facts(&write.service_id)
        {
            self.registry
                .set_deploy_facts(
                    write.service_id.clone(),
                    service_type,
                    health_check_json,
                    None,
                    visibility,
                )
                .await
                .map_err(|e| e.to_string())?;
        }

        Ok(outcomes)
    }

    /// Gates on ownership before tearing anything down
    /// -- a non-owner undeploying someone else's service is the same
    /// escalation as taking it over via redeploy. Checks
    /// `ORCHESTRATOR_UNDEPLOY` specifically (see
    /// `has_node_wide_ability`'s doc comment): a status-only grantee must
    /// not be able to undeploy someone else's app.
    ///
    /// Safe to call from `deploy`'s own rollback path: at that point
    /// `owner_of` is one of (a) `None` (the native-capability-registration
    /// failure path, reached before `set_owner` ever ran), (b) already
    /// `caller.caller_did` (the happy-path retry: this same `deploy` call
    /// already ran `set_owner` successfully once, or this is an ordinary
    /// owner re-deploying their own service), or (c) a *different* DID that
    /// `caller` is redeploying over while holding node-wide authority -- in
    /// which case this gate passes via that authority, not because the row
    /// matches `caller.caller_did`. All three pass; there is no branch where
    /// `deploy`'s own rollback gets rejected by this check.
    ///
    /// Renamed `undeploy_impl` (from `undeploy`) so the trait's own
    /// `undeploy` -- a thin wrapper -- can call it without recursing; the
    /// split mirrors `deploy`/`deploy_with_context` immediately above.
    pub(super) async fn undeploy_impl(
        &self,
        service_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // Same reason and same placement as `deploy_with_context`'s
        // equivalent check: `service_id` is joined verbatim into
        // `hosted_apps_dir/<service_id>.json` below and then deleted,
        // before anything else runs against it.
        if !is_safe_service_id_for_path(&service_id) {
            return Err(format!(
                "service_id '{service_id}' is not a valid undeploy target: it must be non-empty \
                 and contain no '/', '\\\\', or '..' -- it is joined into a stored-record filename"
            ));
        }
        if let Some(owner) = self.registry.owner_of(&service_id)
            && owner != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_UNDEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {owner}; only its owner or a substrate owner \
                 may undeploy it"
            ));
        }

        // Tier-1 undeploy admission, the same shape
        // as `deploy`'s -- the caller must hold `orchestrator/undeploy`
        // covering this app.
        //
        // Interaction with `deploy`'s own rollback path: `deploy`
        // calls `self.undeploy(service_id.clone(), caller)` with the *same*
        // `caller` on two failure paths. Abilities are deliberately flat
        // and independently grantable, so "deploy but not
        // undeploy" is a real, supported shape -- a deploy-only grantee
        // (`roymctl identity issue-grant --can orchestrator/deploy`, no
        // `orchestrator/undeploy`) whose deploy fails partway would be
        // rejected *again* by this check on the rollback attempt, on a
        // confusing second error. This was inert before anything could
        // create a `ControllerAgreement`, when every substrate was unowned
        // and every verified caller held all three abilities together for
        // free -- now that `ControllerAgreement`, and so real app-scoped
        // grants, are live: a grant meant to let its holder
        // deploy reliably should include `orchestrator/undeploy` alongside
        // `orchestrator/deploy` so a failed deploy can clean up after
        // itself. `deploy_grant.rs` documents the partial-grant shapes;
        // this comment records the specific rollback interaction so a
        // future grant-issuing tool does not reintroduce it silently.
        let undeploy_resource =
            ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller.has_capability(
            &undeploy_resource,
            &Ability(Ability::ORCHESTRATOR_UNDEPLOY.to_string()),
        ) {
            return Err(format!(
                "caller {} holds no orchestrator/undeploy grant for '{service_id}' on this \
                 substrate",
                caller.caller_did
            ));
        }

        // `undeploy` is a lifecycle action, gated the same
        // as `deploy`/`restart` -- a superseded supervisor must not be
        // able to tear down services it no longer manages. Ungated for a
        // standalone service with no app context, same as `restart`.
        if let Some((instance, _)) = self.registry.app_context_of(&service_id) {
            let management = self.check_generation(&instance, caller, generation)?;
            self.registry
                .set_app_instance_management(instance, management)
                .await
                .map_err(|e| e.to_string())?;
        }

        info!("Undeploying service: {}", service_id);

        let cert_path = self.hosted_apps_dir.join(format!("{service_id}.json"));
        if cert_path.exists()
            && let Err(e) = fs::remove_file(&cert_path)
        {
            tracing::warn!("Failed to remove registry certificate for {}: {}", service_id, e);
        }

        let endpoints = self.registry.lookup_by_service(&service_id);
        let mut is_wasm = false;
        let mut is_container = false;

        for (interface_name, endpoint) in endpoints {
            if matches!(endpoint, SubstrateEndpoint::WasmChannel { .. }) {
                is_wasm = true;
            } else if matches!(endpoint, SubstrateEndpoint::TcpHostPort { .. }) {
                is_container = true;
            }
            if let Err(e) = self.registry.remove(&service_id, &interface_name).await {
                tracing::warn!(
                    "Failed to remove endpoint {} for service {}: {}",
                    interface_name,
                    service_id,
                    e
                );
            }
        }

        if is_wasm {
            if let Err(e) = self.app_sandbox_engine.stop_wasm(&service_id).await {
                tracing::warn!("Failed to stop WASM engine for service {}: {}", service_id, e);
            }
            if let Err(e) = self.app_sandbox_engine.remove_wasm(&service_id).await {
                tracing::warn!("Failed to remove WASM file for service {}: {}", service_id, e);
            }
        }

        if is_container {
            if let Err(e) = self.podman_sandbox_engine.stop(&service_id).await {
                tracing::warn!("Failed to stop Container engine for service {}: {}", service_id, e);
            }
            if let Err(e) = self.podman_sandbox_engine.remove(&service_id).await {
                tracing::warn!("Failed to remove Container for service {}: {}", service_id, e);
            }
        }

        // Messaging subscriptions have no analogue among the other 4 native
        // capabilities: they're a long-lived stateful subsystem (persisted
        // rows plus live broker registrations), not pure request/response,
        // so they need an explicit "forget this service" step the
        // endpoint-registry loop above doesn't cover.
        if let Err(e) =
            self.storage_provider.delete_all_messaging_subscriptions_for_service(&service_id).await
        {
            tracing::warn!(
                "Failed to remove messaging subscriptions for service {}: {}",
                service_id,
                e
            );
        }
        if is_wasm {
            self.app_sandbox_engine.unsubscribe_all(&service_id);
            self.app_sandbox_engine.forget_guest_http_permits(&service_id);
            self.app_sandbox_engine.forget_websocket_senders(&service_id);
        }
        self.sse_permits.remove(&service_id);

        // An `fdae_policies` row has no in-memory analogue that gets torn
        // down for free elsewhere in this function -- `stop_wasm` above only
        // evicts the WASM engine's *cache* of it, and native dispatch's copy
        // dies with the `SynSvcNativeService` removed below. Without this, a
        // later re-deploy of the same `service_id` with no `fdae` block
        // would still have `AppSandboxEngine::resolve_fdae_policy` resurrect
        // this row from storage on its next cache miss.
        if let Err(e) = self.storage_provider.delete_fdae_policy(&service_id).await {
            tracing::warn!("Failed to remove FDAE policy for service {}: {}", service_id, e);
        }

        // The endpoint-registry loop above already removed the 6 native
        // capability interfaces generically (it iterates every registered
        // interface for this service_id); just drop the in-memory dispatch
        // entry too.
        if let Some(native_dispatch) = self.native_dispatch.upgrade() {
            native_dispatch.remove(&service_id);
        } else {
            tracing::error!(
                "Native dispatch registry unavailable while undeploying service {}: its in-memory \
                 dispatch entry, if any, was left behind",
                service_id
            );
        }
        self.http_routes.remove(&service_id);
        self.full_deploy_completed.remove(&service_id);

        // Nothing survives an undeploy, so there is nothing to
        // keep -- unlike the deploy-time forward cleanup, which diffs
        // against a still-live new generation.
        if let Some((_, old)) = self.assets.remove(&service_id) {
            let remove = assets::hashes_of(&old.manifest, Some(&old.manifest_hash));
            if let Err(e) =
                assets::delete_hashes(&service_id, &remove, &BTreeSet::new(), &self.blob_provider)
                    .await
            {
                tracing::warn!(
                    "Failed to remove asset bundle blobs for service {}: {}",
                    service_id,
                    e
                );
            }
        }

        // Warn-not-fail, matching every other teardown step above (endpoints,
        // subscriptions, http_routes are all best-effort).
        if let Err(e) = self.registry.remove_owner(&service_id).await {
            tracing::warn!("Failed to remove owner record for service {}: {}", service_id, e);
        }
        if let Err(e) = self.registry.remove_instance_cert(&service_id).await {
            tracing::warn!(
                "Failed to remove instance certificate for service {}: {}",
                service_id,
                e
            );
        }
        if let Err(e) = self.registry.remove_deploy_facts(&service_id).await {
            tracing::warn!("Failed to remove deploy facts for {}: {}", service_id, e);
        }
        self.probe_cache.remove(&service_id);
        // Persisted rows only -- the in-memory `StaticInventory` entry
        // stays. A `TopologyEntry` is an app-scoped fact ("where
        // does `backend` live in instance X"), not a per-dependent one;
        // removing it when one of several dependents goes away would break
        // the others.
        //
        // Same call, same reasoning, on `install_app_context`'s redeploy
        // path: a redeploy that drops a dependency from its manifest calls
        // this exact method, and the entry it wrote into `StaticInventory`
        // stays too -- decided here explicitly, not inherited by accident,
        // because the "app-scoped, not per-dependent" argument above holds
        // just as much for a redeploy that stops declaring a dependency as
        // it does for an undeploy that removes the dependent entirely. The
        // two do diverge across a restart: `replay_persisted_bindings`
        // rebuilds `StaticInventory` from `service_bindings` alone, so an
        // entry no longer backed by any persisted row silently drops out on
        // restart even though it kept resolving right up to that point.
        // That is `StaticInventory`'s memory-vs-storage split working as
        // designed (deferred-backlog.md), not a new gap this call opens.
        let app_instance_id =
            self.registry.app_context_of(&service_id).map(|(instance, _)| instance);
        if let Err(e) = self.registry.remove_app_context(&service_id).await {
            tracing::warn!("Failed to remove app context for service {}: {}", service_id, e);
        }

        // Without this, `app_instance_owners` rows never get forgotten.
        // Once no service on this node names the
        // instance any more, its management row is dead weight and its id
        // can never be reclaimed by another caller without this.
        if let Some(instance_id) = app_instance_id
            && self.registry.app_context_of_any(&instance_id).is_none()
            && let Err(e) = self.registry.remove_app_instance_management(&instance_id).await
        {
            tracing::warn!("Failed to remove app instance management for {}: {}", instance_id, e);
        }

        Ok(())
    }

    /// Restart a deployed service in place (ADR-0021 §4's
    /// "lifecycle actions"). Type-dispatched off `service_deploy_facts`
    /// recorded at deploy -- a `tcp` service's process runs outside
    /// this substrate and there is nothing here to restart, so it is
    /// refused rather than silently succeeding, which a supervisor's
    /// remediation budget would otherwise count as a real attempt.
    pub(super) async fn restart_impl(
        &self,
        service_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // Same gate as `deploy`: a restart is a lifecycle write.
        let deploy_resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{service_id}' on this substrate",
                caller.caller_did
            ));
        }

        // `deploy`/`undeploy`/`write-bindings` all refuse a
        // takeover of a service a different caller owns; `restart` was the
        // one lifecycle write missing this check. A node-wide grantee (the
        // same override `undeploy_impl` honours) skips it for free.
        if let Some(owner) = self.registry.owner_of(&service_id)
            && owner != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {owner}; only its owner or a substrate owner \
                 may restart it"
            ));
        }

        // Generation gate, only where an app instance exists --
        // ungated for a standalone service, same as `undeploy`.
        if let Some((instance, _)) = self.registry.app_context_of(&service_id) {
            let management = self.check_generation(&instance, caller, generation)?;
            self.registry
                .set_app_instance_management(instance, management)
                .await
                .map_err(|e| e.to_string())?;
        }

        let Some((recorded_type, ..)) = self.registry.deploy_facts(&service_id) else {
            return Err(format!(
                "no service type recorded for '{service_id}'; redeploy to record it"
            ));
        };
        match parse_service_type(&recorded_type) {
            Some(AppServiceType::Wasm) => {
                self.app_sandbox_engine.reload_wasm(&service_id).await.map_err(|e| e.to_string())
            }
            Some(AppServiceType::Container) => {
                self.podman_sandbox_engine.stop(&service_id).await.map_err(|e| e.to_string())?;
                self.podman_sandbox_engine.start(&service_id).await.map_err(|e| e.to_string())
            }
            Some(AppServiceType::Tcp) => Err(format!(
                "'{service_id}' is a tcp service; its process runs outside this substrate and \
                 cannot be restarted here"
            )),
            Some(AppServiceType::NativeHost) => {
                Err(format!("'{service_id}' is a native-host service and has no restart path"))
            }
            None => Err(format!(
                "'{service_id}' has a recorded service type ('{recorded_type}') this substrate \
                 does not recognize; redeploy to correct it"
            )),
        }
    }

    /// Run one scheduled tick (ADR-0023 §3/§6): dispatch
    /// `interface`/`method` on `service_id` through the local `ServiceProxy`,
    /// as `CallerContext::service_system(service_id)` -- the service acting
    /// as itself, not the supervisor calling it directly. Gated
    /// exactly as `restart_impl`: this is a lifecycle write, not a service
    /// call, so `orchestrator/deploy` decides it, not the target interface's
    /// own authorization.
    pub(super) async fn run_scheduled_impl(
        &self,
        service_id: String,
        generation: u64,
        interface: String,
        method: String,
        params_json: Option<String>,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // Same gate as `restart`: a scheduled run is a lifecycle write.
        let deploy_resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{service_id}' on this substrate",
                caller.caller_did
            ));
        }

        // Same owner check `restart_impl` carries, and for the same reason.
        if let Some(owner) = self.registry.owner_of(&service_id)
            && owner != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {owner}; only its owner or a substrate owner \
                 may run a scheduled task on it"
            ));
        }

        // Generation gate, only where an app instance exists -- the same
        // rule `restart_impl` follows, so a superseded supervisor cannot
        // keep firing ticks at an instance another one now manages.
        if let Some((instance, _)) = self.registry.app_context_of(&service_id) {
            let management = self.check_generation(&instance, caller, generation)?;
            self.registry
                .set_app_instance_management(instance, management)
                .await
                .map_err(|e| e.to_string())?;
        }

        let params = match params_json {
            Some(text) => {
                serde_json::from_str(&text).map_err(|e| format!("params-json is not JSON: {e}"))?
            }
            // An empty positional array, not `Value::Null` -- the shape the
            // one existing in-tree caller of a no-argument guest method
            // sends (the `rpc` readiness probe).
            None => Value::Array(vec![]),
        };

        // The fourth gate, the counterpart of `restart_impl`'s "no service
        // type recorded" refusal, and for a sharper reason.
        // `ProxyRouter::invoke_inner` reads a miss in the local endpoint
        // registry as "the target lives somewhere else" and resolves it
        // through the community registry instead -- so without this, a
        // schedule naming a service this node does not host, or an
        // interface the deployed component does not export, turns into an
        // outbound call. That call carries this node's own key (the proxy
        // has no instance certificate to present for a service with no
        // local instance), and neither the owner check nor the generation
        // check above can see a service this node knows nothing about:
        // both are `if let Some`. The WIT contract for this verb says this
        // node only executes, so refusing here enforces what is already
        // written. The condition is exactly `invoke_inner`'s own
        // local-or-remote test, so this refuses when, and only when, the
        // call would otherwise leave the node.
        if self.registry.lookup(&service_id, &interface).is_none() {
            return Err(format!(
                "'{service_id}' has no local endpoint for interface '{interface}'; a scheduled \
                 run executes on the node that hosts the service and is never forwarded"
            ));
        }

        let proxy = self
            .current_service_proxy()
            .upgrade()
            .ok_or_else(|| "service proxy unavailable for a scheduled run".to_string())?;
        proxy
            .invoke(ProxyRequest {
                target_service: service_id.clone(),
                interface,
                method,
                params,
                caller: CallerContext::service_system(&service_id),
                origin: CallOrigin::Native { service_id: Some(service_id) },
                protocol: ProxyProtocol::JsonRpcV1,
                // A tick is not safe to repeat by default, and therefore
                // never fenced or replayed -- and never queued (ADR-0023 §3):
                // the caller's next tick is the retry.
                idempotent: false,
                idempotency_key: None,
                // The proxy's own default; the guest's epoch budget is the
                // real ceiling on how long this can run.
                timeout: None,
            })
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

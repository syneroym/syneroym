use super::*;

impl SupervisorService {
    /// Reads the held generation across every given client and claims
    /// `held + 1` on each. Split out of `handle_adopt` so that function
    /// can close every client it opened however this returns, success or
    /// failure -- `?` inside either loop here used to return straight out
    /// of `handle_adopt` itself, leaking
    /// every client already connected and every one still left to try.
    pub(in crate::service) async fn claim_next_generation(
        app_instance_id: &str,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> RpcResult<u64> {
        let mut held_max = 0u64;
        for client in clients.values() {
            if let Some(g) =
                client.held_generation(app_instance_id).await.map_err(RpcError::InternalError)?
            {
                held_max = held_max.max(g);
            }
        }
        let next_generation = held_max + 1;

        for client in clients.values() {
            client
                .request(
                    "orchestrator",
                    "claim-app-instance",
                    serde_json::to_value((app_instance_id.to_string(), next_generation))
                        .map_err(|e| RpcError::InternalError(e.to_string()))?,
                )
                .await
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
        }
        Ok(next_generation)
    }

    pub(in crate::service) async fn handle_adopt(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse adopt params: {e}")))?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;

        let state = self
            .store
            .get(&app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(|| {
                RpcError::InternalError(format!(
                    "no desired state submitted for app instance '{app_instance_id}'; run \
                     `supervisor submit` first"
                ))
            })?;

        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let inventory: SupervisorInventory = serde_json::from_str(&state.inventory_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        // Resolved or minted before any substrate connection is opened --
        // same ordering `submit`'s own mint uses
        // ("a locked vault or a bad plan must fail before anything is
        // persisted or a network round trip spent"). A locked vault fails
        // the whole call here, before `claim_next_generation` burns a
        // generation, through the ordinary `VaultError::Locked` message
        // (which already names `inject-kek`) rather than a `kek_is_loaded`
        // pre-check -- that check answers `false` on a working vault
        // whenever `storage.encryption = false`.
        let (app_master_did, app_master_vault_name) =
            keys::app_master(&self.vault, &app_instance_id)
                .await
                .map_err(|e| RpcError::InternalError(e.to_string()))?;

        let aliases = Self::placed_aliases(&plan).map_err(RpcError::InternalError)?;
        let clients =
            self.build_clients(&aliases, &inventory).await.map_err(RpcError::InternalError)?;

        let result = Self::claim_next_generation(&app_instance_id, &clients).await;
        Self::shutdown_clients(clients.into_values()).await;
        let next_generation = result?;

        // `adopt` is the way back in from `retired` -- the message every
        // refusal on a retired instance points to. Idempotent when the
        // instance was never retired.
        //
        // The generation, the un-retired flag, and the resolved app
        // master DID land in one combined store write rather than three
        // separate ones -- a crash between them used to be able to leave a
        // claimed generation with no recorded app master, breaking the
        // invariant that the row always agrees with the vault.
        // The DID is written *after* the claim succeeds, deliberately
        // asymmetric with the mint above, which runs before it: a vault
        // key with no row is recoverable (the next `adopt` resolves the
        // same key), while a row naming a DID whose key was never stored
        // is not. Written on every successful `adopt`, not only the one
        // that minted, so the row always agrees with whatever the vault
        // holds -- this is what makes `import-master` followed by `adopt`
        // correct after a handover.
        self.store
            .record_adopt(&app_instance_id, next_generation, &app_master_did)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        // A fresh generation is a fresh start, so a terminal
        // `InstanceNotRunning` service -- one nothing will ever
        // restart again on its own -- becomes escapable here. Stays a
        // separate, best-effort call (unlike the combined write above):
        // its own failure has never blocked `adopt` from succeeding.
        let _ = self.store.clear_remediation_for_instance(&app_instance_id);
        // Same fresh-start reasoning, applied to what this supervisor has
        // *done* about each schedule. The watermark is kept, not cleared:
        // see `clear_schedule_state_for_instance` for why dropping it
        // would swallow a tick that was legitimately due.
        let _ = self.store.clear_schedule_state_for_instance(&app_instance_id);

        let result = AdoptResult {
            generation: next_generation,
            app_master_did,
            vault_name: app_master_vault_name,
        };
        Ok(NativeResponse { payload: serde_json::to_value(result).unwrap_or(Value::Null) })
    }

    /// Shared by `release` and `retire`: clears the management stamp on
    /// every substrate the instance is placed on that can actually be
    /// reached, and returns the `(alias, reason)` of every one that
    /// could not be -- reachable or not, `release`/`retire` still act on
    /// what they can.
    pub(in crate::service) async fn release_on_every_substrate(
        &self,
        app_instance_id: &str,
    ) -> RpcResult<Vec<(String, String)>> {
        let state = self
            .store
            .get(app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(|| {
                RpcError::InternalError(format!(
                    "no desired state submitted for app instance '{app_instance_id}'"
                ))
            })?;
        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let inventory: SupervisorInventory = serde_json::from_str(&state.inventory_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let aliases = Self::placed_aliases(&plan).map_err(RpcError::InternalError)?;
        let (clients, mut failed) = self.connect_best_effort(&aliases, &inventory).await;

        let params = serde_json::to_value((app_instance_id.to_string(), state.generation))
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        for (alias, client) in &clients {
            if let Err(e) =
                client.request("orchestrator", "release-app-instance", params.clone()).await
            {
                failed.push((alias.to_string(), e.to_string()));
            }
        }
        Self::shutdown_clients(clients.into_values()).await;
        Ok(failed)
    }

    /// A JSON payload reporting which, if any, placed substrates could not
    /// be released -- `unreleased_substrates` is present and non-empty
    /// only then, so an operator (and `roymctl`'s own printout) can tell a
    /// clean release from a partial one without parsing prose.
    pub(in crate::service) fn release_payload(
        status: &str,
        failed: Vec<(String, String)>,
    ) -> Value {
        if failed.is_empty() {
            return serde_json::json!({"status": status});
        }
        serde_json::json!({
            "status": status,
            "warning": "one or more placed substrates could not be reached; their generation \
                        stamp was not cleared and must be released once they are reachable \
                        again",
            "unreleased_substrates": failed
                .into_iter()
                .map(|(alias, reason)| serde_json::json!({"alias": alias, "reason": reason}))
                .collect::<Vec<_>>(),
        })
    }

    pub(in crate::service) async fn handle_release(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse release params: {e}")))?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;
        let failed = self.release_on_every_substrate(&app_instance_id).await?;
        Ok(NativeResponse { payload: Self::release_payload("released", failed) })
    }

    /// `retire` withdraws nothing from the registry: a retired instance
    /// drops out of `all_active`, so its Tier-1 record (if any) simply
    /// stops refreshing and lapses on the registry's own TTL/`not_after`,
    /// the same self-limiting decay a pause causes. Left implicit rather
    /// than an explicit withdraw, since nothing else in this slice
    /// withdraws a record early either.
    pub(in crate::service) async fn handle_retire(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse retire params: {e}")))?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;
        let failed = self.release_on_every_substrate(&app_instance_id).await?;
        // Retiring must not be blocked by a substrate that happens to be
        // down right now -- exactly the state an operator is most likely
        // to be retiring around (S7). The supervisor's own store always
        // stops managing the instance; an unreachable substrate keeps its
        // stale stamp, reported above, until it comes back and is
        // released.
        self.store.retire(&app_instance_id).map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse { payload: Self::release_payload("retired", failed) })
    }

    pub(in crate::service) async fn handle_pause(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse pause params: {e}")))?;
        self.store.pause(&app_instance_id).map_err(|e| RpcError::InternalError(e.to_string()))?;
        let mut payload = serde_json::json!({"status": "paused"});
        // A paused instance gets zero write-phase work (D-A5c-1), so its
        // Tier-1 registry record stops refreshing along with everything
        // else -- `pause`'s own promise ("stops reconciliation and
        // nothing else") does not cover this, since the record decays
        // toward `not_after` on the clock, not on a reconcile. Rather than
        // reopen that promise, the cost is made visible here, at the
        // moment an operator chooses it. The refresh fact is keyed by the
        // app master DID, not the instance id (a handover must not inherit
        // a stale stamp) -- an instance with no DID on its row yet, or one
        // never published, has nothing to warn about.
        let app_master_did =
            self.store.get(&app_instance_id).ok().flatten().map(|s| s.app_master_did);
        if let Some(app_master_did) = app_master_did.filter(|did| !did.is_empty())
            && let Ok(Some(last)) = self.store.last_tier1_refresh(&app_master_did)
        {
            let expires_at = (last as u64).saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);
            tracing::warn!(
                app_instance_id,
                expires_at,
                "pausing this instance stops its Tier-1 registry record from refreshing; callers \
                 outside it will stop being able to discover its supervisor once it passes this \
                 Unix time, unless the instance is resumed before then"
            );
            payload["app_record_expires_at"] = serde_json::json!(expires_at);
        }
        Ok(NativeResponse { payload })
    }

    pub(in crate::service) async fn handle_resume(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse resume params: {e}")))?;
        self.store.resume(&app_instance_id).map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse { payload: serde_json::json!({"status": "resumed"}) })
    }

    pub(in crate::service) async fn handle_force_reconcile(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse force-reconcile params: {e}"))
        })?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;
        let state = self
            .store
            .get(&app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
            .ok_or_else(|| {
                RpcError::InternalError(format!(
                    "no desired state submitted for app instance '{app_instance_id}'"
                ))
            })?;
        // Unlike `submit`, this path never calls `store.submit`, so nothing
        // else on it would ever refuse a retired instance -- it would just
        // redeploy every service indefinitely.
        if state.retired {
            return Err(RpcError::InternalError(format!(
                "app instance '{app_instance_id}' is retired; run `supervisor adopt` to resume \
                 managing it before reconciling"
            )));
        }
        let plan = DeploymentPlan::from_json(&state.plan_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let inventory: SupervisorInventory = serde_json::from_str(&state.inventory_json)
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        // `force-reconcile` never calls `store.submit`, so nothing else on
        // this path checks placement either -- the identical reasoning as
        // the `retired` check above.
        self.refuse_placement_change(&plan, &inventory).await.map_err(RpcError::InternalError)?;
        // The same replica-cap re-check `submit` runs -- a desired-state
        // row written before this check existed must not get a permanent
        // pass.
        Self::refuse_replicas_above_cap(&plan).map_err(RpcError::InternalError)?;
        Self::refuse_unrunnable_schedules(&plan).map_err(RpcError::InternalError)?;
        Self::refuse_unshardable_plan(&plan).map_err(RpcError::InternalError)?;
        // A directed reconcile is a fresh start, regardless of what this
        // call's own outcome turns out to be --
        // a terminal `InstanceNotRunning` service is otherwise never
        // restarted again, so the loop's own healthy-sweep clearing path
        // never fires for it.
        let _ = self.store.clear_remediation_for_instance(&app_instance_id);
        let _ = self.store.clear_schedule_state_for_instance(&app_instance_id);
        self.deploy_submission(plan, &inventory, state.generation)
            .await
            .map_err(RpcError::InternalError)?;
        Ok(NativeResponse { payload: serde_json::json!({"status": "reconciled"}) })
    }
}

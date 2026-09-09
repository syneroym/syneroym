use super::*;

impl SupervisorService {
    /// Refuses a submission whose plan would move an already-landed
    /// service to a different substrate than the journal shows it running
    /// on. An early version of `submit` shipped with no such refusal at
    /// all -- `roymctl`'s own `check_no_placement_change` is private to
    /// that binary and reads a local identity file the supervisor cannot
    /// see, so this is the supervisor's own check, not a reuse. Without
    /// it, a re-submit that changes an alias silently deploys a second
    /// live copy of the same member: the two-publisher state another
    /// refusal exists to prevent, reachable here because nothing on this
    /// path called it.
    ///
    /// Reads only what this supervisor's own journal has recorded landed
    /// -- never `roymctl`'s `--dir` -- so it is safe to call from both
    /// `submit` and `force-reconcile`.
    ///
    /// A refusal here raises `AlertKind::PlacementChangeRefused` -- the
    /// variant existed, tested only in its own `Display`/`FromStr` round
    /// trip, with nothing in either caller ever raising it. Raised (and
    /// published, same as every other alert this file opens) before the
    /// refusal is returned, so a refused submission is visible on `alerts`
    /// even though it is otherwise indistinguishable from a plain RPC
    /// error to whatever received it.
    /// `SynAppManifest::validate()` enforces `MAX_REPLICAS` at
    /// compile time, but `submit`/`force-reconcile` take an already-
    /// compiled `DeploymentPlan` straight as JSON -- nothing between the
    /// compiler and here re-checks it, so a submitted plan can carry an
    /// arbitrary member count for one logical service, each one a minted
    /// vault key, a certificate, a deploy call, and a journal row.
    /// Admin-gated, so this is not a privilege boundary, but the cap's own
    /// reason ("a bound set before the first measurement can never fail")
    /// does not hold if the interface that actually accepts the plan
    /// never enforces it.
    pub(super) fn refuse_replicas_above_cap(plan: &DeploymentPlan) -> Result<(), String> {
        let mut counts: BTreeMap<&LogicalServiceRef, u32> = BTreeMap::new();
        for svc in &plan.services {
            *counts.entry(&svc.logical_ref).or_insert(0) += 1;
        }
        if let Some((l_ref, count)) = counts.into_iter().find(|(_, count)| *count > MAX_REPLICAS) {
            return Err(format!(
                "'{l_ref}' names {count} members in this plan, above the cap of {MAX_REPLICAS}"
            ));
        }
        Ok(())
    }

    /// `refuse_replicas_above_cap`'s sibling, same reason
    /// and same two call sites: `SynAppManifest::validate()` enforces both
    /// of these rules at compile time, but `submit`/`force-reconcile` take
    /// an already-compiled plan, which nothing between the compiler and
    /// here re-checks -- so a hand-edited plan reaches the supervisor with
    /// neither rule applied.
    ///
    /// The cap counts distinct `logical_ref`s carrying a schedule, not
    /// members -- a schedule belongs to the logical service, so a scaled
    /// service with one schedule counts once, not once per member.
    ///
    /// The budget is checked at both ends, and the zero end is the reason
    /// this rule is here rather than left to the runtime clamp: the clamp
    /// is a `min`, so a zero survives it, and
    /// `tokio::time::timeout(Duration::ZERO, ..)` elapses before the call
    /// starts -- after `record_schedule_started` has already written the
    /// watermark. The tick is consumed, an alert is raised, and every
    /// later tick repeats the cycle, forever.
    ///
    /// Refusing the whole submission is right for these two and wrong for
    /// an unparseable cron, which is why the cron is deliberately not
    /// re-validated here: a bad cron degrades to `schedule_decisions`'
    /// watermark branch, which skips that one schedule and leaves the rest
    /// of the instance reconciling. A budget no run can finish in has no
    /// such graceful form -- there is nothing to degrade to.
    pub(super) fn refuse_unrunnable_schedules(plan: &DeploymentPlan) -> Result<(), String> {
        let scheduled: BTreeSet<&LogicalServiceRef> =
            plan.services.iter().filter(|s| s.schedule.is_some()).map(|s| &s.logical_ref).collect();
        if scheduled.len() > MAX_SCHEDULED_SERVICES {
            return Err(format!(
                "{} services declare a schedule in this plan, above the cap of \
                 {MAX_SCHEDULED_SERVICES}",
                scheduled.len()
            ));
        }
        for svc in &plan.services {
            let Some(sched) = &svc.schedule else { continue };
            if sched.timeout_ms == 0 || sched.timeout_ms > MAX_SCHEDULE_TIMEOUT_MS {
                return Err(format!(
                    "'{}' declares a schedule timeout of {}ms in this plan; it must be between 1 \
                     and {MAX_SCHEDULE_TIMEOUT_MS}ms",
                    svc.logical_ref, sched.timeout_ms
                ));
            }
        }
        Ok(())
    }

    /// The manifest's two `sharding_strategy` rules, re-applied to an
    /// already-compiled plan. `SynAppManifest::validate` enforces both at
    /// compile time, and nothing between the compiler and here re-checks
    /// them --
    /// the exact gap the two functions above exist to close, now with a
    /// sharper consequence: a strategy that reaches this supervisor goes
    /// into a *signed* Tier-2 document (ADR-0022 §3), where a reader acts
    /// on it against member ids the plan's author chose.
    pub(super) fn refuse_unshardable_plan(plan: &DeploymentPlan) -> Result<(), String> {
        let mut counts: BTreeMap<&LogicalServiceRef, u32> = BTreeMap::new();
        for svc in &plan.services {
            *counts.entry(&svc.logical_ref).or_insert(0) += 1;
        }
        for svc in &plan.services {
            let Some(strategy) = &svc.sharding_strategy else { continue };
            if matches!(strategy, ShardingStrategy::RangeSharding(_)) {
                return Err(format!(
                    "'{}' declares a range_sharding strategy in this plan; range sharding names \
                     concrete members by ServiceId, which is reachable only once shard \
                     rebalancing assigns them",
                    svc.logical_ref
                ));
            }
            if counts.get(&svc.logical_ref).copied().unwrap_or(0) <= 1 {
                return Err(format!(
                    "'{}' declares a sharding_strategy with one member in this plan; a strategy \
                     over one member is not a selection",
                    svc.logical_ref
                ));
            }
        }
        Ok(())
    }

    pub(super) async fn refuse_placement_change(
        &self,
        plan: &DeploymentPlan,
        inventory: &SupervisorInventory,
    ) -> Result<(), String> {
        let instance_id =
            AppInstanceId::try_new(plan.app_instance_id.to_string()).map_err(|e| e.to_string())?;
        let landed = self
            .store
            .journal
            .get_completed_actions_for_instance(&instance_id)
            .map_err(|e| e.to_string())?;
        for svc in &plan.services {
            let l_ref = svc.member_ref().to_string();
            let Some(prev) = deploy::current_placement(&landed, &l_ref) else { continue };
            let Some(alias) = &svc.substrate else { continue };
            let Some(entry) = inventory.get(alias.as_str()) else { continue };
            if prev.substrate_did != entry.did {
                let detail = format!(
                    "service '{l_ref}' is already deployed on substrate {} and this submission \
                     would place it on {} ('{alias}'); the supervisor does not relocate a running \
                     member -- undeploy it on the old substrate and clear its placement record \
                     (`roymctl app forget`) before resubmitting",
                    prev.substrate_did, entry.did
                );
                if let Ok(true) = self.store.alerts.raise(
                    &instance_id,
                    Some(&l_ref),
                    prev.substrate_alias.as_deref(),
                    &prev.substrate_did,
                    AlertKind::PlacementChangeRefused,
                    &detail,
                ) {
                    self.publish_opened_alerts(
                        &plan.app_instance_id.to_string(),
                        &[(AlertKind::PlacementChangeRefused, l_ref)],
                    )
                    .await;
                }
                return Err(detail);
            }
        }
        Ok(())
    }

    pub(super) async fn handle_submit(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (s,): (Submission,) = serde_json::from_value(params)
            .map_err(|e| RpcError::InvalidParams(format!("failed to parse submit params: {e}")))?;
        // Held for the whole call, so a loop pass or another operator
        // write for this same instance cannot interleave with it -- a
        // read-then-write race otherwise.
        let lock = self.instance_lock(&s.app_instance_id);
        let _guard = lock.lock().await;

        let plan = DeploymentPlan::from_json(&s.plan_json)
            .map_err(|e| RpcError::InvalidParams(format!("invalid plan-json: {e}")))?;
        let inventory: SupervisorInventory = serde_json::from_str(&s.inventory_json)
            .map_err(|e| RpcError::InvalidParams(format!("invalid inventory-json: {e}")))?;

        // `deploy_submission`, the journal, and the vault key all derive
        // from `plan.app_instance_id`; the desired-state row and every
        // later `adopt`/`status`/`retire` key on `s.app_instance_id`
        // instead. A mismatch (both fields are caller-supplied) would
        // split the instance in two -- `status` querying the journal under
        // a key nothing wrote, `adopt` stamping a generation the substrate
        // never associates with the deployed services.
        if plan.app_instance_id.as_str() != s.app_instance_id {
            return Err(RpcError::InvalidParams(format!(
                "submission names app instance '{}' but its plan-json is compiled for '{}'",
                s.app_instance_id, plan.app_instance_id
            )));
        }

        // Checked before any deploy work runs, not only after:
        // `store.submit`'s own guards, below, live past the whole
        // mint/certify/apply pipeline. For `retired` that used to mean
        // only a late rejection. For `generation` it is worse:
        // `deploy_submission` already presents
        // `s.generation` to the substrate's own `check_generation` on the
        // way there, and an `Ordering::Greater` presentation is *accepted*
        // there and advances the substrate's own stamp -- so a wrong
        // upward `--generation` would leave the substrate ahead of this
        // supervisor's own store the instant `store.submit`'s check then
        // refused to record it, making the supervisor immediately
        // superseded by its own write. One read covers both, so both are
        // checked before either has a chance to run.
        if let Some(existing) = self
            .store
            .get(&s.app_instance_id)
            .map_err(|e| RpcError::InternalError(e.to_string()))?
        {
            if existing.retired {
                return Err(RpcError::InternalError(format!(
                    "app instance '{}' is retired; run `supervisor adopt` to resume managing it \
                     before submitting new desired state",
                    s.app_instance_id
                )));
            }
            if s.generation != existing.generation {
                return Err(RpcError::InternalError(format!(
                    "submit presented generation {}, but app instance '{}' is on record at \
                     generation {}; only `adopt` mints a new one -- run `supervisor adopt`, or \
                     omit --generation to resubmit at the current one",
                    s.generation, s.app_instance_id, existing.generation
                )));
            }
        }

        // Checked in the same pre-flight as `retired`/`generation` above,
        // before any deploy work runs -- a changed placement must be
        // refused, not silently applied.
        self.refuse_placement_change(&plan, &inventory).await.map_err(RpcError::InternalError)?;
        // The manifest-time replica cap, re-checked at the interface that
        // actually accepts a compiled plan.
        Self::refuse_replicas_above_cap(&plan).map_err(RpcError::InternalError)?;
        Self::refuse_unrunnable_schedules(&plan).map_err(RpcError::InternalError)?;
        Self::refuse_unshardable_plan(&plan).map_err(RpcError::InternalError)?;
        validate_plan_visibility(&plan).map_err(|errs| RpcError::InvalidParams(errs.join("; ")))?;

        // Mint before connecting anywhere -- a locked vault or a bad plan
        // must fail before anything is persisted or a network round trip
        // spent (unchanged ordering from before this change). The
        // substituted plan is what the stored desired state carries, so
        // the loop and `force-reconcile` see real master DIDs, not the
        // compiler's fabricated ones.
        let aliases = Self::placed_aliases(&plan).map_err(RpcError::InternalError)?;
        let mut plan = plan;
        let (minted, masters) = keys::mint_and_substitute(&mut plan, &self.vault)
            .await
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        let plan_json_substituted =
            plan.to_json().map_err(|e| RpcError::InternalError(e.to_string()))?;

        // ADR-0022 §6/§3: a per-logical-service topology epoch for
        // every service this plan names. Computed here, ahead of
        // `store.submit`'s durable write, alongside everything else that
        // can fail -- `service_topology` can refuse an inconsistent plan
        // (a compiler bug), and that must refuse the submit with nothing
        // written, not land a stored plan no later `resolve` can build a
        // document from. Over `plan` post-`mint_and_substitute`, so the
        // fingerprint is over the members a document will actually carry,
        // not the compiler's fabricated ids.
        let service_names: BTreeSet<_> =
            plan.services.iter().map(|svc| svc.logical_ref.service_name.clone()).collect();
        let mut topology_fingerprints = Vec::with_capacity(service_names.len());
        for service_name in service_names {
            let topo = topology::service_topology(&plan, &service_name)
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
            let fingerprint =
                topology_fingerprint(topo.mode, &topo.members, topo.sharding_strategy.as_ref());
            topology_fingerprints.push((service_name, fingerprint));
        }

        // Persisted here, before the deploy attempt below -- so a
        // substrate that is down or slow at this
        // exact moment does not stop the desired state itself from
        // becoming durable. Every check above (retired/generation/
        // placement) has already refused a configuration problem before
        // this point runs, so nothing that used to be refused before any
        // deploy work ran is now silently accepted instead. The resident
        // loop (or a later `force-reconcile`) retries whatever the
        // best-effort apply just below does not land.
        self.store
            .submit(
                &s.app_instance_id,
                &plan_json_substituted,
                &s.inventory_json,
                &caller.caller_did,
                s.generation,
            )
            .map_err(|e| RpcError::InternalError(e.to_string()))?;

        // Infallible in practice: a failure here is a stale epoch on an
        // otherwise-correct stored plan, which `resolve`'s own insert-only
        // backfill repairs on the next read. The cache eviction is
        // belt-and-braces -- `handle_resolve` re-signs on an epoch
        // mismatch anyway -- but removing it here means a scale-out is
        // visible without waiting for that comparison.
        for (service_name, fingerprint) in topology_fingerprints {
            let before =
                self.store.topology_epoch(&s.app_instance_id, service_name.as_str()).unwrap_or(0);
            match self.store.record_topology_fingerprint(
                &s.app_instance_id,
                service_name.as_str(),
                &fingerprint,
            ) {
                Ok(after) => {
                    if after != before {
                        self.signed_documents
                            .remove(&(s.app_instance_id.clone(), service_name.to_string()));
                    }
                }
                Err(e) => tracing::warn!(
                    app_instance_id = %s.app_instance_id,
                    %service_name,
                    error = %e,
                    "failed to record this submit's topology fingerprint; a later resolve will \
                     repair it"
                ),
            }
        }

        // Best-effort immediate apply: still surfaced to the caller as an
        // error if it does not fully land (an operator's `submit` should
        // know when nothing landed), but the desired state above is
        // already durable regardless of this outcome.
        let clients =
            self.build_clients(&aliases, &inventory).await.map_err(RpcError::InternalError)?;
        let apply_result = self
            .apply_with_membership_pushes(&plan, &masters, &clients, s.generation, minted)
            .await;
        Self::shutdown_clients(clients.into_values()).await;
        // Review finding D-3: this error and a pre-flight refusal
        // (retired/generation/placement, all above) used to read
        // identically to the caller -- a plain string -- despite being
        // opposites: a refusal wrote nothing and needs a corrected plan,
        // while reaching here means the desired state above is already
        // durable and the resident loop will retry whatever did not
        // land. Said explicitly so an operator does not have to already
        // know that ordering to read the error correctly.
        let minted = apply_result.map_err(|e| {
            RpcError::InternalError(format!(
                "desired state was recorded; the immediate apply did not fully land and will be \
                 retried by the resident loop: {e}"
            ))
        })?;

        let result = SubmitResult {
            masters: minted
                .into_iter()
                .map(|m| WitMintedMaster {
                    service_name: m.service_name,
                    master_did: m.master_did,
                    vault_name: m.vault_name,
                    member_index: m.member_index,
                })
                .collect(),
        };
        Ok(NativeResponse { payload: serde_json::to_value(result).unwrap_or(Value::Null) })
    }

    /// Reads the held generation across every given client and claims
    /// `held + 1` on each. Split out of `handle_adopt` so that function
    /// can close every client it opened however this returns, success or
    /// failure -- `?` inside either loop here used to return straight out
    /// of `handle_adopt` itself, leaking
    /// every client already connected and every one still left to try.
    pub(super) async fn claim_next_generation(
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

    pub(super) async fn handle_adopt(
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
    pub(super) async fn release_on_every_substrate(
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
    pub(super) fn release_payload(status: &str, failed: Vec<(String, String)>) -> Value {
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

    pub(super) async fn handle_release(
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
    pub(super) async fn handle_retire(
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

    pub(super) async fn handle_pause(
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

    pub(super) async fn handle_resume(
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

    pub(super) async fn handle_force_reconcile(
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

    pub(super) async fn handle_export_master(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (name,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse export-master params: {e}"))
        })?;
        let path = self
            .vault
            .export_master(&name)
            .await
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse {
            payload: serde_json::to_value(path.to_string_lossy().into_owned())
                .unwrap_or(Value::Null),
        })
    }

    pub(super) async fn handle_import_master(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (name,): (String,) = serde_json::from_value(params).map_err(|e| {
            RpcError::InvalidParams(format!("failed to parse import-master params: {e}"))
        })?;
        self.vault
            .import_master(&name)
            .await
            .map_err(|e| RpcError::InternalError(e.to_string()))?;
        Ok(NativeResponse { payload: serde_json::json!({"status": "imported"}) })
    }

    /// The DID `revoke-instance` actually anchors as revoked. Read from
    /// the hosting substrate rather than a stored table: the substrate is
    /// the authority on what key is actually installed.
    ///
    /// `instance_did` is what *this caller* (this supervisor) would
    /// derive -- correct for the certify flow that reads it before
    /// anything is installed, wrong here whenever the installed
    /// certificate was minted for a different caller (a member deployed
    /// by an operator and only later adopted, not yet redeployed).
    /// Revoking the derived DID in that case anchors a key nothing
    /// presents, while the key actually in use stays fully trusted -- so
    /// this prefers `installed_temporary_did`, the substrate's ground
    /// truth for what is installed right now, and only falls back to the
    /// derived DID when nothing is installed yet (nothing to read, so the
    /// prospective key is the closest thing to "the key this placement
    /// would use"). A free function of the RPC's answer alone, so the
    /// choice is directly testable without a live client.
    pub(super) fn select_revocation_did(identity: syneroym_sdk::InstanceIdentity) -> String {
        identity.installed_temporary_did.unwrap_or(identity.instance_did)
    }

    /// Revoke one placed member's instance key: append its derived DID to
    /// the master anchor's revoked list, then record the placement revoked
    /// so nothing mints it a fresh certificate afterwards.
    ///
    /// Under the instance lock for the whole verb, the same discipline
    /// every other instance-scoped write follows. Without it, this and a
    /// resident pass's renewal of the same member race: the pass could mint
    /// and install a fresh certificate in the gap between the anchor write
    /// and the exclusion write landing, which is precisely the window this
    /// verb exists to close.
    ///
    /// Order matters. The local exclusion is written **after** the anchor
    /// publish succeeds, so a failed publish leaves the placement under
    /// ordinary management rather than half-revoked -- excluded from
    /// renewal here while still fully trusted by every consumer.
    pub(super) async fn handle_revoke_instance(
        &self,
        caller: &CallerContext,
        params: Value,
    ) -> RpcResult<NativeResponse> {
        self.require_admin(caller)?;
        let (app_instance_id, logical_ref): (String, String) = serde_json::from_value(params)
            .map_err(|e| {
                RpcError::InvalidParams(format!("failed to parse revoke-instance params: {e}"))
            })?;
        let lock = self.instance_lock(&app_instance_id);
        let _guard = lock.lock().await;

        // Checked here as well as inside `record_revocation`, so a node
        // with no registry refuses before spending a round trip resolving
        // an instance identity it can do nothing with.
        if self.anchor_writer.is_none() {
            return Err(RpcError::InternalError(
                "this supervisor's node has no registry configured (substrate.registry_url), so \
                 it cannot publish a revocation; a revocation nothing can resolve is not a \
                 revocation"
                    .to_string(),
            ));
        }

        let state = self
            .store
            .get(&app_instance_id)
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

        let svc =
            plan.services.iter().find(|s| s.member_ref().to_string() == logical_ref).ok_or_else(
                || {
                    RpcError::InvalidParams(format!(
                        "app instance '{app_instance_id}' has no member '{logical_ref}' in its \
                         stored plan"
                    ))
                },
            )?;
        let alias = svc.substrate.as_ref().ok_or_else(|| {
            RpcError::InternalError(format!("member '{logical_ref}' has no substrate placement"))
        })?;
        let entry = inventory.get(alias.as_str()).ok_or_else(|| {
            RpcError::InternalError(format!("no inventory entry for substrate alias '{alias}'"))
        })?;

        // `select_revocation_did`'s own doc explains the choice below.
        let mut client = self
            .connected_client(entry)
            .await
            .map_err(|e| RpcError::InternalError(format!("failed to reach '{alias}': {e}")))?;
        let identity = client.instance_identity(svc.service_id.as_str()).await;
        let _ = client.shutdown().await;
        let identity = identity.map_err(|e| {
            RpcError::InternalError(format!(
                "failed to resolve the instance identity for '{logical_ref}': {e}"
            ))
        })?;
        let instance_did = Self::select_revocation_did(identity);

        self.record_revocation(
            &app_instance_id,
            &logical_ref,
            svc.logical_ref.service_name.as_str(),
            svc.member_index,
            &instance_did,
        )
        .await
        .map_err(RpcError::InternalError)?;

        Ok(NativeResponse {
            payload: serde_json::json!({
                "status": "revoked",
                "instance_did": instance_did,
                "note": "the member's process is still running; undeploy it separately if that is \
                         intended",
            }),
        })
    }

    /// `revoke-instance`'s two writes, once the instance DID is known.
    /// Split from the verb so the ordering below is exercisable without a
    /// live substrate answering `resolve-instance-identity` -- which is the
    /// only reason the verb needs a network at all.
    ///
    /// The anchor publish comes first and the local exclusion only after it
    /// succeeds. Reversed, a failed publish would leave the placement
    /// half-revoked: excluded from renewal here, while every consumer still
    /// fully trusts the key -- so it would quietly age out instead of
    /// failing closed, which is the opposite of what was asked for.
    pub(super) async fn record_revocation(
        &self,
        app_instance_id: &str,
        logical_ref: &str,
        service_name: &str,
        member_index: u32,
        instance_did: &str,
    ) -> Result<(), String> {
        let writer = self.anchor_writer.as_ref().ok_or_else(|| {
            "this supervisor's node has no registry configured (substrate.registry_url), so it \
             cannot publish a revocation"
                .to_string()
        })?;
        let master =
            keys::master_for_member(&self.vault, app_instance_id, service_name, member_index)
                .await
                .map_err(|e| e.to_string())?;
        writer
            .revoke_instance(&master, instance_did)
            .await
            .map_err(|e| format!("failed to publish the revocation: {e}"))?;

        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.store
            .revoke_placement(app_instance_id, logical_ref, now as i64)
            .map_err(|e| e.to_string())
    }
}

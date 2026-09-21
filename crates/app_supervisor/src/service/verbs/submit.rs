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
    pub(in crate::service) fn refuse_replicas_above_cap(
        plan: &DeploymentPlan,
    ) -> Result<(), String> {
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
    pub(in crate::service) fn refuse_unrunnable_schedules(
        plan: &DeploymentPlan,
    ) -> Result<(), String> {
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
    pub(in crate::service) fn refuse_unshardable_plan(plan: &DeploymentPlan) -> Result<(), String> {
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

    pub(in crate::service) async fn refuse_placement_change(
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

    pub(in crate::service) async fn handle_submit(
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
        self.validate_submit_preflight(&s, &plan, &inventory).await?;

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

        let topology_fingerprints = Self::compute_topology_fingerprints(&plan)?;

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

        self.record_and_evict_topology(&s.app_instance_id, &topology_fingerprints);

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

    async fn validate_submit_preflight(
        &self,
        s: &Submission,
        plan: &DeploymentPlan,
        inventory: &SupervisorInventory,
    ) -> RpcResult<()> {
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

        self.refuse_placement_change(plan, inventory).await.map_err(RpcError::InternalError)?;
        Self::refuse_replicas_above_cap(plan).map_err(RpcError::InternalError)?;
        Self::refuse_unrunnable_schedules(plan).map_err(RpcError::InternalError)?;
        Self::refuse_unshardable_plan(plan).map_err(RpcError::InternalError)?;
        validate_plan_visibility(plan).map_err(|errs| RpcError::InvalidParams(errs.join("; ")))?;
        Ok(())
    }

    fn compute_topology_fingerprints(
        plan: &DeploymentPlan,
    ) -> RpcResult<Vec<(LogicalServiceName, String)>> {
        let service_names: BTreeSet<_> =
            plan.services.iter().map(|svc| svc.logical_ref.service_name.clone()).collect();
        let mut topology_fingerprints = Vec::with_capacity(service_names.len());
        for service_name in service_names {
            let topo = topology::service_topology(plan, &service_name)
                .map_err(|e| RpcError::InternalError(e.to_string()))?;
            let fingerprint =
                topology_fingerprint(topo.mode, &topo.members, topo.sharding_strategy.as_ref());
            topology_fingerprints.push((service_name, fingerprint));
        }
        Ok(topology_fingerprints)
    }

    fn record_and_evict_topology(
        &self,
        app_instance_id: &str,
        topology_fingerprints: &[(LogicalServiceName, String)],
    ) {
        for (service_name, fingerprint) in topology_fingerprints {
            let before =
                self.store.topology_epoch(app_instance_id, service_name.as_str()).unwrap_or(0);
            match self.store.record_topology_fingerprint(
                app_instance_id,
                service_name.as_str(),
                fingerprint,
            ) {
                Ok(after) => {
                    if after != before {
                        self.signed_documents
                            .remove(&(app_instance_id.to_string(), service_name.to_string()));
                    }
                }
                Err(e) => tracing::warn!(
                    %app_instance_id,
                    %service_name,
                    error = %e,
                    "failed to record this submit's topology fingerprint; a later resolve will \
                     repair it"
                ),
            }
        }
    }
}

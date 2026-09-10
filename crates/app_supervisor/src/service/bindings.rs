use super::*;

impl SupervisorService {
    /// The mint/substitute/certify/apply pipeline shared by `submit` and
    /// `force-reconcile`. Returns the plan with masters substituted in,
    /// not just the minted list -- `handle_submit` used to re-run
    /// `mint_and_substitute` a second time on its own copy to get this
    /// same plan for storing as desired state: one vault open and one
    /// `reveal_secret` per service for a value this call had already
    /// computed.
    pub(super) async fn deploy_submission(
        &self,
        mut plan: DeploymentPlan,
        inventory: &SupervisorInventory,
        generation: u64,
    ) -> Result<(Vec<MintedMaster>, DeploymentPlan), String> {
        let aliases = Self::placed_aliases(&plan)?;

        // Mint before connecting anywhere: a locked vault or a bad plan
        // must fail before the supervisor spends a network round trip on
        // substrates it cannot yet certify anything for.
        let (minted, masters) =
            keys::mint_and_substitute(&mut plan, &self.vault).await.map_err(|e| e.to_string())?;

        let clients = self.build_clients(&aliases, inventory).await?;

        // However this returns, every client this call opened must be
        // closed -- not just on the success path.
        let result =
            self.apply_with_membership_pushes(&plan, &masters, &clients, generation, minted).await;
        Self::shutdown_clients(clients.into_values()).await;
        result.map(|minted| (minted, plan))
    }

    /// Mints, certifies, and applies `plan`, except for whatever member the
    /// same classifier `reconcile_instance_pass` uses
    /// (`classify_update_actions`) would route to a binding push or
    /// exclude outright -- those get `push_bindings` after the redeploy of
    /// the rest, rather than a full `deploy_with_context` reinstall.
    /// Shared by `deploy_submission` (`force-reconcile`) and
    /// `handle_submit`, which each used to call `apply_with_clients`
    /// directly over the whole plan: an operator resubmit that only scales
    /// a dependency now takes the exact same push path the loop's own write
    /// phase does for an identical diff, instead of reinstalling every
    /// dependent every time.
    ///
    /// A push failure does not stop the redeploy half, and a redeploy
    /// failure does not stop the pushes -- the two work lists are
    /// independent members, the same way the loop's write phase treats
    /// them.
    pub(super) async fn apply_with_membership_pushes(
        &self,
        plan: &DeploymentPlan,
        masters: &BTreeMap<ServiceId, Identity>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        generation: u64,
        minted: Vec<MintedMaster>,
    ) -> Result<Vec<MintedMaster>, String> {
        let landed = self
            .store
            .journal
            .get_completed_actions_for_instance(&plan.app_instance_id)
            .unwrap_or_default();
        let (redeploy_exclusions, push_candidates) =
            match Reconciler::new(&self.store.journal).compute_diff(plan) {
                Ok(diff) => Self::classify_update_actions(&landed, &diff.actions),
                Err(_) => (BTreeSet::new(), Vec::new()),
            };

        let mut apply_plan = plan.clone();
        apply_plan.services.retain(|s| !redeploy_exclusions.contains(&s.member_ref().to_string()));

        let apply_result =
            self.apply_with_clients(&apply_plan, plan, masters, clients, generation, minted).await;
        let apply_result_is_ok = apply_result.is_ok();

        let push_errors = self
            .push_submission_membership_candidates(plan, &push_candidates, clients, generation)
            .await;

        // `apply_with_clients` above already journaled `plan` -- the full
        // desired state, including a pushed member's new
        // `resolved_dependencies` -- as `Active` the moment the redeploy
        // half landed, regardless of whether the pushes then succeeded.
        // Left alone, a failed push leaves that `Active` record as the next
        // pass's diff baseline, so `compute_diff` reads the member as
        // already converged and the `BindingConflict` this call raised is
        // never retried. Downgrading to `Degraded` makes the next pass see
        // the same diff and reclassify the member as a push candidate
        // again. Gated on `apply_result_is_ok`: an `Err` means either
        // nothing was journaled this call (a stale, unrelated record must
        // not be touched) or `Degraded` was journaled already.
        if apply_result_is_ok && !push_errors.is_empty() {
            self.downgrade_record_after_failed_submit_push(&plan.app_instance_id);
        }

        match (apply_result, push_errors.is_empty()) {
            (Ok(minted), true) => Ok(minted),
            (Ok(_), false) => {
                Err(format!("binding push did not fully land: {}", push_errors.join("; ")))
            }
            (Err(e), true) => Err(e),
            (Err(e), false) => {
                Err(format!("{e}; binding push did not fully land: {}", push_errors.join("; ")))
            }
        }
    }

    /// Pushes bindings for each membership-change candidate an operator
    /// resubmit produced, and returns one `"<member>: <reason>"` string per
    /// push that did not land. A candidate with no connected client is
    /// queued through `enqueue_unreachable_push` -- as unreachable as one
    /// the resident loop could not connect to, and needing the same
    /// durability -- and also reported. `Deferred` counts as a failure so
    /// `submit`/`force-reconcile` reports it and the caller's downgrade
    /// fires. Publishes its own newly-opened alerts before returning.
    async fn push_submission_membership_candidates(
        &self,
        plan: &DeploymentPlan,
        push_candidates: &[(PlannedService, String)],
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        generation: u64,
    ) -> Vec<String> {
        let mut opened = Vec::new();
        let mut push_errors = Vec::new();
        for (svc, substrate_did) in push_candidates {
            let Some(client) = svc.substrate.as_ref().and_then(|a| clients.get(a)) else {
                self.enqueue_unreachable_push(
                    &plan.app_instance_id,
                    &plan.app_instance_id.to_string(),
                    plan,
                    svc,
                    substrate_did,
                    generation,
                    "not connected to its landed substrate this call",
                    &mut opened,
                )
                .await;
                push_errors.push(format!(
                    "{}: not connected to its landed substrate this call",
                    svc.member_ref()
                ));
                continue;
            };
            let actor = self.durable_actor(
                client.clone(),
                &plan.app_instance_id.to_string(),
                &svc.member_ref().to_string(),
                substrate_did,
            );
            match self
                .push_bindings(
                    &plan.app_instance_id,
                    plan,
                    svc,
                    substrate_did,
                    &actor,
                    generation,
                    &mut opened,
                )
                .await
            {
                Ok(PushOutcome::Deferred) => {
                    push_errors.push(format!(
                        "{}: deferred to an already-pending queued delivery",
                        svc.member_ref()
                    ));
                }
                Ok(PushOutcome::Landed(_)) => {}
                Err(e) => push_errors.push(format!("{}: {e}", svc.member_ref())),
            }
        }
        self.publish_opened_alerts(&plan.app_instance_id.to_string(), &opened).await;
        push_errors
    }

    /// Downgrades this call's just-journaled `Active` record to `Degraded`
    /// after a binding push failed -- see the call site for why this is
    /// what lets the next pass reclassify the member as a push candidate
    /// instead of reading it as converged.
    fn downgrade_record_after_failed_submit_push(&self, app_instance_id: &AppInstanceId) {
        if let Ok(Some(latest)) = self.store.journal.get_latest(app_instance_id)
            && latest.state == DeploymentState::Active
            && let Err(e) = self.store.journal.update_state(latest.id, DeploymentState::Degraded)
        {
            tracing::warn!(
                app_instance_id = %app_instance_id,
                error = %e,
                "failed to mark this submit's record Degraded after a binding push did not land"
            );
        }
    }

    /// `plan` is what this call actually mints, certifies, and deploys.
    /// `record_plan` is what gets journaled as the new baseline for
    /// `Reconciler::compute_diff` to read next time -- equal to `plan`
    /// for every full apply (`deploy_submission`, `handle_submit`), but
    /// deliberately wider than it for the loop's filtered pass: see
    /// `record_plan_for_pass`'s own doc for why the two must not be
    /// conflated.
    pub(super) async fn apply_with_clients(
        &self,
        plan: &DeploymentPlan,
        record_plan: &DeploymentPlan,
        masters: &BTreeMap<ServiceId, Identity>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        generation: u64,
        minted: Vec<MintedMaster>,
    ) -> Result<Vec<MintedMaster>, String> {
        let filtered = self.filter_revoked_placements(plan, record_plan, clients).await;
        let (plan, record_plan) = match &filtered {
            Some((p, r)) => (p, r),
            None => (plan, record_plan),
        };

        let (instance_certs, registry_certs) = deploy::certify_placed_members(
            plan,
            masters,
            clients,
            None,
            self.renewed_cert_expires_hours,
        )
        .await
        .map_err(|e| e.to_string())?;

        let deployment_id = self
            .store
            .journal
            .append(record_plan, DeploymentState::Applying)
            .map_err(|e| e.to_string())?;
        let targets = Self::deploy_targets(clients);
        let binding_epochs = self.advance_binding_epochs_for_apply(plan)?;

        let report = deploy::apply_plan(
            ApplyRequest {
                plan,
                targets: &targets,
                fallback: None,
                instance_certificates: &instance_certs,
                registry_certificates: &registry_certs,
                // Always true on the supervisor's apply path: the
                // supervisor holds masters by construction, so the
                // condition `roymctl app deploy` ties this flag to is
                // always met here.
                emit_bindings: true,
                generation,
                binding_epochs: &binding_epochs,
            },
            &self.store.journal,
            deployment_id,
        )
        .await
        .map_err(|e| e.to_string())?;

        self.store
            .journal
            .update_state(
                deployment_id,
                if report.is_complete() {
                    DeploymentState::Active
                } else {
                    DeploymentState::Degraded
                },
            )
            .map_err(|e| e.to_string())?;

        if !report.is_complete() {
            let failures: Vec<String> =
                report.failures.iter().map(|f| format!("{}: {}", f.member_ref, f.error)).collect();
            return Err(format!("deploy applied with failures: {}", failures.join("; ")));
        }

        Ok(minted)
    }

    /// `apply_with_clients` is the one place every certificate-minting
    /// caller passes through -- the resident loop, `submit`, and
    /// `force-reconcile` alike -- so filtering revoked placements here is
    /// what makes revocation stick: without it an ordinary resubmit would
    /// silently re-mint and reinstall the very key the operator just
    /// revoked. A revoked member is skipped, not failed (the rest of the
    /// plan still reconciles), and raises `InstanceRevoked`.
    ///
    /// Returns `None` on the ordinary path (nothing revoked), so a plan
    /// carrying hex-inlined wasm artifacts is not cloned for nothing.
    /// Otherwise returns `(plan_without_revoked, record_plan_with_revoked)`:
    /// `record_plan` must *keep* the revoked member, since it is the same
    /// baseline `compute_diff` reads next pass -- dropping it there would
    /// have every later pass report it as a fresh `Add` and land right back
    /// here to be filtered out again, a no-op write journaled forever.
    async fn filter_revoked_placements(
        &self,
        plan: &DeploymentPlan,
        record_plan: &DeploymentPlan,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> Option<(DeploymentPlan, DeploymentPlan)> {
        let app_instance_id = plan.app_instance_id.to_string();
        let revoked = self.store.revoked_placements(&app_instance_id).unwrap_or_default();
        if revoked.is_empty() {
            return None;
        }
        let mut opened = Vec::new();
        if let Ok(instance_id) = AppInstanceId::try_new(app_instance_id.clone()) {
            for svc in &plan.services {
                let l_ref = svc.member_ref().to_string();
                if !revoked.contains(&l_ref) {
                    continue;
                }
                // The `substrate_did` column holds a real DID at every
                // other call site -- resolved here through this call's own
                // connected clients, the same source the certify step
                // already trusts for the substrate a service is placed on.
                let substrate_did = svc
                    .substrate
                    .as_ref()
                    .and_then(|a| clients.get(a))
                    .map(|c| c.service_id().to_string())
                    .unwrap_or_default();
                if let Ok(true) = self.store.alerts.raise(
                    &instance_id,
                    Some(&l_ref),
                    svc.substrate.as_ref().map(SubstrateAlias::as_str),
                    &substrate_did,
                    AlertKind::InstanceRevoked,
                    &format!(
                        "'{l_ref}' has a revoked instance key, so it is not reinstalled or \
                         re-certified; the rest of the plan still reconciles. Undeploy it \
                         separately if the process itself should stop"
                    ),
                ) {
                    opened.push((AlertKind::InstanceRevoked, l_ref));
                }
            }
        }
        self.publish_opened_alerts(&app_instance_id, &opened).await;
        let mut filtered = plan.clone();
        filtered.services.retain(|s| !revoked.contains(&s.member_ref().to_string()));
        Some((filtered, record_plan.clone()))
    }

    /// The plain, undurable deploy targets `apply_plan` takes -- one per
    /// connected alias. Deliberately not durable: `apply_plan` only ever
    /// calls `actor.apply_plan(..)` on these, never `write_bindings`, and
    /// there is no per-service logical ref to bind a queue key to here
    /// anyway, since one alias's actor covers every service placed on it.
    fn deploy_targets(
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> BTreeMap<SubstrateAlias, DeployTarget> {
        clients
            .iter()
            .map(|(alias, c)| {
                (
                    alias.clone(),
                    DeployTarget {
                        alias: Some(alias.clone()),
                        substrate_did: c.service_id().to_string(),
                        actor: deploy::build_actor(c.clone()),
                    },
                )
            })
            .collect()
    }

    /// Advances the binding epoch for every dependent service this apply
    /// touches -- a deploy is an authoritative write like any other. A
    /// service with no declared dependencies emits no bindings, so its
    /// epoch is never read and is left alone.
    fn advance_binding_epochs_for_apply(
        &self,
        plan: &DeploymentPlan,
    ) -> Result<BTreeMap<MemberRef, u64>, String> {
        let mut binding_epochs: BTreeMap<MemberRef, u64> = BTreeMap::new();
        for svc in &plan.services {
            if svc.resolved_dependencies.is_empty() {
                continue;
            }
            let epoch = self
                .store
                .advance_binding_epoch(
                    &plan.app_instance_id.to_string(),
                    &svc.member_ref().to_string(),
                )
                .map_err(|e| e.to_string())?;
            binding_epochs.insert(svc.member_ref(), epoch);
        }
        Ok(binding_epochs)
    }

    /// One dependent member's bindings, at its next epoch, without a
    /// redeploy: reuses `map_deployment_plan_to_wit`'s own
    /// binding-construction logic (called the same way `apply_plan` calls
    /// it internally, over `&[svc]`) rather than duplicating it, so the
    /// two paths cannot drift apart on what a binding looks like on the
    /// wire. Its production caller is the membership-change classifier in
    /// `reconcile_instance_pass`/`apply_write_phase`.
    ///
    /// `Stale(held)` is retried exactly once, at `held + 1`: no re-read,
    /// since `Stale` already carries the number a second round trip would
    /// only relearn. `Conflict` is not retried -- a second writer exists,
    /// and retrying would only race it again. Either failure raises
    /// `BindingConflict`, folded into `opened` so the caller can publish
    /// it the same way every other alert this pass raised gets published.
    /// A push that lands cleanly clears it instead -- the clear site this
    /// alert kind never had, without which `Degraded` derived from it
    /// would be permanent.
    ///
    /// `substrate_did` is the member's real, already-landed substrate DID
    /// -- not `svc.substrate`, an operator-chosen
    /// alias (empty when placement falls back), which used to be written
    /// into the alert's `substrate_did` column and could then never match
    /// a clear keyed on the real DID every other alert kind uses.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn push_bindings(
        &self,
        instance_id: &AppInstanceId,
        plan: &DeploymentPlan,
        svc: &PlannedService,
        substrate_did: &str,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) -> Result<PushOutcome, String> {
        let app_instance_id = plan.app_instance_id.to_string();
        let l_ref = svc.member_ref().to_string();

        // This call advances the binding epoch unconditionally, before
        // every attempt. `DurableActor::write_
        // bindings` only enqueues on a transport failure, and its own
        // dedup guard (`already_pending`) discards a second enqueue for a
        // key that already has a pending row -- so a *second* transport
        // failure for the same key (this pass reconnects fine but the
        // write itself times out, a narrower case than the connect-level
        // failure `enqueue_unreachable_push` already guards this way)
        // would advance the local epoch again while the queue still only
        // holds the first, older-epoch payload. `written_epoch` -- read
        // back for convergence from the same counter this advances --
        // would then race ahead of what the worker can ever actually
        // deliver, and `is_converged` would read the eventual successful
        // delivery of the *queued* item as still unconverged. Checking
        // first, and deferring entirely to the queue when a row already
        // exists, is the same fix `enqueue_unreachable_push` already
        // applies to its own, more common case.
        let queue_key = QueueKey {
            app_instance_id: app_instance_id.clone(),
            logical_ref: l_ref.clone(),
            substrate_did: substrate_did.to_string(),
        };
        // Deliberately not `SupervisorOutbox::already_pending`, whose
        // fail-*closed* default (an unreadable queue reads as "already
        // pending") is right for its own purpose -- a guard against
        // writing a duplicate row should err toward not writing. Here it
        // would mean the opposite: an unreadable queue silently skips the
        // live attempt and returns `Ok`, reporting success for a push that
        // never happened and was never durably queued either. Failing
        // *open* instead is safe specifically because the queue and every
        // other supervisor table share one connection -- a genuinely
        // broken connection surfaces a
        // proper `Err` on the very next line's `advance_binding_epoch`
        // instead of a silent no-op.
        if self.store.queue.has_pending(&queue_key.to_string()).unwrap_or(false) {
            return Ok(PushOutcome::Deferred);
        }

        let epoch = self
            .store
            .advance_binding_epoch(&app_instance_id, &l_ref)
            .map_err(|e| e.to_string())?;
        let outcomes = self
            .write_bindings_with_stale_retry(
                instance_id,
                plan,
                svc,
                substrate_did,
                actor,
                generation,
                &app_instance_id,
                &l_ref,
                epoch,
                opened,
            )
            .await?;
        self.settle_binding_push_alert(instance_id, substrate_did, &l_ref, &outcomes, opened);
        Ok(PushOutcome::Landed(outcomes))
    }

    /// Sends `svc`'s bindings at `epoch`, and -- if the substrate answers
    /// `Stale(held)` -- retries exactly once at `held + 1`: no re-read,
    /// since `Stale` already carries the number a second round trip would
    /// only relearn. A `write_bindings` call that fails outright (the
    /// dependent unreachable) raises `BindingConflict` and returns `Err`,
    /// the same alert a `Stale`/`Conflict` *outcome* raises -- an operator
    /// reading `alerts` should not have to know which shape the failure
    /// took.
    #[allow(clippy::too_many_arguments)]
    async fn write_bindings_with_stale_retry(
        &self,
        instance_id: &AppInstanceId,
        plan: &DeploymentPlan,
        svc: &PlannedService,
        substrate_did: &str,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
        app_instance_id: &str,
        l_ref: &str,
        epoch: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        let outcomes = match self.write_bindings_at_epoch(plan, svc, actor, generation, epoch).await
        {
            Ok(o) => o,
            Err(e) => {
                self.raise_binding_push_failure(instance_id, substrate_did, l_ref, &e, opened);
                return Err(e);
            }
        };
        let Some(held) = outcomes.iter().find_map(|o| match o {
            BindingWriteOutcome::Stale(held) => Some(*held),
            _ => None,
        }) else {
            return Ok(outcomes);
        };
        let retry_epoch = held + 1;
        self.store
            .set_binding_epoch_at_least(app_instance_id, l_ref, retry_epoch)
            .map_err(|e| e.to_string())?;
        match self.write_bindings_at_epoch(plan, svc, actor, generation, retry_epoch).await {
            Ok(o) => Ok(o),
            Err(e) => {
                self.raise_binding_push_failure(instance_id, substrate_did, l_ref, &e, opened);
                Err(e)
            }
        }
    }

    /// Raises `BindingConflict` when a push's outcomes still carry a
    /// `Stale`/`Conflict` after the one retry, or clears it when the push
    /// landed cleanly -- the clear site this alert kind never had, without
    /// which a `Degraded` derived from it would be permanent.
    fn settle_binding_push_alert(
        &self,
        instance_id: &AppInstanceId,
        substrate_did: &str,
        l_ref: &str,
        outcomes: &[BindingWriteOutcome],
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let failed = outcomes
            .iter()
            .any(|o| matches!(o, BindingWriteOutcome::Stale(_) | BindingWriteOutcome::Conflict(_)));
        if failed {
            if let Ok(true) = self.store.alerts.raise(
                instance_id,
                Some(l_ref),
                None,
                substrate_did,
                AlertKind::BindingConflict,
                &format!(
                    "a binding push for '{l_ref}' did not land cleanly after one retry: \
                     {outcomes:?}"
                ),
            ) {
                opened.push((AlertKind::BindingConflict, l_ref.to_string()));
            }
        } else {
            let _ = self.store.alerts.clear(
                instance_id,
                Some(l_ref),
                substrate_did,
                AlertKind::BindingConflict,
            );
        }
    }

    /// The alert half of an unreachable dependent: a push that fails to
    /// reach the dependent at all (not a clean `Stale`/`Conflict` outcome)
    /// still
    /// needs to be visible on `alerts`, the same `AlertKind` a bad
    /// outcome raises -- an operator reading `alerts` should not have to
    /// know which of the two shapes a failed push took.
    pub(super) fn raise_binding_push_failure(
        &self,
        instance_id: &AppInstanceId,
        substrate_did: &str,
        l_ref: &str,
        error: &str,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(l_ref),
            None,
            substrate_did,
            AlertKind::BindingConflict,
            &format!("a binding push for '{l_ref}' failed to reach the dependent: {error}"),
        ) {
            opened.push((AlertKind::BindingConflict, l_ref.to_string()));
        }
    }

    /// Builds the `binding-write` a real deploy would emit for `svc`
    /// alone, at `epoch`, and sends it -- the standalone half of
    /// `push_bindings`, split out so a retry at a different epoch is a
    /// second call to this, not a copy of the mapping logic.
    /// The `binding-write` a real deploy would emit for `svc` alone, at
    /// `epoch` -- pure, no actor, no store. Shared by `write_bindings_at_
    /// epoch` (which sends it through a live actor) and
    /// `enqueue_unreachable_push` (which has no actor to send it through
    /// at all and must still capture *what* would have been sent).
    pub(super) fn build_binding_write(
        plan: &DeploymentPlan,
        svc: &PlannedService,
        generation: u64,
        epoch: u64,
    ) -> Result<BindingWrite, String> {
        let binding_epochs = BTreeMap::from([(svc.member_ref(), epoch)]);
        let wit_plan = map_deployment_plan_to_wit(
            plan,
            &[svc],
            &BTreeMap::new(),
            &BTreeMap::new(),
            true,
            generation,
            &binding_epochs,
        )
        .map_err(|e| e.to_string())?;
        let bindings = wit_plan
            .services
            .into_iter()
            .next()
            .and_then(|s| s.app_context)
            .map(|ctx| ctx.bindings)
            .unwrap_or_default();
        Ok(BindingWrite {
            service_id: svc.service_id.to_string(),
            app_instance_id: plan.app_instance_id.to_string(),
            bindings,
            generation,
        })
    }

    pub(super) async fn write_bindings_at_epoch(
        &self,
        plan: &DeploymentPlan,
        svc: &PlannedService,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
        epoch: u64,
    ) -> Result<Vec<BindingWriteOutcome>, String> {
        let write = Self::build_binding_write(plan, svc, generation, epoch)?;
        actor.write_bindings(write).await
    }

    /// The durable half of a push candidate this pass could not even reach
    /// an actor for -- no known alias for its landed DID, or a connect
    /// that timed out before a client existed to wrap in a `DurableActor`
    /// at all. `DurableActor::write_bindings` is what normally enqueues on
    /// a transport failure, but that only fires *inside* an
    /// attempted call; a substrate this pass never managed to dial has no
    /// call to attempt. Left at "raise an alert and move on" (the shape
    /// this had before), a substrate that is durably offline -- the exact
    /// case ADR-0023's reference scenario is built around -- would never
    /// be queued at all, only ever reported.
    ///
    /// Advances the binding epoch itself, the same as `push_bindings`
    /// does before a live attempt: the queued payload must carry a real
    /// epoch for the epoch guard to mean anything once a worker delivers
    /// it, and skipping the advance here would leave every queued item
    /// from this pass sharing the stale epoch a *reachable* pass last
    /// used.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn enqueue_unreachable_push(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        svc: &PlannedService,
        substrate_did: &str,
        generation: u64,
        reason: &str,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let l_ref = svc.member_ref().to_string();
        self.raise_binding_push_failure(instance_id, substrate_did, &l_ref, reason, opened);
        let queue_key = QueueKey {
            app_instance_id: app_instance_id.to_string(),
            logical_ref: l_ref.clone(),
            substrate_did: substrate_did.to_string(),
        };
        let outbox = SupervisorOutbox::new(self.store.queue.clone());
        // The resident loop's own retry (falling `compute_diff` back to
        // the previous baseline on Degraded) reclassifies this member as
        // a push
        // candidate every pass until its push lands, so this branch runs
        // repeatedly while the substrate stays offline. A pending row
        // already covers the intent; advancing the epoch again for a
        // write that will not even be queued would strand the local
        // counter ahead of whatever the eventually-delivered, earlier-
        // epoch write actually lands -- `is_converged` would then never
        // agree, even after delivery succeeds.
        if outbox.already_pending(&queue_key.to_string()) {
            return;
        }
        let epoch = match self.store.advance_binding_epoch(app_instance_id, &l_ref) {
            Ok(epoch) => epoch,
            Err(e) => {
                tracing::warn!(
                    app_instance_id,
                    l_ref,
                    error = %e,
                    "failed to advance the binding epoch for an unreachable push; not queued \
                     this pass"
                );
                return;
            }
        };
        let write = match Self::build_binding_write(plan, svc, generation, epoch) {
            Ok(write) => write,
            Err(e) => {
                tracing::warn!(
                    app_instance_id,
                    l_ref,
                    error = %e,
                    "failed to build the binding write for an unreachable push; not queued this \
                     pass"
                );
                return;
            }
        };
        outbox.enqueue(&queue_key.to_string(), substrate_did, &write).await;
    }

    /// The read half of binding convergence: per declared dependency of
    /// every dependent in the plan, what this supervisor last wrote
    /// (`SupervisorStore::binding_epoch`) versus what the sweep's
    /// `HealthReport` observed the hosting substrate serving *for that
    /// dependent* (`ServiceHealth.binding_epochs`, keyed by dependency
    /// name). A dependent absent from the report (unreachable, or no
    /// completed placement) reports every one of its dependencies
    /// `observed_epoch: None`, `converged: false` -- unconverged, not
    /// silently absent from the list, so an operator sees the gap rather
    /// than an empty table that looks like nothing was ever declared.
    pub(super) fn binding_convergence_rows(
        &self,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        report: &health::HealthReport,
    ) -> Vec<BindingConvergence> {
        let mut rows = Vec::new();
        for svc in &plan.services {
            if svc.resolved_dependencies.is_empty() {
                continue;
            }
            let dependent_ref = svc.member_ref().to_string();
            let written_epoch =
                self.store.binding_epoch(app_instance_id, &dependent_ref).unwrap_or(0);
            let observed: BTreeMap<&str, u64> = report
                .services
                .iter()
                .find(|s| s.member_ref().to_string() == dependent_ref)
                .map(|s| s.binding_epochs.iter().map(|(n, e)| (n.as_str(), *e)).collect())
                .unwrap_or_default();
            for dependency_name in svc.resolved_dependencies.keys() {
                let observed_epoch = observed.get(dependency_name.as_str()).copied();
                rows.push(BindingConvergence {
                    dependent_logical_ref: dependent_ref.clone(),
                    dependency_name: dependency_name.to_string(),
                    written_epoch,
                    observed_epoch,
                    converged: observed_epoch == Some(written_epoch),
                });
            }
        }
        rows
    }
}

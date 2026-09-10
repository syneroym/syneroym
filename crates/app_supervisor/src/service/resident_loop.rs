use super::*;

impl SupervisorService {
    /// One tick of the resident loop: every non-retired, non-paused
    /// instance, in `all_active`'s order, sequentially -- a slow instance
    /// delays later ones in this same pass, accepted for A5c since the
    /// per-instance lock (not a global one) is what keeps that a latency
    /// property rather than a correctness one.
    pub(super) async fn run_pass(&self) {
        let started =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let instances = match self.store.all_active() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read the supervisor's work list this pass");
                return;
            }
        };
        for state in instances {
            let lock = self.instance_lock(&state.app_instance_id);
            let _guard = lock.lock().await;
            self.reconcile_instance_pass(&state.app_instance_id).await;
        }
        // After the sweep, not before: every instance reconciled above must
        // read the *previous* sweep's start, which is the window
        // `schedule_grace_secs` sizes itself from.
        self.previous_pass_started_at.store(started, Ordering::Relaxed);
    }

    /// How far back a schedule may look for an occurrence it has not run
    /// yet (ADR-0023 §6). The floor is two poll intervals, which tolerates
    /// one dropped sweep; above that it is the real sweep-to-sweep gap,
    /// because a sweep that outruns its own interval is the ordinary case,
    /// not the exception -- every pass rebuilds an iroh client per
    /// substrate, so one unreachable substrate alone can push a sweep past
    /// two nominal intervals. Sizing the window from the *configured*
    /// interval instead would cut a hole between the last evaluation and
    /// the start of the window, and every occurrence landing in that hole
    /// would be dropped while the supervisor was awake the whole time --
    /// visible to an operator only as a watermark that keeps advancing
    /// while `last_run_at` never moves.
    ///
    /// A clamp is still needed above the honest watermark, since nothing
    /// runs while the process is down: without it, a supervisor started
    /// after a day off would fire one tick per schedule immediately. The
    /// gap this reads is measured inside one process and reset to zero by a
    /// restart, so downtime never widens the window.
    pub(super) fn schedule_grace_secs(&self, now: u64) -> u64 {
        let floor = 2 * self.poll_interval_secs;
        match self.previous_pass_started_at.load(Ordering::Relaxed) {
            0 => floor,
            previous => floor.max(now.saturating_sub(previous)),
        }
    }

    /// One instance's share of a loop pass: a health sweep (shared by the
    /// alert pass and, unless superseded, the reconcile below), then --
    /// for a non-superseded instance -- a **filtered** redeploy of only
    /// the services `Reconciler::compute_diff` says changed since the last
    /// fully-landed plan, plus any service the current sweep finds with no
    /// completed placement at all (the `missing_placement` case, which a
    /// content-unchanged diff cannot see on its own). One client set for
    /// the whole pass, closed once at the end.
    pub(super) async fn reconcile_instance_pass(&self, app_instance_id: &str) {
        let Some((state, plan, inventory, instance_id)) =
            self.load_active_instance_for_pass(app_instance_id)
        else {
            return;
        };
        self.recover_stuck_applying(&instance_id, app_instance_id);

        let landed =
            self.store.journal.get_completed_actions_for_instance(&instance_id).unwrap_or_default();

        let PassPlacements { expected, missing_placement, did_to_alias } =
            Self::resolve_pass_placements(&landed, &plan);

        let plan_aliases: BTreeSet<String> =
            Self::placed_aliases(&plan).unwrap_or_default().into_iter().collect();
        let connect_aliases = Self::connect_aliases_for_pass(&plan_aliases, &did_to_alias);
        let (clients, failed) = self.connect_best_effort(&connect_aliases, &inventory).await;
        // These used to be discarded entirely. An unreachable substrate
        // is already visible another way (the
        // health sweep reports it as a fault for a service placed
        // there), but an alias with no inventory entry or no credential
        // is a configuration problem the health sweep cannot see at
        // all, since it never gets far enough to try connecting.
        for (alias, reason) in &failed {
            tracing::warn!(
                app_instance_id,
                alias,
                reason,
                "failed to connect to a substrate this pass needs"
            );
        }

        let targets = Self::health_targets(&did_to_alias, &inventory, &clients);
        let report = health::poll_once(&targets, &expected).await;
        drop(targets);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

        let mut opened = self.record_pass_health(
            &instance_id,
            app_instance_id,
            &plan,
            &report,
            &missing_placement,
            now,
        );

        let diff = Reconciler::new(&self.store.journal).compute_diff(&plan);
        // A dependent member whose diff against the last active plan
        // changed *only* `resolved_dependencies` is a
        // membership change in one of its dependencies -- pushed via
        // `push_bindings`, not redeployed. Every other kind of change
        // (config, placement, ...) still takes the redeploy path.
        // A member whose diff changed *only* its
        // `schedule` is excluded the same way, but pushes nothing.
        // `classify_update_actions` is the same classifier an
        // operator-triggered apply uses (`apply_with_membership_pushes`),
        // so a loop pass and a `submit`/`force-reconcile` make the
        // identical redeploy-vs-push-vs-exclude call for the identical
        // diff.
        let (redeploy_exclusions, push_candidates) = diff
            .as_ref()
            .map(|d| Self::classify_update_actions(&landed, &d.actions))
            .unwrap_or_default();
        let diff_actions = diff.as_ref().map(|d| d.actions.as_slice()).unwrap_or_default();
        let needs_work =
            Self::redeploy_work_list(&missing_placement, diff_actions, &redeploy_exclusions);
        self.sync_orphaned_alerts(&instance_id, &plan, &landed, diff_actions, &mut opened);

        // Landed services the sweep just found `InstanceNotRunning` are
        // restart
        // candidates -- distinct from `needs_work` above, which never-
        // landed or content-changed services feed into instead. A
        // healthy service's own remediation bookkeeping resets here too,
        // so the next fault starts counting from zero.
        let restart_candidates = Self::restart_candidates(&report);
        for svc in report.services.iter().filter(|s| s.signal == Signal::Healthy) {
            let _ = self.store.clear_remediation(app_instance_id, &svc.member_ref().to_string());
        }

        // The fourth work-list. Its input is this pass's own health poll
        // -- `ServiceHealth` already carries the certificate's
        // issued/expires pair -- so renewal needs no poll and no cadence of
        // its own. Deduped against `needs_work` (a service about to go
        // through `apply_plan` gets a fresh certificate there, so renewing
        // it here would certify it twice in one pass) but deliberately
        // *not* against `restart_candidates`: a restart reloads the running
        // instance and touches no certificate, so a service under
        // remediation still needs its own renewal check.
        let revoked = self.store.revoked_placements(app_instance_id).unwrap_or_default();
        let renewal_candidates = Self::renewal_candidates(
            &report,
            &needs_work,
            &revoked,
            now,
            self.max_renewals_per_pass,
        );
        // The fifth work-list (ADR-0023 §6): every schedule
        // this instance's plan declares, evaluated against this pass's own
        // health report, over the grace window `schedule_grace_secs`
        // sizes from this supervisor's own sweep cadence.
        let declared_schedules: BTreeSet<String> =
            Self::declared_schedules(&plan).into_keys().collect();
        if let Err(e) = self.store.prune_schedule_states(app_instance_id, &declared_schedules) {
            tracing::warn!(
                app_instance_id,
                error = %e,
                "failed to drop the state of a schedule the plan no longer declares"
            );
        }
        let schedule_states = self.store.schedule_states(app_instance_id).unwrap_or_default();
        let schedule_decisions = Self::schedule_decisions(
            &plan,
            &schedule_states,
            &report,
            now,
            self.schedule_grace_secs(now),
        );
        // Members whose certificate renewed but whose
        // `restart-on-rotation` restart then failed. Independent of the
        // renewal work-list above -- these are no longer near-expiry, so
        // `renewal_candidates` will never see them again.
        let pending_rotation_restarts =
            self.store.pending_rotation_restarts(app_instance_id).unwrap_or_default();
        // D-A5d-9's clearing rule, the same recomputed-not-flagged shape
        // `Superseded` and `remediation.terminal` already use: a member the
        // substrate now reports with a healthy certificate window has no
        // stalled renewal, whatever an earlier pass raised.
        self.clear_settled_renewal_alerts(&instance_id, &report, now);
        self.publish_opened_alerts(app_instance_id, &opened).await;

        let held_max = Self::max_held_generation_from_clients(
            app_instance_id,
            &plan_aliases,
            &Self::actors_from_clients(&clients),
        )
        .await;
        let superseded = self
            .update_superseded_alert(&instance_id, app_instance_id, held_max, state.generation)
            .unwrap_or(false);

        // D-A5c-11: a superseded instance is skipped for every write this
        // pass (no deploy, no push, no restart) but was still polled for
        // health above.
        if superseded {
            self.last_reconciled.insert(app_instance_id.to_string(), now as i64);
            Self::shutdown_clients(clients.into_values()).await;
            return;
        }

        // The anchor refresh is evaluated every pass against a persisted
        // fact rather than on a timer of its own, so it -- unlike the three
        // work-lists -- always has something to check.
        if !needs_work.is_empty()
            || !restart_candidates.is_empty()
            || !renewal_candidates.is_empty()
            || !pending_rotation_restarts.is_empty()
            || !push_candidates.is_empty()
            || !schedule_decisions.is_empty()
            || self.anchor_writer.is_some()
            || self.tier1_writer.is_some()
        {
            self.apply_write_phase(WritePhase {
                instance_id: &instance_id,
                app_instance_id,
                plan: &plan,
                needs_work: &needs_work,
                restart_candidates: &restart_candidates,
                renewal_candidates: &renewal_candidates,
                pending_rotation_restarts: &pending_rotation_restarts,
                push_candidates: &push_candidates,
                schedule_decisions: &schedule_decisions,
                did_to_alias: &did_to_alias,
                clients: &clients,
                now,
            })
            .await;
        }
        self.last_reconciled.insert(app_instance_id.to_string(), now as i64);
        Self::shutdown_clients(clients.into_values()).await;
    }

    /// Reads and parses everything a pass needs about one instance, or
    /// returns `None` (after a `tracing::warn!`) when the instance should
    /// be skipped this pass: it is paused or retired, or one of the four
    /// stored fields does not read or parse. None of the four failures can
    /// raise a *stored* alert -- the failure is in reading the store or
    /// parsing what it returned, so there is no trustworthy instance state
    /// left to attach one to -- but the log line makes the drop observable
    /// rather than indistinguishable from an instance that was never
    /// submitted.
    fn load_active_instance_for_pass(
        &self,
        app_instance_id: &str,
    ) -> Option<(DesiredState, DeploymentPlan, SupervisorInventory, AppInstanceId)> {
        let Ok(Some(state)) = self.store.get(app_instance_id) else {
            tracing::warn!(
                app_instance_id,
                "failed to read this instance's desired state; skipping it this pass"
            );
            return None;
        };
        if state.paused || state.retired {
            return None;
        }
        let Ok(plan) = DeploymentPlan::from_json(&state.plan_json) else {
            tracing::warn!(
                app_instance_id,
                "stored plan-json does not parse as a DeploymentPlan; skipping this instance \
                 until it is resubmitted"
            );
            return None;
        };
        let Ok(inventory) = serde_json::from_str::<SupervisorInventory>(&state.inventory_json)
        else {
            tracing::warn!(
                app_instance_id,
                "stored inventory-json does not parse; skipping this instance until it is \
                 resubmitted"
            );
            return None;
        };
        let Ok(instance_id) = AppInstanceId::try_new(app_instance_id.to_string()) else {
            tracing::warn!(
                app_instance_id,
                "the stored app_instance_id itself is not a valid AppInstanceId; skipping this \
                 instance"
            );
            return None;
        };
        Some((state, plan, inventory, instance_id))
    }

    /// Nothing else ever moves a crashed-mid-apply record out of
    /// `Applying` -- `apply_with_clients` only ever updates one to
    /// `Active`/`Degraded` itself, from inside the same call that appended
    /// it. The per-instance lock this pass holds proves that call is gone,
    /// so a record still reading `Applying` here was abandoned by a process
    /// that exited between appending it and updating it. `Degraded` is the
    /// correct resting state for "we do not know whether this landed":
    /// `handle_status` would otherwise report `Applying` forever, past the
    /// point this pass's own diff has already re-derived and retried
    /// whatever was actually missing.
    fn recover_stuck_applying(&self, instance_id: &AppInstanceId, app_instance_id: &str) {
        if let Ok(Some(latest)) = self.store.journal.get_latest(instance_id)
            && latest.state == DeploymentState::Applying
            && let Err(e) = self.store.journal.update_state(latest.id, DeploymentState::Degraded)
        {
            tracing::warn!(
                app_instance_id,
                error = %e,
                "failed to recover a deployment record stuck in Applying"
            );
        }
    }

    /// Records this pass's health report and syncs the never-landed
    /// `InstanceNotRunning` alert for every planned service, returning the
    /// alerts newly opened. `SUPERVISOR_CERT_ALERT_POLICY` is the same
    /// constant `handle_status` passes. A `record_report` failure is
    /// logged and treated as "nothing opened" rather than propagated -- a
    /// pass must still do its reconcile work.
    fn record_pass_health(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        report: &health::HealthReport,
        missing_placement: &BTreeSet<String>,
        now: u64,
    ) -> Vec<(AlertKind, String)> {
        // The sentinel-keyed exemption `record_report`'s own cleanup needs
        // so it does not clear this alert every call -- see
        // `NEVER_LANDED_SUBSTRATE_DID`'s own doc.
        let extra_live_pairs: Vec<(String, String)> = missing_placement
            .iter()
            .map(|l_ref| (l_ref.clone(), NEVER_LANDED_SUBSTRATE_DID.to_string()))
            .collect();
        let mut opened = match health::record_report(
            &self.store.alerts,
            instance_id,
            report,
            now,
            &extra_live_pairs,
            SUPERVISOR_CERT_ALERT_POLICY,
        ) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(app_instance_id, error = %e, "failed to record this pass's health report");
                Vec::new()
            }
        };
        for svc in &plan.services {
            let l_ref = svc.member_ref().to_string();
            if missing_placement.contains(&l_ref) {
                if let Ok(true) = self.store.alerts.raise(
                    instance_id,
                    Some(&l_ref),
                    None,
                    NEVER_LANDED_SUBSTRATE_DID,
                    AlertKind::InstanceNotRunning,
                    "planned but never deployed; the supervisor holds no completed placement for \
                     this service",
                ) {
                    opened.push((AlertKind::InstanceNotRunning, l_ref));
                }
            } else {
                let _ = self.store.alerts.clear(
                    instance_id,
                    Some(&l_ref),
                    NEVER_LANDED_SUBSTRATE_DID,
                    AlertKind::InstanceNotRunning,
                );
            }
        }
        opened
    }

    /// Raises `OrphanedService` for every `Remove` action whose member is
    /// still running on its substrate (a plan-level removal is never
    /// undeployed here, only alerted on), and clears it for every member
    /// back in the current plan, whatever an older diff once said.
    fn sync_orphaned_alerts(
        &self,
        instance_id: &AppInstanceId,
        plan: &DeploymentPlan,
        landed: &[ActionRecord],
        diff_actions: &[ReconcileAction],
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        for action in diff_actions {
            let ReconcileAction::Remove(l_ref) = action else { continue };
            let l_ref_str = l_ref.to_string();
            if let Some(row) = deploy::current_placement(landed, &l_ref_str)
                && let Ok(true) = self.store.alerts.raise(
                    instance_id,
                    Some(&l_ref_str),
                    row.substrate_alias.as_deref(),
                    &row.substrate_did,
                    AlertKind::OrphanedService,
                    "dropped from the plan but still running on its substrate; not undeployed -- \
                     remove it by hand (`svc remove`) if that is intended",
                )
            {
                opened.push((AlertKind::OrphanedService, l_ref_str));
            }
        }
        for svc in &plan.services {
            let l_ref = svc.member_ref().to_string();
            if let Some(row) = deploy::current_placement(landed, &l_ref) {
                let _ = self.store.alerts.clear(
                    instance_id,
                    Some(&l_ref),
                    &row.substrate_did,
                    AlertKind::OrphanedService,
                );
            }
        }
    }

    /// The write half of a loop pass: mints, certifies, and applies only
    /// `needs_work`'s services, then attempts one bounded restart per
    /// `restart_candidates` entry. Extracted from `reconcile_instance_pass`
    /// so the re-read this opens with is directly testable against a
    /// `pause`/`retire` that lands between the health sweep and here --
    /// neither takes the per-instance lock a pass otherwise holds for its
    /// whole duration, so this is the one window that flag can still land
    /// in, and this fresh read is what closes it (a pause takes effect at
    /// the next write phase, not mid-write; this is that write phase's own
    /// boundary). Also picks up a generation `adopt` may have bumped since
    /// the pass's own early read.
    pub(super) async fn apply_write_phase(&self, phase: WritePhase<'_>) {
        let WritePhase {
            instance_id,
            app_instance_id,
            plan,
            needs_work,
            restart_candidates,
            renewal_candidates,
            pending_rotation_restarts,
            push_candidates,
            schedule_decisions,
            did_to_alias,
            clients,
            now,
        } = phase;
        let Ok(Some(fresh_state)) = self.store.get(app_instance_id) else { return };
        if fresh_state.paused || fresh_state.retired {
            return;
        }
        let generation = fresh_state.generation;

        // `redeployed_this_pass` is the signal the finding-A downgrade
        // below needs to tell "this pass's own `record_plan` might already
        // carry a push candidate's converged state" from "the last Active
        // record is stale and unrelated to this pass's push", which it
        // must not downgrade.
        let redeployed_this_pass =
            self.redeploy_needs_work(plan, app_instance_id, needs_work, clients, generation).await;

        let mut opened = Vec::new();

        let any_push_failed = self
            .push_membership_candidates(
                instance_id,
                app_instance_id,
                plan,
                push_candidates,
                did_to_alias,
                clients,
                generation,
                &mut opened,
            )
            .await;
        // Review round 2, finding A (same shape, narrower window here):
        // `record_plan_for_pass` keeps every push candidate's *new*
        // `resolved_dependencies` in `record_plan` unconditionally (it is
        // not a `needs_work` member, so nothing filters it out) -- so a
        // needs_work redeploy this same pass journals that push candidate
        // as already converged, before the push loop runs. If the push
        // then fails, the next pass's diff would read it as landed and
        // never retry. Gated on `redeployed_this_pass`: the ordinary case
        // (a push with no needs_work redeploy alongside it in the same
        // pass) journals nothing here at all, so the latest record is
        // whatever an earlier pass left -- unrelated to this push, and
        // must not be downgraded just because this pass's push failed.
        if any_push_failed
            && redeployed_this_pass
            && let Ok(Some(latest)) = self.store.journal.get_latest(instance_id)
            && latest.state == DeploymentState::Active
            && let Err(e) = self.store.journal.update_state(latest.id, DeploymentState::Degraded)
        {
            tracing::warn!(
                app_instance_id,
                error = %e,
                "failed to mark this pass's record Degraded after a binding push did not land"
            );
        }

        self.attempt_pass_restarts(
            instance_id,
            app_instance_id,
            restart_candidates,
            did_to_alias,
            clients,
            generation,
            now,
            &mut opened,
        )
        .await;

        self.renew_due_members(
            instance_id,
            app_instance_id,
            plan,
            renewal_candidates,
            did_to_alias,
            &Self::actors_from_clients(clients),
            fresh_state.generation,
            now,
            &mut opened,
        )
        .await;
        if !pending_rotation_restarts.is_empty() {
            self.retry_pending_rotation_restarts(
                instance_id,
                app_instance_id,
                plan,
                pending_rotation_restarts,
                did_to_alias,
                &Self::actors_from_clients(clients),
                fresh_state.generation,
                &mut opened,
            )
            .await;
        }
        self.refresh_due_master_anchors(plan, now).await;
        self.refresh_due_app_tier1_record(instance_id, &fresh_state, now, &mut opened).await;
        self.run_due_schedules(
            instance_id,
            app_instance_id,
            schedule_decisions,
            did_to_alias,
            &Self::actors_from_clients(clients),
            fresh_state.generation,
            now,
            &mut opened,
        )
        .await;
        self.publish_opened_alerts(app_instance_id, &opened).await;
    }

    /// Mints, certifies, and applies only the `needs_work` services this
    /// pass actually connected to a substrate for, and returns whether that
    /// apply landed cleanly. `resolve_targets` (deploy.rs) fails the whole
    /// `apply_plan` call closed if even one service has no built target --
    /// correct for `roymctl app deploy`'s all-or-nothing call, wrong here:
    /// a plan spanning two substrates where only one is reachable this pass
    /// must not block the service that *could* land. An unreachable service
    /// stays in `needs_work` and is picked up again next pass.
    ///
    /// What gets *journaled* as the new baseline is `record_plan_for_pass`,
    /// not the filtered subset -- see that function's own doc for why
    /// conflating the two makes the loop alternate forever instead of
    /// converging. The return is read from `apply_with_clients`'s own
    /// result, not from having reached the call: an `Err` can mean nothing
    /// was journaled this pass (a certify failure before the journal
    /// write), in which case the finding-A downgrade must not fire.
    async fn redeploy_needs_work(
        &self,
        plan: &DeploymentPlan,
        app_instance_id: &str,
        needs_work: &BTreeSet<String>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        generation: u64,
    ) -> bool {
        if needs_work.is_empty() {
            return false;
        }
        let mut filtered_plan = plan.clone();
        filtered_plan.services.retain(|s| {
            needs_work.contains(&s.member_ref().to_string())
                && s.substrate.as_ref().is_some_and(|a| clients.contains_key(a))
        });
        if filtered_plan.services.is_empty() {
            return false;
        }
        let record_plan = Self::record_plan_for_pass(plan, needs_work, clients);
        let (minted, masters) = match keys::mint_and_substitute(&mut filtered_plan, &self.vault)
            .await
        {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(app_instance_id, error = %e, "failed to mint members for this pass");
                return false;
            }
        };
        self.apply_with_clients(&filtered_plan, &record_plan, &masters, clients, generation, minted)
            .await
            .inspect_err(|e| {
                tracing::warn!(
                    app_instance_id,
                    error = %e,
                    "this pass's redeploy did not fully land"
                );
            })
            .is_ok()
    }

    /// Pushes bindings for every member whose only change is which DIDs a
    /// dependency resolves to, and returns whether any push did not land.
    /// A member this pass could not reach an actor for is queued directly
    /// through `enqueue_unreachable_push` (the DLQ's try-then-queue only
    /// fires *inside* an attempted call, and neither branch here gets that
    /// far) so a durably-offline substrate still converges once it returns.
    /// `Deferred` counts the same as an error: a redeploy landing in the
    /// same pass would otherwise journal this member's baseline as
    /// converged while the queue still holds stale content for it.
    #[allow(clippy::too_many_arguments)]
    async fn push_membership_candidates(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        push_candidates: &[(PlannedService, String)],
        did_to_alias: &BTreeMap<String, String>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        generation: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) -> bool {
        let mut any_push_failed = false;
        for (svc, substrate_did) in push_candidates {
            let Some(alias) = did_to_alias.get(substrate_did) else {
                self.enqueue_unreachable_push(
                    instance_id,
                    app_instance_id,
                    plan,
                    svc,
                    substrate_did,
                    generation,
                    "this pass has no known substrate alias for the member's landed DID",
                    opened,
                )
                .await;
                any_push_failed = true;
                continue;
            };
            let Some(client) = clients.get(&SubstrateAlias::new(alias.clone())) else {
                self.enqueue_unreachable_push(
                    instance_id,
                    app_instance_id,
                    plan,
                    svc,
                    substrate_did,
                    generation,
                    &format!("failed to connect to substrate alias '{alias}' this pass"),
                    opened,
                )
                .await;
                any_push_failed = true;
                continue;
            };
            let actor = self.durable_actor(
                client.clone(),
                app_instance_id,
                &svc.member_ref().to_string(),
                substrate_did,
            );
            match self
                .push_bindings(instance_id, plan, svc, substrate_did, &actor, generation, opened)
                .await
            {
                Ok(PushOutcome::Deferred) => any_push_failed = true,
                Ok(PushOutcome::Landed(_)) => {}
                Err(_) => any_push_failed = true,
            }
        }
        any_push_failed
    }

    /// One bounded restart attempt per landed-but-`InstanceNotRunning`
    /// service the sweep found -- skipping any whose substrate this pass
    /// could not name an alias for or connect to.
    #[allow(clippy::too_many_arguments)]
    async fn attempt_pass_restarts(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        restart_candidates: &[(String, String, String)],
        did_to_alias: &BTreeMap<String, String>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        generation: u64,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        for (logical_ref, service_id, substrate_did) in restart_candidates {
            let Some(alias) = did_to_alias.get(substrate_did) else { continue };
            let Some(client) = clients.get(&SubstrateAlias::new(alias.clone())) else { continue };
            let actor =
                self.durable_actor(client.clone(), app_instance_id, logical_ref, substrate_did);
            self.attempt_restart(
                instance_id,
                app_instance_id,
                logical_ref,
                service_id,
                substrate_did,
                &actor,
                generation,
                now,
                opened,
            )
            .await;
        }
    }

    /// The plan to journal as this pass's new baseline, as distinct from
    /// `filtered_plan`, the (possibly smaller) plan this pass actually
    /// deploys. Recording only the touched subset as `Active` made
    /// `Reconciler::compute_diff` -- which reads
    /// the *last* `Active` record wholesale -- forget every already-
    /// landed service the current pass did not happen to touch, so the
    /// next pass read it as missing and redeployed it, dropping today's
    /// subset out of its own new snapshot in turn: two services on two
    /// substrates alternate being redeployed forever instead of the loop
    /// converging. Keeps every service already believed landed
    /// (anything outside `needs_work`) plus whatever this pass is about
    /// to (re)land; excludes only a `needs_work` service with nowhere
    /// reachable to send it this pass, which genuinely has not landed.
    pub(super) fn record_plan_for_pass(
        plan: &DeploymentPlan,
        needs_work: &BTreeSet<String>,
        clients: &BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
    ) -> DeploymentPlan {
        let mut record_plan = plan.clone();
        record_plan.services.retain(|s| {
            !needs_work.contains(&s.member_ref().to_string())
                || s.substrate.as_ref().is_some_and(|a| clients.contains_key(a))
        });
        record_plan
    }

    /// Whether `old` and `new` (the same member, before and after a
    /// resubmit) differ only in which member DIDs a dependency resolves to
    /// -- a membership change in one of `new`'s dependencies, and nothing
    /// else about this member itself. `logical_ref` is
    /// already guaranteed equal: `Reconciler::diff_plans` matches `old` and
    /// `new` by `MemberRef`, which includes it. Also requires `schedule` to
    /// be unchanged: a simultaneous schedule edit must not
    /// classify as membership-only, or the schedule change is silently
    /// dropped from the plan this pass records.
    pub(super) fn only_resolved_dependencies_changed(
        old: &PlannedService,
        new: &PlannedService,
    ) -> bool {
        old.service_id == new.service_id
            && old.substrate == new.substrate
            && old.config == new.config
            && old.topology_mode == new.topology_mode
            && old.member_index == new.member_index
            && old.schedule == new.schedule
            && old.resolved_dependencies != new.resolved_dependencies
    }

    /// Whether `old` and `new` differ only in `schedule`.
    /// A schedule-only edit must not redeploy the service -- the
    /// substrate has no use for the change at all (`ServiceSpec.schedule`'s
    /// own doc) -- and has nothing to push either, since it names no
    /// substrate-visible fact.
    pub(super) fn only_schedule_changed(old: &PlannedService, new: &PlannedService) -> bool {
        old.service_id == new.service_id
            && old.substrate == new.substrate
            && old.config == new.config
            && old.topology_mode == new.topology_mode
            && old.member_index == new.member_index
            && old.resolved_dependencies == new.resolved_dependencies
            && old.schedule != new.schedule
    }

    /// Splits a diff's `Update` actions into (a) members no caller should
    /// redeploy this pass and (b) the subset of those that need a binding
    /// push. The two are not the same set: a member
    /// whose only change is its schedule must not be redeployed (the
    /// substrate has no use for the change) and has nothing to push
    /// either -- so it joins the exclusion set but never the push list.
    ///
    /// The asymmetry in the landed-placement check below is deliberate: a
    /// membership push needs a substrate to push *to*, so a never-landed
    /// member falls through to the redeploy path. A schedule exclusion
    /// needs no substrate, so it applies whether or not the member has
    /// landed.
    ///
    /// Shared by the loop's write phase (`reconcile_instance_pass`) and an
    /// operator-triggered apply (`apply_with_membership_pushes`, under
    /// `handle_submit`/`deploy_submission`) so both make the identical
    /// redeploy-vs-push-vs-exclude call for the identical diff -- fixing
    /// this classification for one path and not the other is exactly the
    /// gap an earlier review round found.
    pub(super) fn classify_update_actions(
        landed: &[ActionRecord],
        actions: &[ReconcileAction],
    ) -> (BTreeSet<String>, Vec<(PlannedService, String)>) {
        let mut redeploy_exclusions = BTreeSet::new();
        let mut push_candidates = Vec::new();
        for action in actions {
            if let ReconcileAction::Update { old, new } = action {
                let member_ref = new.member_ref().to_string();
                if Self::only_schedule_changed(old, new) {
                    redeploy_exclusions.insert(member_ref);
                    continue;
                }
                let landed_row = Self::only_resolved_dependencies_changed(old, new)
                    .then(|| deploy::current_placement(landed, &member_ref))
                    .flatten();
                if let Some(row) = landed_row {
                    redeploy_exclusions.insert(member_ref);
                    push_candidates.push(((**new).clone(), row.substrate_did.clone()));
                }
            }
        }
        (redeploy_exclusions, push_candidates)
    }

    /// The loop's redeploy work list (D-A5c-2/D-A5c-3/D-A5c-21): the diff's
    /// `Add` and `Update` actions -- a plan-level change -- plus
    /// `missing_placement`, a service the current sweep finds with no
    /// landed placement at all, which a content-unchanged diff against an
    /// older `Active` snapshot cannot see on its own (D-A5c-10's gap).
    /// `Remove` is not work: a plan-level removal is never undeployed here,
    /// only raised as `OrphanedService` by the caller.
    ///
    /// `redeploy_exclusions` comes from `classify_update_actions`, and this
    /// is the loop's half of applying it -- the half a test can reach
    /// without a substrate to deploy at. Its counterpart on the operator
    /// path is the `retain` in `apply_with_membership_pushes`.
    pub(super) fn redeploy_work_list(
        missing_placement: &BTreeSet<String>,
        actions: &[ReconcileAction],
        redeploy_exclusions: &BTreeSet<String>,
    ) -> BTreeSet<String> {
        let mut needs_work = missing_placement.clone();
        for action in actions {
            let member_ref = match action {
                ReconcileAction::Add(svc) => svc.member_ref().to_string(),
                ReconcileAction::Update { new, .. } => new.member_ref().to_string(),
                ReconcileAction::Remove(_) => continue,
            };
            if !redeploy_exclusions.contains(&member_ref) {
                needs_work.insert(member_ref);
            }
        }
        needs_work
    }

    /// Every schedule a plan declares, keyed by logical ref. A schedule is
    /// identical across a logical service's members, so the first member
    /// carrying one decides it for the whole group; `BTreeMap` keeps both
    /// the pass and the `schedules` listing in a deterministic order.
    /// Shared by the two so a schedule the pass acts on and a schedule the
    /// operator is shown can never be different sets.
    pub(super) fn declared_schedules(plan: &DeploymentPlan) -> BTreeMap<String, &ScheduleSpec> {
        let mut groups: BTreeMap<String, &ScheduleSpec> = BTreeMap::new();
        for svc in &plan.services {
            if let Some(sched) = &svc.schedule {
                groups.entry(svc.logical_ref.to_string()).or_insert(sched);
            }
        }
        groups
    }

    /// One bounded restart attempt for a landed-but-`InstanceNotRunning`
    /// service: refuses if this service's remediation is already terminal,
    /// or if it is still inside `restart_backoff_secs` of the last
    /// attempt; otherwise calls `SubstrateActor::restart`, records the
    /// attempt regardless of the call's own outcome (an attempt is an
    /// attempt -- the next sweep is what determines whether it worked),
    /// and raises `RemediationExhausted`, naming `force-reconcile` as the
    /// escape hatch, the moment `max_restart_attempts` is reached.
    /// Takes `Arc<dyn SubstrateActor>` so this is directly testable
    /// against a fake actor with no live substrate.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn attempt_restart(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        logical_ref: &str,
        service_id: &str,
        substrate_did: &str,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let now = now as i64;
        let state = self.store.remediation_state(app_instance_id, logical_ref).unwrap_or(None);
        if state.is_some_and(|s| s.terminal) {
            return;
        }
        if let Some(RemediationState { last_attempt_at: Some(last), .. }) = state
            && now.saturating_sub(last) < self.restart_backoff_secs as i64
        {
            return;
        }

        if let Err(e) = actor.restart(service_id.to_string(), generation).await {
            tracing::warn!(app_instance_id, logical_ref, error = %e, "restart attempt failed");
        }

        let attempts = match self.store.record_restart_attempt(app_instance_id, logical_ref, now) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(
                    app_instance_id,
                    logical_ref,
                    error = %e,
                    "failed to record this restart attempt"
                );
                return;
            }
        };
        if attempts < self.max_restart_attempts {
            return;
        }
        if let Err(e) = self.store.mark_remediation_terminal(app_instance_id, logical_ref) {
            tracing::warn!(
                app_instance_id,
                logical_ref,
                error = %e,
                "failed to mark remediation terminal"
            );
            return;
        }
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(logical_ref),
            None,
            substrate_did,
            AlertKind::RemediationExhausted,
            &format!(
                "bounded restart exhausted after {attempts} attempts with no confirmed recovery; \
                 run `supervisor force-reconcile` to try again"
            ),
        ) {
            opened.push((AlertKind::RemediationExhausted, logical_ref.to_string()));
        }
    }

    /// Services this pass's sweep reported `InstanceNotRunning` **and**
    /// landed (a real `substrate_did`) -- restart candidates. Deliberately
    /// excludes `ProbeFailing` (an author-declared assertion, not a
    /// substrate-verified fact -- alert only) and `SubstrateUnreachable`
    /// (restarting cannot fix a substrate that did not answer). Its own
    /// function so this filter is directly testable against a synthetic
    /// `HealthReport`, with no live substrate.
    pub(super) fn restart_candidates(
        report: &health::HealthReport,
    ) -> Vec<(String, String, String)> {
        report
            .services
            .iter()
            .filter(|s| {
                matches!(s.signal, Signal::InstanceNotRunning(_)) && !s.substrate_did.is_empty()
            })
            .map(|s| (s.member_ref().to_string(), s.service_id.clone(), s.substrate_did.clone()))
            .collect()
    }
}

use super::*;

impl SupervisorService {
    /// One tick of the resident loop: every non-retired, non-paused
    /// instance, in `all_active`'s order, sequentially -- a slow instance
    /// delays later ones in this same pass, accepted for A5c since the
    /// per-instance lock (not a global one) is what keeps that a latency
    /// property rather than a correctness one.
    pub(in crate::service) async fn run_pass(&self) {
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
    pub(in crate::service) fn schedule_grace_secs(&self, now: u64) -> u64 {
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
    pub(in crate::service) async fn reconcile_instance_pass(&self, app_instance_id: &str) {
        let Some((state, plan, inventory, instance_id)) =
            self.load_active_instance_for_pass(app_instance_id)
        else {
            return;
        };
        self.recover_stuck_applying(&instance_id, app_instance_id);

        let landed =
            self.store.journal.get_completed_actions_for_instance(&instance_id).unwrap_or_default();

        let (placements, plan_aliases, clients, report) =
            self.connect_and_poll_health(app_instance_id, &landed, &plan, &inventory).await;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

        let mut opened = self.record_pass_health(
            &instance_id,
            app_instance_id,
            &plan,
            &report,
            &placements.missing_placement,
            now,
        );

        let (needs_work, push_candidates) = self.compute_redeploy_and_sync(
            &instance_id,
            &plan,
            &landed,
            &placements.missing_placement,
            &mut opened,
        );

        let restart_candidates = Self::restart_candidates(&report);
        for svc in report.services.iter().filter(|s| s.signal == Signal::Healthy) {
            let _ = self.store.clear_remediation(app_instance_id, &svc.member_ref().to_string());
        }

        let (renewal_candidates, schedule_decisions, pending_rotation_restarts) =
            self.prepare_renewal_and_schedules(app_instance_id, &plan, &report, &needs_work, now);

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

        if superseded {
            self.last_reconciled.insert(app_instance_id.to_string(), now as i64);
            Self::shutdown_clients(clients.into_values()).await;
            return;
        }

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
                did_to_alias: &placements.did_to_alias,
                clients: &clients,
                now,
            })
            .await;
        }
        self.last_reconciled.insert(app_instance_id.to_string(), now as i64);
        Self::shutdown_clients(clients.into_values()).await;
    }

    async fn connect_and_poll_health(
        &self,
        app_instance_id: &str,
        landed: &[ActionRecord],
        plan: &DeploymentPlan,
        inventory: &SupervisorInventory,
    ) -> (
        PassPlacements,
        BTreeSet<String>,
        BTreeMap<SubstrateAlias, Arc<SyneroymClient>>,
        health::HealthReport,
    ) {
        let placements = Self::resolve_pass_placements(landed, plan);
        let plan_aliases: BTreeSet<String> =
            Self::placed_aliases(plan).unwrap_or_default().into_iter().collect();
        let connect_aliases =
            Self::connect_aliases_for_pass(&plan_aliases, &placements.did_to_alias);
        let (clients, failed) = self.connect_best_effort(&connect_aliases, inventory).await;
        for (alias, reason) in &failed {
            tracing::warn!(
                app_instance_id,
                alias,
                reason,
                "failed to connect to a substrate this pass needs"
            );
        }

        let targets = Self::health_targets(&placements.did_to_alias, inventory, &clients);
        let report = health::poll_once(&targets, &placements.expected).await;
        drop(targets);
        (placements, plan_aliases, clients, report)
    }

    fn compute_redeploy_and_sync(
        &self,
        instance_id: &AppInstanceId,
        plan: &DeploymentPlan,
        landed: &[ActionRecord],
        missing_placement: &BTreeSet<String>,
        opened: &mut Vec<(AlertKind, String)>,
    ) -> (BTreeSet<String>, Vec<(PlannedService, String)>) {
        let diff = Reconciler::new(&self.store.journal).compute_diff(plan);
        let (redeploy_exclusions, push_candidates) = diff
            .as_ref()
            .map(|d| Self::classify_update_actions(landed, &d.actions))
            .unwrap_or_default();
        let diff_actions = diff.as_ref().map(|d| d.actions.as_slice()).unwrap_or_default();
        let needs_work =
            Self::redeploy_work_list(missing_placement, diff_actions, &redeploy_exclusions);
        self.sync_orphaned_alerts(instance_id, plan, landed, diff_actions, opened);
        (needs_work, push_candidates)
    }

    fn prepare_renewal_and_schedules(
        &self,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        report: &health::HealthReport,
        needs_work: &BTreeSet<String>,
        now: u64,
    ) -> (Vec<RenewalCandidate>, Vec<ScheduleDecision>, BTreeSet<String>) {
        let revoked = self.store.revoked_placements(app_instance_id).unwrap_or_default();
        let renewal_candidates =
            Self::renewal_candidates(report, needs_work, &revoked, now, self.max_renewals_per_pass);
        let declared_schedules: BTreeSet<String> =
            Self::declared_schedules(plan).into_keys().collect();
        if let Err(e) = self.store.prune_schedule_states(app_instance_id, &declared_schedules) {
            tracing::warn!(
                app_instance_id,
                error = %e,
                "failed to drop the state of a schedule the plan no longer declares"
            );
        }
        let schedule_states = self.store.schedule_states(app_instance_id).unwrap_or_default();
        let schedule_decisions = Self::schedule_decisions(
            plan,
            &schedule_states,
            report,
            now,
            self.schedule_grace_secs(now),
        );
        let pending_rotation_restarts =
            self.store.pending_rotation_restarts(app_instance_id).unwrap_or_default();
        (renewal_candidates, schedule_decisions, pending_rotation_restarts)
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
}

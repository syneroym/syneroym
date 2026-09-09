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
        // Each of these four reads used to fail silently -- no log, no
        // alert -- which drops the instance out
        // of every future pass with nothing anywhere to say why. None of
        // the four can raise a *stored* alert (the failure is in reading
        // the store, or in parsing what it just returned, so there is no
        // instance state left to attach one to that is any more trustworthy
        // than the log line itself), but a `tracing::warn!` at least makes
        // the drop observable instead of indistinguishable from an
        // instance that was never submitted.
        let Ok(Some(state)) = self.store.get(app_instance_id) else {
            tracing::warn!(
                app_instance_id,
                "failed to read this instance's desired state; skipping it this pass"
            );
            return;
        };
        if state.paused || state.retired {
            return;
        }
        let Ok(plan) = DeploymentPlan::from_json(&state.plan_json) else {
            tracing::warn!(
                app_instance_id,
                "stored plan-json does not parse as a DeploymentPlan; skipping this instance \
                 until it is resubmitted"
            );
            return;
        };
        let Ok(inventory) = serde_json::from_str::<SupervisorInventory>(&state.inventory_json)
        else {
            tracing::warn!(
                app_instance_id,
                "stored inventory-json does not parse; skipping this instance until it is \
                 resubmitted"
            );
            return;
        };
        let Ok(instance_id) = AppInstanceId::try_new(app_instance_id.to_string()) else {
            tracing::warn!(
                app_instance_id,
                "the stored app_instance_id itself is not a valid AppInstanceId; skipping this \
                 instance"
            );
            return;
        };

        // Review finding A-7: nothing else ever moves a crashed-mid-apply
        // record out of `Applying` -- `apply_with_clients` only ever
        // updates one to `Active`/`Degraded` itself, from inside the same
        // call that appended it. The per-instance lock this pass holds
        // proves that call is gone: a second apply for this instance
        // cannot be in flight while we hold the lock, so a record still
        // reading `Applying` here was abandoned by a process that exited
        // between appending it and updating it. `Degraded` is the correct
        // resting state for "we do not know whether this landed" --
        // `handle_status` would otherwise report
        // `Applying` forever, past the point this pass's own diff (which
        // reads completed action rows, not this record's state) has
        // already re-derived and retried whatever was actually missing.
        if let Ok(Some(latest)) = self.store.journal.get_latest(&instance_id)
            && latest.state == DeploymentState::Applying
            && let Err(e) = self.store.journal.update_state(latest.id, DeploymentState::Degraded)
        {
            tracing::warn!(
                app_instance_id,
                error = %e,
                "failed to recover a deployment record stuck in Applying"
            );
        }

        let landed =
            self.store.journal.get_completed_actions_for_instance(&instance_id).unwrap_or_default();

        let mut expected = Vec::new();
        let mut missing_placement: BTreeSet<String> = BTreeSet::new();
        let mut did_to_alias: BTreeMap<String, String> = BTreeMap::new();
        for svc in &plan.services {
            match deploy::current_placement(&landed, &svc.member_ref().to_string()) {
                None => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: String::new(),
                        substrate_did: String::new(),
                        member_index: svc.member_index,
                    });
                    missing_placement.insert(svc.member_ref().to_string());
                }
                Some(row) => {
                    expected.push(ExpectedService {
                        logical_ref: svc.logical_ref.clone(),
                        service_id: svc.service_id.to_string(),
                        substrate_did: row.substrate_did.clone(),
                        member_index: svc.member_index,
                    });
                    if let Some(alias) = &row.substrate_alias {
                        did_to_alias.insert(row.substrate_did.clone(), alias.clone());
                    }
                }
            }
        }

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

        let mut targets: BTreeMap<String, HealthTarget> = BTreeMap::new();
        for (did, alias) in &did_to_alias {
            if !inventory.contains_key(alias) {
                continue;
            }
            let query: Arc<dyn StatusQuery> = match clients.get(&SubstrateAlias::new(alias.clone()))
            {
                Some(c) => c.clone() as Arc<dyn StatusQuery>,
                None => Arc::new(UnreachableQuery(format!(
                    "failed to connect to substrate alias '{alias}'"
                ))),
            };
            targets.insert(
                did.clone(),
                HealthTarget {
                    alias: Some(SubstrateAlias::new(alias.clone())),
                    substrate_did: did.clone(),
                    query,
                },
            );
        }

        let report = health::poll_once(&targets, &expected).await;
        drop(targets);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

        // D-A5c-10, the same sentinel-keyed alert `handle_status` raises.
        let extra_live_pairs: Vec<(String, String)> = missing_placement
            .iter()
            .map(|l_ref| (l_ref.clone(), NEVER_LANDED_SUBSTRATE_DID.to_string()))
            .collect();
        // D-A5d-9: `SUPERVISOR_CERT_ALERT_POLICY`'s own doc explains why.
        let mut opened = match health::record_report(
            &self.store.alerts,
            &instance_id,
            &report,
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
                    &instance_id,
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
                    &instance_id,
                    Some(&l_ref),
                    NEVER_LANDED_SUBSTRATE_DID,
                    AlertKind::InstanceNotRunning,
                );
            }
        }
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
        let needs_work = Self::redeploy_work_list(
            &missing_placement,
            diff.as_ref().map(|d| d.actions.as_slice()).unwrap_or_default(),
            &redeploy_exclusions,
        );
        // `Remove` is the one action the work list above ignores: a
        // plan-level removal is never undeployed here, only alerted on.
        for action in diff.iter().flat_map(|d| &d.actions) {
            let ReconcileAction::Remove(l_ref) = action else { continue };
            let l_ref_str = l_ref.to_string();
            if let Some(row) = deploy::current_placement(&landed, &l_ref_str)
                && let Ok(true) = self.store.alerts.raise(
                    &instance_id,
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
        // A member back in the current plan cannot be orphaned this
        // pass, regardless of what an older diff once said.
        for svc in &plan.services {
            let l_ref = svc.member_ref().to_string();
            if let Some(row) = deploy::current_placement(&landed, &l_ref) {
                let _ = self.store.alerts.clear(
                    &instance_id,
                    Some(&l_ref),
                    &row.substrate_did,
                    AlertKind::OrphanedService,
                );
            }
        }

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

        // Set only when `apply_with_clients` below is actually called this
        // pass -- the signal the finding-A downgrade further down needs to
        // tell "this pass's own record_plan might already carry a push
        // candidate's converged state" from "the last Active record is
        // stale and unrelated to this pass's push", which it must not
        // downgrade.
        let mut redeployed_this_pass = false;
        if !needs_work.is_empty() {
            let mut filtered_plan = plan.clone();
            // `resolve_targets` (deploy.rs) fails the *whole* `apply_plan`
            // call closed if even one service in the plan it is given has
            // no built target -- correct for `roymctl app deploy`'s own
            // all-or-nothing call, wrong here: a plan spanning two
            // substrates where only one is reachable this pass must not
            // block the service that *could* land. Only
            // services whose alias this pass actually connected to are
            // included; an unreachable one stays in `needs_work` (nothing
            // landed for it) and is picked up again next pass.
            filtered_plan.services.retain(|s| {
                needs_work.contains(&s.member_ref().to_string())
                    && s.substrate.as_ref().is_some_and(|a| clients.contains_key(a))
            });
            if !filtered_plan.services.is_empty() {
                // What gets *applied* this pass is deliberately narrowed
                // to `filtered_plan`, but what gets
                // *journaled* as the new baseline must not be -- diffing
                // future passes against a snapshot that only ever holds
                // this pass's touched subset drops every untouched,
                // already-landed service out of the baseline, so the next
                // pass reads it as missing and redeploys it, which then
                // drops today's subset out in turn. The loop alternates
                // forever instead of converging. `record_plan` carries
                // every service this supervisor still believes landed
                // (everything outside `needs_work`) plus whatever this
                // pass is about to (re)land, and excludes only a
                // `needs_work` service still unreachable this pass, which
                // genuinely has not landed.
                let record_plan = Self::record_plan_for_pass(plan, needs_work, clients);
                match keys::mint_and_substitute(&mut filtered_plan, &self.vault).await {
                    Ok((minted, masters)) => {
                        // Set from the call's own result, not from having
                        // reached this arm -- mirrors `apply_result_is_ok`
                        // in `apply_with_membership_pushes` and for the
                        // same reason: `apply_with_clients` returning `Err`
                        // can mean nothing was journaled this pass at all
                        // (a certify failure before the journal write), in
                        // which case `redeployed_this_pass` must stay
                        // false, or `Degraded` was already journaled
                        // instead of `Active`, in which case the finding-A
                        // downgrade below is a harmless no-op either way.
                        redeployed_this_pass = self
                            .apply_with_clients(
                                &filtered_plan,
                                &record_plan,
                                &masters,
                                clients,
                                fresh_state.generation,
                                minted,
                            )
                            .await
                            .inspect_err(|e| {
                                tracing::warn!(
                                    app_instance_id,
                                    error = %e,
                                    "this pass's redeploy did not fully land"
                                );
                            })
                            .is_ok();
                    }
                    Err(e) => tracing::warn!(
                        app_instance_id,
                        error = %e,
                        "failed to mint members for this pass"
                    ),
                }
            }
        }

        let mut opened = Vec::new();

        // Every member whose only change is which DIDs a dependency
        // resolves to gets a binding push instead of the
        // redeploy above -- an unreachable member this pass simply retries
        // next pass, since `resolved_dependencies` still disagrees with
        // what was last pushed.
        let mut any_push_failed = false;
        for (svc, substrate_did) in push_candidates {
            // A dependent this pass could not even connect to used to be
            // dropped here with no alert and no `opened` entry, so
            // `BindingConflict` was never set and `Degraded` never derived
            // from it -- indistinguishable from "nothing to push". Raised
            // through the same alert `write_bindings_at_epoch` itself
            // failing would raise, so the operator sees the same row
            // either way.
            //
            // Raising the alert and moving on used to be the whole story
            // here, which left a substrate this pass could
            // not even reach with nothing durable behind it -- the DLQ's
            // try-then-queue only fires *inside* an attempted call
            // (`DurableActor::write_bindings`), and neither branch below
            // gets far enough to make one. `enqueue_unreachable_push`
            // queues the write directly so a substrate that is durably
            // offline, not merely flaky mid-call, still converges once it
            // returns.
            let Some(alias) = did_to_alias.get(substrate_did) else {
                self.enqueue_unreachable_push(
                    instance_id,
                    app_instance_id,
                    plan,
                    svc,
                    substrate_did,
                    fresh_state.generation,
                    "this pass has no known substrate alias for the member's landed DID",
                    &mut opened,
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
                    fresh_state.generation,
                    &format!("failed to connect to substrate alias '{alias}' this pass"),
                    &mut opened,
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
            // `Deferred` means the push did not land this pass -- it must
            // count the same as an error here, or a redeploy landing in
            // the same pass would journal this member's new baseline as
            // converged while the queue still holds stale content for it.
            // Distinct from `Landed` with zero outcomes (every dependency
            // was just removed from this member's manifest), which is a
            // real, converged success, not deferred -- an earlier version
            // of this match used an empty `Vec` as the deferred sentinel,
            // which that case collided with.
            match self
                .push_bindings(
                    instance_id,
                    plan,
                    svc,
                    substrate_did,
                    &actor,
                    fresh_state.generation,
                    &mut opened,
                )
                .await
            {
                Ok(PushOutcome::Deferred) => any_push_failed = true,
                Ok(PushOutcome::Landed(_)) => {}
                Err(_) => any_push_failed = true,
            }
        }
        // Review round 2, finding A (same shape, narrower window here):
        // `record_plan_for_pass` above keeps every push candidate's *new*
        // `resolved_dependencies` in `record_plan` unconditionally (it is
        // not a `needs_work` member, so nothing filters it out) -- so a
        // needs_work redeploy this same pass journals that push candidate
        // as already converged, before this loop ever runs. If the push
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
                fresh_state.generation,
                now,
                &mut opened,
            )
            .await;
        }

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

use super::*;

impl SupervisorService {
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
    pub(in crate::service) async fn apply_write_phase(&self, phase: WritePhase<'_>) {
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
    pub(in crate::service) fn record_plan_for_pass(
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
}

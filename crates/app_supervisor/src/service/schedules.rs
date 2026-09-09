use super::*;

impl SupervisorService {
    /// The selection rule for one schedule this pass, pure over the pass's
    /// own inputs -- testable with a fixed
    /// `now`, no vault, no client, no store, the same reason
    /// `renewal_candidates` is pure.
    ///
    /// No `landed` argument: `health::ServiceHealth` already carries
    /// `substrate_did` and `member_index` from this pass's own report, and
    /// a member with `Signal::Healthy` is by definition one the sweep
    /// reached on a real substrate. Reading the placement from the report
    /// keeps this a pure fold over one input rather than a join across two
    /// that could disagree.
    pub(super) fn schedule_decisions(
        plan: &DeploymentPlan,
        states: &BTreeMap<String, ScheduleState>,
        report: &health::HealthReport,
        now: u64,
        grace_secs: u64,
    ) -> Vec<ScheduleDecision> {
        let groups = Self::declared_schedules(plan);
        let mut decisions = Vec::with_capacity(groups.len());
        for (l_ref, sched) in groups {
            let Ok(cron) = sched.parsed() else {
                // A plan that got past validation with a bad cron can only
                // come from a hand-edited submission. Watermark and move
                // on; the failure is reported by the alert the run would
                // have raised.
                decisions.push(ScheduleDecision::Watermark { logical_ref: l_ref });
                continue;
            };
            let Some(state) = states.get(&l_ref) else {
                // First sight: created with `evaluated_at = now` and no
                // run, so a schedule never fires for the past.
                decisions.push(ScheduleDecision::Watermark { logical_ref: l_ref });
                continue;
            };
            let window_start =
                u64::try_from(state.evaluated_at).unwrap_or(0).max(now.saturating_sub(grace_secs));
            // Anything other than a definite yes -- including an
            // evaluation error -- is treated exactly as the parse failure
            // above: watermark and move on.
            if !matches!(has_occurrence_in(&cron, window_start, now), Ok(true)) {
                decisions.push(ScheduleDecision::Watermark { logical_ref: l_ref });
                continue;
            }

            // The report is the single source for "which members are
            // runnable and where they are": a `Healthy` signal carries the
            // substrate that answered.
            let mut healthy: Vec<&health::ServiceHealth> = report
                .services
                .iter()
                .filter(|h| h.logical_ref.to_string() == l_ref && h.signal == Signal::Healthy)
                .collect();
            healthy.sort_by_key(|h| h.member_index);
            if healthy.is_empty() {
                // A skipped tick, not a late one.
                decisions.push(ScheduleDecision::Watermark { logical_ref: l_ref });
                continue;
            }

            // Round-robin: the first member strictly after the last one
            // used, wrapping. A member that has gone away simply drops out
            // of the ring. `None` -- never run -- sorts below every index,
            // so the first tick starts at the lowest healthy member rather
            // than skipping it.
            let pick = healthy
                .iter()
                .find(|h| Some(h.member_index) > state.last_member_index)
                .copied()
                .unwrap_or(healthy[0]);
            decisions.push(ScheduleDecision::Run {
                logical_ref: l_ref,
                service_id: pick.service_id.clone(),
                substrate_did: pick.substrate_did.clone(),
                member_index: pick.member_index,
                schedule: sched.clone(),
            });
        }
        decisions
    }

    /// Runs every due schedule and advances every watermark, in the order
    /// `schedule_decisions` produced. Never touches the
    /// outbox: a scheduled run is never queued (ADR-0023 §3) --
    /// the intent expires, and the next tick is a better retry than a
    /// delivery hours later. `actors` is built with `actors_from_clients`,
    /// not `durable_actor`, for exactly that reason -- the same call
    /// `renew_due_members` already makes.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_due_schedules(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        decisions: &[ScheduleDecision],
        did_to_alias: &BTreeMap<String, String>,
        actors: &BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>>,
        generation: u64,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let now_i64 = now as i64;
        for decision in decisions {
            match decision {
                ScheduleDecision::Watermark { logical_ref } => {
                    if let Err(e) =
                        self.store.record_schedule_evaluated(app_instance_id, logical_ref, now_i64)
                    {
                        tracing::warn!(
                            app_instance_id,
                            logical_ref,
                            error = %e,
                            "failed to advance a schedule's watermark"
                        );
                    }
                }
                ScheduleDecision::Run {
                    logical_ref,
                    service_id,
                    substrate_did,
                    member_index,
                    schedule,
                } => {
                    self.run_one_schedule(
                        instance_id,
                        app_instance_id,
                        logical_ref,
                        service_id,
                        substrate_did,
                        *member_index,
                        schedule,
                        did_to_alias,
                        actors,
                        generation,
                        now_i64,
                        opened,
                    )
                    .await;
                }
            }
        }
    }

    /// One due schedule's tick: resolves the picked member's actor, records
    /// the run's start *before* the call (a supervisor that dies inside the
    /// call must skip this tick on restart, not repeat it), then runs it
    /// under the manifest's own timeout budget and records the outcome. The
    /// two "target not reachable" early returns advance the watermark
    /// rather than leaving it -- the tick's window has passed, which is a
    /// documented cost, not a delivery to retry.
    #[allow(clippy::too_many_arguments)]
    async fn run_one_schedule(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        logical_ref: &str,
        service_id: &str,
        substrate_did: &str,
        member_index: u32,
        schedule: &ScheduleSpec,
        did_to_alias: &BTreeMap<String, String>,
        actors: &BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>>,
        generation: u64,
        now_i64: i64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let Some(alias) = did_to_alias.get(substrate_did) else {
            let _ = self.store.record_schedule_evaluated(app_instance_id, logical_ref, now_i64);
            return;
        };
        let Some(actor) = actors.get(&SubstrateAlias::new(alias.clone())) else {
            let _ = self.store.record_schedule_evaluated(app_instance_id, logical_ref, now_i64);
            return;
        };

        if let Err(e) =
            self.store.record_schedule_started(app_instance_id, logical_ref, now_i64, member_index)
        {
            tracing::warn!(
                app_instance_id,
                logical_ref,
                error = %e,
                "failed to record a scheduled run's start; skipping this tick"
            );
            return;
        }

        let budget =
            Duration::from_millis(u64::from(schedule.timeout_ms)).min(SCHEDULED_RUN_CEILING);
        let outcome = tokio::time::timeout(
            budget,
            actor.run_scheduled(
                service_id.to_string(),
                generation,
                schedule.interface.to_string(),
                schedule.method.clone(),
                schedule.params.clone(),
            ),
        )
        .await;
        self.record_scheduled_run_outcome(
            instance_id,
            app_instance_id,
            logical_ref,
            substrate_did,
            schedule,
            budget,
            outcome,
            opened,
        );
    }

    /// Records a scheduled run's result and syncs its `ScheduledRunFailed`
    /// alert: a clean run clears it, a failure or a timeout raises it with
    /// a detail naming the substrate that ran the tick. The alert is keyed
    /// on `SCHEDULE_SUBSTRATE_DID`, never the real DID -- see that
    /// constant's own doc.
    #[allow(clippy::too_many_arguments)]
    fn record_scheduled_run_outcome(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        logical_ref: &str,
        substrate_did: &str,
        schedule: &ScheduleSpec,
        budget: Duration,
        outcome: Result<Result<(), String>, tokio::time::error::Elapsed>,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let detail = match outcome {
            Ok(Ok(())) => {
                let _ = self.store.record_schedule_outcome(app_instance_id, logical_ref, None);
                let _ = self.store.alerts.clear(
                    instance_id,
                    Some(logical_ref),
                    SCHEDULE_SUBSTRATE_DID,
                    AlertKind::ScheduledRunFailed,
                );
                return;
            }
            Ok(Err(e)) => format!(
                "scheduled run of '{}/{}' failed on substrate '{substrate_did}': {e}",
                schedule.interface, schedule.method
            ),
            Err(_elapsed) => format!(
                "scheduled run of '{}/{}' on substrate '{substrate_did}' timed out after {}ms",
                schedule.interface,
                schedule.method,
                budget.as_millis()
            ),
        };
        let _ = self.store.record_schedule_outcome(app_instance_id, logical_ref, Some(&detail));
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(logical_ref),
            None,
            SCHEDULE_SUBSTRATE_DID,
            AlertKind::ScheduledRunFailed,
            &detail,
        ) {
            opened.push((AlertKind::ScheduledRunFailed, logical_ref.to_string()));
        }
    }
}

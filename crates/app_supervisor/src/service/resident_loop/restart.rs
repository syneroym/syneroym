use super::*;

impl SupervisorService {
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
    pub(in crate::service) async fn attempt_restart(
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
    pub(in crate::service) fn restart_candidates(
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

use super::*;

impl SupervisorService {
    /// The placed members whose installed certificate is inside its
    /// near-expiry window this pass, minus the two exclusions D-A5d-12
    /// names and capped at `max_renewals_per_pass`.
    ///
    /// A pure function of the pass's own health report, so the whole
    /// selection rule is testable with no vault, no client, and no store.
    /// The near-expiry decision itself is `is_near_expiry_parts` -- the
    /// same 25%-of-lifetime definition the substrate's own sweep uses, so
    /// the two cannot disagree about what "due" means.
    pub(super) fn renewal_candidates(
        report: &health::HealthReport,
        needs_work: &BTreeSet<String>,
        revoked: &BTreeSet<String>,
        now: u64,
        cap: u32,
    ) -> Vec<RenewalCandidate> {
        let mut candidates: Vec<RenewalCandidate> = report
            .services
            .iter()
            .filter(|svc| {
                let l_ref = svc.member_ref().to_string();
                !needs_work.contains(&l_ref) && !revoked.contains(&l_ref)
            })
            .filter_map(|svc| {
                let issued = svc.instance_certificate_issued_at?;
                let expires = svc.instance_certificate_expires_at?;
                is_near_expiry_parts(issued, expires, now).then(|| RenewalCandidate {
                    member_ref: svc.member_ref().to_string(),
                    service_name: svc.logical_ref.service_name.to_string(),
                    service_id: svc.service_id.clone(),
                    substrate_did: svc.substrate_did.clone(),
                    expires_at: expires,
                    member_index: svc.member_index,
                })
            })
            .collect();
        // Report order (a `BTreeMap` over substrate DID,
        // then plan order) has no relation to urgency, so the cap used to
        // keep whichever members happened to sort first -- a member whose
        // renewal keeps failing stays near-expiry and occupies the same
        // slot every pass, starving everything past the cap. Sorted by
        // `expires_at` ascending first, the cap always keeps the most
        // urgent members.
        candidates.sort_by_key(|c| c.expires_at);
        candidates.truncate(cap as usize);
        candidates
    }

    /// Mint, install, and (if the plan says so) rotate, once per due
    /// member.
    ///
    /// The vault check comes first and covers the whole work-list:
    /// `kek_is_loaded` is a cheap, no-I/O read, and a locked vault means
    /// *every* mint below would fail identically. Skipping the list rather
    /// than the pass is deliberate -- health, remediation, and the anchor
    /// refresh all continue, since none of them opens the vault.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn renew_due_members(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        candidates: &[RenewalCandidate],
        did_to_alias: &BTreeMap<String, String>,
        actors: &BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>>,
        generation: u64,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        if candidates.is_empty() {
            return;
        }
        if !self.vault.kek_is_loaded() {
            for candidate in candidates {
                self.raise_vault_locked(instance_id, candidate, opened);
            }
            return;
        }

        for candidate in candidates {
            let Some(alias) = did_to_alias.get(&candidate.substrate_did) else { continue };
            let Some(actor) = actors.get(&SubstrateAlias::new(alias.clone())) else { continue };
            if let Err(failure) = self.renew_one_member(plan, candidate, actor, generation).await {
                match failure {
                    // D-A5d-17: one root cause, one alert kind. A vault
                    // locked between `kek_is_loaded` above and the mint
                    // below is the same condition, found later, and must
                    // not surface under a different name for it.
                    RenewalFailure::VaultLocked => {
                        self.raise_vault_locked(instance_id, candidate, opened);
                    }
                    RenewalFailure::Step { step, error } => {
                        self.raise_renewal_stalled(
                            instance_id,
                            candidate,
                            &format!(
                                "renewal {step} for '{}' failed: {error}",
                                candidate.member_ref
                            ),
                            now,
                            opened,
                        );
                    }
                    RenewalFailure::RotationRestart { error } => {
                        if let Err(e) = self.store.mark_rotation_restart_owed(
                            app_instance_id,
                            &candidate.member_ref,
                            now as i64,
                        ) {
                            tracing::warn!(
                                app_instance_id,
                                logical_ref = %candidate.member_ref,
                                error = %e,
                                "failed to persist an owed rotation restart"
                            );
                        }
                        self.raise_rotation_restart_pending(
                            instance_id,
                            candidate,
                            &format!(
                                "'{}' renewed its certificate but its restart-on-rotation restart \
                                 failed: {error}; retrying next pass",
                                candidate.member_ref
                            ),
                            opened,
                        );
                    }
                }
                tracing::warn!(
                    app_instance_id,
                    logical_ref = %candidate.member_ref,
                    "certificate renewal did not complete this pass; retrying next pass"
                );
            }
        }
    }

    /// One member's mint -> install -> rotate, in that order, stopping at
    /// the first failure. A restart is deliberately not attempted after a
    /// failed install: rotating a service whose new certificate never
    /// landed serves nothing and spends a lifecycle action for no gain.
    pub(super) async fn renew_one_member(
        &self,
        plan: &DeploymentPlan,
        candidate: &RenewalCandidate,
        actor: &Arc<dyn SubstrateActor>,
        generation: u64,
    ) -> Result<(), RenewalFailure> {
        let master = keys::master_for_member(
            &self.vault,
            &plan.app_instance_id.to_string(),
            &candidate.service_name,
            candidate.member_index,
        )
        .await
        .map_err(|e| match e {
            keys::VaultError::Locked => RenewalFailure::VaultLocked,
            other => RenewalFailure::Step { step: "master lookup", error: other.to_string() },
        })?;

        let cert = deploy::certify_instance_via_actor(
            actor,
            &master,
            &candidate.service_id,
            self.renewed_cert_expires_hours,
        )
        .await
        .map_err(|e| RenewalFailure::Step { step: "mint", error: e.to_string() })?;
        let cert_json = cert
            .to_json()
            .map_err(|e| RenewalFailure::Step { step: "mint", error: e.to_string() })?;

        actor
            .renew_cert(candidate.service_id.clone(), generation, cert_json)
            .await
            .map_err(|error| RenewalFailure::Step { step: "install", error })?;

        // The one place `RotationPolicy` is read. The substrate never sees
        // it: the supervisor holds the stored plan, so this is a local
        // decision made once the new certificate is known to be installed.
        let rotation = plan
            .services
            .iter()
            .find(|svc| svc.member_ref().to_string() == candidate.member_ref)
            .map(|svc| svc.config.rotation_policy);
        if rotation == Some(RotationPolicy::RestartOnRotation) {
            actor
                .restart(candidate.service_id.clone(), generation)
                .await
                .map_err(|error| RenewalFailure::RotationRestart { error })?;
        }
        Ok(())
    }

    pub(super) fn raise_vault_locked(
        &self,
        instance_id: &AppInstanceId,
        candidate: &RenewalCandidate,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(&candidate.member_ref),
            None,
            &candidate.substrate_did,
            AlertKind::VaultLocked,
            &format!(
                "'{}' needs its instance certificate renewed, but this supervisor's vault is \
                 locked so its member master cannot be read; run: roymctl --substrate <this node> \
                 security inject-kek --kek-hex <...>",
                candidate.member_ref
            ),
        ) {
            opened.push((AlertKind::VaultLocked, candidate.member_ref.clone()));
        }
    }

    /// The certificate half of the renewal already landed,
    /// so this is deliberately not `raise_renewal_stalled` -- that pair
    /// clears the moment the health poll sees a fresh window, which this
    /// renewal already produced. Cleared only by
    /// `retry_pending_rotation_restarts` actually succeeding.
    pub(super) fn raise_rotation_restart_pending(
        &self,
        instance_id: &AppInstanceId,
        candidate: &RenewalCandidate,
        detail: &str,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(&candidate.member_ref),
            None,
            &candidate.substrate_did,
            AlertKind::RotationRestartPending,
            detail,
        ) {
            opened.push((AlertKind::RotationRestartPending, candidate.member_ref.clone()));
        }
    }

    /// One retry per pass, per member still owing a `restart-on-rotation`
    /// restart from an earlier renewal. Resolved against
    /// this pass's own plan and clients, the same shape `renew_due_members`
    /// uses -- an unreachable substrate simply leaves the marker in place
    /// for the next pass to retry.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn retry_pending_rotation_restarts(
        &self,
        instance_id: &AppInstanceId,
        app_instance_id: &str,
        plan: &DeploymentPlan,
        pending: &BTreeSet<String>,
        did_to_alias: &BTreeMap<String, String>,
        actors: &BTreeMap<SubstrateAlias, Arc<dyn SubstrateActor>>,
        generation: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        for l_ref in pending {
            let Some(svc) = plan.services.iter().find(|s| &s.member_ref().to_string() == l_ref)
            else {
                // A resubmit dropped this member from the plan (D-A5c-3:
                // not undeployed, just no longer named) -- this loop is
                // keyed off `plan.services`, so no future pass will ever
                // reach this logical ref here again. Unlike a member that
                // is merely unreachable this pass, there is no "retry
                // later" for a row nothing will ever revisit -- clearing
                // it, and whatever `RotationRestartPending` row it opened,
                // is the only way either one is not permanent.
                if let Err(e) = self.store.clear_rotation_restart_owed(app_instance_id, l_ref) {
                    tracing::warn!(
                        app_instance_id,
                        logical_ref = l_ref,
                        error = %e,
                        "failed to clear an owed rotation restart for a member dropped from the \
                         plan"
                    );
                }
                if let Ok(active) = self.store.alerts.active(instance_id) {
                    for row in active
                        .iter()
                        .filter(|r| r.kind == AlertKind::RotationRestartPending)
                        .filter(|r| r.logical_ref.as_deref() == Some(l_ref.as_str()))
                    {
                        let _ = self.store.alerts.clear(
                            instance_id,
                            Some(l_ref),
                            &row.substrate_did,
                            AlertKind::RotationRestartPending,
                        );
                    }
                }
                continue;
            };
            let Some(alias) = svc.substrate.as_ref() else { continue };
            // The plan only carries the alias; the alert row wants the
            // real DID (an alias in that column is a different bug this
            // must not repeat -- see `InstanceRevoked`'s own raise below),
            // so this reverses the same `did_to_alias` map every other
            // renewal path reads forwards.
            let Some(substrate_did) =
                did_to_alias.iter().find(|(_, a)| a.as_str() == alias.as_str()).map(|(did, _)| did)
            else {
                continue;
            };
            let Some(actor) = actors.get(alias) else { continue };
            match actor.restart(svc.service_id.to_string(), generation).await {
                Ok(()) => {
                    if let Err(e) = self.store.clear_rotation_restart_owed(app_instance_id, l_ref) {
                        tracing::warn!(
                            app_instance_id,
                            logical_ref = l_ref,
                            error = %e,
                            "failed to clear an owed rotation restart after it succeeded"
                        );
                    }
                    let _ = self.store.alerts.clear(
                        instance_id,
                        Some(l_ref),
                        substrate_did,
                        AlertKind::RotationRestartPending,
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        app_instance_id,
                        logical_ref = l_ref,
                        error,
                        "rotation restart still owed; retrying next pass"
                    );
                    if let Ok(true) = self.store.alerts.raise(
                        instance_id,
                        Some(l_ref),
                        None,
                        substrate_did,
                        AlertKind::RotationRestartPending,
                        &format!(
                            "'{l_ref}' still owes a restart-on-rotation restart: {error}; \
                             retrying next pass"
                        ),
                    ) {
                        opened.push((AlertKind::RotationRestartPending, l_ref.clone()));
                    }
                }
            }
        }
    }

    /// A renewal that did not complete. `CertificateExpired` once the
    /// window has actually closed -- a current outage, not a reminder --
    /// and `CertificateNearExpiry` while there is still time (A4-04's own
    /// distinction, applied to the renewal path).
    pub(super) fn raise_renewal_stalled(
        &self,
        instance_id: &AppInstanceId,
        candidate: &RenewalCandidate,
        detail: &str,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let kind = if is_expired_parts(candidate.expires_at, now) {
            AlertKind::CertificateExpired
        } else {
            AlertKind::CertificateNearExpiry
        };
        if let Ok(true) = self.store.alerts.raise(
            instance_id,
            Some(&candidate.member_ref),
            None,
            &candidate.substrate_did,
            kind,
            detail,
        ) {
            opened.push((kind, candidate.member_ref.clone()));
        }
    }

    /// Clears every renewal-related alert for a member the substrate now
    /// reports with a certificate comfortably inside its window. Recomputed
    /// from the substrate's own answer each pass rather than tracked as a
    /// flag, so a renewal that succeeded out of band clears these just as a
    /// supervisor-driven one does.
    pub(super) fn clear_settled_renewal_alerts(
        &self,
        instance_id: &AppInstanceId,
        report: &health::HealthReport,
        now: u64,
    ) {
        for svc in &report.services {
            let (Some(issued), Some(expires)) =
                (svc.instance_certificate_issued_at, svc.instance_certificate_expires_at)
            else {
                continue;
            };
            if is_near_expiry_parts(issued, expires, now) {
                continue;
            }
            let l_ref = svc.member_ref().to_string();
            for kind in [
                AlertKind::CertificateNearExpiry,
                AlertKind::CertificateExpired,
                AlertKind::VaultLocked,
            ] {
                let _ =
                    self.store.alerts.clear(instance_id, Some(&l_ref), &svc.substrate_did, kind);
            }
        }
    }

    /// Republishes each master this instance's plan names, but only once
    /// its `master_anchor_refresh_interval_secs` has elapsed since the last
    /// successful publication. Evaluated on the ordinary pass tick against
    /// a persisted fact rather than on a timer of its own -- the same shape
    /// the loop's other periodic decisions already use.
    ///
    /// Failures are logged, never alerted: an anchor that is still inside
    /// its 24-hour validity window is not yet a fault, and the interval
    /// leaves several passes of margin before it becomes one.
    pub(super) async fn refresh_due_master_anchors(&self, plan: &DeploymentPlan, now: u64) {
        let Some(writer) = &self.anchor_writer else { return };
        // Logged rather than alerted. A locked vault is already alerted on,
        // per member, the moment a renewal is due -- and that fires on a
        // four-hour clock against this refresh's twelve-hour one, so a
        // supervisor whose vault stays shut long enough for an anchor to
        // matter has already raised `VaultLocked` several times over.
        // Raising a second kind here would be the same fact twice.
        if !self.vault.kek_is_loaded() {
            tracing::warn!(
                app_instance_id = %plan.app_instance_id,
                "vault locked; skipping this instance's master-anchor refresh check this pass"
            );
            return;
        }
        let now = now as i64;
        let interval = self.master_anchor_refresh_interval_secs as i64;
        let mut refreshed: BTreeSet<String> = BTreeSet::new();
        for svc in &plan.services {
            let master_did = svc.service_id.to_string();
            // Two services naming one master (not reachable from today's
            // compiler, but cheap to be right about) share one anchor and
            // must not each republish it in the same pass.
            if !refreshed.insert(master_did.clone()) {
                continue;
            }
            let last = self.store.last_master_anchor_refresh(&master_did).unwrap_or(None);
            // `at > now` (a backwards clock step, or a restored database)
            // must count as due immediately, not be suppressed until the
            // wall clock catches back up to it.
            if last.is_some_and(|at| at <= now && now.saturating_sub(at) < interval) {
                continue;
            }
            let master = match keys::master_for_member(
                &self.vault,
                &plan.app_instance_id.to_string(),
                svc.logical_ref.service_name.as_str(),
                svc.member_index,
            )
            .await
            {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(master_did, error = %e, "cannot read this master to refresh its anchor");
                    continue;
                }
            };
            match writer.refresh(&master).await {
                Ok(()) => {
                    if let Err(e) = self.store.record_master_anchor_refresh(&master_did, now) {
                        tracing::warn!(master_did, error = %e, "failed to stamp a master-anchor refresh");
                    }
                }
                Err(e) => tracing::warn!(
                    master_did,
                    error = %e,
                    "failed to refresh a master anchor; retrying on a later pass"
                ),
            }
        }
    }

    /// Publishes or refreshes this instance's Tier-1 registry record
    /// (ADR-0022 §2) -- "which supervisor holds this app" -- once
    /// `master_anchor_refresh_interval_secs` has elapsed since the last
    /// successful publish. Evaluated on the ordinary pass tick against a
    /// persisted fact, the same shape `refresh_due_master_anchors` uses,
    /// reusing its interval rather than a second config field.
    ///
    /// Keyed by `state.app_master_did`, not `app_instance_id`: the fact
    /// belongs to the DID being published, not to the human name, so a
    /// handover that changes which DID this instance publishes under
    /// (`import-master` under a new key, then `adopt`) starts that new
    /// DID's own refresh history at "never refreshed" rather than
    /// inheriting the old DID's recent stamp.
    ///
    /// Skipped, never minted, when this instance has no app master DID on
    /// its row yet: an instance adopted before that column existed gains
    /// one at its next `adopt`, and nowhere else -- minting here would
    /// create an app identity outside `adopt`, the one place that owns it.
    ///
    /// A locked vault raises `AlertKind::VaultLocked` (app-level,
    /// `logical_ref: None`) rather than only logging: the per-member raise
    /// from certificate renewal only fires for an instance with a member
    /// inside its near-expiry window, so an instance with none would
    /// otherwise get no signal at all while this record decays. Cleared
    /// once a refresh succeeds again.
    pub(super) async fn refresh_due_app_tier1_record(
        &self,
        instance_id: &AppInstanceId,
        state: &DesiredState,
        now: u64,
        opened: &mut Vec<(AlertKind, String)>,
    ) {
        let Some(writer) = &self.tier1_writer else { return };
        if state.app_master_did.is_empty() {
            return;
        }
        let now_i = now as i64;
        let interval = self.master_anchor_refresh_interval_secs as i64;
        let last = self.store.last_tier1_refresh(&state.app_master_did).unwrap_or(None);
        // `at > now_i` (a backwards clock step, or a restored database)
        // must count as due immediately, not be suppressed until the wall
        // clock catches back up to it.
        if last.is_some_and(|at| at <= now_i && now_i.saturating_sub(at) < interval) {
            return;
        }
        let signed = match tier1::sign_tier1_record(
            &self.vault,
            &state.app_instance_id,
            &state.app_master_did,
            &self.node_did,
            state.generation,
            self.master_anchor_refresh_interval_secs,
        )
        .await
        {
            Ok(s) => s,
            // Reached by attempting the read, not by pre-checking
            // `kek_is_loaded()` first: that check reads the `KeyStore`,
            // not whether the storage
            // provider's own encryption is even on, so on a node with
            // `storage.encryption = false` it always answers `false` even
            // though every vault read succeeds -- a pre-check here would
            // skip this instance's Tier-1 publish forever, silently, on
            // exactly that node, and raise a `VaultLocked` alert that is
            // never true. Reading `VaultError::Locked` off the real
            // attempt is correct on both an encrypted-and-locked vault and
            // an unencrypted one, and additionally catches the vault
            // locked *between* an early check and this call, which a
            // pre-check cannot.
            Err(tier1::Tier1SignError::Vault(keys::VaultError::Locked)) => {
                tracing::warn!(
                    app_instance_id = %state.app_instance_id,
                    "vault locked; skipping this instance's Tier-1 record refresh this pass"
                );
                if let Ok(true) = self.store.alerts.raise(
                    instance_id,
                    None,
                    None,
                    &self.node_did,
                    AlertKind::VaultLocked,
                    &format!(
                        "'{}' cannot refresh its Tier-1 registry record because this supervisor's \
                         vault is locked; callers outside the app will lose the ability to \
                         discover its supervisor once the currently-published record lapses. Run: \
                         roymctl --substrate {} security inject-kek --kek-hex <...>",
                        state.app_instance_id, self.node_did
                    ),
                ) {
                    opened.push((AlertKind::VaultLocked, state.app_instance_id.clone()));
                }
                return;
            }
            Err(e @ tier1::Tier1SignError::IdentityMismatch { .. }) => {
                tracing::warn!(
                    app_instance_id = %state.app_instance_id,
                    error = %e,
                    "refusing to publish this instance's Tier-1 record under the wrong identity"
                );
                if let Ok(true) = self.store.alerts.raise(
                    instance_id,
                    None,
                    None,
                    &self.node_did,
                    AlertKind::AppIdentityMismatch,
                    &e.to_string(),
                ) {
                    opened.push((AlertKind::AppIdentityMismatch, state.app_instance_id.clone()));
                }
                return;
            }
            Err(e) => {
                tracing::warn!(
                    app_instance_id = %state.app_instance_id,
                    error = %e,
                    "cannot sign this instance's Tier-1 record this pass"
                );
                return;
            }
        };
        let _ = self.store.alerts.clear(instance_id, None, &self.node_did, AlertKind::VaultLocked);
        let _ = self.store.alerts.clear(
            instance_id,
            None,
            &self.node_did,
            AlertKind::AppIdentityMismatch,
        );
        match writer.publish(&signed).await {
            Ok(()) => {
                if let Err(e) = self.store.record_tier1_refresh(&state.app_master_did, now_i) {
                    tracing::warn!(
                        app_instance_id = %state.app_instance_id,
                        error = %e,
                        "failed to stamp a Tier-1 refresh"
                    );
                }
            }
            Err(e) => tracing::warn!(
                app_instance_id = %state.app_instance_id,
                error = %e,
                "failed to publish this instance's Tier-1 record; retrying on a later pass"
            ),
        }
    }
}

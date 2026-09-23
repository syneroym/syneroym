use std::collections::BTreeMap;

use syneroym_identity::DelegationCertificate;

use super::{super::*, helpers::*};

#[tokio::test]
async fn a_pass_renews_a_member_within_the_near_expiry_window() {
    let s = service();
    let master_did = seeded_member(&s, "backend").await;
    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
    let report = report_of(vec![near_expiry_health("backend", &master_did)]);

    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 5);
    assert_eq!(candidates.len(), 1, "{candidates:?}");

    let actor = Arc::new(RenewalActor::default());
    let mut opened = Vec::new();
    s.renew_due_members(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        &plan,
        &candidates,
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        7,
        NOW,
        &mut opened,
    )
    .await;

    let renewed = actor.renewed.lock().unwrap();
    assert_eq!(renewed.len(), 1, "the member must have had a certificate installed");
    assert_eq!(renewed[0].0, master_did);
    assert_eq!(renewed[0].1, 7, "the install must carry this supervisor's generation");
    let cert = DelegationCertificate::from_json(&renewed[0].2).unwrap();
    assert_eq!(cert.master_did, master_did);
    assert!(opened.is_empty(), "a successful renewal raises no alert: {opened:?}");
}

#[tokio::test]
async fn a_pass_does_not_renew_a_member_outside_the_near_expiry_window() {
    let master_did = "did:key:hBackend";
    let report = report_of(vec![fresh_health("backend", master_did)]);
    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 5);
    assert!(candidates.is_empty(), "{candidates:?}");
}

/// D-A5d-12: a service already in `needs_work` is about to be
/// re-certified by `apply_plan` this same pass, so renewing it here
/// would mint it a second certificate for no reason.
#[test]
fn a_member_in_needs_work_is_not_also_renewed_this_pass() {
    let report = report_of(vec![near_expiry_health("backend", "did:key:hBackend")]);
    let needs_work = BTreeSet::from(["inst-1/backend#0".to_string()]);
    let candidates =
        SupervisorService::renewal_candidates(&report, &needs_work, &BTreeSet::new(), NOW, 5);
    assert!(candidates.is_empty(), "{candidates:?}");
}

/// The other half of D-A5d-12: a restart reloads the running instance
/// and touches no certificate, so a member under remediation still
/// needs its own, independent renewal check. `restart_candidates` is
/// therefore not an input to this decision at all.
#[test]
fn a_member_under_restart_remediation_is_still_checked_for_renewal() {
    let mut unhealthy = near_expiry_health("backend", "did:key:hBackend");
    unhealthy.signal = Signal::InstanceNotRunning("down".to_string());
    let report = report_of(vec![unhealthy]);
    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 5);
    assert_eq!(candidates.len(), 1, "{candidates:?}");
}

/// D-A5d-4: a locked vault skips the renewal work-list and nothing
/// else -- health, remediation, and the anchor check all continue,
/// since none of them opens the vault.
#[tokio::test]
async fn a_locked_vault_skips_renewal_but_not_health_or_remediation_this_pass() {
    let s = service_with_locked_vault();
    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", "did:key:hBackend", "none"))
            .unwrap();
    let candidates = SupervisorService::renewal_candidates(
        &report_of(vec![near_expiry_health("backend", "did:key:hBackend")]),
        &BTreeSet::new(),
        &BTreeSet::new(),
        NOW,
        5,
    );
    let actor = Arc::new(RenewalActor::default());
    let mut opened = Vec::new();

    s.renew_due_members(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        &plan,
        &candidates,
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        0,
        NOW,
        &mut opened,
    )
    .await;

    assert!(
        actor.renewed.lock().unwrap().is_empty(),
        "a locked vault must not reach the substrate at all"
    );
    assert_eq!(opened, vec![(AlertKind::VaultLocked, "inst-1/backend#0".to_string())]);

    // The rest of the pass is unaffected: a restart candidate on the
    // same instance still records its attempt.
    let restart_actor = Arc::new(CountingActor::default());
    let dyn_actor: Arc<dyn SubstrateActor> = restart_actor.clone();
    let mut opened2 = Vec::new();
    s.attempt_restart(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        "inst-1/backend",
        "did:key:hBackend",
        "did:key:zEdge1",
        &dyn_actor,
        0,
        NOW,
        &mut opened2,
    )
    .await;
    assert_eq!(*restart_actor.restart_calls.lock().unwrap(), 1);
}

/// One root cause, one row per affected member -- the same fan-out
/// `SubstrateUnreachable` already uses, and what an operator reading
/// `alerts <instance>` needs to see.
#[tokio::test]
async fn a_locked_vault_raises_vault_locked_for_every_near_expiry_member() {
    let s = service_with_locked_vault();
    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", "did:key:hBackend", "none"))
            .unwrap();
    let report = report_of(vec![
        near_expiry_health("backend", "did:key:hBackend"),
        near_expiry_health("frontend", "did:key:hFrontend"),
        fresh_health("worker", "did:key:hWorker"),
    ]);
    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 5);
    let mut opened = Vec::new();

    s.renew_due_members(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        &plan,
        &candidates,
        &edge_1_alias(),
        &edge_1_actor(Arc::new(RenewalActor::default())),
        0,
        NOW,
        &mut opened,
    )
    .await;

    let instance_id = AppInstanceId::new("inst-1");
    let locked: Vec<_> = s
        .store
        .alerts
        .active(&instance_id)
        .unwrap()
        .into_iter()
        .filter(|a| a.kind == AlertKind::VaultLocked)
        .collect();
    assert_eq!(locked.len(), 2, "one row per affected member, not one per fact: {locked:?}");
    let refs: BTreeSet<_> = locked.iter().filter_map(|a| a.logical_ref.clone()).collect();
    assert_eq!(
        refs,
        BTreeSet::from(["inst-1/backend#0".to_string(), "inst-1/frontend#0".to_string()]),
        "the member whose certificate is nowhere near expiry must not be alerted on"
    );
    assert!(
        locked.iter().all(|a| a.detail.contains("inject-kek")),
        "the alert must name the operator action that fixes it"
    );
}

/// D-A5d-6: `RotationPolicy` is read from the supervisor's own stored
/// plan, after the new certificate has installed successfully. The
/// substrate never sees it.
#[tokio::test]
async fn restart_on_rotation_follows_a_successful_install_with_a_restart_call() {
    let s = service();
    let master_did = seeded_member(&s, "backend").await;
    let plan = DeploymentPlan::from_json(&plan_json_with_master(
        "backend",
        &master_did,
        "restart-on-rotation",
    ))
    .unwrap();
    let actor = Arc::new(RenewalActor::default());
    let mut opened = Vec::new();

    s.renew_due_members(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        &plan,
        &SupervisorService::renewal_candidates(
            &report_of(vec![near_expiry_health("backend", &master_did)]),
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        ),
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        0,
        NOW,
        &mut opened,
    )
    .await;

    assert_eq!(actor.renewed.lock().unwrap().len(), 1);
    assert_eq!(*actor.restarted.lock().unwrap(), vec![master_did]);
}

#[tokio::test]
async fn rotation_policy_none_installs_without_restarting() {
    let s = service();
    let master_did = seeded_member(&s, "backend").await;
    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
    let actor = Arc::new(RenewalActor::default());
    let mut opened = Vec::new();

    s.renew_due_members(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        &plan,
        &SupervisorService::renewal_candidates(
            &report_of(vec![near_expiry_health("backend", &master_did)]),
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        ),
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        0,
        NOW,
        &mut opened,
    )
    .await;

    assert_eq!(actor.renewed.lock().unwrap().len(), 1);
    assert!(actor.restarted.lock().unwrap().is_empty());
}

/// D-A5d-13, first step: a mint that fails must not go on to install
/// or restart. `CertificateNearExpiry` names the step, and the member
/// is retried next pass rather than failing the whole instance.
#[tokio::test]
async fn a_failed_mint_does_not_attempt_install_or_restart_for_that_member() {
    let s = service();
    let master_did = seeded_member(&s, "backend").await;
    let plan = DeploymentPlan::from_json(&plan_json_with_master(
        "backend",
        &master_did,
        "restart-on-rotation",
    ))
    .unwrap();
    let actor = Arc::new(RenewalActor {
        instance_identity_error: Some("substrate refused the identity query".to_string()),
        ..RenewalActor::default()
    });
    let mut opened = Vec::new();

    s.renew_due_members(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        &plan,
        &SupervisorService::renewal_candidates(
            &report_of(vec![near_expiry_health("backend", &master_did)]),
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        ),
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        0,
        NOW,
        &mut opened,
    )
    .await;

    assert!(actor.renewed.lock().unwrap().is_empty());
    assert!(actor.restarted.lock().unwrap().is_empty());
    assert_eq!(opened, vec![(AlertKind::CertificateNearExpiry, "inst-1/backend#0".to_string())]);
    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    assert!(alerts.iter().any(|a| a.detail.contains("mint")), "{alerts:?}");
}

/// D-A5d-13, second step: restarting a service whose new certificate
/// never landed serves nothing and spends a lifecycle action for no
/// gain.
#[tokio::test]
async fn a_failed_install_does_not_attempt_restart_for_that_member() {
    let s = service();
    let master_did = seeded_member(&s, "backend").await;
    let plan = DeploymentPlan::from_json(&plan_json_with_master(
        "backend",
        &master_did,
        "restart-on-rotation",
    ))
    .unwrap();
    let actor = Arc::new(RenewalActor {
        renew_error: Some("substrate unreachable".to_string()),
        ..RenewalActor::default()
    });
    let mut opened = Vec::new();

    s.renew_due_members(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        &plan,
        &SupervisorService::renewal_candidates(
            &report_of(vec![near_expiry_health("backend", &master_did)]),
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        ),
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        0,
        NOW,
        &mut opened,
    )
    .await;

    assert!(actor.restarted.lock().unwrap().is_empty());
    assert_eq!(opened, vec![(AlertKind::CertificateNearExpiry, "inst-1/backend#0".to_string())]);
    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    assert!(alerts.iter().any(|a| a.detail.contains("install")), "{alerts:?}");
}

/// Mint and install both landed, only the
/// `restart-on-rotation` restart failed. This must not be reported as
/// a stalled renewal (the certificate is fine, and the very next
/// health poll would clear that kind out from under the real
/// problem) -- it gets its own alert kind and a persisted marker that
/// survives the certificate's own alert lifecycle.
#[tokio::test]
async fn a_failed_rotation_restart_raises_its_own_alert_and_is_not_cleared_by_a_fresh_cert() {
    let s = service();
    let master_did = seeded_member(&s, "backend").await;
    let plan = DeploymentPlan::from_json(&plan_json_with_master(
        "backend",
        &master_did,
        "restart-on-rotation",
    ))
    .unwrap();
    let actor = Arc::new(RenewalActor {
        restart_error: Some("substrate refused the restart".to_string()),
        ..RenewalActor::default()
    });
    let mut opened = Vec::new();

    s.renew_due_members(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        &plan,
        &SupervisorService::renewal_candidates(
            &report_of(vec![near_expiry_health("backend", &master_did)]),
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        ),
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        0,
        NOW,
        &mut opened,
    )
    .await;

    // The certificate itself landed.
    assert_eq!(actor.renewed.lock().unwrap().len(), 1);
    assert_eq!(opened, vec![(AlertKind::RotationRestartPending, "inst-1/backend#0".to_string())]);
    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    assert!(
        !alerts.iter().any(|a| a.kind == AlertKind::CertificateNearExpiry),
        "a landed renewal must not also read as a stalled one: {alerts:?}"
    );
    assert!(
        s.store.pending_rotation_restarts("inst-1").unwrap().contains("inst-1/backend#0"),
        "the owed restart must be persisted, not just alerted"
    );

    // A fresh certificate window alone must not clear it.
    s.clear_settled_renewal_alerts(
        &AppInstanceId::new("inst-1"),
        &report_of(vec![fresh_health("backend", &master_did)]),
        NOW,
    );
    let alerts = s.store.alerts.active(&AppInstanceId::new("inst-1")).unwrap();
    assert!(
        alerts.iter().any(|a| a.kind == AlertKind::RotationRestartPending),
        "only a successful retry clears it: {alerts:?}"
    );
}

/// The retry half of the fix: once persisted, the owed restart is
/// retried on a later pass, independent of the renewal work-list that
/// no longer names this member (its certificate is no longer near
/// expiry) -- `retry_pending_rotation_restarts` is `renew_due_members`'
/// own sibling call inside `apply_write_phase`, tested the same
/// direct way (see the "wiring" test below for the
/// `plan -> did_to_alias -> clients` lookup itself).
#[tokio::test]
async fn a_pending_rotation_restart_is_retried_and_cleared_on_success() {
    let s = service();
    let master_did = seeded_member(&s, "backend").await;
    let plan = DeploymentPlan::from_json(&plan_json_with_master(
        "backend",
        &master_did,
        "restart-on-rotation",
    ))
    .unwrap();
    let instance_id = AppInstanceId::new("inst-1");
    s.store.mark_rotation_restart_owed("inst-1", "inst-1/backend#0", NOW as i64).unwrap();
    s.store
        .alerts
        .raise(
            &instance_id,
            Some("inst-1/backend#0"),
            None,
            "did:key:zEdge1",
            AlertKind::RotationRestartPending,
            "owed",
        )
        .unwrap();
    let actor = Arc::new(RenewalActor::default());
    let pending: BTreeSet<String> = ["inst-1/backend#0".to_string()].into_iter().collect();
    let mut opened = Vec::new();

    s.retry_pending_rotation_restarts(
        &instance_id,
        "inst-1",
        &plan,
        &pending,
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        0,
        &mut opened,
    )
    .await;

    assert_eq!(actor.restarted.lock().unwrap().len(), 1);
    assert!(s.store.pending_rotation_restarts("inst-1").unwrap().is_empty());
    assert!(s.store.alerts.active(&instance_id).unwrap().is_empty());
}

/// The failure half: a still-failing restart leaves the marker in
/// place for the next pass, rather than clearing it or forgetting it.
#[tokio::test]
async fn a_still_failing_rotation_restart_stays_pending() {
    let s = service();
    let master_did = seeded_member(&s, "backend").await;
    let plan = DeploymentPlan::from_json(&plan_json_with_master(
        "backend",
        &master_did,
        "restart-on-rotation",
    ))
    .unwrap();
    let instance_id = AppInstanceId::new("inst-1");
    s.store.mark_rotation_restart_owed("inst-1", "inst-1/backend#0", NOW as i64).unwrap();
    let actor = Arc::new(RenewalActor {
        restart_error: Some("still refusing".to_string()),
        ..RenewalActor::default()
    });
    let pending: BTreeSet<String> = ["inst-1/backend#0".to_string()].into_iter().collect();
    let mut opened = Vec::new();

    s.retry_pending_rotation_restarts(
        &instance_id,
        "inst-1",
        &plan,
        &pending,
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        0,
        &mut opened,
    )
    .await;

    assert!(s.store.pending_rotation_restarts("inst-1").unwrap().contains("inst-1/backend#0"));
    assert!(
        s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::RotationRestartPending)
    );
}

/// A member dropped from the plan by a
/// resubmit is never reached by this loop again -- it is keyed off
/// `plan.services` -- so unlike an unreachable-this-pass member,
/// there is no future retry to defer to. Leaving the marker and the
/// alert in place would make both permanent, with nothing left that
/// could ever clear either.
#[tokio::test]
async fn a_member_dropped_from_the_plan_has_its_owed_restart_and_alert_cleared() {
    let s = service();
    // A plan that no longer names `inst-1/backend` at all.
    let plan = DeploymentPlan::from_json(&plan_json_no_services("inst-1")).unwrap();
    let instance_id = AppInstanceId::new("inst-1");
    s.store.mark_rotation_restart_owed("inst-1", "inst-1/backend", NOW as i64).unwrap();
    s.store
        .alerts
        .raise(
            &instance_id,
            Some("inst-1/backend"),
            None,
            "did:key:zEdge1",
            AlertKind::RotationRestartPending,
            "owed",
        )
        .unwrap();
    let pending: BTreeSet<String> = ["inst-1/backend".to_string()].into_iter().collect();
    let mut opened = Vec::new();

    s.retry_pending_rotation_restarts(
        &instance_id,
        "inst-1",
        &plan,
        &pending,
        &BTreeMap::new(),
        &BTreeMap::new(),
        0,
        &mut opened,
    )
    .await;

    assert!(s.store.pending_rotation_restarts("inst-1").unwrap().is_empty());
    assert!(s.store.alerts.active(&instance_id).unwrap().is_empty());
}

/// The clearing rule: a raised alert with no path back to cleared is a
/// bug. Recomputed from the substrate's own answer, not tracked as a
/// flag -- so a renewal that succeeded out of band clears these too.
#[tokio::test]
async fn certificate_near_expiry_clears_on_the_next_passs_healthy_read() {
    let s = service();
    let instance_id = AppInstanceId::new("inst-1");
    for kind in
        [AlertKind::CertificateNearExpiry, AlertKind::CertificateExpired, AlertKind::VaultLocked]
    {
        s.store
            .alerts
            .raise(&instance_id, Some("inst-1/backend#0"), None, "did:key:zEdge1", kind, "stalled")
            .unwrap();
    }

    // Still near expiry: nothing clears.
    s.clear_settled_renewal_alerts(
        &instance_id,
        &report_of(vec![near_expiry_health("backend", "did:key:hBackend")]),
        NOW,
    );
    assert_eq!(s.store.alerts.active(&instance_id).unwrap().len(), 3);

    // A healthy certificate window clears all three at once.
    s.clear_settled_renewal_alerts(
        &instance_id,
        &report_of(vec![fresh_health("backend", "did:key:hBackend")]),
        NOW,
    );
    assert!(s.store.alerts.active(&instance_id).unwrap().is_empty());
}

/// D-A5d-16: the supervisor mints at its own, short lifetime -- not
/// the attended posture's 24-hour deploy default, which serves an
/// operator with no renewal loop behind them.
#[tokio::test]
async fn renewal_mints_at_renewed_cert_expires_hours_not_the_deploy_default() {
    let s = service();
    let master_did = seeded_member(&s, "backend").await;
    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
    let actor = Arc::new(RenewalActor::default());
    let mut opened = Vec::new();

    s.renew_due_members(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        &plan,
        &SupervisorService::renewal_candidates(
            &report_of(vec![near_expiry_health("backend", &master_did)]),
            &BTreeSet::new(),
            &BTreeSet::new(),
            NOW,
            5,
        ),
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        0,
        NOW,
        &mut opened,
    )
    .await;

    let renewed = actor.renewed.lock().unwrap();
    let cert = DelegationCertificate::from_json(&renewed[0].2).unwrap();
    let lifetime = cert.expires_at_secs - cert.issued_at_secs;
    assert_eq!(lifetime, s.renewed_cert_expires_hours * 3600);
    assert!(
        lifetime < deploy::DEFAULT_INSTANCE_CERT_EXPIRES_HOURS * 3600,
        "a renewed certificate must be strictly shorter-lived than the attended default"
    );
}

/// D-A5d-17: the same root cause must surface under the same alert
/// kind whichever of the two checks catches it. This is the defensive
/// per-call path -- `kek_is_loaded` said unlocked, and the vault read
/// itself then failed -- distinct from the up-front check's own test
/// above.
#[tokio::test]
async fn a_vault_error_locked_race_during_mint_raises_vault_locked_not_certificate_near_expiry() {
    // An encrypted vault, open at construction so the cheap up-front
    // check passes, then closed again before the mint -- the exact
    // ordering the carve-out exists for, produced directly rather than
    // raced.
    let (s, key_store) =
        Fixture { locked_vault: true, inject_kek_anyway: true, ..Fixture::default() }
            .build_with_key_store();
    assert!(s.vault.kek_is_loaded());

    let plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", "did:key:hBackend", "none"))
            .unwrap();
    let candidate = RenewalCandidate {
        member_ref: "inst-1/backend#0".to_string(),
        service_name: "backend".to_string(),
        service_id: "did:key:hBackend".to_string(),
        substrate_did: "did:key:zEdge1".to_string(),
        expires_at: NOW + 1_440,
        member_index: 0,
    };
    let actor = Arc::new(RenewalActor::default());
    let mut opened = Vec::new();
    key_store.clear_kek();

    s.renew_due_members(
        &AppInstanceId::new("inst-1"),
        "inst-1",
        &plan,
        std::slice::from_ref(&candidate),
        &edge_1_alias(),
        &edge_1_actor(actor.clone()),
        0,
        NOW,
        &mut opened,
    )
    .await;

    assert!(actor.renewed.lock().unwrap().is_empty());
    assert_eq!(
        opened,
        vec![(AlertKind::VaultLocked, "inst-1/backend#0".to_string())],
        "a vault lock found mid-mint is the same condition as one found up front, and must not \
         surface under a different alert kind"
    );
}

/// D-A5d-21: renewal is the one work-list whose arrivals are
/// correlated by construction -- every member of an instance is minted
/// in the same call at the same lifetime, so a whole instance reaches
/// its near-expiry window in the same pass, every cycle. The cap
/// bounds how long one pass holds the instance lock; the remainder
/// rolls to the next pass, recomputed from live health data rather
/// than queued.
#[test]
fn a_pass_renews_at_most_max_renewals_per_pass_candidates_and_defers_the_rest() {
    let names = ["a", "b", "c", "d", "e", "f", "g"];
    let report = report_of(
        names.iter().map(|n| near_expiry_health(n, &format!("did:key:h{n}"))).collect::<Vec<_>>(),
    );

    let first =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 5);
    assert_eq!(first.len(), 5, "the cap must hold: {first:?}");

    // The next pass recomputes from live health. The five that landed
    // now report fresh certificates; the deferred two are still due
    // and are picked up.
    let taken: BTreeSet<String> = first.iter().map(|c| c.member_ref.clone()).collect();
    let next_report = report_of(
        names
            .iter()
            .map(|n| {
                let id = format!("did:key:h{n}");
                if taken.contains(&format!("inst-1/{n}#0")) {
                    fresh_health(n, &id)
                } else {
                    near_expiry_health(n, &id)
                }
            })
            .collect::<Vec<_>>(),
    );
    let second = SupervisorService::renewal_candidates(
        &next_report,
        &BTreeSet::new(),
        &BTreeSet::new(),
        NOW,
        5,
    );
    let deferred: BTreeSet<String> = second.iter().map(|c| c.member_ref.clone()).collect();
    assert_eq!(deferred.len(), 2, "{second:?}");
    assert!(deferred.is_disjoint(&taken));
}

/// Without a sort, report order alone decided who kept
/// the cap's slots, which a persistently-failing member (still
/// near-expiry every pass, since its renewal never lands) could hold
/// forever if it happened to sort first -- starving every member past
/// the cap even though they are genuinely more urgent. `b` is one
/// second from expiring; `a`, `c`, and `d` have a full hour left, but
/// `a` sorts first alphabetically. The cap must still pick `b`.
#[test]
fn the_cap_keeps_the_most_urgent_candidates_not_whichever_sort_first() {
    // Same 4-hour lifetime `near_expiry_health`/`fresh_health` use;
    // only how much of it remains differs per member. 3,600s
    // remaining sits exactly on the 25%-of-lifetime boundary (still
    // near-expiry, inclusive); 1s remaining is far past it.
    let report = report_of(vec![
        health_with_cert("a", "did:key:ha", "did:key:zEdge1", NOW - 10_800, NOW + 3_600),
        health_with_cert("b", "did:key:hb", "did:key:zEdge1", NOW - 14_399, NOW + 1),
        health_with_cert("c", "did:key:hc", "did:key:zEdge1", NOW - 10_800, NOW + 3_600),
        health_with_cert("d", "did:key:hd", "did:key:zEdge1", NOW - 10_800, NOW + 3_600),
    ]);

    let candidates =
        SupervisorService::renewal_candidates(&report, &BTreeSet::new(), &BTreeSet::new(), NOW, 1);

    assert_eq!(
        candidates.iter().map(|c| c.member_ref.as_str()).collect::<Vec<_>>(),
        vec!["inst-1/b#0"],
        "the one slot must go to the member closest to expiring: {candidates:?}"
    );
}

/// `.take(0)` silently disables renewal for the whole
/// node, with no warning and nothing rejecting the config. The
/// existing config-level test only pins the *default* at 1, which
/// says nothing about a configured 0 -- clamped at construction
/// instead, so every caller gets the guard regardless of how it built
/// the config.
#[test]
fn a_configured_zero_max_renewals_per_pass_is_clamped_to_one() {
    let s = Fixture { max_renewals_per_pass: Some(0), ..Fixture::default() }.build();
    assert_eq!(s.max_renewals_per_pass, 1);
}

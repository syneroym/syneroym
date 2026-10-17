//! The per-service certificate lifetime at renewal.

use syneroym_identity::DelegationCertificate;

use super::{super::*, helpers::*};

/// Renews `backend` once and returns the lifetime, in seconds, of the
/// certificate the substrate received.
async fn renewed_lifetime(hours: Option<u32>) -> (u64, u64) {
    let s = service();
    let master_did = seeded_member(&s, "backend").await;
    let mut plan =
        DeploymentPlan::from_json(&plan_json_with_master("backend", &master_did, "none")).unwrap();
    plan.services[0].instance_cert_expires_hours = hours;
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
    (cert.expires_at_secs - cert.issued_at_secs, s.renewed_cert_expires_hours * 3600)
}

#[tokio::test]
async fn a_member_whose_service_sets_a_lifetime_is_renewed_with_that_lifetime() {
    let (lifetime, node_default) = renewed_lifetime(Some(168)).await;
    assert_eq!(lifetime, 168 * 3600);
    assert_ne!(lifetime, node_default);
}

#[tokio::test]
async fn a_member_without_its_own_lifetime_is_renewed_at_the_node_default() {
    let (lifetime, node_default) = renewed_lifetime(None).await;
    assert_eq!(lifetime, node_default);
}

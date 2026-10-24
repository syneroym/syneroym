//! The per-service certificate lifetime at renewal, and the mailbox settings
//! at submit.

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

const MAILBOX_HOST: &str = "did:key:h7wy641jeaawpdosg7mwogqromgksnkk3rh7faa93ga5cntowi1duxwe";

async fn submit_with_mailbox_methods(methods: serde_json::Value) -> RpcResult<NativeResponse> {
    let s = service();
    let plan_json = serde_json::json!({
        "app_instance_id": "inst-mailbox",
        "blueprint_id": "syneroym:test",
        "version": "1.0.0",
        "services": [{
            "service_id": "did:key:hFabricated",
            "logical_ref": "inst-mailbox/backend",
            "substrate": null,
            "service_type": "tcp", "source": "127.0.0.1:9000",
            "rotation_policy": "none",
            "resolved_dependencies": {},
            "topology_mode": "singleton",
            "visibility": "public",
            "mailbox": MAILBOX_HOST,
            "mailbox_methods": methods,
        }]
    })
    .to_string();
    dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "submit",
        serde_json::json!([{
            "app_instance_id": "inst-mailbox",
            "plan_json": plan_json,
            "inventory_json": "{}",
            "generation": 0,
        }]),
    )
    .await
}

#[tokio::test]
async fn submit_is_refused_when_a_service_lists_a_node_interface_as_a_mailbox_method() {
    let err =
        submit_with_mailbox_methods(serde_json::json!(["security#inject-kek"])).await.unwrap_err();

    assert!(matches!(err, RpcError::InvalidParams(_)), "expected InvalidParams, got {err:?}");
    assert!(err.to_string().contains("node or platform interface"), "{err}");
}

#[tokio::test]
async fn submit_does_not_refuse_a_guests_own_mailbox_methods_for_that_reason() {
    // It may fail later for other reasons (nothing is placed in this test),
    // but not because of the listed method.
    let result = submit_with_mailbox_methods(serde_json::json!(["greeter#greet"])).await;

    if let Err(err) = result {
        assert!(!err.to_string().contains("node or platform interface"), "{err}");
    }
}

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Stable member identity (ADR-0020 §1-§5), proven across two genuinely
//! independent `syneroym-substrate` instances rather than the in-process
//! coverage `crates/control_plane/src/service/orchestration.rs`'s own tests
//! and `crates/router/src/proxy.rs`'s unit tests already give the
//! mechanism.
//!
//! What this proves live, over two real substrates: `orchestrator/instance-
//! identity` derives a *different* instance key per hosting node for the
//! identical `(caller, service_id)` pair; `deploy` verifies an installed
//! instance certificate and rejects a wrong one; `list` reports the
//! installed certificate's expiry; and deploying the *same* member master
//! (`service_id`) to a *second*, independently-keyed node produces a new
//! instance key there too, certified cleanly by a fresh certificate from
//! that same master. The reference scenario's step 4 claim that the member
//! master *itself* persists across reinstantiation is proven at unit scale
//! instead, in `crates/router/src/handshake.rs`.
//!
//! Both nodes come from `common::SubstrateNode`, owned by one `operator`
//! identity and self-hosting their own registries -- `instance-identity` is
//! a direct RPC to each node, so no shared registry or relay is needed.
//! `common::serial_guard` keeps this binary's tests from running stacks at
//! once.
//!
//! **Deliberately not covered here:** a live, wire-level proof that a guest-
//! origin `ProxyRouter` call presents its certified instance key across a
//! real cross-node hop -- that arm only fires for a WASM guest's own
//! outbound call (`crates/router/src/proxy.rs`'s `CallOrigin::Guest` arm),
//! and every real cross-node call this fixture's sibling
//! (`federated_fdae_e2e.rs`) drives is `CallOrigin::Native` on a TCP
//! service, which that arm does not touch. Building a WASM-guest two-node
//! harness for that one wire hop is out of scope here; the
//! guest-arm's actual code path is proven at the router level instead
//! (`crates/router/src/proxy.rs`'s `a_guest_call_travels_under_its_
//! services_member_master_not_the_node_identity` and neighboring tests).
//! Tracked in `docs/planning/deferred-backlog.md`.

use common::SubstrateNode;
use ed25519_dalek::VerifyingKey;
use rustls::crypto::ring;
use serde_json::json;
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_sdk::{
    DeployManifest, NetworkEndpoint, ServiceConfig, ServiceType, SyneroymClient, TcpManifest,
};

mod common;

/// A minimal TCP service carrying an optional instance certificate -- this
/// fixture cares only about the orchestrator's own deploy/list/instance-
/// identity surface, never actually dials `port`. One real endpoint is
/// required for the service to appear in `list()` at all: `list()` filters
/// out the native-capability interfaces (`data-layer`/`vault`/etc.) every
/// deploy registers regardless of manifest, so an `endpoints: vec![]`
/// manifest registers only those and the service never surfaces (see
/// `router/tests/service_ownership.rs`'s `tcp_manifest` for the same
/// precedent).
fn bare_tcp_manifest(port: u16, instance_certificate: Option<String>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: ServiceType::Tcp(TcpManifest {
            endpoints: vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port,
            }],
        }),
        registry_certificate: None,
        instance_certificate,
    }
}

async fn deploy(
    client: &SyneroymClient,
    service_id: &str,
    manifest: DeployManifest,
) -> anyhow::Result<()> {
    let params = serde_json::to_value((service_id.to_string(), manifest))?;
    let res = client.request("orchestrator", "deploy", params).await?;
    if res.result == json!({"status": "deployed"}) {
        Ok(())
    } else {
        Err(anyhow::anyhow!("deploy did not report success: {:?}", res.result))
    }
}

#[tokio::test]
async fn a_member_master_authorizes_a_distinct_instance_key_on_each_real_node_it_deploys_to() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    // One operator identity, reused against both nodes as both their owner
    // (an unowned substrate fails closed) and the caller of
    // every `orchestrator/*` call below: `instance-identity` derives from
    // (node identity, caller_did, service_id), so holding the caller and
    // service_id fixed isolates the node as the only varying input in the
    // comparison below.
    let operator = Identity::generate().unwrap();

    // Each node injects its KEK during its own boot, before the other node's
    // boot can leave its connection idle -- so no post-boot redial is needed.
    let node_a = SubstrateNode::builder().owner(&operator).inject_kek().boot().await;
    let node_b = SubstrateNode::builder().owner(&operator).inject_kek().boot().await;

    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    let mut operator_a = node_a.client_as(Identity::from_bytes(&operator.to_bytes()));
    operator_a.connect().await.expect("failed to connect to node A");
    let mut operator_b = node_b.client_as(Identity::from_bytes(&operator.to_bytes()));
    operator_b.connect().await.expect("failed to connect to node B");

    let instance_identity_a = operator_a
        .instance_identity(&member_master_did)
        .await
        .expect("instance-identity query against node A failed");
    let instance_identity_b = operator_b
        .instance_identity(&member_master_did)
        .await
        .expect("instance-identity query against node B failed");
    assert_ne!(
        instance_identity_a.instance_did, instance_identity_b.instance_did,
        "two distinct real substrates must derive two distinct instance keys for the identical \
         (caller, service_id) pair -- the derivation is bound to the hosting node's own identity, \
         not just the caller and service_id"
    );

    let pubkey_a = VerifyingKey::from_bytes(
        &hex::decode(&instance_identity_a.pubkey_hex).unwrap().try_into().unwrap(),
    )
    .unwrap();
    let cert_a = DelegationCertificate::issue(
        &member_master,
        pubkey_a,
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();

    // A wrong-scope certificate over the *same* correctly-derived key is
    // rejected at deploy -- the install-time check, live against a real
    // substrate.
    let wrong_scope_cert =
        DelegationCertificate::issue(&member_master, pubkey_a, 3600, "routing".to_string())
            .unwrap();
    let rejected = deploy(
        &operator_a,
        &member_master_did,
        bare_tcp_manifest(40001, Some(wrong_scope_cert.to_json().unwrap())),
    )
    .await;
    assert!(rejected.is_err(), "a routing-scoped certificate must be rejected at deploy");

    // The correctly-scoped certificate installs cleanly.
    deploy(
        &operator_a,
        &member_master_did,
        bare_tcp_manifest(40002, Some(cert_a.to_json().unwrap())),
    )
    .await
    .expect("deploy with a valid instance certificate must succeed");

    let services_a = operator_a.list_svcs().await.expect("list on node A failed");
    let listed_a = services_a
        .iter()
        .find(|s| s.service_id == member_master_did)
        .expect("the deployed member master must appear in node A's list");
    assert_eq!(
        listed_a.instance_certificate_expires_at,
        Some(cert_a.expires_at_secs),
        "list must report the installed certificate's real expiry"
    );

    // Reinstantiation on a second, independently-keyed real node: the same
    // service_id (the member master DID) gets a *new* instance key (proven
    // above by `assert_ne!` on the two `instance_did`s). This deploy
    // certifies that new key from the identical `member_master` used for
    // node A -- by construction here, since both certificates are minted
    // from the same in-test `Identity` -- so what this half of the fixture
    // actually exercises is that a fresh certificate for the *same* member
    // master installs cleanly on a second real substrate. The system-level
    // claim that a member master itself persists across reinstantiation is
    // proven at unit scale instead, in `crates/router/src/handshake.rs`'s
    // `a_revoked_instance_key_is_rejected_while_the_member_master_still_
    // certifies_a_new_one`.
    let pubkey_b = VerifyingKey::from_bytes(
        &hex::decode(&instance_identity_b.pubkey_hex).unwrap().try_into().unwrap(),
    )
    .unwrap();
    let cert_b = DelegationCertificate::issue(
        &member_master,
        pubkey_b,
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    deploy(
        &operator_b,
        &member_master_did,
        bare_tcp_manifest(40003, Some(cert_b.to_json().unwrap())),
    )
    .await
    .expect("deploy of the reinstantiated member to node B must succeed");

    let services_b = operator_b.list_svcs().await.expect("list on node B failed");
    let listed_b = services_b
        .iter()
        .find(|s| s.service_id == member_master_did)
        .expect("the reinstantiated member master must appear in node B's list");
    assert_eq!(listed_b.instance_certificate_expires_at, Some(cert_b.expires_at_secs));

    // Undeploy removes the deployed service; the certificate itself is
    // covered at the control-plane unit level
    // (`undeploy_removes_the_instance_certificate_with_the_owner_row`).
    operator_a.undeploy(member_master_did.clone(), 0).await.expect("undeploy on node A failed");
    let services_a_after =
        operator_a.list_svcs().await.expect("list on node A after undeploy failed");
    assert!(
        !services_a_after.iter().any(|s| s.service_id == member_master_did),
        "an undeployed service must not still appear in list"
    );

    node_a.teardown().await;
    node_b.teardown().await;
}

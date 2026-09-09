#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Unattended certificate renewal and instance-key revocation (M05A A5d),
//! proven against real, running `syneroym-substrate` instances rather than
//! the in-process coverage `orchestration.rs` and `app_supervisor` already
//! give the two mechanisms.
//!
//! Two properties, one fixture:
//!
//! 1. **`renew-cert` installs in place, over the wire.** A member deployed on
//!    node B with one instance certificate is handed a second one through the
//!    new verb -- no manifest, no artifact, no reinstall -- and the substrate
//!    afterwards reports the *new* certificate as the one it holds. The
//!    install-time verification block runs live on this path too: a certificate
//!    minted for node A's derived key is refused by node B, and the previously
//!    installed one is left untouched.
//!
//! 2. **A revoked instance key stops verifying, while a fresh one from the same
//!    master still does.** Driven through the real revocation writer
//!    (`RegistryClient::revoke_instance_key`, what `roymctl supervisor
//!    revoke-instance` calls) against a real HTTP registry, then read back
//!    through the real ingress check (`HandshakeVerifier::verify_preamble`
//!    against a real `RegistryClient` resolver). Before A5d nothing in the tree
//!    could *write* a non-empty `revoked_keys` list outside a unit test's
//!    in-memory mock, so this is the half of failure-matrix row 14 that had a
//!    mechanism and no trigger.
//!
//! **Deliberately not covered here:** a wire-level proof that a guest's own
//! outbound call presents the renewed certificate across a real cross-node
//! hop. That arm only fires for a WASM guest (`CallOrigin::Guest` in
//! `crates/router/src/proxy.rs`), and building a WASM-guest two-node harness
//! for it is out of scope for this slice -- the same scoping, for the same
//! reason, `instance_identity_e2e.rs`'s own module doc records. What the
//! renewed certificate is *used for* once installed is covered at unit
//! scale by `renew_cert_rebuilds_syn_svc_native_service_with_the_new_
//! certificate`, which signs a relationship proof and checks it verifies
//! against the new certificate.
//!
//! Both nodes come from `common::SubstrateNode`. Node B publishes into and
//! resolves through node A's registry (`shared_registry`), so a cross-node
//! anchor lookup resolves against a real record. `common::serial_guard`
//! keeps the two tests here from running substrate stacks at once.

mod common;

use common::SubstrateNode;
use ed25519_dalek::VerifyingKey;
use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::dht_registry::RegistryClient;
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_router::{RoutePreamble, handshake::HandshakeVerifier};
use syneroym_sdk::{
    DeployManifest, NetworkEndpoint, ServiceConfig, ServiceType, SyneroymClient, TcpManifest,
};

/// A minimal TCP service, never actually dialed -- this fixture exercises
/// only the orchestrator's certificate surface. One real endpoint is
/// required for the service to appear in `list()` at all (see
/// `instance_identity_e2e.rs`'s own note on why).
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

fn pubkey_from_hex(hex_str: &str) -> VerifyingKey {
    VerifyingKey::from_bytes(&hex::decode(hex_str).unwrap().try_into().unwrap()).unwrap()
}

/// The preamble a connection presenting `cert` over `pubkey_hex` would
/// carry -- exactly what `HandshakeVerifier::verify_preamble` reads at
/// ingress.
fn delegated_preamble(
    target_service_id: &str,
    pubkey_hex: &str,
    cert: &DelegationCertificate,
) -> RoutePreamble {
    RoutePreamble {
        transport: syneroym_router::RouteTransport::Binary,
        protocol: syneroym_router::RouteProtocol::JsonRpc,
        interface: "default".to_string(),
        service_id: target_service_id.to_string(),
        enc: None,
        pubkey: Some(pubkey_hex.to_string()),
        delegation: Some(cert.clone()),
        ucan: None,
        dir: None,
    }
}

/// The renewal half: `renew-cert` over the real wire installs in place, its
/// verification block is live on the new path too (a certificate minted for
/// the *other* node's derived key is refused, and the previously installed
/// one survives the refusal), and the substrate afterwards reports the
/// renewed certificate as the one it holds -- read back through `list-svcs`,
/// not through a live handshake. A WASM-guest two-node
/// harness for that arm is the same out-of-proportion item
/// `instance_identity_e2e.rs` and three backlog rows already decline; see
/// `status.md`'s own scoping note for this test).
#[tokio::test]
async fn renew_cert_installs_over_the_real_wire_and_refuses_a_certificate_for_the_wrong_derived_key()
 {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let operator = Identity::generate().unwrap();
    let mut node_a = SubstrateNode::builder().owner(&operator).boot().await;
    let node_b = SubstrateNode::builder()
        .owner(&operator)
        .shared_registry(node_a.registry_url())
        .boot()
        .await;

    // Default `storage.encryption = true` needs a KEK before any deployed
    // service's native-capability endpoints can be set up -- the same
    // precedent every e2e fixture in this crate follows.
    // `node_a`'s connection was dialed and proven live by its own
    // `wait_for_ready` during boot, then sat idle for the entire `node_b`
    // boot that followed -- long enough under CI's scheduling pressure for
    // the peer to abandon that idle path ("no viable network path exists:
    // last path abandoned by peer"). Recover by explicit shutdown→reconnect
    // before one retry.
    if node_a.substrate_client.inject_kek("aa".repeat(32)).await.is_err() {
        node_a
            .substrate_client
            .shutdown()
            .await
            .expect("failed to reset node A's stale connection");
        node_a.substrate_client.connect().await.expect("failed to reconnect node A");
        node_a
            .substrate_client
            .inject_kek("aa".repeat(32))
            .await
            .expect("node A inject_kek failed");
    }
    node_b.substrate_client.inject_kek("bb".repeat(32)).await.expect("node B inject_kek failed");

    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    let mut operator_a = node_a.client_as(Identity::from_bytes(&operator.to_bytes()));
    operator_a.connect().await.expect("failed to connect to node A");
    let mut operator_b = node_b.client_as(Identity::from_bytes(&operator.to_bytes()));
    operator_b.connect().await.expect("failed to connect to node B");

    let identity_b = operator_b
        .instance_identity(&member_master_did)
        .await
        .expect("instance-identity query against node B failed");
    let pubkey_b = pubkey_from_hex(&identity_b.pubkey_hex);

    let first = DelegationCertificate::issue(
        &member_master,
        pubkey_b,
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    deploy(
        &operator_b,
        &member_master_did,
        bare_tcp_manifest(43001, Some(first.to_json().unwrap())),
    )
    .await
    .expect("the first deploy must install its instance certificate");

    let listed = operator_b.list_svcs().await.expect("list on node B failed");
    assert_eq!(
        listed
            .iter()
            .find(|s| s.service_id == member_master_did)
            .expect("the member must be listed")
            .instance_certificate_expires_at,
        Some(first.expires_at_secs)
    );

    // A certificate over node A's derived key, offered to node B: the
    // install-time verification block must run on the renewal path exactly
    // as it does on the deploy path, and must leave the installed
    // certificate untouched when it refuses.
    let identity_a = operator_a
        .instance_identity(&member_master_did)
        .await
        .expect("instance-identity query against node A failed");
    assert_ne!(
        identity_a.instance_did, identity_b.instance_did,
        "the two nodes must derive different instance keys for this fixture to mean anything"
    );
    let for_the_wrong_node = DelegationCertificate::issue(
        &member_master,
        pubkey_from_hex(&identity_a.pubkey_hex),
        7200,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let refused = operator_b
        .renew_cert(member_master_did.clone(), 0, for_the_wrong_node.to_json().unwrap())
        .await;
    assert!(refused.is_err(), "node B must refuse a certificate over node A's derived key");
    assert_eq!(
        operator_b
            .list_svcs()
            .await
            .unwrap()
            .iter()
            .find(|s| s.service_id == member_master_did)
            .unwrap()
            .instance_certificate_expires_at,
        Some(first.expires_at_secs),
        "a refused renewal must leave the previously installed certificate in place"
    );

    // The real renewal: a fresh certificate over the same derived key,
    // installed in place with no manifest and no reinstall.
    let renewed = DelegationCertificate::issue(
        &member_master,
        pubkey_b,
        7200,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    assert_ne!(renewed.to_json().unwrap(), first.to_json().unwrap());
    operator_b
        .renew_cert(member_master_did.clone(), 0, renewed.to_json().unwrap())
        .await
        .expect("renew-cert over the wire must succeed");

    assert_eq!(
        operator_b
            .list_svcs()
            .await
            .unwrap()
            .iter()
            .find(|s| s.service_id == member_master_did)
            .unwrap()
            .instance_certificate_expires_at,
        Some(renewed.expires_at_secs),
        "the substrate must now hold the renewed certificate, not the one installed at deploy"
    );

    node_a.teardown().await;
    node_b.teardown().await;
}

/// Failure-matrix row 14's automation half: a revoked instance key fails
/// while a fresh one from the same master still verifies -- driven through
/// the real revocation writer and read back through the real ingress check,
/// against a real registry.
#[tokio::test]
async fn a_revoked_instance_key_handshake_fails_while_a_fresh_one_verifies() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let operator = Identity::generate().unwrap();
    // One node is enough here: what is under test is the registry
    // round trip between the revocation writer and the ingress check, and
    // the substrate's role in it is to host the registry.
    let node = SubstrateNode::builder().owner(&operator).inject_kek_bytes([0xcc; 32]).boot().await;

    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());
    let registry = RegistryClient::new(false, Some(node.registry_url().to_string()));
    registry
        .publish_master_anchor(&member_master_did, vec![], None, &member_master, false)
        .await
        .expect("failed to publish the member master's anchor");

    // Two instance keys certified by the same master -- the "reinstantiated
    // elsewhere" case row 14 is about.
    let doomed = Identity::generate().unwrap();
    let replacement = Identity::generate().unwrap();
    let doomed_did = substrate::derive_did_key(&doomed.public_key());
    let doomed_cert = DelegationCertificate::issue(
        &member_master,
        doomed.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let replacement_cert = DelegationCertificate::issue(
        &member_master,
        replacement.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();

    let doomed_preamble =
        delegated_preamble(node.did(), &hex::encode(doomed.public_key().to_bytes()), &doomed_cert);
    let replacement_preamble = delegated_preamble(
        node.did(),
        &hex::encode(replacement.public_key().to_bytes()),
        &replacement_cert,
    );

    // Before the revocation, both verify.
    HandshakeVerifier::verify_preamble(&doomed_preamble, &registry)
        .await
        .expect("an un-revoked instance key must verify");
    HandshakeVerifier::verify_preamble(&replacement_preamble, &registry)
        .await
        .expect("the replacement key must verify too");

    // The production write path -- what `roymctl supervisor revoke-instance`
    // calls. Before A5d nothing outside a unit test's in-memory mock could
    // put a key on this list at all.
    registry
        .revoke_instance_key(&member_master, &doomed_did)
        .await
        .expect("the revocation must publish");

    let err = HandshakeVerifier::verify_preamble(&doomed_preamble, &registry)
        .await
        .expect_err("a revoked instance key must be refused at ingress");
    assert!(err.to_string().contains("revoked"), "{err}");

    HandshakeVerifier::verify_preamble(&replacement_preamble, &registry).await.expect(
        "a fresh key from the same master must still verify -- revocation is scoped to the one \
         instance key, not to the member master",
    );

    // The list is carried forward, not overwritten: a second revocation
    // must not un-revoke the first. This is the read-modify-write the
    // backlog row called out, exercised against a real registry.
    let second = Identity::generate().unwrap();
    registry
        .revoke_instance_key(&member_master, &substrate::derive_did_key(&second.public_key()))
        .await
        .expect("the second revocation must publish");
    let err = HandshakeVerifier::verify_preamble(&doomed_preamble, &registry)
        .await
        .expect_err("the first revocation must survive the second");
    assert!(err.to_string().contains("revoked"), "{err}");

    node.teardown().await;
}

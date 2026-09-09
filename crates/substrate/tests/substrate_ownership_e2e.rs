#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The operator-facing ownership path, over a real substrate: a
//! `ControllerAgreement` (the artifact `roymctl substrate claim` writes)
//! placed at `app_data_dir/agreement.json` *before* the substrate starts,
//! then a single boot that must come up owned by implicit discovery, with no
//! `[identity].agreement` config line at all.
//!
//! Proves, live: a claimed substrate lets its controller deploy a service
//! and inject a KEK, and denies an unrelated but verified identity both; an
//! unowned substrate -- no agreement at all -- denies every deploy. Every
//! other test of this area drives `SubstrateIdentityState::init`
//! (`crates/identity/src/substrate.rs`) or `build_caller`
//! (`crates/router/src/route_handler/io.rs`) directly; this is the only one
//! that goes through a real boot, the handshake, and both admission gates
//! (`orchestrator/deploy` and `security`) together.

use std::fs;

use common::SubstrateNode;
use rustls::crypto::ring;
use syneroym_core::config::{DEFAULT_CONTROLLER_AGREEMENT_FILE, DEFAULT_SUBSTRATE_KEY_FILE};
use syneroym_identity::{
    Identity,
    substrate::{ControllerAgreement, SubstrateIdentityStatus},
};
use syneroym_rpc::{JsonRpcError, PERMISSION_DENIED_CODE};
use syneroym_sdk::{NetworkEndpoint, Publication};
use tempfile::TempDir;

mod common;

/// Mints the node's own key and a `ControllerAgreement` binding it to
/// `controller` into a fresh directory, writes the agreement to
/// `user_data/agreement.json`, and boots from that directory. The substrate
/// must discover ownership from the file -- not from config -- so the boot
/// comes up `Verified` with no `admin_ucan_root` set anywhere.
///
/// The returned [`TempDir`] owns the directory and must outlive the node.
async fn boot_claimed(controller: &Identity) -> (SubstrateNode, TempDir) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let data = dir.path().join("user_data");
    fs::create_dir_all(&data).expect("create user_data");

    let node_key = Identity::generate().expect("node identity");
    node_key.save_to_path(data.join(DEFAULT_SUBSTRATE_KEY_FILE)).expect("save node key");
    let agreement =
        ControllerAgreement::issue(&node_key, controller, None).expect("issue agreement");
    fs::write(
        data.join(DEFAULT_CONTROLLER_AGREEMENT_FILE),
        serde_json::to_string(&agreement).unwrap(),
    )
    .expect("write agreement.json");

    let node = SubstrateNode::builder()
        .unowned() // ownership comes from the discovered agreement.json, not config
        .base_path(dir.path())
        .inspect_identity(|state| {
            assert_eq!(
                state.status,
                SubstrateIdentityStatus::Verified,
                "the discovered agreement must verify before the substrate starts routing",
            )
        })
        .boot()
        .await;
    (node, dir)
}

/// Boots with no agreement at all -- an ordinary, never-claimed substrate.
async fn boot_unowned() -> SubstrateNode {
    SubstrateNode::builder()
        .unowned()
        .inspect_identity(|state| {
            assert_eq!(
                state.status,
                SubstrateIdentityStatus::None,
                "a substrate with no agreement.json must boot unowned",
            )
        })
        .boot()
        .await
}

#[tokio::test]
async fn a_claimed_substrate_admits_its_controller_and_denies_everyone_else() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let controller = Identity::generate().unwrap();
    let controller_for_client = Identity::from_bytes(&controller.to_bytes());
    let (node, _dir) = boot_claimed(&controller).await;

    // --- The controller: deploys and injects a KEK, both of which an
    // unowned substrate denies entirely. ---
    let mut controller_client = node.client_as(controller_for_client);
    controller_client.connect().await.expect("controller failed to connect");

    controller_client
        .deploy_svc_tcp(
            "did:key:zP0OwnershipTestService".to_string(),
            vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                // A declared manifest address; this test never dials it.
                port: 30099,
            }],
            Publication::Private,
            None,
        )
        .await
        .expect("the controller must be able to deploy on a substrate it claimed");

    controller_client
        .inject_kek("aa".repeat(32))
        .await
        .expect("the controller must be able to inject a KEK on a substrate it claimed");

    // --- A stranger: verified over the wire, but never delegated anything
    // by the controller and not the node's own key. Both the `security`
    // interface and `orchestrator/deploy` must deny it. ---
    let stranger = Identity::generate().unwrap();
    let mut stranger_client = node.client_as(stranger);
    stranger_client.connect().await.expect("stranger failed to connect");

    let deploy_err = stranger_client
        .deploy_svc_tcp(
            "did:key:zP0OwnershipStrangerService".to_string(),
            vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                // A declared manifest address; this test never dials it.
                port: 30098,
            }],
            Publication::Private,
            None,
        )
        .await
        .expect_err(
            "an unrelated identity must not be able to deploy on a claimed substrate it does not \
             control",
        );
    // Unlike `security`, the orchestrator's Tier-1 admission check
    // (`ControlPlaneService::deploy`) has no distinct denial code -- every
    // cause maps through `.map_err(RpcError::InternalError)`
    // (`crates/control_plane/src/service.rs`'s `"deploy"` arm), so the
    // message is the only way to confirm this failed for lack of a grant
    // and not some other reason.
    let deploy_err_string = deploy_err.to_string();
    assert!(
        deploy_err_string.contains("holds no orchestrator/deploy grant"),
        "must be denied for lack of a grant, not fail for some other reason: {deploy_err_string}"
    );

    // The controller already injected a KEK above, so a *second* injection
    // by anyone -- controller or stranger -- would also fail with
    // `KekAlreadyInjected` (-32603). Asserting only `is_err()` here cannot
    // tell that apart from the security gate actually denying the caller,
    // so the code must be checked specifically.
    let kek_err = stranger_client.inject_kek("bb".repeat(32)).await.expect_err(
        "an unrelated identity must not be able to reach the security interface on a claimed \
         substrate it does not control",
    );
    assert_eq!(
        kek_err.downcast_ref::<JsonRpcError>().map(|e| e.code),
        Some(PERMISSION_DENIED_CODE),
        "must be denied specifically, not fail on KekAlreadyInjected: {kek_err:?}"
    );

    controller_client.shutdown().await.ok();
    stranger_client.shutdown().await.ok();
    node.teardown().await;
}

/// The over-the-wire proof that an unowned substrate denies a deploy: a
/// substrate with no agreement at all denies a real deploy from a real,
/// verified caller. The claimed-node test above proves the same property
/// for a caller who is a *stranger to the controller*; this proves it for a
/// substrate that has no controller in the first place, which is the
/// failure mode the fail-closed flip exists to fix.
#[tokio::test]
async fn an_unowned_substrate_rejects_a_deploy() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let node = boot_unowned().await;

    let caller = Identity::generate().unwrap();
    let mut client = node.client_as(caller);
    client.connect().await.expect("caller failed to connect");

    let deploy_err = client
        .deploy_svc_tcp(
            "did:key:zP0UnownedDeployService".to_string(),
            vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                // A declared manifest address; this test never dials it.
                port: 30097,
            }],
            Publication::Private,
            None,
        )
        .await
        .expect_err("an unowned substrate must deny every deploy");
    // As above: `deploy`'s Tier-1 admission denial has no distinct code, so
    // the message is what confirms this is a lack-of-grant denial.
    let deploy_err_string = deploy_err.to_string();
    assert!(
        deploy_err_string.contains("holds no orchestrator/deploy grant"),
        "must be denied for lack of a grant, not fail for some other reason: {deploy_err_string}"
    );

    client.shutdown().await.ok();
    node.teardown().await;
}

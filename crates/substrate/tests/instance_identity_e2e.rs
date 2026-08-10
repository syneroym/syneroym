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
//! Uses its own `Node`, not `tests/common`'s `SubstrateTestContext`:
//! `SubstrateTestContext` holds its setup-serialization lock for the whole
//! struct's lifetime (released on `Drop`), which is correct for the one-
//! node-per-test-function shape every other consumer of that module uses,
//! but deadlocks a test that needs two nodes alive at once in the same
//! function -- the second `setup()` call blocks forever on a lock the
//! first, still-live node never releases. `federated_fdae_e2e.rs`'s own
//! `Node` sidesteps this the same way, independently, for the same reason.
//!
//! **Deliberately not covered here:** a live, wire-level proof that a guest-
//! origin `ProxyRouter` call presents its certified instance key across a
//! real cross-node hop -- that arm only fires for a WASM guest's own
//! outbound call (`crates/router/src/proxy.rs`'s `CallOrigin::Guest` arm),
//! and every real cross-node call this fixture's sibling
//! (`federated_fdae_e2e.rs`) drives is `CallOrigin::Native` on a TCP
//! service, which that arm does not touch. Building a WASM-guest two-node
//! harness for that one wire hop is out of scope for this slice; the
//! guest-arm's actual code path is proven at the router level instead
//! (`crates/router/src/proxy.rs`'s `a_guest_call_travels_under_its_
//! services_member_master_not_the_node_identity` and neighboring tests).
//! Tracked in `docs/planning/deferred-backlog.md`.

use std::time::Duration;

use ed25519_dalek::VerifyingKey;
use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::config::{
    ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
    ServiceRegistryRole, SubstrateConfig,
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_sdk::{
    DeployManifest, NetworkEndpoint, ServiceConfig, ServiceType, SyneroymClient, TcpManifest,
};
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
};

const NODE_A_IROH_PORT: u16 = 8200;
const NODE_A_REGISTRY_PORT: u16 = 8201;
const NODE_A_GATEWAY_PORT: u16 = 8202;
const NODE_B_IROH_PORT: u16 = 8300;
const NODE_B_REGISTRY_PORT: u16 = 8301;
const NODE_B_GATEWAY_PORT: u16 = 8302;

/// A full, independently-identified `syneroym-substrate` instance -- mirrors
/// `federated_fdae_e2e.rs`'s own `Node` (each multi-node fixture in this
/// directory keeps its own near-verbatim copy rather than sharing
/// `tests/common`'s single-node-shaped harness).
struct Node {
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    _temp_dir: TempDir,
}

impl Node {
    /// `owner`'s DID becomes this node's `[iam].admin_ucan_root` (an
    /// unowned substrate now fails closed, so the fixture must
    /// own its own nodes) -- both nodes are owned by the same `operator`
    /// identity below, which is also what every `orchestrator_client` call
    /// presents, so no separate grant is needed.
    async fn boot(iroh_port: u16, registry_port: u16, gateway_port: u16, owner: &Identity) -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let base_path = temp_dir.path();
        let mut config = SubstrateConfig {
            app_local_data_dir: base_path.join("data"),
            app_data_dir: base_path.join("user_data"),
            app_cache_dir: base_path.join("cache"),
            app_log_dir: base_path.join("logs"),
            profile: "full".to_string(),
            ..SubstrateConfig::default()
        };
        config.resolve_paths();
        config.logging.target = LogTarget::Stdout;

        config.roles.coordinator = Some(CoordinatorRole {
            iroh: Some(CoordinatorIrohConfig {
                enable_relay: true,
                http_bind_address: format!("0.0.0.0:{iroh_port}"),
                ..Default::default()
            }),
            ..Default::default()
        });
        config.roles.community_registry = Some(ServiceRegistryRole {
            http_bind_address: format!("0.0.0.0:{registry_port}"),
            ..Default::default()
        });
        let registry_url = format!("http://localhost:{registry_port}");
        config.substrate.registry_url = Some(registry_url.clone());
        config.parent_coordinator.iroh =
            Some(IrohParentConfig { url: format!("http://localhost:{iroh_port}") });
        config.roles.client_gateway =
            Some(ClientGatewayRole { http_port: gateway_port, ..Default::default() });
        config.iam.admin_ucan_root = Some(substrate::derive_did_key(&owner.public_key()));

        let substrate_identity_state =
            identity::setup_substrate_identity(&config.identity, &config.app_data_dir)
                .expect("failed to setup identity");
        let substrate_service_id = substrate_identity_state.did.clone();

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let runtime =
            syneroym_substrate::init(config.clone()).await.expect("failed to initialize runtime");

        let config_clone = config.clone();
        let substrate_handle = tokio::spawn(async move {
            syneroym_substrate::run_with_signal(config_clone, runtime, async {
                let _ = shutdown_rx.recv().await;
            })
            .await
            .expect("substrate failed to run");
        });

        let mut substrate_client = SyneroymClient::new_with_identity(
            substrate_service_id.clone(),
            registry_url.clone(),
            Identity::from_bytes(&owner.to_bytes()),
        );
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("substrate did not become available in time");

        Self { registry_url, substrate_client, shutdown_tx, substrate_handle, _temp_dir: temp_dir }
    }

    fn did(&self) -> &str {
        self.substrate_client.service_id()
    }

    async fn teardown(mut self) {
        let _ = self.substrate_client.shutdown().await;
        let _ = self.shutdown_tx.send(()).await;
        let _ = self.substrate_handle.await;
    }
}

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

/// A client that dispatches `orchestrator/*` against `node`'s own root DID
/// (deploy/undeploy/list/instance-identity are all keyed there, never at the
/// deployed service's own `service_id` -- mirrors `federated_fdae_e2e.rs`'s
/// `hr_deployer`/`alice_deployer` pattern), presenting `caller`.
fn orchestrator_client(node: &Node, caller: Identity) -> SyneroymClient {
    SyneroymClient::new_with_identity(node.did().to_string(), node.registry_url.clone(), caller)
}

#[tokio::test]
async fn a_member_master_authorizes_a_distinct_instance_key_on_each_real_node_it_deploys_to() {
    let _ = ring::default_provider().install_default();

    // One operator identity, reused against both nodes as both their owner
    // (an unowned substrate fails closed) and the caller of
    // every `orchestrator/*` call below: `instance-identity` derives from
    // (node identity, caller_did, service_id), so holding the caller and
    // service_id fixed isolates the node as the only varying input in the
    // comparison below.
    let operator = Identity::generate().unwrap();

    let mut node_a =
        Node::boot(NODE_A_IROH_PORT, NODE_A_REGISTRY_PORT, NODE_A_GATEWAY_PORT, &operator).await;
    let node_b =
        Node::boot(NODE_B_IROH_PORT, NODE_B_REGISTRY_PORT, NODE_B_GATEWAY_PORT, &operator).await;

    // Default `storage.encryption = true` requires a KEK before any deployed
    // service's native-capability endpoints (registered regardless of
    // service type) can be set up -- same precedent as this crate's other
    // e2e fixtures.
    // `node_a`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `Node::boot`, then sat idle for the entire
    // `node_b` boot that followed -- long enough under CI's scheduling
    // pressure for the peer to abandon that idle path ("no viable network
    // path exists: last path abandoned by peer"; same root cause fixed in
    // `binding_push_e2e.rs`). Recover by explicit shutdown→reconnect before
    // one retry.
    if node_a.substrate_client.inject_kek("55".repeat(32)).await.is_err() {
        node_a
            .substrate_client
            .shutdown()
            .await
            .expect("failed to reset node A's stale connection");
        node_a.substrate_client.connect().await.expect("failed to reconnect node A");
        node_a
            .substrate_client
            .inject_kek("55".repeat(32))
            .await
            .expect("node A inject_kek failed");
    }
    node_b.substrate_client.inject_kek("66".repeat(32)).await.expect("node B inject_kek failed");

    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    let mut operator_a = orchestrator_client(&node_a, Identity::from_bytes(&operator.to_bytes()));
    operator_a.connect().await.expect("failed to connect to node A");
    let mut operator_b = orchestrator_client(&node_b, Identity::from_bytes(&operator.to_bytes()));
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
    // substrate (matrix row 2's install-time half).
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

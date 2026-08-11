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
//! Uses its own `Node`, not `tests/common`'s `SubstrateTestContext`: that
//! harness holds its setup-serialization lock for the whole struct's
//! lifetime, which deadlocks a test needing two nodes alive at once. Every
//! multi-node fixture in this directory keeps its own near-verbatim copy for
//! the same reason.

use std::time::Duration;

use ed25519_dalek::VerifyingKey;
use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::{
    config::{
        ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
        ServiceRegistryRole, SubstrateConfig,
    },
    dht_registry::RegistryClient,
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_router::{RoutePreamble, handshake::HandshakeVerifier};
use syneroym_sdk::{
    DeployManifest, NetworkEndpoint, ServiceConfig, ServiceType, SyneroymClient, TcpManifest,
};
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{Mutex, mpsc, mpsc::Sender},
    task::JoinHandle,
};

// Every `#[tokio::test]` in one file runs concurrently by default, so each
// test here needs its own non-overlapping block. Spaced by 100, not 10:
// `coordinator_iroh`'s "/v1/info" server always binds `http_bind_address`'s
// port + 10, which is why every other multi-node harness in this crate uses
// the same spacing. Continues past the highest block already in use
// (`binding_push_e2e.rs`, 10_900).
const NODE_A_IROH_PORT: u16 = 12300;
const NODE_A_REGISTRY_PORT: u16 = 12301;
const NODE_A_GATEWAY_PORT: u16 = 12302;
const NODE_B_IROH_PORT: u16 = 12400;
const NODE_B_REGISTRY_PORT: u16 = 12401;
const NODE_B_GATEWAY_PORT: u16 = 12402;
const REVOCATION_IROH_PORT: u16 = 12500;
const REVOCATION_REGISTRY_PORT: u16 = 12501;
const REVOCATION_GATEWAY_PORT: u16 = 12502;

/// Not sharing a port block keeps the tests here from colliding, but they
/// still each boot full substrate instances (real iroh QUIC socket,
/// self-hosted relay, mainline DHT, wasmtime), and running that many
/// concurrently starves the CI runner's CPU badly enough that iroh's QUIC
/// path validation times out with "no viable network path exists" even
/// though nothing is actually broken. Serializing full-substrate-instance
/// lifetimes within this binary (same fix as `tests/common/mod.rs`'s
/// `SUBSTRATE_TEST_LOCK`) avoids it.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

struct Node {
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    _temp_dir: TempDir,
}

impl Node {
    /// `owner`'s DID becomes this node's `[iam].admin_ucan_root` -- an
    /// unowned substrate fails closed, so the fixture owns its own nodes.
    /// `shared_registry_url` lets node B publish into and resolve through
    /// node A's registry, which is what makes a cross-node anchor lookup
    /// resolve against something real.
    async fn boot(
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        owner: &Identity,
    ) -> Self {
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
        let effective_registry_url =
            shared_registry_url.unwrap_or_else(|| format!("http://localhost:{registry_port}"));
        config.substrate.registry_url = Some(effective_registry_url.clone());
        config.substrate.enable_bep0044_dht = false;
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
            substrate_service_id,
            effective_registry_url.clone(),
            Identity::from_bytes(&owner.to_bytes()),
        );
        substrate_client
            .wait_for_ready(Duration::from_secs(30))
            .await
            .expect("substrate did not become available in time");

        Self {
            registry_url: effective_registry_url,
            substrate_client,
            shutdown_tx,
            substrate_handle,
            _temp_dir: temp_dir,
        }
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

fn orchestrator_client(node: &Node, caller: Identity) -> SyneroymClient {
    SyneroymClient::new_with_identity(node.did().to_string(), node.registry_url.clone(), caller)
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
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let _ = ring::default_provider().install_default();

    let operator = Identity::generate().unwrap();
    let mut node_a =
        Node::boot(NODE_A_IROH_PORT, NODE_A_REGISTRY_PORT, NODE_A_GATEWAY_PORT, None, &operator)
            .await;
    let shared_registry = node_a.registry_url.clone();
    let node_b = Node::boot(
        NODE_B_IROH_PORT,
        NODE_B_REGISTRY_PORT,
        NODE_B_GATEWAY_PORT,
        Some(shared_registry.clone()),
        &operator,
    )
    .await;

    // Default `storage.encryption = true` needs a KEK before any deployed
    // service's native-capability endpoints can be set up -- the same
    // precedent every e2e fixture in this crate follows.
    // `node_a`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `Node::boot`, then sat idle for the entire
    // `node_b` boot that followed -- long enough under CI's scheduling
    // pressure for the peer to abandon that idle path ("no viable network
    // path exists: last path abandoned by peer"; same root cause fixed in
    // `binding_push_e2e.rs`). Recover by explicit shutdown→reconnect before
    // one retry.
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

    let mut operator_a = orchestrator_client(&node_a, Identity::from_bytes(&operator.to_bytes()));
    operator_a.connect().await.expect("failed to connect to node A");
    let mut operator_b = orchestrator_client(&node_b, Identity::from_bytes(&operator.to_bytes()));
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
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let _ = ring::default_provider().install_default();

    let operator = Identity::generate().unwrap();
    // One node is enough here: what is under test is the registry
    // round trip between the revocation writer and the ingress check, and
    // the substrate's role in it is to host the registry.
    let node = Node::boot(
        REVOCATION_IROH_PORT,
        REVOCATION_REGISTRY_PORT,
        REVOCATION_GATEWAY_PORT,
        None,
        &operator,
    )
    .await;
    node.substrate_client.inject_kek("cc".repeat(32)).await.expect("inject_kek failed");

    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());
    let registry = RegistryClient::new(false, Some(node.registry_url.clone()));
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

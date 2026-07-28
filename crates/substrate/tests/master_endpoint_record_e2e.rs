#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Slice A1: endpoint records published under the member master DID
//! (ADR-0020 §6), proven across two genuinely independent
//! `syneroym-substrate` instances -- the mapping this slice exists to build,
//! since nothing before it lets a dependent turn a member master DID into a
//! network address at all.
//!
//! Node A hosts the shared community registry (D-A1-2: a delegation-signed
//! record has no DHT-only home under its master DID, so master-DID
//! resolution requires a configured HTTP registry); node B points at it.
//! A member master `M` is certified and deployed on node B; its record --
//! built and signed by the substrate itself (D-A1-3), never by an operator
//! -- is resolved by `M`, not by node B's own DID, proving the master-DID
//! ->  address mapping works end to end. `M` is then relocated to node A: a
//! clean relocation (`undeploy` before the second `deploy`, deliberately --
//! leaving both deployed would create D-A1-11's two-publisher flap, which
//! this fixture does not exercise), after which the same DID resolves to
//! node A's address instead, with no operator republish action -- the
//! reference scenario's step 4.
//!
//! Uses its own `Node`, not `tests/common`'s `SubstrateTestContext` (for the
//! same reason `instance_identity_e2e.rs`'s own module doc gives -- two live
//! nodes at once deadlock on that harness's setup-serialization lock), with
//! one addition mirrored from `federated_fdae_e2e.rs`: `shared_registry_url`
//! and a shared relay, so node B's own self-published record and node A's
//! registry are the same registry a real cross-node lookup needs.

use std::time::Duration;

use ed25519_dalek::VerifyingKey;
use reqwest::Client as HttpClient;
use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::{
    config::{
        ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
        ServiceRegistryRole, SubstrateConfig,
    },
    dht_registry::{EndpointInfo, EndpointType, RecordTrust, RegistryClient},
};
use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
use syneroym_router::net_iroh::resolve_iroh_addr;
use syneroym_sdk::{
    DeployManifest, NetworkEndpoint, ServiceConfig, ServiceType, SyneroymClient, TcpManifest,
};
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
};

const NODE_A_IROH_PORT: u16 = 8400;
const NODE_A_REGISTRY_PORT: u16 = 8401;
const NODE_A_GATEWAY_PORT: u16 = 8402;
const NODE_B_IROH_PORT: u16 = 8500;
const NODE_B_REGISTRY_PORT: u16 = 8501;
const NODE_B_GATEWAY_PORT: u16 = 8502;

/// A full, independently-identified `syneroym-substrate` instance. Mirrors
/// `federated_fdae_e2e.rs`'s own `Node`: `shared_registry_url` and
/// `shared_relay_url` let node B point itself at node A's registry and relay
/// instead of its own, so a cross-node registry lookup has something real to
/// resolve against.
struct Node {
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    _temp_dir: TempDir,
}

impl Node {
    async fn boot(
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        shared_relay_url: Option<String>,
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
        let own_registry_url = format!("http://localhost:{registry_port}");
        let effective_registry_url = shared_registry_url.unwrap_or(own_registry_url);
        config.substrate.registry_url = Some(effective_registry_url.clone());
        // See federated_fdae_e2e.rs's `Node::boot` for why a shared relay
        // (rather than each node self-relaying through its own) matters here
        // too: cross-relay direct-path negotiation between two localhost
        // peers otherwise adds real latency this fixture has no need to pay.
        let relay_url = shared_relay_url.unwrap_or_else(|| format!("http://localhost:{iroh_port}"));
        config.parent_coordinator.iroh = Some(IrohParentConfig { url: relay_url });
        config.roles.client_gateway = Some(ClientGatewayRole { http_port: gateway_port });

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

        let mut substrate_client =
            SyneroymClient::new(substrate_service_id.clone(), effective_registry_url.clone());
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

/// Mirrors `instance_identity_e2e.rs`'s own `bare_tcp_manifest`: a minimal
/// TCP service carrying an instance certificate, exercising only the
/// orchestrator's deploy/instance-identity surface and the substrate's own
/// endpoint-publishing hook -- never actually dialed.
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

#[tokio::test]
async fn a_member_master_did_resolves_to_an_address_and_follows_the_member_across_nodes() {
    let _ = ring::default_provider().install_default();

    let node_a =
        Node::boot(NODE_A_IROH_PORT, NODE_A_REGISTRY_PORT, NODE_A_GATEWAY_PORT, None, None).await;
    let node_a_registry_url = node_a.registry_url.clone();
    let node_a_relay_url = format!("http://localhost:{NODE_A_IROH_PORT}");
    let node_b = Node::boot(
        NODE_B_IROH_PORT,
        NODE_B_REGISTRY_PORT,
        NODE_B_GATEWAY_PORT,
        Some(node_a_registry_url.clone()),
        Some(node_a_relay_url),
    )
    .await;

    // Default `storage.encryption = true` requires a KEK before any deployed
    // service's native-capability endpoints (registered regardless of
    // service type) can be set up -- same precedent as this crate's other
    // e2e fixtures.
    node_a.substrate_client.inject_kek("77".repeat(32)).await.expect("node A inject_kek failed");
    node_b.substrate_client.inject_kek("88".repeat(32)).await.expect("node B inject_kek failed");

    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());

    // Publish the member master's anchor (D-A1-7): the realistic operator
    // flow, and a certificate over an anchor-less master would be rejected
    // at any real handshake even though this fixture never drives one.
    let registry_client = RegistryClient::new(false, Some(node_a_registry_url.clone()));
    registry_client
        .publish_master_anchor(&member_master_did, vec![], None, &member_master, true)
        .await
        .expect("failed to publish member master anchor");

    let operator = Identity::generate().unwrap();
    let mut operator_b = orchestrator_client(&node_b, Identity::from_bytes(&operator.to_bytes()));
    operator_b.connect().await.expect("failed to connect to node B");

    let instance_identity_b = operator_b
        .instance_identity(&member_master_did)
        .await
        .expect("instance-identity query against node B failed");
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
        bare_tcp_manifest(41001, Some(cert_b.to_json().unwrap())),
    )
    .await
    .expect("deploy on node B with a valid instance certificate must succeed");

    // Resolve, not just read -- the assertion this slice exists for.
    let signed = registry_client
        .lookup(&member_master_did, true)
        .await
        .expect("lookup(resolve = true) for the member master DID must succeed");
    assert_eq!(signed.info.service_id, member_master_did);
    assert_eq!(
        signed.info.delegation.as_ref().map(|c| c.temporary_did.clone()),
        Some(instance_identity_b.instance_did.clone()),
        "the record's certificate must name node B's derived instance key"
    );

    // `verify` re-checks the whole record against the signed pkarr packet
    // (D-A1-9), and `resolve = true` deliberately overwrites `mechanisms`
    // *after* verification -- so re-verifying that copy would fail for a
    // reason unrelated to what this assertion means to prove. A direct,
    // unresolved lookup is the one `verify` actually applies to.
    let signed_direct = registry_client
        .lookup(&member_master_did, false)
        .await
        .expect("direct (non-resolving) lookup for the member master DID must succeed");
    assert!(
        signed_direct.verify(RecordTrust::Publishing).is_ok(),
        "the substrate-built record must verify under the strict, admission-time trust chain"
    );

    let node_b_substrate_record = registry_client
        .lookup(node_b.did(), false)
        .await
        .expect("lookup of node B's own substrate record failed");
    assert!(
        !node_b_substrate_record.info.mechanisms.is_empty(),
        "node B's own record must carry real mechanisms for the indirection below to be evidence \
         of anything"
    );
    assert_eq!(
        signed.info.mechanisms, node_b_substrate_record.info.mechanisms,
        "resolve = true must copy node B's substrate mechanisms -- D-A1-4's empty-mechanisms + \
         substrate_id indirection, proven through resolution rather than by reading fields"
    );

    let resolved_addr = resolve_iroh_addr(&registry_client, &member_master_did)
        .await
        .expect("resolve_iroh_addr for the member master DID must succeed");
    let node_b_addr = resolve_iroh_addr(&registry_client, node_b.did())
        .await
        .expect("resolve_iroh_addr for node B's own DID must succeed");
    assert_eq!(
        resolved_addr.id, node_b_addr.id,
        "the member master DID must resolve to node B's own Iroh address"
    );

    // Reference-scenario step 4, as a clean relocation. Undeploying first is
    // deliberate: leaving both deployed creates D-A1-11's two-publisher
    // flap, and a test that passes only because the heartbeat is hourly
    // passes for the wrong reason.
    operator_b.undeploy(member_master_did.clone()).await.expect("undeploy from node B failed");

    let mut operator_a = orchestrator_client(&node_a, Identity::from_bytes(&operator.to_bytes()));
    operator_a.connect().await.expect("failed to connect to node A");
    let instance_identity_a = operator_a
        .instance_identity(&member_master_did)
        .await
        .expect("instance-identity query against node A failed");
    assert_ne!(
        instance_identity_a.instance_did, instance_identity_b.instance_did,
        "the two hosting nodes must derive distinct instance keys for the identical member master"
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
    deploy(
        &operator_a,
        &member_master_did,
        bare_tcp_manifest(41002, Some(cert_a.to_json().unwrap())),
    )
    .await
    .expect("deploy on node A with a fresh certificate from the same master must succeed");

    let signed_after = registry_client
        .lookup(&member_master_did, true)
        .await
        .expect("post-relocation lookup(resolve = true) failed");
    assert_eq!(
        signed_after.info.service_id, member_master_did,
        "service_id -- the member master DID -- is unchanged by relocation"
    );
    assert_eq!(
        signed_after.info.delegation.as_ref().map(|c| c.temporary_did.clone()),
        Some(instance_identity_a.instance_did.clone()),
        "the record now names node A's derived instance key"
    );

    let resolved_addr_after = resolve_iroh_addr(&registry_client, &member_master_did)
        .await
        .expect("resolve_iroh_addr after relocation must succeed");
    let node_a_addr = resolve_iroh_addr(&registry_client, node_a.did())
        .await
        .expect("resolve_iroh_addr for node A's own DID must succeed");
    assert_eq!(
        resolved_addr_after.id, node_a_addr.id,
        "the member master DID must now resolve to node A's own Iroh address"
    );
    assert_ne!(
        resolved_addr_after.id, resolved_addr.id,
        "relocation must actually move the resolved address, with no operator republish action"
    );

    // Negative, failure-matrix row 4, over the wire: a record keyed by the
    // master but signed by a key holding no certificate from it at all.
    let uncertified = Identity::generate().unwrap();
    let forged = EndpointInfo {
        service_id: member_master_did.clone(),
        substrate_id: node_a.did().to_string(),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        delegation: None,
    }
    .sign(&uncertified)
    .expect("failed to sign forged record");
    let res = HttpClient::new()
        .post(format!("{node_a_registry_url}/register"))
        .json(&forged)
        .send()
        .await
        .expect("failed to POST forged record");
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);

    node_a.teardown().await;
    node_b.teardown().await;
}

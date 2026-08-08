#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Health, read-only (M05A Slice A4), proven across two genuinely independent
//! `syneroym-substrate` instances -- the reference scenario's own two-node
//! topology, this time polled rather than deployed.
//!
//! `Node`/`boot_pair` are copied from `multi_substrate_placement_e2e.rs`,
//! trimmed: A4's status query needs no member master, no instance
//! certificate, and no app context, so this file skips A3's minting/
//! certifying machinery entirely and deploys each service with a single raw
//! `orchestrator/deploy` call, the same shape
//! `an_endpoint_cert_bound_to_one_node_is_rejected_by_another` already uses
//! in that file.

use std::{net::TcpListener, sync::Arc, time::Duration};

use rustls::crypto::ring;
use serde_json::Map;
use syneroym_app_orchestration::{AlertStore, models::AppInstanceId};
use syneroym_core::config::{
    ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
    ServiceRegistryRole, SubstrateConfig,
};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::{
    DeployManifest, HealthCheck as WitHealthCheck, NetworkEndpoint,
    ServiceConfig as WitServiceConfig, ServiceType as WitServiceType, SubstrateStatus,
    SyneroymClient, TcpManifest, TcpProbe as WitTcpProbe,
    health::{self, ExpectedService, HealthTarget},
};
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{Mutex, mpsc, mpsc::Sender},
    task::JoinHandle,
};

/// Every `#[tokio::test]` in this file runs concurrently, so each of the
/// four tests needs its own, non-overlapping port block (see
/// `multi_substrate_placement_e2e.rs`'s own note on the same convention --
/// blocks spaced by 100, above the highest block any other e2e file in this
/// crate already uses).
#[derive(Clone, Copy)]
struct PortBlock {
    node_a_iroh: u16,
    node_a_registry: u16,
    node_a_gateway: u16,
    node_b_iroh: u16,
    node_b_registry: u16,
    node_b_gateway: u16,
}

const PORTS_BOTH_HEALTHY: PortBlock = PortBlock {
    node_a_iroh: 9800,
    node_a_registry: 9801,
    node_a_gateway: 9802,
    node_b_iroh: 9900,
    node_b_registry: 9901,
    node_b_gateway: 9902,
};
const PORTS_UNREACHABLE_SUBSTRATE: PortBlock = PortBlock {
    node_a_iroh: 10000,
    node_a_registry: 10001,
    node_a_gateway: 10002,
    node_b_iroh: 10100,
    node_b_registry: 10101,
    node_b_gateway: 10102,
};
const PORTS_FAILING_PROBE: PortBlock = PortBlock {
    node_a_iroh: 10200,
    node_a_registry: 10201,
    node_a_gateway: 10202,
    node_b_iroh: 10300,
    node_b_registry: 10301,
    node_b_gateway: 10302,
};
const PORTS_ALERT_LIFECYCLE: PortBlock = PortBlock {
    node_a_iroh: 10400,
    node_a_registry: 10401,
    node_a_gateway: 10402,
    node_b_iroh: 10500,
    node_b_registry: 10501,
    node_b_gateway: 10502,
};

/// Not sharing a port block keeps the tests here from colliding, but they
/// still each boot full substrate instances (real iroh QUIC socket,
/// self-hosted relay, mainline DHT, wasmtime), and running that many
/// concurrently starves the CI runner's CPU badly enough that iroh's QUIC
/// path validation times out with "no viable network path exists" even
/// though nothing is actually broken. Serializing full-substrate-instance
/// lifetimes within this binary (same fix as `tests/common/mod.rs`'s
/// `SUBSTRATE_TEST_LOCK`) avoids it.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

/// A full, independently-identified `syneroym-substrate` instance. Mirrors
/// `multi_substrate_placement_e2e.rs`'s own `Node`.
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
        owner: &Identity,
    ) -> Self {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let base_path = temp_dir.path().to_path_buf();

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
        let relay_url = shared_relay_url.unwrap_or_else(|| format!("http://localhost:{iroh_port}"));
        config.parent_coordinator.iroh = Some(IrohParentConfig { url: relay_url });
        config.roles.client_gateway = Some(ClientGatewayRole { http_port: gateway_port });
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

/// An app-scoped `orchestrator/{deploy,undeploy,status}` grant, mirroring
/// `multi_substrate_placement_e2e.rs`'s own `app_deploy_grant`.
fn app_deploy_grant(node_owner: &Identity, grantee_did: &str, node_did: &str) -> CapabilityToken {
    let resource = ResourceUri(format!("substrate:{node_did}/app/*"));
    CapabilityToken::issue(
        node_owner,
        grantee_did,
        [
            Ability::ORCHESTRATOR_DEPLOY,
            Ability::ORCHESTRATOR_UNDEPLOY,
            Ability::ORCHESTRATOR_STATUS,
        ]
        .into_iter()
        .map(|a| Capability { with: resource.clone(), can: Ability(a.to_string()), caveats: None })
        .collect(),
        Map::new(),
        3600,
        vec![],
    )
    .expect("issue app deploy grant")
}

async fn client_for(node: &Node, operator: &Identity, grant: CapabilityToken) -> SyneroymClient {
    let mut client = SyneroymClient::new_with_identity(
        node.did().to_string(),
        node.registry_url.clone(),
        Identity::from_bytes(&operator.to_bytes()),
    )
    .with_ucan(grant);
    client.connect().await.expect("failed to connect client");
    client
}

/// The owner's own client, node-wide by construction (`admin_ucan_root`) --
/// no grant needed. This is what a caller reading `node-facts` (D-A4-18)
/// must be.
async fn owner_client_for(node: &Node, owner: &Identity) -> SyneroymClient {
    let mut client = SyneroymClient::new_with_identity(
        node.did().to_string(),
        node.registry_url.clone(),
        Identity::from_bytes(&owner.to_bytes()),
    );
    client.connect().await.expect("failed to connect owner client");
    client
}

/// A single node, self-hosting its own registry and relay -- for a test that
/// only needs one substrate and must not leak a second, idle one (a `Node`
/// has no `Drop` impl; an unused pair's second node would otherwise keep
/// running, bound to its ports, for the rest of the process).
async fn boot_single(owner: &Identity, ports: PortBlock) -> Node {
    let _ = ring::default_provider().install_default();
    Node::boot(ports.node_a_iroh, ports.node_a_registry, ports.node_a_gateway, None, None, owner)
        .await
}

async fn boot_pair(owner: &Identity, ports: PortBlock) -> (Node, Node) {
    let _ = ring::default_provider().install_default();

    let node_a = Node::boot(
        ports.node_a_iroh,
        ports.node_a_registry,
        ports.node_a_gateway,
        None,
        None,
        owner,
    )
    .await;
    let node_a_registry_url = node_a.registry_url.clone();
    let node_a_relay_url = format!("http://localhost:{}", ports.node_a_iroh);
    let node_b = Node::boot(
        ports.node_b_iroh,
        ports.node_b_registry,
        ports.node_b_gateway,
        Some(node_a_registry_url),
        Some(node_a_relay_url),
        owner,
    )
    .await;

    node_a.substrate_client.inject_kek("aa".repeat(32)).await.expect("node A inject_kek failed");
    node_b.substrate_client.inject_kek("bb".repeat(32)).await.expect("node B inject_kek failed");

    (node_a, node_b)
}

/// Deploys a bare TCP service directly against `client`'s own node,
/// optionally with a declared health check, via a single raw
/// `orchestrator/deploy` call -- A4's status query needs no member master,
/// instance certificate, or app context, so this skips A3's minting
/// machinery entirely.
async fn deploy_tcp(
    client: &SyneroymClient,
    service_id: &str,
    port: u16,
    health_check: Option<WitHealthCheck>,
) {
    let manifest = DeployManifest {
        config: WitServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check,
        },
        service_type: WitServiceType::Tcp(TcpManifest {
            endpoints: vec![NetworkEndpoint {
                interface_name: "main".to_string(),
                host: "127.0.0.1".to_string(),
                port,
            }],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let params = serde_json::to_value((service_id.to_string(), manifest)).unwrap();
    let res = client.request("orchestrator", "deploy", params).await.unwrap();
    assert!(res.result.get("status").is_some(), "deploy of {service_id} failed: {:?}", res);
}

fn tcp_connect_check() -> WitHealthCheck {
    WitHealthCheck::TcpConnect(WitTcpProbe { interface_name: "main".to_string(), timeout_ms: 2000 })
}

/// Binds an ephemeral port and keeps the listener alive so a `tcp-connect`
/// probe against it passes. No `accept()` call is needed: the kernel
/// completes the handshake and queues the connection in the listen backlog
/// as soon as the port is bound, which is all a `tcp-connect` probe checks.
fn bind_and_accept() -> (u16, TcpListener) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    (port, listener)
}

/// A port nothing listens on -- a `tcp-connect` probe against it fails with
/// connection refused, distinct from a bind error.
fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn expected(name: &str, service_id: &str, did: &str) -> ExpectedService {
    ExpectedService {
        logical_ref: syneroym_app_orchestration::models::LogicalServiceRef {
            app_instance_id: AppInstanceId::new("health-e2e-inst"),
            service_name: syneroym_app_orchestration::models::LogicalServiceName::new(name),
        },
        service_id: service_id.to_string(),
        substrate_did: did.to_string(),
        member_index: 0,
    }
}

#[tokio::test]
async fn both_services_report_healthy_and_each_node_reports_its_own_registry() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b) = boot_pair(&owner, PORTS_BOTH_HEALTHY).await;

    let (port_a, _listener_a) = bind_and_accept();
    let (port_b, _listener_b) = bind_and_accept();

    let operator_did = substrate::derive_did_key(&operator.public_key());
    let client_a =
        client_for(&node_a, &operator, app_deploy_grant(&owner, &operator_did, node_a.did())).await;
    let client_b =
        client_for(&node_b, &operator, app_deploy_grant(&owner, &operator_did, node_b.did())).await;
    deploy_tcp(&client_a, "e2e-frontend", port_a, Some(tcp_connect_check())).await;
    deploy_tcp(&client_b, "e2e-backend", port_b, Some(tcp_connect_check())).await;

    // Node-wide (owner) clients, so `node-facts` (D-A4-18) come back too.
    let owner_client_a = Arc::new(owner_client_for(&node_a, &owner).await);
    let owner_client_b = Arc::new(owner_client_for(&node_b, &owner).await);

    let targets = std::collections::BTreeMap::from([
        (
            node_a.did().to_string(),
            HealthTarget {
                alias: None,
                substrate_did: node_a.did().to_string(),
                query: owner_client_a,
            },
        ),
        (
            node_b.did().to_string(),
            HealthTarget {
                alias: None,
                substrate_did: node_b.did().to_string(),
                query: owner_client_b,
            },
        ),
    ]);
    let expected_services = vec![
        expected("frontend", "e2e-frontend", node_a.did()),
        expected("backend", "e2e-backend", node_b.did()),
    ];

    let report = health::poll_once(&targets, &expected_services).await;
    assert_eq!(report.services.len(), 2, "{report:?}");
    for s in &report.services {
        assert_eq!(s.signal, health::Signal::Healthy, "{s:?}");
    }
    assert_eq!(report.substrates.len(), 2);
    for sub in &report.substrates {
        let facts = sub.node.as_ref().expect("owner client must see node facts");
        let expected_url = if sub.substrate_did == node_a.did() {
            &node_a.registry_url
        } else {
            &node_b.registry_url
        };
        assert_eq!(facts.registry_url.as_deref(), Some(expected_url.as_str()));
    }

    node_a.teardown().await;
    node_b.teardown().await;
}

#[tokio::test]
async fn a_stopped_substrate_is_reported_unreachable_while_the_other_stays_healthy() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b) = boot_pair(&owner, PORTS_UNREACHABLE_SUBSTRATE).await;

    let (port_a, _listener_a) = bind_and_accept();
    let (port_b, _listener_b) = bind_and_accept();

    let operator_did = substrate::derive_did_key(&operator.public_key());
    let client_a =
        client_for(&node_a, &operator, app_deploy_grant(&owner, &operator_did, node_a.did())).await;
    let client_b =
        client_for(&node_b, &operator, app_deploy_grant(&owner, &operator_did, node_b.did())).await;
    deploy_tcp(&client_a, "e2e-frontend", port_a, Some(tcp_connect_check())).await;
    deploy_tcp(&client_b, "e2e-backend", port_b, Some(tcp_connect_check())).await;

    let did_a = node_a.did().to_string();
    let did_b = node_b.did().to_string();
    let owner_client_a: Arc<dyn health::StatusQuery> =
        Arc::new(owner_client_for(&node_a, &owner).await);
    // Node B goes down entirely -- the reference scenario's own step 3.
    node_b.teardown().await;

    let unreachable = UnreachableQuery;
    let targets = std::collections::BTreeMap::from([
        (
            did_a.clone(),
            HealthTarget { alias: None, substrate_did: did_a.clone(), query: owner_client_a },
        ),
        (
            did_b.clone(),
            HealthTarget {
                alias: None,
                substrate_did: did_b.clone(),
                query: Arc::new(unreachable),
            },
        ),
    ]);
    let expected_services = vec![
        expected("frontend", "e2e-frontend", &did_a),
        expected("backend", "e2e-backend", &did_b),
    ];

    let report = health::poll_once(&targets, &expected_services).await;
    let frontend = report.services.iter().find(|s| s.service_id == "e2e-frontend").unwrap();
    let backend = report.services.iter().find(|s| s.service_id == "e2e-backend").unwrap();
    assert_eq!(frontend.signal, health::Signal::Healthy, "{frontend:?}");
    assert!(matches!(backend.signal, health::Signal::SubstrateUnreachable(_)), "{backend:?}");
    // D-A4-13: exactly one substrate-level fault, not a per-service one.
    assert_eq!(
        report.substrates.iter().filter(|s| s.fault.is_some()).count(),
        1,
        "{:?}",
        report.substrates
    );

    node_a.teardown().await;
}

/// A `StatusQuery` that always fails, standing in for a substrate that never
/// came up -- mirrors `roymctl`'s own `UnreachableTarget`.
#[derive(Debug)]
struct UnreachableQuery;

#[async_trait::async_trait]
impl health::StatusQuery for UnreachableQuery {
    async fn status(&self, _service_ids: Vec<String>) -> Result<SubstrateStatus, String> {
        Err("connection refused".to_string())
    }
}

#[tokio::test]
async fn a_failing_readiness_probe_is_distinct_from_a_stopped_instance() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let node_a = boot_single(&owner, PORTS_FAILING_PROBE).await;

    let closed = closed_port();

    let operator_did = substrate::derive_did_key(&operator.public_key());
    let client =
        client_for(&node_a, &operator, app_deploy_grant(&owner, &operator_did, node_a.did())).await;
    deploy_tcp(&client, "e2e-tcp-svc", closed, Some(tcp_connect_check())).await;

    let did = node_a.did().to_string();
    let owner_client = Arc::new(owner_client_for(&node_a, &owner).await);
    let targets = std::collections::BTreeMap::from([(
        did.clone(),
        HealthTarget { alias: None, substrate_did: did.clone(), query: owner_client },
    )]);
    let expected_services = vec![expected("tcpsvc", "e2e-tcp-svc", &did)];

    let report = health::poll_once(&targets, &expected_services).await;
    assert_eq!(report.services.len(), 1);
    // Buildable only because of §0.5's fix: a `tcp` service's phase stays
    // `unknown` (nothing runs "on" this substrate), and the probe -- not the
    // phase -- is the only liveness signal there is.
    assert!(
        matches!(report.services[0].signal, health::Signal::ProbeFailing(_)),
        "{:?}",
        report.services[0]
    );

    node_a.teardown().await;
}

#[tokio::test]
async fn alerts_are_recorded_deduplicated_and_cleared_across_three_sweeps() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let node_a = boot_single(&owner, PORTS_ALERT_LIFECYCLE).await;

    // Nothing listens here yet -- the probe fails.
    let target_port = closed_port();

    let operator_did = substrate::derive_did_key(&operator.public_key());
    let client =
        client_for(&node_a, &operator, app_deploy_grant(&owner, &operator_did, node_a.did())).await;
    deploy_tcp(&client, "e2e-alert-svc", target_port, Some(tcp_connect_check())).await;

    let did = node_a.did().to_string();
    let owner_client = Arc::new(owner_client_for(&node_a, &owner).await);
    let targets = std::collections::BTreeMap::from([(
        did.clone(),
        HealthTarget { alias: None, substrate_did: did.clone(), query: owner_client },
    )]);
    let expected_services = vec![expected("alertsvc", "e2e-alert-svc", &did)];
    let instance_id = AppInstanceId::new("health-e2e-inst");
    let alerts = AlertStore::open_in_memory().unwrap();

    // Sweep 1: fault, opens a new incident.
    let report1 = health::poll_once(&targets, &expected_services).await;
    assert!(matches!(report1.services[0].signal, health::Signal::ProbeFailing(_)));
    let opened1 = health::record_report(
        &alerts,
        &instance_id,
        &report1,
        1000,
        &[],
        health::CertAlertPolicy::Reminder,
    )
    .unwrap();
    assert_eq!(opened1.len(), 1);
    assert_eq!(alerts.active(&instance_id).unwrap().len(), 1);

    // Sweep 2: still failing -- refreshes the existing row, opens nothing new.
    let report2 = health::poll_once(&targets, &expected_services).await;
    let opened2 = health::record_report(
        &alerts,
        &instance_id,
        &report2,
        1001,
        &[],
        health::CertAlertPolicy::Reminder,
    )
    .unwrap();
    assert!(opened2.is_empty(), "a second failing sweep must not open a second row");
    assert_eq!(alerts.active(&instance_id).unwrap().len(), 1);

    // The orchestrator caches a probe result for a few seconds (D-A4-8: a
    // supervisor polling every few seconds must not turn into probe load on
    // the target), keyed off sweep 1's timestamp. Sweeps 1-2 run back to
    // back, well inside that window, so without a real wait here sweep 3
    // would just replay sweep 1's cached failing result instead of probing
    // the now-fixed listener.
    tokio::time::sleep(Duration::from_secs(6)).await;

    // Sweep 3: fixed -- a real listener now answers, the probe passes, and
    // the alert clears. No `accept()` call needed (see `bind_and_accept`):
    // the kernel completes the handshake and queues it in the listen
    // backlog as soon as the socket is bound, which is all a `tcp-connect`
    // probe checks.
    let listener = TcpListener::bind(format!("127.0.0.1:{target_port}")).unwrap();
    let report3 = health::poll_once(&targets, &expected_services).await;
    assert_eq!(report3.services[0].signal, health::Signal::Healthy, "{:?}", report3.services[0]);
    health::record_report(
        &alerts,
        &instance_id,
        &report3,
        1002,
        &[],
        health::CertAlertPolicy::Reminder,
    )
    .unwrap();
    assert!(alerts.active(&instance_id).unwrap().is_empty());
    assert_eq!(
        alerts.all(&instance_id).unwrap().len(),
        1,
        "the cleared row must still be readable"
    );

    drop(listener);
    node_a.teardown().await;
}

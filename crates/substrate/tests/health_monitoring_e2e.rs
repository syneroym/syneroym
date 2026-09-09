#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Health, read-only (M05A Slice A4), proven across two genuinely independent
//! `syneroym-substrate` instances -- the reference scenario's own two-node
//! topology, this time polled rather than deployed.
//!
//! Both nodes come from `common::SubstrateNode`, sharing one registry and
//! relay. The status query here needs no member master, no instance
//! certificate, and no app context, so this file deploys each service with a
//! single raw `orchestrator/deploy` call.

use std::{net::TcpListener, sync::Arc, time::Duration};

use common::SubstrateNode;
use rustls::crypto::ring;
use serde_json::Map;
use syneroym_app_orchestration::{AlertStore, models::AppInstanceId};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::{
    DeployManifest, HealthCheck as WitHealthCheck, NetworkEndpoint,
    ServiceConfig as WitServiceConfig, ServiceType as WitServiceType, SubstrateStatus,
    SyneroymClient, TcpManifest, TcpProbe as WitTcpProbe,
    health::{self, ExpectedService, HealthTarget},
};

mod common;

/// An app-scoped `orchestrator/{deploy,undeploy,status}` grant.
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

async fn client_for(
    node: &SubstrateNode,
    operator: &Identity,
    grant: CapabilityToken,
) -> SyneroymClient {
    let mut client = node.client_as(Identity::from_bytes(&operator.to_bytes())).with_ucan(grant);
    client.connect().await.expect("failed to connect client");
    client
}

/// The owner's own client, node-wide by construction (`admin_ucan_root`) --
/// no grant needed. This is what a caller reading `node-facts` (D-A4-18)
/// must be.
async fn owner_client_for(node: &SubstrateNode, owner: &Identity) -> SyneroymClient {
    let mut client = node.client_as(Identity::from_bytes(&owner.to_bytes()));
    client.connect().await.expect("failed to connect owner client");
    client
}

/// A single node, self-hosting its own registry and relay -- for a test that
/// only needs one substrate.
async fn boot_single(owner: &Identity) -> SubstrateNode {
    let _ = ring::default_provider().install_default();
    SubstrateNode::builder().owner(owner).boot().await
}

/// Two nodes, the second sharing the first's registry and relay. Each
/// injects a KEK during its own boot -- before the other node's boot can
/// leave its connection idle -- so no post-boot redial is needed.
async fn boot_pair(owner: &Identity) -> (SubstrateNode, SubstrateNode) {
    let _ = ring::default_provider().install_default();

    let node_a = SubstrateNode::builder().owner(owner).inject_kek().boot().await;
    let node_b = SubstrateNode::builder()
        .owner(owner)
        .shared_registry(node_a.registry_url())
        .shared_relay(node_a.relay_url())
        .inject_kek()
        .boot()
        .await;
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
            assets: None,
            visibility: None,
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
    assert!(res.result.get("status").is_some(), "deploy of {service_id} failed: {res:?}");
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
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b) = boot_pair(&owner).await;

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
            node_a.registry_url()
        } else {
            node_b.registry_url()
        };
        assert_eq!(facts.registry_url.as_deref(), Some(expected_url));
    }

    node_a.teardown().await;
    node_b.teardown().await;
}

#[tokio::test]
async fn a_stopped_substrate_is_reported_unreachable_while_the_other_stays_healthy() {
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let (node_a, node_b) = boot_pair(&owner).await;

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
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let node_a = boot_single(&owner).await;

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
    let _serial_guard = common::serial_guard().await;
    let owner = Identity::generate().unwrap();
    let operator = Identity::generate().unwrap();
    let node_a = boot_single(&owner).await;

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

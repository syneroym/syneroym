#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! M05A A5c phase 7 (§23), test 48, matrix row 12: a partial deploy across
//! two real managed substrates, one of which is down at `submit` time.
//! `submit` now persists desired state before its own best-effort deploy
//! attempt (so a substrate being unreachable does not stop the *other*
//! service's placement from being recorded), and the resident loop's own
//! pass -- unlike `submit`'s all-or-nothing `build_clients` -- connects
//! best-effort and only asks `apply_plan` to install what it actually
//! connected to (see `SupervisorService::apply_write_phase`'s own comment on
//! why `resolve_targets` would otherwise fail the *whole* filtered plan
//! closed over one missing target). The surviving service is never rolled
//! back, and the next pass retries only the one that did not land, once its
//! substrate answers again.
//!
//! `Node` is copied from `supervisor_interface_e2e.rs` (itself from
//! `multi_substrate_placement_e2e.rs`), with one addition: it boots from a
//! caller-supplied directory instead of a self-owned `TempDir`, so the same
//! managed substrate identity can be torn down and rebooted later in the
//! same test -- simulating "the node comes back", not "a different node
//! joins".

use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use rustls::crypto::ring;
use semver::Version;
use serde_json::{Map, json};
use syneroym_app_orchestration::{
    LocalFilesystemCatalog, compile,
    models::{
        AppBlueprintId, AppInstanceId, LogicalServiceName, PlacementSelector, ServiceConfig,
        ServiceSpec, ServiceType, SubstrateAlias, SynAppManifest,
    },
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_core::config::{
    ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
    ServiceRegistryRole, SubstrateConfig, SupervisorRole,
};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

#[derive(Clone, Copy)]
struct PortBlock {
    supervisor_iroh: u16,
    supervisor_registry: u16,
    supervisor_gateway: u16,
    managed_a_iroh: u16,
    managed_a_registry: u16,
    managed_a_gateway: u16,
    managed_b_iroh: u16,
    managed_b_registry: u16,
    managed_b_gateway: u16,
}

// Continues the running e2e port sequence from 12_400 --
// `supervisor_alerts_e2e.rs` (12_200-12_399) is the last block before this
// one. Each node gets its own hundred-port block (matching every other
// multi-node file's own convention, e.g. `multi_substrate_placement_e2e.rs`)
// -- a closer spacing was tried first and reliably failed a node's relay
// bind with "Address already in use", the same failure that convention
// exists to avoid.
const PORTS: PortBlock = PortBlock {
    supervisor_iroh: 12_400,
    supervisor_registry: 12_401,
    supervisor_gateway: 12_402,
    managed_a_iroh: 12_500,
    managed_a_registry: 12_501,
    managed_a_gateway: 12_502,
    managed_b_iroh: 12_600,
    managed_b_registry: 12_601,
    managed_b_gateway: 12_602,
};

const MANAGED_A_ALIAS: &str = "managed-a";
const MANAGED_B_ALIAS: &str = "managed-b";

/// A full, independently-identified `syneroym-substrate` instance, booted
/// from a caller-owned directory (not a self-owned `TempDir`) so the same
/// identity can be rebooted later in the same test.
struct Node {
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
}

impl Node {
    #[allow(clippy::too_many_arguments)]
    async fn boot(
        base_path: PathBuf,
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        shared_relay_url: Option<String>,
        owner: &Identity,
        supervisor: Option<SupervisorRole>,
    ) -> Self {
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
        config.roles.supervisor = supervisor;

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
        substrate_client.inject_kek(hex::encode([0xabu8; 32])).await.expect("inject_kek failed");

        Self {
            registry_url: effective_registry_url,
            substrate_client,
            shutdown_tx,
            substrate_handle,
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

fn supervisor_role() -> SupervisorRole {
    SupervisorRole {
        // Short enough that the test does not wait a real 30s for a
        // second pass, long enough that two passes cannot be confused
        // for one in flight.
        poll_interval_secs: 3,
        db_name: "supervisor.db".to_string(),
        max_restart_attempts: 3,
        restart_backoff_secs: 30,
        alert_topic: "supervisor/alerts".to_string(),
        master_backup_dir: "master-backups".to_string(),
    }
}

fn node_wide_supervisor_grant(
    node_owner: &Identity,
    grantee_did: &str,
    node_did: &str,
) -> CapabilityToken {
    let resource = ResourceUri::substrate(node_did);
    CapabilityToken::issue(
        node_owner,
        grantee_did,
        [Ability::ORCHESTRATOR_DEPLOY, Ability::ORCHESTRATOR_STATUS]
            .into_iter()
            .map(|a| Capability {
                with: resource.clone(),
                can: Ability(a.to_string()),
                caveats: None,
            })
            .collect(),
        Map::new(),
        3600,
        vec![],
    )
    .expect("issue node-wide supervisor grant")
}

/// Two independent (no `depends_on` between them) services, one per
/// managed alias -- partial failure on one substrate must not depend on
/// binding/dependency resolution succeeding on the other.
fn two_service_manifest() -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("svc-a"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41701".to_string(),
                hash: None,
                interfaces: vec![],
                env: BTreeMap::new(),
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: Default::default(),
                fdae: None,
                health_check: None,
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_A_ALIAS))),
        },
    );
    services.insert(
        LogicalServiceName::new("svc-b"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41702".to_string(),
                hash: None,
                interfaces: vec![],
                env: BTreeMap::new(),
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: Default::default(),
                fdae: None,
                health_check: None,
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_B_ALIAS))),
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:a5c-loop-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

async fn compiled_plan_json(manifest: &SynAppManifest, instance_id: &str) -> String {
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    let compiled = compile(AppInstanceId::new(instance_id), manifest, &catalog).await.unwrap();
    compiled.plans.last().unwrap().to_json().unwrap()
}

fn submission(
    instance_id: &str,
    plan_json: String,
    inventory_json: String,
    generation: u64,
) -> serde_json::Value {
    json!([{
        "app_instance_id": instance_id,
        "plan_json": plan_json,
        "inventory_json": inventory_json,
        "generation": generation,
    }])
}

/// One service's `signal` string out of a `status` response's `services`
/// array, or `None` if that logical ref is not present at all.
fn signal_of<'a>(services: &'a [serde_json::Value], logical_ref: &str) -> Option<&'a str> {
    services
        .iter()
        .find(|s| s.get("logical_ref").and_then(|v| v.as_str()) == Some(logical_ref))
        .and_then(|s| s.get("signal"))
        .and_then(|v| v.as_str())
}

#[tokio::test]
async fn a_partial_deploy_is_degraded_and_its_failed_service_is_retried_without_rollback() {
    let _ = ring::default_provider().install_default();

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    let supervisor_dir = tempfile::tempdir().expect("failed to create temp dir");
    let supervisor_node = Node::boot(
        supervisor_dir.path().to_path_buf(),
        PORTS.supervisor_iroh,
        PORTS.supervisor_registry,
        PORTS.supervisor_gateway,
        None,
        None,
        &supervisor_owner,
        Some(supervisor_role()),
    )
    .await;
    let shared_registry = supervisor_node.registry_url.clone();
    let shared_relay = format!("http://localhost:{}", PORTS.supervisor_iroh);

    let managed_a_dir = tempfile::tempdir().expect("failed to create temp dir");
    let managed_a = Node::boot(
        managed_a_dir.path().to_path_buf(),
        PORTS.managed_a_iroh,
        PORTS.managed_a_registry,
        PORTS.managed_a_gateway,
        Some(shared_registry.clone()),
        Some(shared_relay.clone()),
        &managed_owner,
        None,
    )
    .await;

    // `managed_b_dir` is a `TempDir` this test owns directly (not a `Node`
    // field) precisely so it survives `managed_b`'s own teardown below --
    // the whole point is to reboot the *same* identity, not a fresh one.
    let managed_b_dir = tempfile::tempdir().expect("failed to create temp dir");
    let managed_b = Node::boot(
        managed_b_dir.path().to_path_buf(),
        PORTS.managed_b_iroh,
        PORTS.managed_b_registry,
        PORTS.managed_b_gateway,
        Some(shared_registry.clone()),
        Some(shared_relay.clone()),
        &managed_owner,
        None,
    )
    .await;
    let managed_b_did = managed_b.did().to_string();
    let managed_b_registry_url = managed_b.registry_url.clone();

    let grant_a =
        node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), managed_a.did());
    let grant_b = node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), &managed_b_did);
    let inventory_json = serde_json::to_string(&BTreeMap::from([
        (
            MANAGED_A_ALIAS.to_string(),
            SupervisorInventoryEntry {
                did: managed_a.did().to_string(),
                api_url: Some(managed_a.registry_url.clone()),
                ucan: Some(grant_a),
            },
        ),
        (
            MANAGED_B_ALIAS.to_string(),
            SupervisorInventoryEntry {
                did: managed_b_did.clone(),
                api_url: Some(managed_b_registry_url.clone()),
                ucan: Some(grant_b),
            },
        ),
    ]))
    .unwrap();

    let instance_id = "a5c-loop-inst";
    let manifest = two_service_manifest();
    let plan_json = compiled_plan_json(&manifest, instance_id).await;

    // Managed B goes down before submit -- the reference scenario's own
    // "one substrate is unreachable at submit time" step. `submit`'s own
    // best-effort apply (build_clients is all-or-nothing across the
    // *whole* plan) cannot land anything synchronously once any one
    // alias is unreachable, so this call is expected to error -- but the
    // desired state underneath it must still be durably recorded.
    managed_b.teardown().await;

    let submit_res = time::timeout(
        Duration::from_secs(20),
        supervisor_node.substrate_client.request(
            "supervisor",
            "submit",
            submission(instance_id, plan_json, inventory_json, 0),
        ),
    )
    .await
    .expect("submit call timed out");
    assert!(submit_res.is_err(), "submit with one substrate down must surface the failure");

    // The resident loop's own pass (poll_interval_secs=3) connects
    // best-effort and lands `svc-a` on the reachable substrate while
    // `svc-b` stays missing -- polled with a generous budget since each
    // pass that touches the still-down `managed-b` spends up to
    // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` (10s) doing so. The manifest
    // declares no health check (D-A4-19: a `tcp` service with no probe
    // reports `unknown`, not a fault), so "landed" is read off `signal
    // != "not-deployed"`, not `"healthy"`.
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let status = supervisor_node
            .substrate_client
            .request("supervisor", "status", json!([instance_id]))
            .await
            .expect("status failed");
        let services =
            status.result.get("services").and_then(|s| s.as_array()).cloned().unwrap_or_default();
        let state = status.result.get("state").and_then(|v| v.as_str()).unwrap_or("");
        if signal_of(&services, "a5c-loop-inst/svc-a") == Some("unknown") && state == "Degraded" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "svc-a never landed (state={state}, services={services:?})"
        );
        time::sleep(Duration::from_millis(500)).await;
    }

    // The landed service is not rolled back by any later pass.
    let status = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!([instance_id]))
        .await
        .expect("status failed");
    let services =
        status.result.get("services").and_then(|s| s.as_array()).cloned().unwrap_or_default();
    assert_eq!(signal_of(&services, "a5c-loop-inst/svc-a"), Some("unknown"), "{services:?}");

    // Managed B comes back, same identity -- the loop's next pass must
    // retry only the failed service.
    let managed_b = Node::boot(
        managed_b_dir.path().to_path_buf(),
        PORTS.managed_b_iroh,
        PORTS.managed_b_registry,
        PORTS.managed_b_gateway,
        Some(shared_registry),
        Some(shared_relay),
        &managed_owner,
        None,
    )
    .await;
    assert_eq!(managed_b.did(), managed_b_did, "the rebooted node must keep its identity");

    // A wider budget than the first loop: this pass needs a fresh connect
    // to the just-rebooted node *and* a `resolve-instance-identity`
    // certification round trip *and* the deploy call itself, all against a
    // node whose relay/registry registration may itself still be settling
    // -- confirmed by a diagnostic run that a pass connecting to both
    // substrates can still be mid-`apply_write_phase` when a tighter
    // deadline elapses.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let status = supervisor_node
            .substrate_client
            .request("supervisor", "status", json!([instance_id]))
            .await
            .expect("status failed");
        let services =
            status.result.get("services").and_then(|s| s.as_array()).cloned().unwrap_or_default();
        let state = status.result.get("state").and_then(|v| v.as_str()).unwrap_or("");
        if signal_of(&services, "a5c-loop-inst/svc-a") == Some("unknown")
            && signal_of(&services, "a5c-loop-inst/svc-b") == Some("unknown")
            && state == "Active"
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "svc-b was not retried and landed once managed-b returned (state={state}, \
             services={services:?})"
        );
        time::sleep(Duration::from_millis(500)).await;
    }

    supervisor_node.teardown().await;
    managed_a.teardown().await;
    managed_b.teardown().await;
}

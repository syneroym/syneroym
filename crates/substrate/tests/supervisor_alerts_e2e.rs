#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! M05A A5c phase 4 (§19.5, D-A5c-6), test 27: the `messaging` registration
//! and the publish/subscribe string symmetry, proven live across two real
//! `syneroym-substrate` instances -- no unit test can prove the router's
//! subscribe-side namespacing (`dispatch.rs::subscribe_namespaced_topic`)
//! actually lines up with the supervisor's own publish-side namespacing
//! (`SupervisorService::publish_opened_alerts`) end to end.
//!
//! `Node` is copied from `supervisor_interface_e2e.rs` (itself copied from
//! `multi_substrate_placement_e2e.rs`), not `tests/common`'s harness -- that
//! module's own doc says two live nodes at once deadlock on its
//! setup-serialization lock.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use rustls::crypto::ring;
use semver::Version;
use serde_json::{Map, json};
use syneroym_app_orchestration::{
    AlertKind, LocalFilesystemCatalog, compile,
    models::{
        AppBlueprintId, AppInstanceId, LogicalServiceName, PlacementSelector, ServiceConfig,
        ServiceSpec, ServiceType, SubstrateAlias, SynAppManifest,
    },
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_control_plane::SUPERVISOR_RESERVED_SERVICE_ID;
use syneroym_core::config::{
    ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
    ServiceRegistryRole, SubstrateConfig, SupervisorRole,
};
use syneroym_identity::{Identity, substrate};
use syneroym_mqtt_broker::namespace_topic_for_publish;
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
use tempfile::TempDir;
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
    managed_iroh: u16,
    managed_registry: u16,
    managed_gateway: u16,
}

// New e2e port blocks start at 12_200 -- 8800-12_100 are taken
// (`multi_substrate_placement_e2e.rs`, `health_monitoring_e2e.rs`,
// `binding_push_e2e.rs`, `supervisor_interface_e2e.rs`; see that last file's
// own comment for the running total). Phase 7's `supervisor_loop_e2e.rs`
// continues the sequence from 12_400.
const PORTS_ALERT_PUBLISH: PortBlock = PortBlock {
    supervisor_iroh: 12_200,
    supervisor_registry: 12_201,
    supervisor_gateway: 12_202,
    managed_iroh: 12_300,
    managed_registry: 12_301,
    managed_gateway: 12_302,
};

const MANAGED_ALIAS: &str = "managed";

/// A full, independently-identified `syneroym-substrate` instance.
struct Node {
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    _temp_dir: TempDir,
}

impl Node {
    #[allow(clippy::too_many_arguments)]
    async fn boot(
        iroh_port: u16,
        registry_port: u16,
        gateway_port: u16,
        shared_registry_url: Option<String>,
        shared_relay_url: Option<String>,
        owner: &Identity,
        supervisor: Option<SupervisorRole>,
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
        config.substrate.enable_bep0044_dht = false;
        let relay_url = shared_relay_url.unwrap_or_else(|| format!("http://localhost:{iroh_port}"));
        config.parent_coordinator.iroh = Some(IrohParentConfig { url: relay_url });
        config.roles.client_gateway =
            Some(ClientGatewayRole { http_port: gateway_port, ..Default::default() });
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
        )
        .with_registry_dht(false);
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

fn supervisor_role() -> SupervisorRole {
    SupervisorRole {
        poll_interval_secs: 30,
        db_name: "supervisor.db".to_string(),
        max_restart_attempts: 3,
        restart_backoff_secs: 30,
        alert_topic: "supervisor/alerts".to_string(),
        master_backup_dir: "master-backups".to_string(),
        ..SupervisorRole::default()
    }
}

/// Node-wide `orchestrator/deploy` **and** `orchestrator/status` for
/// `grantee_did` on `node_did` -- what a supervisor needs on every substrate
/// it manages (§13's fixture note, copied unchanged from
/// `supervisor_interface_e2e.rs`).
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

/// A single-service manifest, `backend` placed on `MANAGED_ALIAS`.
fn one_service_manifest() -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41601".to_string(),
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
                assets: None,
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:a5c-alerts-test-app"),
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

/// Test 27 (§23 phase 4): an operator connects to the supervisor node's own
/// DID, subscribes over its `messaging` native capability to the alert
/// topic, and receives the notification the supervisor publishes when its
/// next `status` sweep opens a `SubstrateUnreachable` alert -- proving
/// D-A5c-6's `messaging` registration and D-A5c-6/§19.5c's publish-side
/// namespacing rule produce the exact same topic string the router's
/// subscribe-side fix (`dispatch.rs::subscribe_namespaced_topic`) computes,
/// with a real broker and a real wire round trip on both ends.
#[tokio::test]
async fn an_operator_subscribed_to_the_alert_topic_receives_an_opened_alert() {
    let _ = ring::default_provider().install_default();

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    let mut supervisor_node = Node::boot(
        PORTS_ALERT_PUBLISH.supervisor_iroh,
        PORTS_ALERT_PUBLISH.supervisor_registry,
        PORTS_ALERT_PUBLISH.supervisor_gateway,
        None,
        None,
        &supervisor_owner,
        Some(supervisor_role()),
    )
    .await;
    let shared_registry = supervisor_node.registry_url.clone();
    let shared_relay = format!("http://localhost:{}", PORTS_ALERT_PUBLISH.supervisor_iroh);
    let managed_node = Node::boot(
        PORTS_ALERT_PUBLISH.managed_iroh,
        PORTS_ALERT_PUBLISH.managed_registry,
        PORTS_ALERT_PUBLISH.managed_gateway,
        Some(shared_registry),
        Some(shared_relay),
        &managed_owner,
        None,
    )
    .await;
    let managed_did = managed_node.did().to_string();

    let grant =
        node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), managed_node.did());
    let inventory_json = serde_json::to_string(&BTreeMap::from([(
        MANAGED_ALIAS.to_string(),
        SupervisorInventoryEntry {
            did: managed_did.clone(),
            api_url: Some(managed_node.registry_url.clone()),
            ucan: Some(grant),
        },
    )]))
    .unwrap();

    let instance_id = "a5c-alerts-inst";
    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, instance_id).await;
    let submit_params = submission(instance_id, plan_json, inventory_json, 0);

    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `Node::boot`, then sat idle for the entire
    // `managed_node` boot that followed (a second full substrate start) --
    // long enough under CI's scheduling pressure for the peer to abandon
    // that idle path ("no viable network path exists: last path abandoned
    // by peer"; same root cause fixed in `app_instance_identity_e2e.rs`).
    // `SyneroymClient::connect` no-ops on an already-`Some` connection, so
    // recovering means an explicit `shutdown`-then-`connect` (redial)
    // before one retry, not just retrying the same request on the same
    // dead connection.
    if supervisor_node
        .substrate_client
        .request("supervisor", "submit", submit_params.clone())
        .await
        .is_err()
    {
        supervisor_node
            .substrate_client
            .shutdown()
            .await
            .expect("failed to reset supervisor_node's stale connection");
        supervisor_node
            .substrate_client
            .connect()
            .await
            .expect("failed to reconnect supervisor_node");
    }
    let submit_res = supervisor_node
        .substrate_client
        .request("supervisor", "submit", submit_params)
        .await
        .expect("submit failed");
    let masters =
        submit_res.result.get("masters").and_then(|m| m.as_array()).expect("masters array");
    assert_eq!(masters.len(), 1, "the single-service manifest mints exactly one master");

    // The operator subscribes before the fault that will raise the alert,
    // as a real subscriber would -- the target is the supervisor node's own
    // DID (the same one `submit`/`status` are addressed to), and the topic
    // is unnamespaced, exactly what `SyneroymClient::subscribe` expects and
    // exactly what the router's `subscribe_namespaced_topic` and the
    // supervisor's own `publish_opened_alerts` independently namespace to
    // the same final string.
    let operator_identity = Identity::generate().unwrap();
    let mut operator = SyneroymClient::new_with_identity(
        supervisor_node.did().to_string(),
        supervisor_node.registry_url.clone(),
        operator_identity,
    )
    .with_registry_dht(false);
    operator.connect().await.expect("operator failed to connect to the supervisor node");
    let mut alert_stream = time::timeout(
        Duration::from_secs(10),
        operator.subscribe("messaging", &format!("supervisor/alerts/{instance_id}")),
    )
    .await
    .expect("subscribe timed out")
    .expect("subscribe failed");

    // Tear the managed node down entirely -- the reference scenario's own
    // "substrate goes away" step. The next `status` sweep's best-effort
    // connect to it fails, which is a live `SubstrateUnreachable`, not a
    // fabricated one.
    managed_node.teardown().await;

    time::timeout(
        Duration::from_secs(20),
        supervisor_node.substrate_client.request("supervisor", "status", json!([instance_id])),
    )
    .await
    .expect("status call timed out")
    .expect("status failed");

    let (topic, payload) = time::timeout(Duration::from_secs(5), alert_stream.recv())
        .await
        .expect("did not time out waiting for the published alert")
        .expect("alert stream closed unexpectedly");
    let expected_topic = namespace_topic_for_publish(
        SUPERVISOR_RESERVED_SERVICE_ID,
        &format!("supervisor/alerts/{instance_id}"),
    );
    assert_eq!(topic, expected_topic);
    let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(value["app_instance_id"], instance_id);
    assert_eq!(value["kind"], AlertKind::SubstrateUnreachable.to_string());
    assert_eq!(value["label"], managed_did);

    let _ = operator.shutdown().await;
    supervisor_node.teardown().await;
}

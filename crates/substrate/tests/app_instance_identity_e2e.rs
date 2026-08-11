#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The app-instance master identity end to end (M05A A7), across two
//! genuinely independent `syneroym-substrate` instances -- the operator's
//! own sequence: `submit`, `adopt`, `status`, `export-master`, a second
//! `adopt`. `Node` is copied from `supervisor_interface_e2e.rs`, with one
//! addition (`app_data_dir`) so this file can confirm `export-master`
//! wrote a real file under the node's own `master_backup_dir`, not just
//! that the RPC returned a path string.

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

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
use tempfile::TempDir;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
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

// The next free block after `reference_scenario_e2e.rs`'s 12_700-12_902,
// the highest claimed so far -- cargo runs integration-test binaries
// concurrently, so reusing a claimed block crashes whichever binary binds
// second with "Address already in use".
const PORTS_APP_INSTANCE_IDENTITY: PortBlock = PortBlock {
    supervisor_iroh: 13_000,
    supervisor_registry: 13_001,
    supervisor_gateway: 13_002,
    managed_iroh: 13_100,
    managed_registry: 13_101,
    managed_gateway: 13_102,
};

const MANAGED_ALIAS: &str = "managed";

/// A full, independently-identified `syneroym-substrate` instance.
struct Node {
    registry_url: String,
    substrate_client: SyneroymClient,
    shutdown_tx: Sender<()>,
    substrate_handle: JoinHandle<()>,
    /// M05A A7: kept (unlike the file this struct is copied from) so the
    /// test can find the real file `export-master` writes under this
    /// node's own `master_backup_dir`, not just trust the RPC's returned
    /// path string.
    app_data_dir: PathBuf,
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
            app_data_dir: config.app_data_dir,
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
/// `grantee_did` on `node_did`, issued by `node_owner` -- what a supervisor
/// needs on every substrate it manages.
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
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:a7-test-app"),
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

/// Boots a supervisor node and a managed node (B sharing A's
/// registry/relay), grants the supervisor's own node-wide `orchestrator/
/// deploy` on the managed node, and returns everything a test needs to
/// call `submit`.
async fn boot_pair(
    supervisor_owner: &Identity,
    managed_owner: &Identity,
    ports: PortBlock,
) -> (Node, Node, String) {
    let _ = ring::default_provider().install_default();

    let supervisor_node = Node::boot(
        ports.supervisor_iroh,
        ports.supervisor_registry,
        ports.supervisor_gateway,
        None,
        None,
        supervisor_owner,
        Some(supervisor_role()),
    )
    .await;
    let shared_registry = supervisor_node.registry_url.clone();
    let shared_relay = format!("http://localhost:{}", ports.supervisor_iroh);
    let managed_node = Node::boot(
        ports.managed_iroh,
        ports.managed_registry,
        ports.managed_gateway,
        Some(shared_registry),
        Some(shared_relay),
        managed_owner,
        None,
    )
    .await;

    let grant =
        node_wide_supervisor_grant(managed_owner, supervisor_node.did(), managed_node.did());
    let inventory_json = serde_json::to_string(&BTreeMap::from([(
        MANAGED_ALIAS.to_string(),
        SupervisorInventoryEntry {
            did: managed_node.did().to_string(),
            api_url: Some(managed_node.registry_url.clone()),
            ucan: Some(grant),
        },
    )]))
    .unwrap();

    (supervisor_node, managed_node, inventory_json)
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

/// Test 98: the operator's own sequence over a real supervisor and a real
/// managed substrate -- `submit`, `adopt`, then (a) `adopt`'s result
/// carries a `did:key:` app master and a vault name, (b) `status` reports
/// the same DID, (c) `export-master` with that name writes a file under
/// the node's `master_backup_dir`, and (d) a second `adopt` reports the
/// identical DID at a higher generation. The individual properties are
/// already proven at unit scale (tests 90-97, 99-101); the claim here is
/// the sequence, in the operator's own order.
#[tokio::test]
async fn an_adopted_app_instance_carries_an_exportable_master_did() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_APP_INSTANCE_IDENTITY).await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "a7-adopt-inst").await;
    let submit_params = submission("a7-adopt-inst", plan_json, inventory_json, 0);
    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `boot_pair`, then sat idle for the entire
    // `managed_node` boot that followed (a second full substrate start --
    // wasmtime/DHT/relay init) before this, the test's first real call,
    // reuses it -- long enough under CI's scheduling pressure for the peer
    // to abandon that idle path ("no viable network path exists: last path
    // abandoned by peer"; same root cause as `federated_fdae_e2e.rs`'s
    // `hr_data_client`). `SyneroymClient::connect` no-ops on an
    // already-`Some` connection, so recovering means an explicit
    // `shutdown`-then-`connect` (redial) before one retry, not just
    // retrying the same request on the same dead connection.
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
    supervisor_node
        .substrate_client
        .request("supervisor", "submit", submit_params)
        .await
        .expect("submit failed");

    // (a) `adopt`'s result carries a `did:key:` app master and a vault
    // name (D-A7-8).
    let adopted = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a7-adopt-inst"]))
        .await
        .expect("adopt failed");
    assert_eq!(adopted.result.get("generation").and_then(serde_json::Value::as_u64), Some(1));
    let app_master_did = adopted
        .result
        .get("app_master_did")
        .and_then(|v| v.as_str())
        .expect("adopt-result carries app_master_did")
        .to_string();
    assert!(app_master_did.starts_with("did:key:"), "{app_master_did}");
    let vault_name = adopted
        .result
        .get("vault_name")
        .and_then(|v| v.as_str())
        .expect("adopt-result carries vault_name")
        .to_string();
    assert_eq!(vault_name, "app-a7-adopt-inst");

    // (b) `status` reports the same DID (D-A7-6).
    let status = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["a7-adopt-inst"]))
        .await
        .expect("status failed");
    assert_eq!(
        status.result.get("app_master_did").and_then(|v| v.as_str()),
        Some(app_master_did.as_str())
    );

    // (c) `export-master` with that name writes a real file under this
    // node's own `master_backup_dir` (`task.md`'s "movable through
    // `export-master`/`import-master`", D-A7-3).
    let exported = supervisor_node
        .substrate_client
        .request("supervisor", "export-master", json!([vault_name.clone()]))
        .await
        .expect("export-master failed");
    let exported_path =
        PathBuf::from(exported.result.as_str().expect("export-master returns a path string"));
    assert_eq!(
        exported_path,
        supervisor_node.app_data_dir.join("master-backups").join(format!("{vault_name}.key"))
    );
    assert!(
        tokio::fs::metadata(&exported_path).await.is_ok(),
        "export-master must have written a real file at {}",
        exported_path.display()
    );

    // (d) a second `adopt` reports the identical DID at a higher
    // generation (D-A7-5).
    let adopted_again = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a7-adopt-inst"]))
        .await
        .expect("second adopt failed");
    assert_eq!(adopted_again.result.get("generation").and_then(serde_json::Value::as_u64), Some(2));
    assert_eq!(
        adopted_again.result.get("app_master_did").and_then(|v| v.as_str()),
        Some(app_master_did.as_str())
    );

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

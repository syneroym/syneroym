#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Tier 1 of the logical discovery overlay (ADR-0022 §2), proven across two
//! genuinely independent `syneroym-substrate` instances -- a caller outside
//! the app instance resolving "which supervisor holds this app" through the
//! same registry every other DID in the system already uses.
//!
//! `Node`, `boot_pair`, `supervisor_role`, `one_service_manifest`,
//! `compiled_plan_json`, `node_wide_supervisor_grant`, and `submission` are
//! copied from `app_instance_identity_e2e.rs`, with `poll_interval_secs`
//! lowered so the resident loop's own Tier-1 publish -- which nothing on the
//! `supervisor` RPC surface triggers synchronously (`force-reconcile` calls
//! `deploy_submission` directly, not the write-phase gate the resident loop
//! evaluates) -- lands inside this test's own poll budget.

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
use syneroym_core::{
    config::{
        ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
        ServiceRegistryRole, SubstrateConfig, SupervisorRole,
    },
    dht_registry::{EndpointInfo, EndpointType, RegistryClient},
};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
use tempfile::TempDir;
use tokio::{
    sync::{Mutex, mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

/// Every test in this binary boots one or more full substrate
/// instances (real iroh QUIC socket, self-hosted relay, wasmtime).
/// Running every test's own full stack concurrently (Rust's default
/// test harness) means many simultaneous substrate processes' worth
/// of sockets/fds at once -- CPU starvation and, on a low
/// `ulimit -n`, real fd exhaustion. Same fix as `tests/common/mod.rs`'s
/// `SUBSTRATE_TEST_LOCK`.
static SUBSTRATE_TEST_LOCK: Mutex<()> = Mutex::const_new(());

#[path = "common/retry.rs"]
mod retry;

#[derive(Clone, Copy)]
struct PortBlock {
    supervisor_iroh: u16,
    supervisor_registry: u16,
    supervisor_gateway: u16,
    managed_iroh: u16,
    managed_registry: u16,
    managed_gateway: u16,
}

// The next free block after `saga_e2e.rs`'s 14_200-14_302, the highest
// claimed at the time this file was written -- cargo runs integration-test
// binaries concurrently, so reusing a claimed block crashes whichever binary
// binds second with "Address already in use". Each of this file's two tests
// gets its own block: they run concurrently within one binary too.
const PORTS_RESOLVES_THROUGH_REGISTRY: PortBlock = PortBlock {
    supervisor_iroh: 15_000,
    supervisor_registry: 15_001,
    supervisor_gateway: 15_002,
    managed_iroh: 15_100,
    managed_registry: 15_101,
    managed_gateway: 15_102,
};
const PORTS_FORGED_RECORD_REJECTED: PortBlock = PortBlock {
    supervisor_iroh: 15_200,
    supervisor_registry: 15_201,
    supervisor_gateway: 15_202,
    managed_iroh: 15_300,
    managed_registry: 15_301,
    managed_gateway: 15_302,
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

/// `poll_interval_secs` is lowered to 2s from the 30s default so this
/// test's own poll budget does not have to be minutes long -- nothing on
/// the `supervisor` RPC surface triggers a Tier-1 publish synchronously,
/// so the resident loop's own tick is what this test is actually waiting
/// on.
fn supervisor_role() -> SupervisorRole {
    SupervisorRole {
        poll_interval_secs: 2,
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
/// it manages.
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
                source: "127.0.0.1:41901".to_string(),
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
        id: AppBlueprintId::new("syneroym:tier1-test-app"),
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
/// deploy` on the managed node, and returns everything a test needs to call
/// `submit`. Copied from `app_instance_identity_e2e.rs`.
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

/// The reference scenario's steps 1-2: submit and adopt an app instance,
/// confirm the app master DID on `status`, then assert the Tier-1 record
/// resolves through the registry -- naming the supervisor and verifying
/// against the app DID with no other trust input.
#[tokio::test]
async fn an_app_did_resolves_to_its_supervising_node_through_the_registry() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_RESOLVES_THROUGH_REGISTRY).await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "tier1-resolve-inst").await;
    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `Node::boot` inside `boot_pair`, then sat
    // idle through the managed node's own full boot that followed -- long
    // enough under CI's scheduling pressure for the peer to abandon that
    // idle path ("no viable network path exists: last path abandoned by
    // peer"; same root cause fixed throughout this crate's e2e tests, e.g.
    // `binding_push_e2e.rs`). Recover by explicit shutdown→reconnect
    // before one retry.
    let submit_params = submission("tier1-resolve-inst", plan_json, inventory_json, 0);
    crate::call_with_reconnect!(
        supervisor_node.substrate_client,
        "supervisor",
        "submit",
        submit_params
    );
    let adopted = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["tier1-resolve-inst"]))
        .await
        .expect("adopt failed");
    let app_did = adopted
        .result
        .get("app_master_did")
        .and_then(|v| v.as_str())
        .expect("adopt-result carries app_master_did")
        .to_string();
    assert!(app_did.starts_with("did:key:"), "{app_did}");

    // Confirmed on `status` too (D-A7-6, already proven at unit scale) --
    // the DID this test then resolves through the registry is the exact
    // one the operator would read off `status`.
    let status = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["tier1-resolve-inst"]))
        .await
        .expect("status failed");
    assert_eq!(
        status.result.get("app_master_did").and_then(|v| v.as_str()),
        Some(app_did.as_str())
    );

    // The resident loop's own tick publishes the Tier-1 record; nothing on
    // the RPC surface triggers it synchronously (`force-reconcile` bypasses
    // the write-phase gate the loop itself evaluates), so this polls for
    // it rather than asserting immediately.
    let registry_client = RegistryClient::new(false, Some(supervisor_node.registry_url.clone()));
    let deadline = Instant::now() + Duration::from_secs(60);
    let signed = loop {
        match registry_client.lookup(&app_did, false).await {
            Ok(signed) => break signed,
            Err(e) => {
                assert!(
                    Instant::now() < deadline,
                    "the Tier-1 record never resolved through the registry: {e}"
                );
                time::sleep(Duration::from_millis(300)).await;
            }
        }
    };

    // Looking up the app DID returns the supervising node, and the record
    // verifies against the app DID with no other trust input.
    assert_eq!(
        signed.info.substrate_id,
        supervisor_node.did(),
        "the record must name the substrate supervising this app, not the app DID itself"
    );
    assert_eq!(signed.info.service_id, app_did);
    assert_eq!(signed.info.endpoint_type, EndpointType::Substrate);
    assert!(signed.verify().is_ok(), "a freshly published Tier-1 record must verify");

    // `status`'s own expiry field (D-C-2) is populated once a publish has
    // actually landed.
    let status_after = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["tier1-resolve-inst"]))
        .await
        .expect("status failed");
    assert!(
        status_after.result.get("app_record_expires_at").and_then(|v| v.as_u64()).is_some(),
        "status must report the published record's expiry: {:?}",
        status_after.result
    );

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// Failure-matrix row 1: a Tier-1 record claiming an app DID as its
/// `service_id` but signed by a key unrelated to it must be rejected at the
/// registry, in the shape `master_endpoint_record_e2e.rs`'s own
/// hand-forged-record case already uses.
#[tokio::test]
async fn a_forged_tier1_record_is_rejected_at_the_registry() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (supervisor_node, managed_node, _inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_FORGED_RECORD_REJECTED).await;

    let claimed_app_master = Identity::generate().unwrap();
    let claimed_app_did = substrate::derive_did_key(&claimed_app_master.public_key());
    let uncertified = Identity::generate().unwrap();

    let forged = EndpointInfo {
        service_id: claimed_app_did,
        substrate_id: supervisor_node.did().to_string(),
        endpoint_type: EndpointType::Substrate,
        mechanisms: vec![],
        nickname: Some("forged".to_string()),
        is_private: false,
        ttl: None,
        not_after: u64::MAX / 2,
        generation: 0,
    }
    .sign(&uncertified)
    .expect("failed to sign forged record");

    let res = reqwest::Client::new()
        .post(format!("{}/register", supervisor_node.registry_url))
        .json(&forged)
        .send()
        .await
        .expect("failed to POST forged record");
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The `supervisor` interface end to end (M05A A5b), across two genuinely
//! independent `syneroym-substrate` instances: one running the supervisor
//! role, one plain managed substrate. `Node` is copied from
//! `multi_substrate_placement_e2e.rs`, not `tests/common`'s harness -- that
//! module's own doc says two live nodes at once deadlock on its
//! setup-serialization lock.
//!
//! The supervisor's own grant on a managed substrate is hand-issued here
//! (§13's fixture note): `submit` cannot bootstrap its own authority, and
//! nothing in this milestone issues a supervisor a grant automatically.

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

// Ports 8800-10900 are already claimed, in that same six-per-block shape,
// by `multi_substrate_placement_e2e.rs` (8800-9700),
// `health_monitoring_e2e.rs` (9800-10500), and `binding_push_e2e.rs`
// (10600-10900) -- cargo runs integration-test binaries concurrently, so
// reusing any of those crashes whichever binary binds second with "Address
// already in use" (B2, Slice A5b review). This file continues the same
// global sequence from 11_000.
const PORTS_SUBMIT_AND_STATUS: PortBlock = PortBlock {
    supervisor_iroh: 11_000,
    supervisor_registry: 11_001,
    supervisor_gateway: 11_002,
    managed_iroh: 11_100,
    managed_registry: 11_101,
    managed_gateway: 11_102,
};
const PORTS_SECOND_SUPERVISOR_LOSES: PortBlock = PortBlock {
    supervisor_iroh: 11_200,
    supervisor_registry: 11_201,
    supervisor_gateway: 11_202,
    managed_iroh: 11_300,
    managed_registry: 11_301,
    managed_gateway: 11_302,
};
const PORTS_DEPLOYS_A_BOUND_APP: PortBlock = PortBlock {
    supervisor_iroh: 11_400,
    supervisor_registry: 11_401,
    supervisor_gateway: 11_402,
    managed_iroh: 11_500,
    managed_registry: 11_501,
    managed_gateway: 11_502,
};
const PORTS_ADOPT_CLAIMS_THE_NEXT: PortBlock = PortBlock {
    supervisor_iroh: 11_600,
    supervisor_registry: 11_601,
    supervisor_gateway: 11_602,
    managed_iroh: 11_700,
    managed_registry: 11_701,
    managed_gateway: 11_702,
};
const PORTS_PUSHED_BINDING: PortBlock = PortBlock {
    supervisor_iroh: 11_800,
    supervisor_registry: 11_801,
    supervisor_gateway: 11_802,
    managed_iroh: 11_900,
    managed_registry: 11_901,
    managed_gateway: 11_902,
};
const PORTS_SUPERSEDED: PortBlock = PortBlock {
    supervisor_iroh: 12_000,
    supervisor_registry: 12_001,
    supervisor_gateway: 12_002,
    managed_iroh: 12_100,
    managed_registry: 12_101,
    managed_gateway: 12_102,
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
    }
}

/// Node-wide `orchestrator/deploy` **and** `orchestrator/status` for
/// `grantee_did` on `node_did`, issued by `node_owner` -- what a supervisor
/// needs on every substrate it manages, not an app-scoped selector.
/// `deploy` alone (§0.28/D-A5-24's own focus, `claim`/`release`'s gate) is
/// not enough: `resolve-instance-identity` (certification) and
/// `app-instance-management-of` (`adopt`'s read half, before any claim
/// exists to match ownership against) both gate on `orchestrator/status`
/// instead, and the two abilities are deliberately flat -- neither entails
/// the other.
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
                source: "127.0.0.1:41401".to_string(),
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
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:a5b-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

/// `frontend` (depends on `backend`), both placed on `MANAGED_ALIAS` -- "a
/// bound app" for test 25/27: one substrate, a real dependency between two
/// members the supervisor deploys together.
fn bound_app_manifest() -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41501".to_string(),
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
        },
    );
    services.insert(
        LogicalServiceName::new("frontend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41502".to_string(),
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
            depends_on: vec![LogicalServiceName::new("backend")],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_ALIAS))),
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:a5b-bound-app"),
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

#[tokio::test]
async fn an_operator_submits_and_reads_back_status_over_the_supervisor_interface() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_SUBMIT_AND_STATUS).await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-submit-inst").await;

    let res = supervisor_node
        .substrate_client
        .request(
            "supervisor",
            "submit",
            submission("a5b-submit-inst", plan_json, inventory_json, 0),
        )
        .await
        .expect("submit failed");
    let masters = res.result.get("masters").and_then(|m| m.as_array()).expect("masters array");
    assert_eq!(masters.len(), 1);

    let status = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["a5b-submit-inst"]))
        .await
        .expect("status failed");
    let services =
        status.result.get("services").and_then(|s| s.as_array()).expect("services array");
    assert_eq!(services.len(), 1);
    // The manifest declares no health check, and a `tcp` service runs
    // outside the substrate, which has no other liveness signal for it
    // (sdk::health::Signal) -- "unknown", not a fault, and not "healthy"
    // either, since nothing here can actually confirm that.
    assert_eq!(services[0].get("signal").and_then(|v| v.as_str()), Some("unknown"));

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

#[tokio::test]
async fn a_second_supervisor_that_has_not_adopted_loses_every_write() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_SECOND_SUPERVISOR_LOSES).await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-second-inst").await;

    // First supervisor submits and adopts, claiming generation 1.
    supervisor_node
        .substrate_client
        .request(
            "supervisor",
            "submit",
            submission("a5b-second-inst", plan_json, inventory_json, 0),
        )
        .await
        .expect("first submit failed");
    let adopted = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a5b-second-inst"]))
        .await
        .expect("adopt failed");
    assert_eq!(adopted.result.as_u64(), Some(1));

    let status = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["a5b-second-inst"]))
        .await
        .expect("status failed");
    let service_id = status
        .result
        .get("services")
        .and_then(|s| s.as_array())
        .and_then(|s| s.first())
        .and_then(|s| s.get("service_id"))
        .and_then(|v| v.as_str())
        .expect("deployed service_id in status")
        .to_string();

    // A rogue second writer -- one that never adopted -- presents
    // generation 0 directly against the managed substrate. N1 (Slice A5b
    // review round 2) added a local pre-flight check to `handle_submit`,
    // so reusing the *same* supervisor's own `submit` no longer reaches
    // the substrate at all once its own store disagrees -- it would now
    // prove H3's local guard, not row 8's substrate-side one. A raw
    // second client, the same shape B4's row-9 test uses to simulate a
    // second writer, reaches the managed node's own generation gate
    // directly instead.
    let mut second_writer = SyneroymClient::new_with_identity(
        managed_node.did().to_string(),
        managed_node.registry_url.clone(),
        Identity::from_bytes(&managed_owner.to_bytes()),
    );
    second_writer
        .wait_for_ready(Duration::from_secs(30))
        .await
        .expect("second writer could not reach the managed node");
    let err = second_writer
        .request("orchestrator", "restart", json!([service_id, 0]))
        .await
        .expect_err("a stale-generation write must fail");
    // `check_generation`'s own `Ordering::Less` message (ADR-0021 §4) --
    // asserting on it, rather than on any non-empty error (B5, Slice A5b
    // review), proves this failed on the generation gate specifically and
    // not on a connection timeout, a parse failure, or the unrelated
    // "tcp service cannot be restarted" refusal the same call would hit
    // next if the generation gate did not fire first.
    let err = err.to_string();
    assert!(err.contains("never self-increment"), "{err}");
    let _ = second_writer.shutdown().await;

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

#[tokio::test]
async fn a_supervisor_deploys_a_bound_app_using_masters_it_minted() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_DEPLOYS_A_BOUND_APP).await;

    let manifest = bound_app_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-bound-inst").await;

    let res = supervisor_node
        .substrate_client
        .request("supervisor", "submit", submission("a5b-bound-inst", plan_json, inventory_json, 0))
        .await
        .expect("submit of a bound app failed -- regresses if custody is removed from A5b");
    let masters = res.result.get("masters").and_then(|m| m.as_array()).expect("masters array");
    assert_eq!(masters.len(), 2, "one master per member, minted by the supervisor itself");

    let status = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["a5b-bound-inst"]))
        .await
        .expect("status failed");
    let services =
        status.result.get("services").and_then(|s| s.as_array()).expect("services array");
    assert_eq!(services.len(), 2);
    // Neither service declares a health check, so both report "unknown"
    // (a `tcp` service's only other signal) rather than a fault -- what
    // matters here is that the deploy succeeded and both members are
    // visible, not that a liveness check nothing declared somehow passed.
    for svc in services {
        assert_eq!(svc.get("signal").and_then(|v| v.as_str()), Some("unknown"), "{svc:?}");
    }

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

#[tokio::test]
async fn adopt_reads_the_held_generation_from_the_managed_node_and_claims_the_next() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_ADOPT_CLAIMS_THE_NEXT).await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-adopt-inst").await;
    supervisor_node
        .substrate_client
        .request("supervisor", "submit", submission("a5b-adopt-inst", plan_json, inventory_json, 0))
        .await
        .expect("submit failed");

    // No prior adopt: the managed node holds no generation for this
    // instance's un-adopted (generation-0) deploy, so the first adopt
    // claims generation 1.
    let first = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a5b-adopt-inst"]))
        .await
        .expect("first adopt failed");
    assert_eq!(first.result.as_u64(), Some(1));

    // A second adopt (the same supervisor, simulating a rebuilt one) reads
    // the now-held generation 1 back and claims 2.
    let second = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a5b-adopt-inst"]))
        .await
        .expect("second adopt failed");
    assert_eq!(second.result.as_u64(), Some(2));

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

#[tokio::test]
async fn a_pushed_binding_reaches_a_dependent_the_supervisor_deployed() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_PUSHED_BINDING).await;

    let manifest = bound_app_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-binding-inst").await;
    supervisor_node
        .substrate_client
        .request(
            "supervisor",
            "submit",
            submission("a5b-binding-inst", plan_json, inventory_json, 0),
        )
        .await
        .expect("submit failed");

    let status = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["a5b-binding-inst"]))
        .await
        .expect("status failed");
    let services =
        status.result.get("services").and_then(|s| s.as_array()).expect("services array");
    let frontend_id = services
        .iter()
        .find(|s| {
            s.get("logical_ref").and_then(|v| v.as_str()).map(|r| r.ends_with("/frontend"))
                == Some(true)
        })
        .and_then(|s| s.get("service_id"))
        .and_then(|v| v.as_str())
        .expect("frontend not in status")
        .to_string();

    // `emit_bindings: true` on the supervisor's apply path (§12) means the
    // frontend's binding to backend was populated at deploy time --
    // readable back from the managed node's own service-status, without
    // needing a second `write-bindings` push.
    let managed_status = managed_node
        .substrate_client
        .status(vec![frontend_id])
        .await
        .expect("managed node status failed");
    let frontend_status = &managed_status.services[0];
    assert!(
        frontend_status.binding_epochs.iter().any(|(name, _)| name == "backend"),
        "{:?}",
        frontend_status.binding_epochs
    );

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

/// Matrix row 9 (§13 test 9,
/// `a_supervisor_that_reads_a_higher_generation_marks_the_instance_
/// superseded_and_alerts`) -- planned as a unit test but, like test 7's own
/// disclosed substitution, undeliverable as one: there is no injectable
/// trait over the management verbs, so "a substrate reports a higher
/// generation than this supervisor holds" can only be produced by a real
/// second write against a real substrate. Absent entirely from A5b as
/// shipped (B4, Slice A5b review) -- this is the missing live proof.
///
/// The managed node's own owner, who already holds `substrate/admin` there,
/// claims a higher generation directly against the managed substrate's
/// `orchestrator` interface -- standing in for a second supervisor's
/// `adopt`, the same way `a_second_supervisor_that_has_not_adopted_loses_
/// every_write` stands in for one with a stale-generation `submit`.
#[tokio::test]
async fn a_supervisor_that_reads_a_higher_generation_marks_the_instance_superseded_and_alerts() {
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (supervisor_node, managed_node, inventory_json) =
        boot_pair(&supervisor_owner, &managed_owner, PORTS_SUPERSEDED).await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-superseded-inst").await;
    supervisor_node
        .substrate_client
        .request(
            "supervisor",
            "submit",
            submission("a5b-superseded-inst", plan_json, inventory_json, 0),
        )
        .await
        .expect("submit failed");
    let adopted = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a5b-superseded-inst"]))
        .await
        .expect("adopt failed");
    assert_eq!(adopted.result.as_u64(), Some(1));

    // A second writer claims generation 2 directly against the managed
    // substrate. The first supervisor's own store still holds generation 1.
    let mut second_writer = SyneroymClient::new_with_identity(
        managed_node.did().to_string(),
        managed_node.registry_url.clone(),
        Identity::from_bytes(&managed_owner.to_bytes()),
    );
    second_writer
        .wait_for_ready(Duration::from_secs(30))
        .await
        .expect("second writer could not reach the managed node");
    second_writer
        .request("orchestrator", "claim-app-instance", json!(["a5b-superseded-inst", 2]))
        .await
        .expect("second writer's claim failed");
    let _ = second_writer.shutdown().await;

    // The first supervisor's next status sweep must read the substrate's
    // now-higher generation, report itself superseded, and raise the alert
    // -- not bump its own stamp to match (ADR-0021 §4).
    let status = supervisor_node
        .substrate_client
        .request("supervisor", "status", json!(["a5b-superseded-inst"]))
        .await
        .expect("status failed");
    assert_eq!(status.result.get("state").and_then(|v| v.as_str()), Some("Superseded"));
    assert_eq!(status.result.get("generation").and_then(serde_json::Value::as_u64), Some(1));

    let alerts = supervisor_node
        .substrate_client
        .request("supervisor", "alerts", json!(["a5b-superseded-inst", false]))
        .await
        .expect("alerts failed");
    let alerts = alerts.result.as_array().expect("alerts array");
    assert!(
        alerts
            .iter()
            .any(|a| a.get("kind").and_then(|v| v.as_str()) == Some("SUPERVISOR_SUPERSEDED")),
        "{alerts:?}"
    );

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

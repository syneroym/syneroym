#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The milestone's own reference scenario (M05A Slice A5e, D-A5e-15), end to
//! end over two genuinely independent `syneroym-substrate` instances:
//! `frontend` (a WASM `proxy-test` component) on `managed-a`, depending on
//! `backend` (a WASM `greeter` component) on `managed-b`, both managed by one
//! resident supervisor. Steps 1-6 in one test, per the plan -- the claim
//! under test is the *sequence*, not any one outcome in isolation (several
//! of which already have their own unit or narrower e2e coverage):
//!
//! 1. Deploy, with the binding pushed at deploy time (no second write).
//! 2. A resolved dependency call actually reaches `backend` -- the first
//!    full-substrate exercise of `syneroym:proxy/proxy::call`'s
//!    `dependency(...)` target in this tree (`binding_push_e2e.rs`'s own doc
//!    comment notes no wasm fixture existed yet when it was written).
//! 3. `managed-b` goes down; the supervisor reports `SubstrateUnreachable`, not
//!    some other fault kind.
//! 4. `managed-b` returns under the same identity; `backend` is recognized as
//!    the same member (its master DID is unchanged) with no push -- for a WASM
//!    component this is `AppSandboxEngine::init`'s own boot-time warm-up loop
//!    re-populating its component cache from the persisted artifact, not a
//!    redeploy or a supervisor-triggered restart.
//! 5. `backend` scales to two members. `frontend`'s binding advances and it
//!    resolves across both members afterward -- proven here by making repeated
//!    dependency calls and asserting both members actually answer (via
//!    `greeter`'s own `component_id`, echoed through the pre-existing
//!    `syneroym:host/context::get-test-context` capability -- the one thing
//!    that differs between two otherwise byte-identical `replicas` members).
//!    **What this step does not prove** (M05A A5e review, second round):
//!    `frontend`'s stable `service_id` and its advanced written epoch are both
//!    consistent with a full reinstall as much as with a push -- neither is a
//!    WASM instantiation counter, and this harness has no live component to
//!    sample one from. The push-not-reinstall claim itself is proven where it
//!    can actually be told apart: at the unit level, against a fake substrate
//!    that records which calls it saw (`syneroym-app-supervisor`'s
//!    `a_diff_whose_only_change_is_resolved_dependencies_pushes_instead_of_redeploying`
//!    and its `_config_still_takes_the_redeploy_path` counterpart).
//! 6. A stale-epoch binding write is rejected without regressing the held
//!    mapping, and an identical re-send at the current epoch is a no-op.
//!    `binding_push_e2e.rs` already proves the first two operations in
//!    isolation at the SDK level; this step adds the `NoOp` leg neither of its
//!    two tests covers, self-contained rather than reverse-engineered from the
//!    supervisor's own just-pushed content (whose exact member order is an
//!    implementation detail this test should not depend on).
//!
//! `Node` is copied from `supervisor_loop_e2e.rs` (itself from
//! `supervisor_interface_e2e.rs`, itself from `multi_substrate_placement_
//! e2e.rs`), with the same "boots from a caller-supplied directory" addition
//! so `managed-b` can be torn down and rebooted under its own identity for
//! steps 3/4.
//!
//! Steps 2 and 5's dependency calls need `proxy-test` and `greeter`'s wasm
//! artifacts built (`cargo build --target wasm32-wasip2 --release` in each
//! `test-components/` dir, or `cargo component build`) -- `greeter`'s is
//! built automatically by `crates/substrate/build.rs` in a non-release
//! profile, but `proxy-test` is not (no `build.rs` covers it, and no `mise`
//! task builds every fixture), matching every other test in this tree that
//! depends on it. This test skips itself, rather than failing, when either
//! is missing.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use rustls::crypto::ring;
use semver::Version;
use serde_json::{Map, Value, json};
use syneroym_app_orchestration::{
    AlertKind, LocalFilesystemCatalog, compile,
    models::{
        AppBlueprintId, AppInstanceId, InterfaceName, LogicalServiceName, PlacementSelector,
        ServiceConfig, ServiceSpec, ServiceType, SubstrateAlias, SynAppManifest,
    },
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_core::{
    config::{
        ClientGatewayRole, CoordinatorIrohConfig, CoordinatorRole, IrohParentConfig, LogTarget,
        ServiceRegistryRole, SubstrateConfig, SupervisorRole,
    },
    test_constants,
};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{Ability, Capability, CapabilityToken, ResourceUri};
use syneroym_sdk::{
    BindingWrite, BindingWriteOutcome, DependencyBinding, SyneroymClient, TopologyMode,
};
use syneroym_substrate::identity;
use tokio::{
    sync::{mpsc, mpsc::Sender},
    task::JoinHandle,
    time,
};

#[path = "common/retry.rs"]
mod retry;

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

// `supervisor_loop_e2e.rs` already claims 12_600-12_602 (its `managed_b`
// block) -- the plan's own port note ("continue at 12_600") predates that
// file. Continuing the sequence from 12_700 instead, one hundred-port block
// past the highest one actually claimed in this directory.
const PORTS: PortBlock = PortBlock {
    supervisor_iroh: 12_700,
    supervisor_registry: 12_701,
    supervisor_gateway: 12_702,
    managed_a_iroh: 12_800,
    managed_a_registry: 12_801,
    managed_a_gateway: 12_802,
    managed_b_iroh: 12_900,
    managed_b_registry: 12_901,
    managed_b_gateway: 12_902,
};

const MANAGED_A_ALIAS: &str = "managed-a";
const MANAGED_B_ALIAS: &str = "managed-b";
const INSTANCE_ID: &str = "a5e-ref-scenario-inst";

/// A full, independently-identified `syneroym-substrate` instance, booted
/// from a caller-owned directory so the same managed substrate identity can
/// be torn down and rebooted later in the same test (steps 3/4).
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
        poll_interval_secs: 3,
        db_name: "supervisor.db".to_string(),
        max_restart_attempts: 3,
        restart_backoff_secs: 30,
        alert_topic: "supervisor/alerts".to_string(),
        master_backup_dir: "master-backups".to_string(),
        ..SupervisorRole::default()
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

/// `frontend` (a `proxy-test` WASM component, depends on `backend`) on
/// `managed-a`; `backend` (a `greeter` WASM component) on `managed-b`, scaled
/// to `backend_replicas` members.
fn reference_scenario_manifest(backend_replicas: u32) -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Wasm,
                source: test_constants::greeter_wasm_path().to_string_lossy().to_string(),
                hash: None,
                interfaces: vec![InterfaceName::new(test_constants::GREETER_INTERFACE_NAME)],
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
            replicas: backend_replicas,
            sharding_strategy: None,
            schedule: None,
        },
    );
    services.insert(
        LogicalServiceName::new("frontend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Wasm,
                source: test_constants::proxy_test_wasm_path().to_string_lossy().to_string(),
                hash: None,
                interfaces: vec![InterfaceName::new(test_constants::PROXY_TEST_DRIVER_INTERFACE)],
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
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_A_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:a5e-reference-scenario"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

async fn compiled_plan_json(backend_replicas: u32) -> String {
    let manifest = reference_scenario_manifest(backend_replicas);
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    let compiled = compile(AppInstanceId::new(INSTANCE_ID), &manifest, &catalog).await.unwrap();
    compiled.plans.last().unwrap().to_json().unwrap()
}

fn submission(plan_json: String, inventory_json: String, generation: u64) -> Value {
    json!([{
        "app_instance_id": INSTANCE_ID,
        "plan_json": plan_json,
        "inventory_json": inventory_json,
        "generation": generation,
    }])
}

fn service_of<'a>(services: &'a [Value], suffix: &str) -> Option<&'a Value> {
    services.iter().find(|s| {
        s.get("logical_ref").and_then(|v| v.as_str()).map(|r| r.ends_with(suffix)) == Some(true)
    })
}

fn str_field<'a>(v: &'a Value, field: &str) -> Option<&'a str> {
    v.get(field).and_then(|f| f.as_str())
}

async fn supervisor_status(supervisor_node: &Node) -> Value {
    supervisor_node
        .substrate_client
        .request("supervisor", "status", json!([INSTANCE_ID]))
        .await
        .expect("status failed")
        .result
}

async fn active_alert_kinds(supervisor_node: &Node, substrate_did: &str) -> BTreeSet<String> {
    let alerts = supervisor_node
        .substrate_client
        .request("supervisor", "alerts", json!([INSTANCE_ID, false]))
        .await
        .expect("alerts failed")
        .result;
    alerts
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| str_field(a, "substrate_did") == Some(substrate_did))
        .filter_map(|a| str_field(a, "kind").map(str::to_string))
        .collect()
}

/// `frontend`'s own written epoch for its `backend` dependency, off the
/// supervisor's `status` -- the fact that advances the instant a push lands,
/// distinct from `binding-epochs`' own read lag (D-A5e-9).
fn frontend_written_epoch(status: &Value) -> Option<u64> {
    status.get("bindings").and_then(Value::as_array).into_iter().flatten().find_map(|b| {
        (str_field(b, "dependent_logical_ref")?.ends_with("/frontend#0")
            && str_field(b, "dependency_name")? == "backend")
            .then(|| b.get("written_epoch").and_then(Value::as_u64))
            .flatten()
    })
}

/// Calls `frontend`'s `call-peer` with a `dependency("backend")` target and
/// returns `greeter`'s reply string -- the one real proof this scenario's
/// binding actually resolves, not just that its epoch advanced.
async fn call_backend_through_frontend(frontend_client: &SyneroymClient) -> String {
    let res = time::timeout(
        Duration::from_secs(30),
        frontend_client.request(
            test_constants::PROXY_TEST_DRIVER_INTERFACE,
            "call-peer",
            json!({
                "service": "backend",
                "interface": test_constants::GREETER_INTERFACE_NAME,
                "method": "greet",
                "params": "[\"World\"]",
                "target-kind": "dependency",
            }),
        ),
    )
    .await
    .expect("call-peer timed out")
    .expect("call-peer failed");
    res.result.as_str().expect("call-peer result must be a string").to_string()
}

/// `greeter`'s reply carries `"Component: <component_id>"` (via
/// `syneroym:host/context::get-test-context`) -- the one thing that differs
/// between two otherwise byte-identical `replicas` members.
fn component_id_from_reply(reply: &str) -> String {
    reply
        .split("Component: ")
        .nth(1)
        .and_then(|rest| rest.split(" | ").next())
        .unwrap_or(reply)
        .trim()
        .to_string()
}

#[tokio::test]
async fn the_reference_scenario_runs_end_to_end_over_two_substrates() {
    if !test_constants::greeter_wasm_path().exists()
        || !test_constants::proxy_test_wasm_path().exists()
    {
        eprintln!(
            "skipping: greeter/proxy-test wasm artifacts not built (cargo build --target \
             wasm32-wasip2 --release in test-components/greeter and test-components/proxy-test)"
        );
        return;
    }
    let _ = ring::default_provider().install_default();

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    let supervisor_dir = tempfile::tempdir().expect("failed to create temp dir");
    let mut supervisor_node = Node::boot(
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
    let mut managed_a = Node::boot(
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

    // Owned directly (not a `Node` field) so it survives `managed_b`'s own
    // teardown in step 3 -- the whole point is to reboot the *same*
    // identity, not a fresh one.
    let managed_b_dir = tempfile::tempdir().expect("failed to create temp dir");
    let mut managed_b = Node::boot(
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
                api_url: Some(managed_b.registry_url.clone()),
                ucan: Some(grant_b),
            },
        ),
    ]))
    .unwrap();

    // ---- Step 1: deploy + push (the initial deploy emits bindings at
    // deploy time, D-A5e-2's per-member epoch). ----
    let plan_json = compiled_plan_json(1).await;
    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `Node::boot`, then sat idle through both
    // managed nodes' full boots that followed -- long enough under CI's
    // scheduling pressure for the peer to abandon that idle path ("no
    // viable network path exists: last path abandoned by peer"; same root
    // cause fixed throughout this crate's e2e tests, e.g.
    // `binding_push_e2e.rs`). Recover by explicit shutdown→reconnect
    // before one retry, not just retrying the same request on the same
    // dead connection.
    let submit_params = submission(plan_json, inventory_json.clone(), 0);
    let submit_res = crate::call_with_reconnect!(
        supervisor_node.substrate_client,
        "supervisor",
        "submit",
        submit_params
    );
    let masters =
        submit_res.result.get("masters").and_then(Value::as_array).expect("masters array");
    assert_eq!(masters.len(), 2, "one master per member: frontend#0, backend#0: {masters:?}");
    let backend_member0_did = masters
        .iter()
        .find(|m| str_field(m, "service_name") == Some("backend"))
        .and_then(|m| str_field(m, "master_did"))
        .expect("backend master in submit result")
        .to_string();

    let status = supervisor_status(&supervisor_node).await;
    let services = status.get("services").and_then(Value::as_array).cloned().unwrap_or_default();
    assert_eq!(services.len(), 2, "{services:?}");
    let frontend_service_id =
        str_field(service_of(&services, "/frontend#0").expect("frontend in status"), "service_id")
            .expect("frontend service_id")
            .to_string();
    assert_eq!(
        str_field(service_of(&services, "/backend#0").expect("backend#0 in status"), "service_id"),
        Some(backend_member0_did.as_str()),
        "backend#0's deployed service_id must be the master submit minted for it"
    );

    let epoch_after_deploy = frontend_written_epoch(&status)
        .expect("frontend must carry a written epoch for its backend dependency after deploy");

    // ---- Step 2: a resolved dependency call actually reaches backend. ----
    let mut frontend_client =
        SyneroymClient::new(frontend_service_id.clone(), shared_registry.clone());
    frontend_client.connect().await.expect("failed to connect to frontend");

    let reply = call_backend_through_frontend(&frontend_client).await;
    assert!(reply.contains("Hello, World!"), "{reply:?}");
    let member0_component_id = component_id_from_reply(&reply);
    assert_eq!(
        member0_component_id, backend_member0_did,
        "the resolved call must reach backend#0's own deployed component: {reply:?}"
    );

    // ---- Step 3: managed-b goes down; the supervisor must report
    // SubstrateUnreachable, not some other fault kind. ----
    managed_b.teardown().await;

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let kinds = active_alert_kinds(&supervisor_node, &managed_b_did).await;
        if kinds.contains(&AlertKind::SubstrateUnreachable.to_string()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "SubstrateUnreachable was never raised for managed-b: {kinds:?}"
        );
        time::sleep(Duration::from_millis(500)).await;
    }

    // ---- Step 4: managed-b returns under the same identity; backend is the
    // same member (unchanged master DID) with no push. For a WASM component
    // this is the substrate's own boot-time warm-up loop re-populating its
    // component cache from the persisted artifact -- nothing here redeploys
    // or explicitly restarts it. ----
    managed_b = Node::boot(
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
    assert_eq!(managed_b.did(), managed_b_did, "the rebooted node must keep its identity");

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let kinds = active_alert_kinds(&supervisor_node, &managed_b_did).await;
        let status = supervisor_status(&supervisor_node).await;
        let state = str_field(&status, "state").unwrap_or("");
        if !kinds.contains(&AlertKind::SubstrateUnreachable.to_string()) && state == "Active" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "managed-b's return was never recognized (state={state}, alerts={kinds:?})"
        );
        time::sleep(Duration::from_millis(500)).await;
    }

    let status = supervisor_status(&supervisor_node).await;
    let services = status.get("services").and_then(Value::as_array).cloned().unwrap_or_default();
    assert_eq!(
        str_field(service_of(&services, "/backend#0").expect("backend#0 in status"), "service_id"),
        Some(backend_member0_did.as_str()),
        "backend#0's master DID must be unchanged after the substrate's own reboot"
    );
    assert_eq!(
        frontend_written_epoch(&status),
        Some(epoch_after_deploy),
        "a substrate reboot with no membership change must not push a new binding"
    );

    // ---- Step 5: backend scales to two members and frontend's binding
    // converges, then (R5) actually resolves across both members. Whether
    // frontend got there via a push or a full reinstall is not
    // distinguishable from these RPC-surface assertions alone (M05A A5e
    // review, second round) -- see the module doc's step 5 note; that
    // claim is proven separately, at the unit level. ----
    let scaled_plan_json = compiled_plan_json(2).await;
    let scale_res = supervisor_node
        .substrate_client
        .request("supervisor", "submit", submission(scaled_plan_json, inventory_json, 0))
        .await
        .expect("scale-out submit failed");
    let scale_masters =
        scale_res.result.get("masters").and_then(Value::as_array).expect("masters array");
    let backend_member1_did = scale_masters
        .iter()
        .find(|m| {
            str_field(m, "service_name") == Some("backend")
                && m.get("member_index").and_then(Value::as_u64) == Some(1)
        })
        .and_then(|m| str_field(m, "master_did"))
        .expect("backend#1 master in scale-out submit result")
        .to_string();
    assert_ne!(backend_member1_did, backend_member0_did);

    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let status = supervisor_status(&supervisor_node).await;
        let services =
            status.get("services").and_then(Value::as_array).cloned().unwrap_or_default();
        let backend1_landed = service_of(&services, "/backend#1")
            .and_then(|s| str_field(s, "service_id"))
            .is_some_and(|id| id == backend_member1_did);
        let pushed = frontend_written_epoch(&status).is_some_and(|e| e > epoch_after_deploy);
        // frontend's own master DID is stable across the whole scenario
        // (`--mint-masters` custody means it is minted once, at step 1,
        // and never again) -- true whether it was pushed or reinstalled,
        // since a redeploy resolves the same member master by the same
        // computable name. Not proof of "not reinstalled" on its own; see
        // the module doc's step 5 note.
        let frontend_unchanged = service_of(&services, "/frontend#0")
            .and_then(|s| str_field(s, "service_id"))
            .is_some_and(|id| id == frontend_service_id);
        assert!(
            frontend_unchanged,
            "frontend's master DID must stay stable across the scale-out: {services:?}"
        );
        if backend1_landed && pushed {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "backend#1 never landed and/or frontend's binding was never pushed \
             (backend1_landed={backend1_landed}, pushed={pushed})"
        );
        time::sleep(Duration::from_millis(500)).await;
    }

    // R5's own fix: assert actual cross-member resolution, not merely that a
    // push happened. `Redundant` round-robins, so a handful of calls must
    // see both members' own `component_id`.
    let mut seen_components: BTreeSet<String> = BTreeSet::new();
    for _ in 0..8 {
        let reply = call_backend_through_frontend(&frontend_client).await;
        assert!(reply.contains("Hello, World!"), "{reply:?}");
        seen_components.insert(component_id_from_reply(&reply));
        if seen_components.len() == 2 {
            break;
        }
    }
    assert_eq!(
        seen_components,
        BTreeSet::from([backend_member0_did.clone(), backend_member1_did.clone()]),
        "frontend must resolve across both backend members, not just member 0 under a topology \
         mode the push forgot to flip to Redundant"
    );

    // ---- Step 6: a stale-epoch write is rejected without regressing the
    // held mapping, and an identical re-send at the current epoch is a
    // no-op. Self-contained (this test's own write, not a guess at the
    // supervisor's just-pushed content) -- `binding_push_e2e.rs` already
    // proves the first two legs in isolation; this adds the third. ----
    let status = supervisor_status(&supervisor_node).await;
    let current_epoch =
        frontend_written_epoch(&status).expect("frontend must still carry a written epoch");

    let test_epoch = current_epoch + 1;
    let test_members = vec![backend_member0_did.clone(), backend_member1_did.clone()];
    let first_write = BindingWrite {
        service_id: frontend_service_id.clone(),
        app_instance_id: INSTANCE_ID.to_string(),
        bindings: vec![DependencyBinding {
            dependency_name: "backend".to_string(),
            app_instance_id: INSTANCE_ID.to_string(),
            mode: TopologyMode::Redundant,
            members: test_members.clone(),
            epoch: test_epoch,
            cache_ttl_ms: 60_000,
        }],
        generation: 0,
    };
    // This is the first call this test makes directly on `managed_a`'s own
    // client -- everything up to here went through `supervisor_node` or
    // `frontend_client`. That connection was dialed and proven live by its
    // own `wait_for_ready` during `Node::boot`, then sat idle through this
    // entire test's deploy, scale-out, and cross-member-resolution steps --
    // long enough under CI's scheduling pressure for the peer to abandon
    // that idle path ("no viable network path exists: last path abandoned
    // by peer"; same root cause fixed throughout this crate's e2e tests).
    // Recover by explicit shutdown→reconnect before one retry.
    let applied = crate::call_with_reconnect!(
        managed_a.substrate_client,
        managed_a.substrate_client.write_bindings(first_write.clone()).await
    );
    assert!(matches!(applied[0], BindingWriteOutcome::Applied), "{applied:?}");

    let stale_members = vec![backend_member0_did.clone()];
    let stale_outcome = managed_a
        .substrate_client
        .write_bindings(BindingWrite {
            service_id: frontend_service_id.clone(),
            app_instance_id: INSTANCE_ID.to_string(),
            bindings: vec![DependencyBinding {
                dependency_name: "backend".to_string(),
                app_instance_id: INSTANCE_ID.to_string(),
                mode: TopologyMode::Singleton,
                members: stale_members,
                epoch: test_epoch - 1,
                cache_ttl_ms: 60_000,
            }],
            generation: 0,
        })
        .await
        .expect("stale write_bindings call failed");
    assert!(
        matches!(stale_outcome[0], BindingWriteOutcome::Stale(e) if e == test_epoch),
        "{stale_outcome:?}"
    );

    let noop_outcome = managed_a
        .substrate_client
        .write_bindings(BindingWrite {
            service_id: frontend_service_id.clone(),
            app_instance_id: INSTANCE_ID.to_string(),
            bindings: vec![DependencyBinding {
                dependency_name: "backend".to_string(),
                app_instance_id: INSTANCE_ID.to_string(),
                mode: TopologyMode::Redundant,
                members: test_members,
                epoch: test_epoch,
                cache_ttl_ms: 60_000,
            }],
            generation: 0,
        })
        .await
        .expect("identical write_bindings call failed");
    assert!(matches!(noop_outcome[0], BindingWriteOutcome::NoOp), "{noop_outcome:?}");

    let final_status =
        managed_a.substrate_client.status(vec![frontend_service_id.clone()]).await.unwrap();
    assert_eq!(
        final_status.services[0].binding_epochs,
        vec![("backend".to_string(), test_epoch)],
        "neither the stale retry nor the no-op re-send may have moved the held epoch"
    );

    let _ = frontend_client.shutdown().await;
    supervisor_node.teardown().await;
    managed_a.teardown().await;
    managed_b.teardown().await;
}

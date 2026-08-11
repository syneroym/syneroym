#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The reference scenario for scheduled tasks (ADR-0023 §6): one real
//! substrate hosting a scheduled WASM service, one real supervisor evaluating
//! and firing its schedule on its own reconcile pass. `Node` is copied from
//! `supervisor_loop_e2e.rs`'s (itself from `supervisor_interface_e2e.rs`),
//! with the same "boots from a caller-supplied directory" addition -- this
//! file needs it for the **supervisor** node, not a managed one: test 2's
//! restart must come back under the same identity and the same
//! `supervisor.db`, not a fresh one.
//!
//! Needs `test-components/scheduled-test`'s wasm artifact built by hand
//! first (`cargo component build --release --target wasm32-wasip2`, from
//! that directory -- there is no `mise` task that builds test components,
//! and no `build.rs` covers this one). Unlike the tree's other
//! component-backed tests, both tests here **fail, not skip**, when the
//! artifact is missing: an e2e that skips silently on a missing
//! fixture could not have caught the exact regression it exists to catch.
//!
//! Bounded by real cron-minute boundaries, not by anything this test
//! controls -- each test waits for at least one live "* * * * *" occurrence
//! (up to ~100s, which includes the resident loop's own real per-pass
//! connect latency, not just the minute boundary). Test 2's downtime is
//! anchored to actual minute boundaries (`next_minute_boundary`,
//! `sleep_until_unix_secs`) rather than accumulated relative sleeps -- a
//! fixed-duration version of it is exactly what let a *second*, legitimate
//! tick masquerade as evidence the first, missed one ran late. Run this
//! file at least three times before trusting a green result: a wall-clock
//! race is not distinguished from a fix by one pass.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use rustls::crypto::ring;
use semver::Version;
use serde_json::{Map, Value, json};
use syneroym_app_orchestration::{
    LocalFilesystemCatalog, compile,
    models::{
        AppBlueprintId, AppInstanceId, InterfaceName, LogicalServiceName, PlacementSelector,
        ScheduleSpec, ServiceConfig, ServiceSpec, ServiceType, SubstrateAlias, SynAppManifest,
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
use syneroym_sdk::SyneroymClient;
use syneroym_substrate::identity;
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

// Continues the running e2e port sequence from 13_900 --
// `proxy_outbox_e2e.rs` (13_800-13_902) is the last block before this one.
// Test 2 offsets every port by +500, the same convention
// `durable_outbox_e2e.rs`'s own two tests already use to share one block.
const PORTS: PortBlock = PortBlock {
    supervisor_iroh: 14_000,
    supervisor_registry: 14_001,
    supervisor_gateway: 14_002,
    managed_iroh: 14_100,
    managed_registry: 14_101,
    managed_gateway: 14_102,
};

const MANAGED_ALIAS: &str = "managed";
const INSTANCE_ID: &str = "b3-scheduled-inst";

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

/// `poll_interval_secs` of 10 sets the *floor* of the watermark's grace
/// window (`2 * poll_interval_secs`) at 20s; a sweep that runs longer than
/// that widens the window to the real gap on its own, so a slow pass can no
/// longer drop a tick. 10s is still chosen deliberately, for the case the
/// floor is all there is: the first pass after a restart has no previous
/// sweep to measure, and test 2 depends on that pass *not* running the tick
/// it missed. Measured against this tree's own resident loop, where a pass
/// reconnects a fresh iroh endpoint to the managed substrate every time
/// (`connect_best_effort` builds and tears down its clients per pass) and
/// costs several seconds even locally: 20s covers that comfortably while
/// staying well under the whole-occurrence gap test 2's downtime creates.
fn supervisor_role(poll_interval_secs: u64) -> SupervisorRole {
    SupervisorRole {
        poll_interval_secs,
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

/// One `scheduled-test` WASM service, ticking every minute.
fn scheduled_manifest() -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("worker"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Wasm,
                source: test_constants::scheduled_test_wasm_path().to_string_lossy().to_string(),
                hash: None,
                interfaces: vec![InterfaceName::new(
                    test_constants::SCHEDULED_TEST_DRIVER_INTERFACE,
                )],
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
            schedule: Some(ScheduleSpec {
                cron: "* * * * *".to_string(),
                interface: InterfaceName::new(test_constants::SCHEDULED_TEST_DRIVER_INTERFACE),
                method: "tick".to_string(),
                params: None,
                timeout_ms: 10_000,
            }),
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:b3-scheduled-test-app"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

async fn compiled_plan_json() -> String {
    let catalog = LocalFilesystemCatalog::new(PathBuf::from("."));
    let compiled =
        compile(AppInstanceId::new(INSTANCE_ID), &scheduled_manifest(), &catalog).await.unwrap();
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

fn str_field<'a>(v: &'a Value, field: &str) -> Option<&'a str> {
    v.get(field).and_then(Value::as_str)
}

fn u64_field(v: &Value, field: &str) -> Option<u64> {
    v.get(field).and_then(Value::as_u64)
}

async fn supervisor_status(supervisor_node: &Node) -> Value {
    supervisor_node
        .substrate_client
        .request("supervisor", "status", json!([INSTANCE_ID]))
        .await
        .expect("status failed")
        .result
}

async fn supervisor_schedules(supervisor_node: &Node) -> Vec<Value> {
    supervisor_node
        .substrate_client
        .request("supervisor", "schedules", json!([INSTANCE_ID]))
        .await
        .expect("schedules failed")
        .result
        .as_array()
        .cloned()
        .unwrap_or_default()
}

async fn deployed_service_id(supervisor_node: &Node) -> String {
    let status = supervisor_status(supervisor_node).await;
    let services = status.get("services").and_then(Value::as_array).cloned().unwrap_or_default();
    services
        .iter()
        .find(|s| str_field(s, "logical_ref").is_some_and(|r| r.ends_with("/worker#0")))
        .and_then(|s| str_field(s, "service_id"))
        .expect("worker#0 in status")
        .to_string()
}

/// Reads the deployed service's own persisted counter directly -- a client
/// connected straight to it, the same shape
/// `reference_scenario_e2e.rs::call_backend_through_frontend` uses.
async fn tick_count(worker_client: &SyneroymClient) -> u32 {
    let res = time::timeout(
        Duration::from_secs(15),
        worker_client.request(
            test_constants::SCHEDULED_TEST_DRIVER_INTERFACE,
            "tick-count",
            json!([]),
        ),
    )
    .await
    .expect("tick-count timed out")
    .expect("tick-count failed");
    res.result.as_u64().expect("tick-count result must be a number") as u32
}

async fn wait_for_tick_count(worker_client: &SyneroymClient, want: u32, deadline: Instant) {
    loop {
        let count = tick_count(worker_client).await;
        if count == want {
            return;
        }
        assert!(
            Instant::now() < deadline && count < want,
            "tick-count is {count}, expected to reach {want} (never exceed it): "
        );
        time::sleep(Duration::from_millis(500)).await;
    }
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is before the Unix epoch")
        .as_secs()
}

/// The next minute boundary (a multiple of 60 Unix seconds) strictly after
/// `after` -- the moment a `"* * * * *"` cron next fires, in UTC (Unix
/// seconds carry no time zone to begin with).
fn next_minute_boundary(after: u64) -> u64 {
    (after / 60 + 1) * 60
}

/// Sleeps until the given Unix-seconds wall-clock target, or returns
/// immediately if it has already passed.
async fn sleep_until_unix_secs(target: u64) {
    let now = now_unix_secs();
    if target > now {
        time::sleep(Duration::from_secs(target - now)).await;
    }
}

fn fail_if_fixture_missing() {
    // This e2e must fail, not skip, on a missing artifact -- a
    // passing run against a missing component is the exact failure the
    // operator surface exists to prevent.
    assert!(
        test_constants::scheduled_test_wasm_path().exists(),
        "test-components/scheduled-test's wasm artifact is not built. Run, from that directory: \
         `cargo component build --release --target wasm32-wasip2`"
    );
}

#[tokio::test]
async fn a_scheduled_task_runs_on_its_own_cadence_and_only_once_per_tick() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    fail_if_fixture_missing();
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
        Some(supervisor_role(10)),
    )
    .await;
    let shared_registry = supervisor_node.registry_url.clone();
    let shared_relay = format!("http://localhost:{}", PORTS.supervisor_iroh);

    let managed_dir = tempfile::tempdir().expect("failed to create temp dir");
    let managed = Node::boot(
        managed_dir.path().to_path_buf(),
        PORTS.managed_iroh,
        PORTS.managed_registry,
        PORTS.managed_gateway,
        Some(shared_registry.clone()),
        Some(shared_relay),
        &managed_owner,
        None,
    )
    .await;

    let grant = node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), managed.did());
    let inventory_json = serde_json::to_string(&BTreeMap::from([(
        MANAGED_ALIAS.to_string(),
        SupervisorInventoryEntry {
            did: managed.did().to_string(),
            api_url: Some(managed.registry_url.clone()),
            ucan: Some(grant),
        },
    )]))
    .unwrap();

    // ---- Step 1: deploy + adopt (a plain `submit` at generation 0 needs
    // no separate `adopt` -- see `reference_scenario_e2e.rs`). ----
    //
    // Submitted just after a minute boundary, so step 2's "nothing has run
    // yet" has a full cron minute to hold in. Without the anchor, a
    // boundary falling inside the convergence loop below (up to 30s, three
    // passes) fires a legitimate tick and the exact-zero assertion fails --
    // the same wall-clock assumption test 2's fixed sleep already had to
    // shed.
    let plan_json = compiled_plan_json().await;
    sleep_until_unix_secs(next_minute_boundary(now_unix_secs())).await;
    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during `Node::boot`, then sat idle through the
    // `managed` node's full boot plus the wait for the minute boundary
    // above -- long enough under CI's scheduling pressure for the peer to
    // abandon that idle path ("no viable network path exists: last path
    // abandoned by peer"; same root cause fixed in `binding_push_e2e.rs`).
    // `SyneroymClient::connect` no-ops on an already-`Some` connection, so
    // recovering means an explicit `shutdown`-then-`connect` (redial)
    // before one retry, not just retrying the same request on the same
    // dead connection.
    let submit_params = submission(plan_json, inventory_json, 0);
    crate::call_with_reconnect!(
        supervisor_node.substrate_client,
        "supervisor",
        "submit",
        submit_params
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = supervisor_status(&supervisor_node).await;
        if str_field(&status, "state") == Some("Active") {
            break;
        }
        assert!(Instant::now() < deadline, "the instance never converged: {status:?}");
        time::sleep(Duration::from_millis(500)).await;
    }

    let worker_service_id = deployed_service_id(&supervisor_node).await;
    let mut worker_client =
        SyneroymClient::new(worker_service_id, shared_registry.clone()).with_registry_dht(false);
    worker_client.connect().await.expect("failed to connect to the deployed worker");

    // ---- Step 2: nothing has run yet. ----
    assert_eq!(tick_count(&worker_client).await, 0);
    let schedules = supervisor_schedules(&supervisor_node).await;
    assert_eq!(schedules.len(), 1, "{schedules:?}");
    assert_eq!(
        str_field(&schedules[0], "logical_ref"),
        Some(format!("{INSTANCE_ID}/worker").as_str())
    );
    assert!(
        schedules[0].get("last_run_at").is_none_or(Value::is_null),
        "a schedule that has never run must report no last-run-at: {schedules:?}"
    );

    // ---- Step 3: wait past one cron minute boundary. ----
    let deadline = Instant::now() + Duration::from_secs(100);
    wait_for_tick_count(&worker_client, 1, deadline).await;
    let schedules = supervisor_schedules(&supervisor_node).await;
    assert_eq!(u64_field(&schedules[0], "last_member_index"), Some(0));
    assert!(
        schedules[0].get("last_run_at").is_some_and(|v| !v.is_null()),
        "a run must record its own last-run-at: {schedules:?}"
    );

    // ---- Step 4: several more passes inside the same cron minute must
    // not run it again -- the watermark, and failure-matrix row 10. ----
    let hold_until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < hold_until {
        assert_eq!(
            tick_count(&worker_client).await,
            1,
            "the watermark must prevent a second run inside the same cron minute"
        );
        time::sleep(Duration::from_millis(1_500)).await;
    }

    // ---- Observed, not just argued: a scheduled run never touches the
    // outbox or the DLQ. ----
    let outbox = supervisor_node
        .substrate_client
        .request("supervisor", "outbox", json!([INSTANCE_ID]))
        .await
        .expect("outbox failed")
        .result;
    assert_eq!(outbox.as_array().map(Vec::len), Some(0), "{outbox:?}");
    let dead_letters = supervisor_node
        .substrate_client
        .request("supervisor", "dead-letters", json!([INSTANCE_ID]))
        .await
        .expect("dead-letters failed")
        .result;
    assert_eq!(dead_letters.as_array().map(Vec::len), Some(0), "{dead_letters:?}");

    worker_client.shutdown().await.ok();
    supervisor_node.teardown().await;
    managed.teardown().await;
}

#[tokio::test]
async fn a_supervisor_restart_skips_the_ticks_it_missed() {
    let _serial_guard = SUBSTRATE_TEST_LOCK.lock().await;
    fail_if_fixture_missing();
    let _ = ring::default_provider().install_default();

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    // A caller-owned `TempDir`, kept alive across the supervisor's own
    // teardown/reboot below -- the whole point is to bring back the *same*
    // identity and the same `supervisor.db`, not a fresh one.
    let supervisor_dir = tempfile::tempdir().expect("failed to create temp dir");
    let mut supervisor_node = Node::boot(
        supervisor_dir.path().to_path_buf(),
        PORTS.supervisor_iroh + 500,
        PORTS.supervisor_registry + 500,
        PORTS.supervisor_gateway + 500,
        None,
        None,
        &supervisor_owner,
        Some(supervisor_role(10)),
    )
    .await;
    let supervisor_did = supervisor_node.did().to_string();
    let shared_registry = supervisor_node.registry_url.clone();
    let shared_relay = format!("http://localhost:{}", PORTS.supervisor_iroh + 500);

    let managed_dir = tempfile::tempdir().expect("failed to create temp dir");
    let managed = Node::boot(
        managed_dir.path().to_path_buf(),
        PORTS.managed_iroh + 500,
        PORTS.managed_registry + 500,
        PORTS.managed_gateway + 500,
        Some(shared_registry.clone()),
        Some(shared_relay.clone()),
        &managed_owner,
        None,
    )
    .await;

    let grant = node_wide_supervisor_grant(&managed_owner, &supervisor_did, managed.did());
    let inventory_json = serde_json::to_string(&BTreeMap::from([(
        MANAGED_ALIAS.to_string(),
        SupervisorInventoryEntry {
            did: managed.did().to_string(),
            api_url: Some(managed.registry_url.clone()),
            ucan: Some(grant),
        },
    )]))
    .unwrap();

    let plan_json = compiled_plan_json().await;
    // See the comment on the equivalent `submit` in
    // `a_scheduled_task_runs_on_its_own_cadence_and_only_once_per_tick`
    // above: `supervisor_node`'s connection sat idle through the `managed`
    // node's full boot, long enough under CI's scheduling pressure for the
    // peer to abandon that idle path.
    let submit_params = submission(plan_json, inventory_json, 0);
    crate::call_with_reconnect!(
        supervisor_node.substrate_client,
        "supervisor",
        "submit",
        submit_params
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = supervisor_status(&supervisor_node).await;
        if str_field(&status, "state") == Some("Active") {
            break;
        }
        assert!(Instant::now() < deadline, "the instance never converged: {status:?}");
        time::sleep(Duration::from_millis(500)).await;
    }

    let worker_service_id = deployed_service_id(&supervisor_node).await;
    let mut worker_client =
        SyneroymClient::new(worker_service_id, shared_registry.clone()).with_registry_dht(false);
    worker_client.connect().await.expect("failed to connect to the deployed worker");

    // ---- First tick: prove the schedule runs normally before the
    // restart, at the receiver. ----
    let deadline = Instant::now() + Duration::from_secs(100);
    wait_for_tick_count(&worker_client, 1, deadline).await;

    // The same run as the supervisor recorded it. This, not the worker's
    // counter, is what the post-restart assertion compares against -- see
    // the comment on that assertion for why the counter cannot be read
    // through the reboot.
    let before = supervisor_schedules(&supervisor_node).await;
    let run_before = u64_field(&before[0], "last_run_at")
        .expect("the pre-restart run must have recorded a last-run-at");

    // ---- The supervisor process goes down; the managed substrate (and
    // the worker's own persisted counter) does not. ----
    supervisor_node.teardown().await;

    // Anchored to real minute boundaries, not to accumulated relative
    // sleeps: an earlier draft slept a fixed 70s down + 30s settle, and on
    // an unlucky alignment that 100s window crossed *two* cron boundaries
    // -- the second one fired legitimately (this is not a bug; it is the
    // schedule doing exactly what it should), and the test misread that
    // second, real tick as evidence the first, missed one had run late.
    //
    // `next_boundary` is the occurrence that is about to become "missed":
    // the supervisor comes back only once the watermark's grace window
    // (20s, `2 * poll_interval_secs`) has unambiguously elapsed past it.
    // `following_boundary` is the *next* legitimate tick after that, which
    // this test must observe the counter still stopped short of.
    let now = now_unix_secs();
    let next_boundary = next_minute_boundary(now);
    let following_boundary = next_boundary + 60;

    // Comfortably past the grace window, so the missed occurrence is
    // unambiguously behind the watermark's clamp by the time any pass
    // looks at it.
    sleep_until_unix_secs(next_boundary + 22).await;

    let mut supervisor_node = Node::boot(
        supervisor_dir.path().to_path_buf(),
        PORTS.supervisor_iroh + 500,
        PORTS.supervisor_registry + 500,
        PORTS.supervisor_gateway + 500,
        Some(shared_registry),
        Some(shared_relay),
        &supervisor_owner,
        Some(supervisor_role(10)),
    )
    .await;
    assert_eq!(
        supervisor_node.did(),
        supervisor_did,
        "the rebooted supervisor must keep its identity"
    );
    // Settle, then check strictly before `following_boundary` -- the next
    // legitimate tick must not have had a chance to fire yet, so any
    // advance observed here can only be the missed one running late.
    let settle_until = (following_boundary.saturating_sub(8)).max(now_unix_secs() + 5);
    assert!(
        settle_until < following_boundary,
        "the resident loop's own pass latency leaves no safety margin before the next legitimate \
         tick; widen the grace window or the boundary spacing"
    );
    sleep_until_unix_secs(settle_until).await;

    // The witness is the supervisor's own durable record, read over the
    // client the reboot just created -- deliberately *not* the worker's
    // counter. `worker_client` reaches the worker through the registry and
    // the relay this test just took down and brought back, and a
    // connection opened before that does not reliably survive it: reading
    // the counter here failed with `tick-count failed: timed out` on one
    // run in three, reporting a dead transport as a scheduling result, and
    // re-dialling does not fit inside the window either (the managed
    // node's own endpoint takes its time re-attaching to the restarted
    // relay). The counter *is* still the witness for the pre-restart tick
    // above, and for "only once per tick" in the first test; what this
    // assertion needs is narrower and the supervisor records it exactly:
    // `last_run_at` unmoved says the missed occurrence did not run, and
    // `evaluated_at` moved past it says the rebooted supervisor really did
    // look at the schedule and choose to skip -- a stronger statement than
    // an unchanged counter, which an idle supervisor would also produce.
    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during the `Node::boot` reboot above, then sat idle
    // through the `sleep_until_unix_secs(settle_until)` wait -- long enough
    // under CI's scheduling pressure for the peer to abandon that idle
    // path ("no viable network path exists: last path abandoned by peer";
    // same root cause fixed throughout this crate's e2e tests). Recover by
    // explicit shutdown→reconnect before one retry, not just retrying the
    // same request on the same dead connection -- `supervisor_schedules`
    // itself takes `&Node` and cannot redial, so this call is inlined here.
    let schedules = crate::call_with_reconnect!(
        supervisor_node.substrate_client,
        "supervisor",
        "schedules",
        json!([INSTANCE_ID])
    )
    .result
    .as_array()
    .cloned()
    .unwrap_or_default();
    assert_eq!(schedules.len(), 1, "{schedules:?}");
    assert_eq!(
        u64_field(&schedules[0], "last_run_at"),
        Some(run_before),
        "a supervisor restart across a missed cron occurrence must skip it, not run it late: \
         {schedules:?}"
    );
    assert!(
        u64_field(&schedules[0], "evaluated_at").is_some_and(|e| e > run_before),
        "the rebooted supervisor must have evaluated the schedule and skipped it, not merely have \
         been idle: {schedules:?}"
    );
    assert_eq!(
        u64_field(&schedules[0], "last_member_index"),
        Some(0),
        "the last real run's own record must survive the restart: {schedules:?}"
    );

    let outbox = supervisor_node
        .substrate_client
        .request("supervisor", "outbox", json!([INSTANCE_ID]))
        .await
        .expect("outbox failed")
        .result;
    assert_eq!(outbox.as_array().map(Vec::len), Some(0), "{outbox:?}");

    worker_client.shutdown().await.ok();
    supervisor_node.teardown().await;
    managed.teardown().await;
}

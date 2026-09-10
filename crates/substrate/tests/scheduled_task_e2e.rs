#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The reference scenario for scheduled tasks (ADR-0023 §6): one real
//! substrate hosting a scheduled WASM service, one real supervisor evaluating
//! and firing its schedule on its own reconcile pass. The restart test's
//! supervisor node boots with `SubstrateNode::builder().base_path(..)` so its
//! restart comes back under the same identity and the same `supervisor.db`, not
//! a fresh one.
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
//! connect latency, not just the minute boundary). The restart test's
//! downtime is anchored to actual minute boundaries (`next_minute_boundary`,
//! `sleep_until_unix_secs`) rather than accumulated relative sleeps -- a
//! fixed-duration version of it is exactly what let a *second*, legitimate
//! tick masquerade as evidence the first, missed one ran late. Run this
//! file at least three times before trusting a green result: a wall-clock
//! race is not distinguished from a fix by one pass.

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use common::{SubstrateNode, node_wide_supervisor_grant, supervisor_role};
use rustls::crypto::ring;
use semver::Version;
use serde_json::{Value, json};
use syneroym_app_orchestration::{
    Visibility,
    models::{
        AppBlueprintId, InterfaceName, LogicalServiceName, PlacementSelector, ScheduleSpec,
        ServiceConfig, ServiceSpec, ServiceType, SubstrateAlias, SynAppManifest,
    },
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_core::test_constants;
use syneroym_identity::Identity;
use syneroym_sdk::SyneroymClient;
use tokio::time;

mod common;

#[path = "common/retry.rs"]
mod retry;

const MANAGED_ALIAS: &str = "managed";
const INSTANCE_ID: &str = "b3-scheduled-inst";

/// `poll_interval_secs` of 10 sets the *floor* of the watermark's grace
/// window (`2 * poll_interval_secs`) at 20s; a sweep that runs longer than
/// that widens the window to the real gap on its own, so a slow pass can no
/// longer drop a tick. 10s is still chosen deliberately, for the case the
/// floor is all there is: the first pass after a restart has no previous
/// sweep to measure, and the restart test depends on that pass *not*
/// running the tick it missed. Measured against this tree's own resident
/// loop, where a pass reconnects a fresh iroh endpoint to the managed
/// substrate every time and costs several seconds even locally: 20s covers
/// that comfortably while staying well under the whole-occurrence gap the
/// restart test's downtime creates.
const POLL_INTERVAL_SECS: u64 = 10;

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
                assets: None,
                // The supervisor's own scheduled tick dials this worker by
                // DID through the registry, since it is placed on a
                // different substrate (`MANAGED_ALIAS`) -- undeclared
                // (private) visibility means `certify_placed_members`
                // mints no record for it, and the tick has nothing to
                // resolve (ADR-0018 §4).
                visibility: Visibility::Internal,
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
            topology_visibility: Default::default(),
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
    common::compiled_plan_json(&scheduled_manifest(), INSTANCE_ID).await
}

fn submission(plan_json: String, inventory_json: String, generation: u64) -> Value {
    common::submission(INSTANCE_ID, plan_json, inventory_json, generation)
}

fn str_field<'a>(v: &'a Value, field: &str) -> Option<&'a str> {
    v.get(field).and_then(Value::as_str)
}

fn u64_field(v: &Value, field: &str) -> Option<u64> {
    v.get(field).and_then(Value::as_u64)
}

async fn supervisor_status(supervisor_node: &SubstrateNode) -> Value {
    supervisor_node
        .substrate_client
        .request("supervisor", "status", json!([INSTANCE_ID]))
        .await
        .expect("status failed")
        .result
}

async fn supervisor_schedules(supervisor_node: &SubstrateNode) -> Vec<Value> {
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

async fn deployed_service_id(supervisor_node: &SubstrateNode) -> String {
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
        Duration::from_secs(25),
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
    let _serial_guard = common::serial_guard().await;
    fail_if_fixture_missing();
    let _ = ring::default_provider().install_default();

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    let mut supervisor_node = SubstrateNode::builder()
        .owner(&supervisor_owner)
        .supervisor(supervisor_role(POLL_INTERVAL_SECS))
        .inject_kek()
        .boot()
        .await;
    let shared_registry = supervisor_node.registry_url().to_string();
    let shared_relay = supervisor_node.relay_url().to_string();

    let managed = SubstrateNode::builder()
        .owner(&managed_owner)
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek()
        .boot()
        .await;

    let grant = node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), managed.did());
    let inventory_json = serde_json::to_string(&BTreeMap::from([(
        MANAGED_ALIAS.to_string(),
        SupervisorInventoryEntry {
            did: managed.did().to_string(),
            api_url: Some(managed.registry_url().to_string()),
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
    // the same wall-clock assumption the restart test's fixed sleep
    // already had to shed.
    let plan_json = compiled_plan_json().await;
    sleep_until_unix_secs(next_minute_boundary(now_unix_secs())).await;
    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during boot, then sat idle through the
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
    // not run it again -- the watermark holds. ----
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
    let _serial_guard = common::serial_guard().await;
    fail_if_fixture_missing();
    let _ = ring::default_provider().install_default();

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    // A caller-owned `TempDir`, kept alive across the supervisor's own
    // teardown/reboot below -- the whole point is to bring back the *same*
    // identity and the same `supervisor.db`, not a fresh one. The builder is
    // reused for the reboot so the rebooted node keeps the same ports too,
    // which the managed node still resolves the registry through.
    let supervisor_dir = tempfile::tempdir().expect("failed to create temp dir");
    let supervisor_builder = SubstrateNode::builder()
        .owner(&supervisor_owner)
        .base_path(supervisor_dir.path())
        .supervisor(supervisor_role(POLL_INTERVAL_SECS))
        .inject_kek();
    let mut supervisor_node = supervisor_builder.clone().boot().await;
    let supervisor_did = supervisor_node.did().to_string();
    let shared_registry = supervisor_node.registry_url().to_string();
    let shared_relay = supervisor_node.relay_url().to_string();

    let managed = SubstrateNode::builder()
        .owner(&managed_owner)
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek()
        .boot()
        .await;

    let grant = node_wide_supervisor_grant(&managed_owner, &supervisor_did, managed.did());
    let inventory_json = serde_json::to_string(&BTreeMap::from([(
        MANAGED_ALIAS.to_string(),
        SupervisorInventoryEntry {
            did: managed.did().to_string(),
            api_url: Some(managed.registry_url().to_string()),
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

    let mut supervisor_node = supervisor_builder.boot().await;
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
    // `wait_for_ready` during the reboot above, then sat idle
    // through the `sleep_until_unix_secs(settle_until)` wait -- long enough
    // under CI's scheduling pressure for the peer to abandon that idle
    // path ("no viable network path exists: last path abandoned by peer";
    // same root cause fixed throughout this crate's e2e tests). Recover by
    // explicit shutdown→reconnect before one retry, not just retrying the
    // same request on the same dead connection -- `supervisor_schedules`
    // itself takes `&SubstrateNode` and cannot redial, so this call is inlined
    // here.
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

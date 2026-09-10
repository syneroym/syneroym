#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The durable-outbox reference scenario, end to end over two
//! real `syneroym-substrate` instances: `backend` (the dependency) on
//! `managed-a`, `frontend` (the dependent) on `managed-b`. Both plain TCP
//! services -- this scenario is about durable *delivery* of a binding
//! write, not about a live dependency call resolving through one, so it
//! needs no WASM fixture (unlike `reference_scenario_e2e.rs`).
//!
//! Steps:
//!
//! 1. Deploy, converged.
//! 2. Stop `managed-b` (`frontend`'s host).
//! 3. Scale `backend` on `managed-a` -- a membership change that makes a
//!    binding push to `frontend` (on the now-offline `managed-b`) due.
//! 4. The synchronous push fails: `Degraded`, unconverged, and `submit` itself
//!    already returned rather than retrying forever in-process.
//! 5. Restart the supervisor process. The queued item survives.
//! 6. Bring `managed-b` back, same identity.
//! 7. Within one worker tick, convergence resumes and the alert the failure
//!    raised clears.
//! 8. (separate test) Take `managed-b` down permanently and exhaust the attempt
//!    budget: the item lands in the DLQ, an alert is raised, `dead-letters`
//!    lists it, and `replay` re-queues it rather than executing it inline.
//!
//! All three nodes come from `common::SubstrateNode`; the supervisor and
//! `managed-b` boot from caller-owned directories so they can be rebooted
//! under the same identity. `queue_*` are configured far below their
//! production ~10-hour attempt budget so step 8 completes in seconds rather
//! than hours -- that arithmetic is pinned by `syneroym-async-queue`'s own
//! unit tests, not re-proven here; this file proves the *sequence*.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use common::SubstrateNode;
use rustls::crypto::ring;
use semver::Version;
use serde_json::{Value, json};
use syneroym_app_orchestration::{
    Visibility,
    models::{
        AppBlueprintId, LogicalServiceName, PlacementSelector, ServiceConfig, ServiceSpec,
        ServiceType, SubstrateAlias, SynAppManifest,
    },
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_core::config::SupervisorRole;
use syneroym_identity::Identity;
use tokio::time;

mod common;

#[path = "common/retry.rs"]
mod retry;

const MANAGED_A_ALIAS: &str = "managed-a";
const MANAGED_B_ALIAS: &str = "managed-b";
const INSTANCE_ID: &str = "b1-ref-scenario-inst";

/// Fast, test-only queue knobs. The production defaults give a ~10-hour
/// attempt budget -- deliberately not reproduced here; this scenario needs
/// the *sequence* to happen, not the real window.
///
/// `poll_interval_secs` is a caller-chosen parameter: the resident loop's
/// own pass is what makes the *initial* scale-out push due
/// (`submit`/`force-reconcile` are both all-or-nothing across every placed
/// alias, so neither can land backend's redeploy while managed-b is down --
/// only the loop's own best-effort connect can). Discovery needs a short
/// interval; the recovery budget ("within one worker tick, not one poll
/// interval") needs a long one for the *second* boot, so a passing
/// convergence cannot be a coincidence of the loop also happening to retry
/// in time.
///
/// `queue_max_attempts` is caller-chosen too: a low budget is the point for
/// the DLQ test (it wants delivery to exhaust and dead-letter); a caller
/// asserting the item is *still pending* across a restart needs enough
/// headroom that a slow CI runner's extra wall-clock time can't dead-letter
/// it out from under that assertion first.
fn supervisor_role(poll_interval_secs: u64, queue_max_attempts: u8) -> SupervisorRole {
    SupervisorRole {
        queue_tick_secs: 1,
        queue_max_attempts,
        queue_max_backoff_secs: 1,
        queue_visibility_timeout_secs: 5,
        queue_dlq_max_rows: 10,
        ..common::supervisor_role(poll_interval_secs)
    }
}

/// `backend` (the dependency) on `managed-a`, scaled to `backend_replicas`;
/// `frontend` (the dependent) on `managed-b`, depending on it. Plain TCP
/// services -- this scenario is about durable delivery of the binding
/// write, not about a live dependency call resolving through it.
fn manifest(backend_replicas: u32) -> SynAppManifest {
    let mut services = BTreeMap::new();
    services.insert(
        LogicalServiceName::new("backend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41801".to_string(),
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
                visibility: Visibility::Internal,
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_A_ALIAS))),
            replicas: backend_replicas,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
        },
    );
    services.insert(
        LogicalServiceName::new("frontend"),
        ServiceSpec {
            config: ServiceConfig {
                service_type: ServiceType::Tcp,
                source: "127.0.0.1:41802".to_string(),
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
                visibility: Visibility::Internal,
            },
            depends_on: vec![LogicalServiceName::new("backend")],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_B_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
        },
    );
    SynAppManifest {
        id: AppBlueprintId::new("syneroym:b1-ref-scenario"),
        version: Version::new(0, 1, 0),
        description: None,
        placement: None,
        services,
        dependencies: BTreeMap::new(),
    }
}

async fn compiled_plan_json(backend_replicas: u32) -> String {
    common::compiled_plan_json(&manifest(backend_replicas), INSTANCE_ID).await
}

fn submission(plan_json: String, inventory_json: String, generation: u64) -> Value {
    common::submission(INSTANCE_ID, plan_json, inventory_json, generation)
}

fn str_field<'a>(v: &'a Value, field: &str) -> Option<&'a str> {
    v.get(field).and_then(|f| f.as_str())
}

/// The very first `submit` after booting every node needs the supervisor
/// to reach *both* freshly-started managed substrates at once --
/// `connected_client`'s own 10s budget (`MANAGED_SUBSTRATE_CONNECT_
/// TIMEOUT`) can be tighter than DHT/relay propagation for an endpoint
/// record published moments earlier. Every later call in this file (after
/// the first pass has already reached each substrate once) needs no such
/// retry.
///
/// `supervisor_node`'s own connection can independently be the thing at
/// fault here too: it was dialed and proven live by its own
/// `wait_for_ready` during its own boot, then sat idle through both managed
/// nodes' full boots -- long enough under CI's scheduling pressure for the
/// peer to abandon that idle path ("no viable network path exists: last
/// path abandoned by peer"; same root cause fixed throughout this crate's
/// e2e tests, e.g. `binding_push_e2e.rs`). A dead connection object never
/// heals itself by retrying the same request against it, so one
/// `shutdown`-then-`connect` redial is folded into the loop below whenever
/// a retriable attempt fails -- cheap insurance on top of the DHT-wait loop
/// this helper already had.
async fn submit_with_retry(supervisor_node: &mut SubstrateNode, params: Value) {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        match supervisor_node.substrate_client.request("supervisor", "submit", params.clone()).await
        {
            Ok(_) => return,
            Err(e) if Instant::now() < deadline => {
                tracing::debug!(error = %e, "initial submit not yet reachable, retrying");
                if supervisor_node.substrate_client.shutdown().await.is_ok() {
                    let _ = supervisor_node.substrate_client.connect().await;
                }
                time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => panic!("submit failed after retrying for 180s: {e}"),
        }
    }
}

async fn supervisor_status(supervisor_node: &SubstrateNode) -> Value {
    supervisor_node
        .substrate_client
        .request("supervisor", "status", json!([INSTANCE_ID]))
        .await
        .expect("status failed")
        .result
}

/// `frontend`'s convergence row for its `backend` dependency, off the
/// supervisor's `status` -- `None` until the first push lands at all.
fn frontend_binding(status: &Value) -> Option<&Value> {
    status.get("bindings").and_then(Value::as_array).into_iter().flatten().find(|b| {
        str_field(b, "dependent_logical_ref").is_some_and(|r| r.ends_with("/frontend#0"))
            && str_field(b, "dependency_name") == Some("backend")
    })
}

fn is_converged(status: &Value) -> bool {
    frontend_binding(status).and_then(|b| b.get("converged")).and_then(Value::as_bool) == Some(true)
}

/// The ids of every item this instance's outbox currently holds against
/// `substrate_did`. Steps 4/5/7 of the reference scenario assert the item
/// is "in the outbox"/"still queued"/"the outbox is empty", which only
/// this verb (not `alerts` or `is_converged`) can answer directly.
async fn outbox_item_ids(supervisor_node: &SubstrateNode, substrate_did: &str) -> Vec<u64> {
    let items = supervisor_node
        .substrate_client
        .request("supervisor", "outbox", json!([INSTANCE_ID]))
        .await
        .expect("outbox failed")
        .result;
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| str_field(i, "substrate_did") == Some(substrate_did))
        .filter_map(|i| i.get("id").and_then(Value::as_u64))
        .collect()
}

async fn active_alert_kinds(
    supervisor_node: &SubstrateNode,
    substrate_did: &str,
) -> BTreeSet<String> {
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

#[tokio::test]
async fn a_binding_push_to_an_offline_substrate_converges_after_it_returns() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    // The supervisor hosts the registry and restarts at step 5, so it must
    // come back on the same ports and identity -- reuse one captured
    // builder. Short poll interval on the first boot so the resident loop's
    // own pass discovers step 3's scale-out and enqueues the failed push
    // while managed-b is down; high queue_max_attempts so a slow CI runner
    // can't dead-letter that item before step 5 asserts it is still pending.
    let supervisor_dir = tempfile::tempdir().expect("failed to create temp dir");
    let supervisor_builder = SubstrateNode::builder()
        .owner(&supervisor_owner)
        .base_path(supervisor_dir.path())
        .inject_kek();
    let mut supervisor_node =
        supervisor_builder.clone().supervisor(supervisor_role(3, 100)).boot().await;
    let shared_registry = supervisor_node.registry_url().to_string();
    let shared_relay = supervisor_node.relay_url().to_string();

    let mut managed_a = SubstrateNode::builder()
        .owner(&managed_owner)
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek()
        .boot()
        .await;

    // `managed_b_dir` outlives managed-b's own teardown in step 2 so step 6
    // can reboot the *same* identity.
    let managed_b_dir = tempfile::tempdir().expect("failed to create temp dir");
    let managed_b = SubstrateNode::builder()
        .owner(&managed_owner)
        .base_path(managed_b_dir.path())
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek()
        .boot()
        .await;
    let managed_b_did = managed_b.did().to_string();
    let managed_b_registry_url = managed_b.registry_url().to_string();

    let grant_a =
        common::node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), managed_a.did());
    let grant_b =
        common::node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), &managed_b_did);
    let inventory_json = serde_json::to_string(&BTreeMap::from([
        (
            MANAGED_A_ALIAS.to_string(),
            SupervisorInventoryEntry {
                did: managed_a.did().to_string(),
                api_url: Some(managed_a.registry_url().to_string()),
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

    // Step 1: deploy, converged.
    let plan_json = compiled_plan_json(1).await;
    submit_with_retry(&mut supervisor_node, submission(plan_json, inventory_json.clone(), 0)).await;

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = supervisor_status(&supervisor_node).await;
        if is_converged(&status) {
            break;
        }
        assert!(Instant::now() < deadline, "initial deploy never converged: {status:?}");
        time::sleep(Duration::from_millis(300)).await;
    }

    // Step 2: managed-b (frontend's host) goes down.
    managed_b.teardown().await;

    // Step 3: scale backend on managed-a -- a membership change that makes
    // a binding push to frontend (on the now-offline managed-b) due.
    // `submit`'s own best-effort apply is all-or-nothing across every
    // targeted alias (`build_clients`), so this call itself is expected to
    // surface an error even though the desired state underneath it is
    // still durably recorded -- the same shape `supervisor_loop_e2e.rs`
    // already establishes for a partial deploy.
    let scaled_plan_json = compiled_plan_json(2).await;
    let scale_out_res = supervisor_node
        .substrate_client
        .request("supervisor", "submit", submission(scaled_plan_json, inventory_json.clone(), 0))
        .await;
    assert!(scale_out_res.is_err(), "submit with managed-b down must surface the failure");

    // Step 4: the synchronous push fails. Degraded, unconverged -- and not
    // retried forever in-process: the `submit` call above already
    // returned rather than blocking on delivery.
    //
    // Waited for directly, not inferred from `status.state == Degraded`:
    // the health sweep alone reports Degraded the moment backend's new
    // replica is merely `missing_placement` (before it has even landed on
    // managed-a), which is unrelated to -- and can precede -- the push
    // failure this step actually asserts. Only once backend#1 has landed
    // does frontend's diff become a push candidate at all.
    //
    // A generous deadline: every pass while managed-b is down pays its own
    // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` (10s) trying to reach it, and
    // `MissedTickBehavior::Skip` means a pass that overruns
    // `poll_interval_secs` (3s) drops the tick it overran rather than
    // queueing a burst -- so the loop's real cadence here is close to 10s
    // per pass, not 3s, and landing backend's redeploy plus attempting
    // frontend's push can take a few such passes.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if active_alert_kinds(&supervisor_node, &managed_b_did).await.contains("BINDING_CONFLICT") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a push that cannot reach its target must still raise BindingConflict (matrix row 11)"
        );
        time::sleep(Duration::from_millis(300)).await;
    }
    let status = supervisor_status(&supervisor_node).await;
    assert!(
        str_field(&status, "state") == Some("Degraded") && !is_converged(&status),
        "once the push has failed the instance must report Degraded, unconverged: {status:?}"
    );
    let outbox_ids_before_restart = outbox_item_ids(&supervisor_node, &managed_b_did).await;
    assert_eq!(
        outbox_ids_before_restart.len(),
        1,
        "task.md step 4: the failed push must be in the outbox, not merely inferred from alerts"
    );

    // Step 5: restart the supervisor process. The queued item survives --
    // this is the step no in-process retry can.
    //
    // Long poll interval on this second boot so the resident loop's own
    // next pass is nowhere near step 7's window, and the convergence it
    // asserts there can only have come from the queue worker. High
    // queue_max_attempts for the same reason as the first boot: managed-b
    // is still rebooting when this worker's first ticks fire.
    supervisor_node.teardown().await;
    let mut supervisor_node =
        supervisor_builder.supervisor(supervisor_role(3600, 100)).boot().await;

    // The community registry supervisor_node hosts keeps its records in
    // memory, so its own restart empties it -- including managed-a's own
    // record, even though managed-a itself was never restarted. Nothing
    // else re-publishes that record on managed-a's behalf before its own
    // hourly heartbeat, so a health-sweep pass connecting to it here would
    // otherwise find it genuinely absent from the registry (not merely
    // slow to reach) until then. Forced immediately via `republish` rather
    // than waiting on it.
    crate::call_with_reconnect!(
        managed_a.substrate_client,
        managed_a.substrate_client.republish().await
    );
    let status = supervisor_status(&supervisor_node).await;
    assert!(
        !is_converged(&status),
        "the item must still be queued right after restart: {status:?}"
    );
    assert_eq!(
        outbox_item_ids(&supervisor_node, &managed_b_did).await,
        outbox_ids_before_restart,
        "task.md step 5: the exact same item must still be queued across the restart, not a \
         different or duplicated one"
    );

    // Step 6: bring managed-b back, same identity (same `base_path`). Fresh
    // ports are fine -- it re-publishes its new address to the supervisor's
    // registry, which the supervisor resolves it through.
    let managed_b = SubstrateNode::builder()
        .owner(&managed_owner)
        .base_path(managed_b_dir.path())
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek()
        .boot()
        .await;
    assert_eq!(managed_b.did(), managed_b_did, "the rebooted node must keep its identity");

    // `supervisor_node`'s own connection sat idle through managed-b's full
    // boot just above, long enough under CI's scheduling pressure for the
    // peer to abandon that idle path ("no viable network path exists: last
    // path abandoned by peer"; same root cause fixed throughout this
    // crate's e2e tests). `supervisor_status` panics on any error, so a
    // stale connection here would abort the test with a misleading
    // "status failed" instead of the deadline loop below ever getting a
    // chance to run -- redial once, before entering that loop, rather than
    // letting the first iteration find out the hard way.
    let _ = crate::call_with_reconnect!(
        supervisor_node.substrate_client,
        "supervisor",
        "status",
        json!([INSTANCE_ID])
    );

    // Step 7: within one worker tick (1s here; the budget is
    // "not one poll interval", 3600s in this test), convergence resumes
    // and the queue's own delivery clears the alert it raised. The
    // deadline is generous (matching step 4's) because it bounds a freshly
    // rebooted node's own relay/DHT re-announce, not the worker tick
    // itself -- the worker retries every `queue_tick_secs` (1s) regardless,
    // but each attempt still pays its own connect timeout until managed-b
    // is genuinely reachable again.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let status = supervisor_status(&supervisor_node).await;
        if is_converged(&status) && str_field(&status, "state") == Some("Active") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "recovery after managed-b returned took longer than one worker tick: {status:?}"
        );
        time::sleep(Duration::from_millis(300)).await;
    }
    assert!(
        active_alert_kinds(&supervisor_node, &managed_b_did).await.is_empty(),
        "the earlier BindingConflict must clear once delivery converges"
    );
    assert!(
        outbox_item_ids(&supervisor_node, &managed_b_did).await.is_empty(),
        "task.md step 7: the outbox must be empty once delivery converges, not merely the alert \
         cleared"
    );

    supervisor_node.teardown().await;
    managed_a.teardown().await;
    managed_b.teardown().await;
}

/// Step 8: a substrate that never comes back exhausts the outbox's attempt
/// budget, and the item becomes visible and replayable rather than
/// silently lost.
#[tokio::test]
async fn a_permanently_unreachable_substrate_lands_in_the_dlq_and_replays() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    // Short poll interval, the same reason the reference-scenario test's
    // first boot has one: the resident loop's pass discovers the scale-out
    // diff and enqueues the failed push. Unlike that test, this one *wants*
    // the low queue_max_attempts -- it asserts the item does dead-letter.
    // The supervisor never restarts here.
    let mut supervisor_node = SubstrateNode::builder()
        .owner(&supervisor_owner)
        .supervisor(supervisor_role(3, 3))
        .inject_kek()
        .boot()
        .await;
    let shared_registry = supervisor_node.registry_url().to_string();
    let shared_relay = supervisor_node.relay_url().to_string();

    let managed_a = SubstrateNode::builder()
        .owner(&managed_owner)
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek()
        .boot()
        .await;

    let managed_b = SubstrateNode::builder()
        .owner(&managed_owner)
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek()
        .boot()
        .await;
    let managed_b_did = managed_b.did().to_string();
    let managed_b_registry_url = managed_b.registry_url().to_string();

    let grant_a =
        common::node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), managed_a.did());
    let grant_b =
        common::node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), &managed_b_did);
    let inventory_json = serde_json::to_string(&BTreeMap::from([
        (
            MANAGED_A_ALIAS.to_string(),
            SupervisorInventoryEntry {
                did: managed_a.did().to_string(),
                api_url: Some(managed_a.registry_url().to_string()),
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

    let plan_json = compiled_plan_json(1).await;
    submit_with_retry(&mut supervisor_node, submission(plan_json, inventory_json.clone(), 0)).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if is_converged(&supervisor_status(&supervisor_node).await) {
            break;
        }
        assert!(Instant::now() < deadline, "initial deploy never converged");
        time::sleep(Duration::from_millis(300)).await;
    }

    // managed-b goes down permanently -- unlike the first test, it never
    // comes back.
    managed_b.teardown().await;

    // Expected to error, the same all-or-nothing reason as the first
    // test's step 3 -- the desired state underneath it is still recorded.
    let scaled_plan_json = compiled_plan_json(2).await;
    let scale_out_res = supervisor_node
        .substrate_client
        .request("supervisor", "submit", submission(scaled_plan_json, inventory_json, 0))
        .await;
    assert!(scale_out_res.is_err(), "submit with managed-b down must surface the failure");

    // The test role's queue_max_attempts (3) and queue_max_backoff_secs
    // (1) put the item in the DLQ well inside this deadline, not the
    // ~10-hour production window (pinned separately by
    // syneroym-async-queue's own unit tests) -- but each of those 3
    // attempts still pays a real `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` (10s)
    // against a genuinely offline node, on top of the resident loop's own
    // pass first discovering the diff and enqueueing it at all. The two
    // take turns rather than racing (the `instance_lock`), so a slow
    // pass and a slow delivery attempt do not overlap -- roughly doubling
    // the real wall-clock cost of each round relative to either alone.
    let deadline = Instant::now() + Duration::from_secs(280);
    let dead_letters = loop {
        let rows = supervisor_node
            .substrate_client
            .request("supervisor", "dead-letters", json!([INSTANCE_ID]))
            .await
            .expect("dead-letters failed")
            .result;
        let rows = rows.as_array().cloned().unwrap_or_default();
        if !rows.is_empty() {
            break rows;
        }
        assert!(Instant::now() < deadline, "the item never reached the DLQ");
        time::sleep(Duration::from_millis(300)).await;
    };
    assert_eq!(dead_letters.len(), 1, "{dead_letters:?}");
    assert!(
        active_alert_kinds(&supervisor_node, &managed_b_did).await.contains("DELIVERY_EXHAUSTED"),
        "an exhausted delivery budget must raise an alert, not fail silently"
    );

    // `replay` re-queues rather than executing inline: the RPC call itself
    // returns promptly, well before this permanently-down substrate could
    // ever actually answer.
    let dead_letter_id = dead_letters[0].get("id").and_then(Value::as_u64).expect("id");
    time::timeout(
        Duration::from_secs(5),
        supervisor_node.substrate_client.request(
            "supervisor",
            "replay",
            json!([INSTANCE_ID, dead_letter_id]),
        ),
    )
    .await
    .expect("replay must not block on delivery")
    .expect("replay failed");

    // managed-b is still down, so the replayed item fails again and
    // returns to the DLQ -- with its attempt history intact, not
    // a fresh budget. Still generous, the same reason the first wait
    // above is: a real connect timeout per attempt, serialized against the
    // resident loop's own pass via `instance_lock`.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let rows = supervisor_node
            .substrate_client
            .request("supervisor", "dead-letters", json!([INSTANCE_ID]))
            .await
            .expect("dead-letters failed")
            .result;
        let rows = rows.as_array().cloned().unwrap_or_default();
        if rows.len() == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "the replayed item never returned to the DLQ");
        time::sleep(Duration::from_millis(300)).await;
    }

    // managed-b was already torn down above and never rebooted -- this
    // test's whole point is that it stays permanently unreachable.
    supervisor_node.teardown().await;
    managed_a.teardown().await;
}

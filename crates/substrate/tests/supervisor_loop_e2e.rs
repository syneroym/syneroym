#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A partial deploy across two real managed substrates, one of which is
//! down at `submit` time.
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
//! The managed-b node boots from a caller-owned directory (via
//! `SubstrateNode::builder().base_path(..)`) so the same identity can be
//! torn down and rebooted later in the same test -- simulating "the node
//! comes back", not "a different node joins".

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use common::{
    SubstrateNode, compiled_plan_json, node_wide_supervisor_grant, submission, supervisor_role,
};
use rustls::crypto::ring;
use semver::Version;
use serde_json::json;
use syneroym_app_orchestration::models::{
    AppBlueprintId, LogicalServiceName, PlacementSelector, ServiceConfig, ServiceSpec, ServiceType,
    SubstrateAlias, SynAppManifest,
};
use syneroym_app_supervisor::inventory::SupervisorInventoryEntry;
use syneroym_identity::Identity;
use tokio::time;

mod common;

#[path = "common/retry.rs"]
mod retry;

const MANAGED_A_ALIAS: &str = "managed-a";
const MANAGED_B_ALIAS: &str = "managed-b";

/// Short enough that the test does not wait a real 30s for a second pass,
/// long enough that two passes cannot be confused for one in flight.
const POLL_INTERVAL_SECS: u64 = 3;

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
                assets: None,
                visibility: Default::default(),
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_A_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
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
                assets: None,
                visibility: Default::default(),
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_B_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
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
    let _serial_guard = common::serial_guard().await;
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

    let managed_a = SubstrateNode::builder()
        .owner(&managed_owner)
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek()
        .boot()
        .await;

    // `managed_b_dir` is a `TempDir` this test owns directly precisely so it
    // survives `managed_b`'s own teardown below -- the whole point is to
    // reboot the *same* identity, not a fresh one. Booting `managed_b` with
    // `.base_path` on it keeps the on-disk identity in that directory.
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
        node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), managed_a.did());
    let grant_b = node_wide_supervisor_grant(&managed_owner, supervisor_node.did(), &managed_b_did);
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

    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during boot, then sat idle through both managed
    // nodes' full boots plus the submit attempt above -- long enough under
    // CI's scheduling pressure for the peer to abandon that idle path ("no
    // viable network path exists: last path abandoned by peer"). The
    // status loop below panics via `.expect("status failed")` rather than
    // looping past a connection error, so redial once here, before
    // entering it, rather than letting its first iteration find out the
    // hard way.
    let _ = crate::call_with_reconnect!(
        supervisor_node.substrate_client,
        "supervisor",
        "status",
        json!([instance_id])
    );

    // The resident loop's own pass (poll_interval_secs=3) connects
    // best-effort and lands `svc-a` on the reachable substrate while
    // `svc-b` stays missing -- polled with a generous budget since each
    // pass that touches the still-down `managed-b` spends up to
    // `MANAGED_SUBSTRATE_CONNECT_TIMEOUT` (10s) doing so. The manifest
    // declares no health check (a `tcp` service with no probe
    // reports `unknown`, not a fault), so "landed" is read off `signal
    // != "not-deployed"`, not `"healthy"`.
    //
    // A 40s deadline was too tight against this
    // loop's own cost. `svc-a` lands at 28-30s in every run measured, and
    // `status`'s own on-demand sweep reads the journal at the
    // *start* of the call, before it blocks up to 10s reaching the still-
    // down `managed-b` -- so the 40s deadline left room for roughly one
    // more poll after landing, and whether that poll's journal read fell
    // before or after the deploy committed was a coin flip. Widened to
    // match the second wait loop's own margin below (90s), which leaves
    // several full 10s-worst-case polls of headroom after landing rather
    // than one.
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
        if signal_of(&services, "a5c-loop-inst/svc-a#0") == Some("unknown") && state == "Degraded" {
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
    assert_eq!(signal_of(&services, "a5c-loop-inst/svc-a#0"), Some("unknown"), "{services:?}");

    // Managed B comes back, same identity (same `base_path`) -- the loop's
    // next pass must retry only the failed service.
    let managed_b = SubstrateNode::builder()
        .owner(&managed_owner)
        .base_path(managed_b_dir.path())
        .shared_registry(&shared_registry)
        .shared_relay(&shared_relay)
        .inject_kek()
        .boot()
        .await;
    assert_eq!(managed_b.did(), managed_b_did, "the rebooted node must keep its identity");

    // Same idle-path risk as the preflight above, this time across
    // managed-b's own reboot: redial `supervisor_node` once before the
    // next status loop if the connection did not survive the wait.
    let _ = crate::call_with_reconnect!(
        supervisor_node.substrate_client,
        "supervisor",
        "status",
        json!([instance_id])
    );

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
        if signal_of(&services, "a5c-loop-inst/svc-a#0") == Some("unknown")
            && signal_of(&services, "a5c-loop-inst/svc-b#0") == Some("unknown")
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

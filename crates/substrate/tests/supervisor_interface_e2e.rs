#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The `supervisor` interface end to end (M05A A5b), across two genuinely
//! independent `syneroym-substrate` instances: one running the supervisor
//! role, one plain managed substrate. The pair and the submit helpers come
//! from `common`; the manifests are local.
//!
//! The supervisor's own grant on a managed substrate is hand-issued
//! (`common::node_wide_supervisor_grant`): `submit` cannot bootstrap its own
//! authority, and nothing issues a supervisor a grant automatically.

use std::{collections::BTreeMap, time::Duration};

use anyhow::Result;
use common::{SubstrateNode, compiled_plan_json, submission, supervisor_and_managed};
use semver::Version;
use serde_json::json;
use syneroym_app_orchestration::models::{
    AppBlueprintId, LogicalServiceName, PlacementSelector, ServiceConfig, ServiceSpec, ServiceType,
    SubstrateAlias, SynAppManifest,
};
use syneroym_identity::Identity;
use syneroym_rpc::JsonRpcResponse;

mod common;

#[path = "common/retry.rs"]
mod retry;

const MANAGED_ALIAS: &str = "managed";

/// No test here waits on the resident loop, so the poll interval stays near
/// the production default.
const POLL_INTERVAL_SECS: u64 = 30;

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
                assets: None,
                visibility: Default::default(),
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
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
                assets: None,
                visibility: Default::default(),
            },
            depends_on: vec![],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_ALIAS))),
            replicas: 1,
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
                assets: None,
                visibility: Default::default(),
            },
            depends_on: vec![LogicalServiceName::new("backend")],
            placement: Some(PlacementSelector::Substrate(SubstrateAlias::new(MANAGED_ALIAS))),
            replicas: 1,
            sharding_strategy: None,
            schedule: None,
            topology_visibility: Default::default(),
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

/// Every test in this file reaches the supervisor node here as its first
/// call. That connection was dialed and proven live by its own
/// `wait_for_ready` during boot, then sat idle through the managed node's
/// own full boot inside `supervisor_and_managed` -- long enough under CI's
/// scheduling pressure for the peer to abandon that idle path ("no viable
/// network path exists: last path abandoned by peer").
/// `SyneroymClient::connect` no-ops on an already-`Some` connection, so
/// recovering means an explicit `shutdown`-then-`connect` (redial) before
/// one retry, not just retrying the same request on the same dead
/// connection.
async fn submit_after_boot(
    supervisor_node: &mut SubstrateNode,
    params: serde_json::Value,
) -> Result<JsonRpcResponse> {
    if let Ok(resp) =
        supervisor_node.substrate_client.request("supervisor", "submit", params.clone()).await
    {
        return Ok(resp);
    }
    supervisor_node.substrate_client.shutdown().await?;
    supervisor_node.substrate_client.connect().await?;
    supervisor_node.substrate_client.request("supervisor", "submit", params).await
}

#[tokio::test]
async fn an_operator_submits_and_reads_back_status_over_the_supervisor_interface() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) = supervisor_and_managed(
        &supervisor_owner,
        &managed_owner,
        POLL_INTERVAL_SECS,
        MANAGED_ALIAS,
    )
    .await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-submit-inst").await;

    let res = submit_after_boot(
        &mut supervisor_node,
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
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) = supervisor_and_managed(
        &supervisor_owner,
        &managed_owner,
        POLL_INTERVAL_SECS,
        MANAGED_ALIAS,
    )
    .await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-second-inst").await;

    // First supervisor submits and adopts, claiming generation 1.
    submit_after_boot(
        &mut supervisor_node,
        submission("a5b-second-inst", plan_json, inventory_json, 0),
    )
    .await
    .expect("first submit failed");
    let adopted = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a5b-second-inst"]))
        .await
        .expect("adopt failed");
    assert_eq!(adopted.result.get("generation").and_then(serde_json::Value::as_u64), Some(1));

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
    let mut second_writer = managed_node.client_as(Identity::from_bytes(&managed_owner.to_bytes()));
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
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) = supervisor_and_managed(
        &supervisor_owner,
        &managed_owner,
        POLL_INTERVAL_SECS,
        MANAGED_ALIAS,
    )
    .await;

    let manifest = bound_app_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-bound-inst").await;

    let res = submit_after_boot(
        &mut supervisor_node,
        submission("a5b-bound-inst", plan_json, inventory_json, 0),
    )
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
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) = supervisor_and_managed(
        &supervisor_owner,
        &managed_owner,
        POLL_INTERVAL_SECS,
        MANAGED_ALIAS,
    )
    .await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-adopt-inst").await;
    submit_after_boot(
        &mut supervisor_node,
        submission("a5b-adopt-inst", plan_json, inventory_json, 0),
    )
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
    assert_eq!(first.result.get("generation").and_then(serde_json::Value::as_u64), Some(1));

    // A second adopt (the same supervisor, simulating a rebuilt one) reads
    // the now-held generation 1 back and claims 2.
    let second = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a5b-adopt-inst"]))
        .await
        .expect("second adopt failed");
    assert_eq!(second.result.get("generation").and_then(serde_json::Value::as_u64), Some(2));

    supervisor_node.teardown().await;
    managed_node.teardown().await;
}

#[tokio::test]
async fn a_pushed_binding_reaches_a_dependent_the_supervisor_deployed() {
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, mut managed_node, inventory_json) = supervisor_and_managed(
        &supervisor_owner,
        &managed_owner,
        POLL_INTERVAL_SECS,
        MANAGED_ALIAS,
    )
    .await;

    let manifest = bound_app_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-binding-inst").await;
    submit_after_boot(
        &mut supervisor_node,
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
            s.get("logical_ref").and_then(|v| v.as_str()).map(|r| r.ends_with("/frontend#0"))
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
    //
    // This is the first call this test makes directly on `managed_node`'s
    // own client -- its connection was dialed and proven live by its own
    // `wait_for_ready` inside `supervisor_and_managed`, then sat idle through the
    // `submit`/`status` calls above, long enough under CI's scheduling
    // pressure for the peer to abandon that idle path. Recover by explicit
    // shutdown→reconnect before one retry.
    let managed_status = crate::call_with_reconnect!(
        managed_node.substrate_client,
        managed_node.substrate_client.status(vec![frontend_id.clone()]).await
    );
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
    let _serial_guard = common::serial_guard().await;
    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();
    let (mut supervisor_node, managed_node, inventory_json) = supervisor_and_managed(
        &supervisor_owner,
        &managed_owner,
        POLL_INTERVAL_SECS,
        MANAGED_ALIAS,
    )
    .await;

    let manifest = one_service_manifest();
    let plan_json = compiled_plan_json(&manifest, "a5b-superseded-inst").await;
    submit_after_boot(
        &mut supervisor_node,
        submission("a5b-superseded-inst", plan_json, inventory_json, 0),
    )
    .await
    .expect("submit failed");
    let adopted = supervisor_node
        .substrate_client
        .request("supervisor", "adopt", json!(["a5b-superseded-inst"]))
        .await
        .expect("adopt failed");
    assert_eq!(adopted.result.get("generation").and_then(serde_json::Value::as_u64), Some(1));

    // A second writer claims generation 2 directly against the managed
    // substrate. The first supervisor's own store still holds generation 1.
    let mut second_writer = managed_node.client_as(Identity::from_bytes(&managed_owner.to_bytes()));
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

#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The `messaging` registration and the publish/subscribe string symmetry,
//! proven live across two real
//! `syneroym-substrate` instances -- no unit test can prove the router's
//! subscribe-side namespacing (`dispatch.rs::subscribe_namespaced_topic`)
//! actually lines up with the supervisor's own publish-side namespacing
//! (`SupervisorService::publish_opened_alerts`) end to end.
//!
//! Both nodes come from `common::SubstrateNode`: a supervisor node hosting
//! the registry and a managed node publishing into it. `common::serial_guard`
//! keeps this binary's tests from running substrate stacks at once.

use std::time::Duration;

use common::{MANAGED_ALIAS, compiled_plan_json, submission, supervisor_and_managed};
use serde_json::json;
use syneroym_app_orchestration::AlertKind;
use syneroym_control_plane::SUPERVISOR_RESERVED_SERVICE_ID;
use syneroym_identity::Identity;
use syneroym_mqtt_broker::namespace_topic_for_publish;
use tokio::time;

mod common;

/// An operator connects to the supervisor node's own DID, subscribes over
/// its `messaging` native capability to the alert topic, and receives the
/// notification the supervisor publishes when its next `status` sweep opens
/// a `SubstrateUnreachable` alert -- proving the supervisor's `messaging`
/// registration and its publish-side namespacing rule produce the exact
/// same topic string the router's subscribe-side fix
/// (`dispatch.rs::subscribe_namespaced_topic`) computes, with a real broker
/// and a real wire round trip on both ends.
#[tokio::test]
async fn an_operator_subscribed_to_the_alert_topic_receives_an_opened_alert() {
    let _serial_guard = common::serial_guard().await;

    let supervisor_owner = Identity::generate().unwrap();
    let managed_owner = Identity::generate().unwrap();

    let (mut supervisor_node, managed_node, inventory_json) =
        supervisor_and_managed(&supervisor_owner, &managed_owner, 30, MANAGED_ALIAS).await;
    let managed_did = managed_node.did().to_string();

    let instance_id = "a5c-alerts-inst";
    let manifest = common::one_service_manifest("syneroym:a5c-alerts-test-app");
    let plan_json = compiled_plan_json(&manifest, instance_id).await;
    let submit_params = submission(instance_id, plan_json, inventory_json, 0);

    // `supervisor_node`'s connection was dialed and proven live by its own
    // `wait_for_ready` during boot, then sat idle for the entire
    // `managed_node` boot that followed (a second full substrate start) --
    // long enough under CI's scheduling pressure for the peer to abandon
    // that idle path ("no viable network path exists: last path abandoned
    // by peer").
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
    let mut operator = supervisor_node.client_as(operator_identity);
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

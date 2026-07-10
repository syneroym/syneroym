use std::time::Duration;

use crate::{MessagingError, MqttBroker, MqttBrokerConfig, namespace_topic};

fn test_broker() -> MqttBroker {
    MqttBroker::new(MqttBrokerConfig::default()).expect("broker construction")
}

#[tokio::test]
async fn publish_and_subscribe_same_topic_delivers_message() {
    let broker = test_broker();
    let (_handle, mut rx) = broker.subscribe("hello/world").await.expect("subscribe");
    broker.publish("hello/world", b"hi there".to_vec()).await.expect("publish");

    let (topic, payload) = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("did not time out")
        .expect("channel not closed");
    assert_eq!(topic, "hello/world");
    assert_eq!(payload, b"hi there");
}

#[tokio::test]
async fn wildcard_subscription_matches_single_level() {
    let broker = test_broker();
    let (_handle, mut rx) = broker.subscribe("sensors/+/temp").await.expect("subscribe");
    broker.publish("sensors/room1/temp", b"21c".to_vec()).await.expect("publish");

    let (topic, payload) = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("did not time out")
        .expect("channel not closed");
    assert_eq!(topic, "sensors/room1/temp");
    assert_eq!(payload, b"21c");
}

#[tokio::test]
async fn retained_message_delivered_to_late_subscriber() {
    let broker = test_broker();
    broker
        .publish_retained_for_test("retained/topic", b"retained-payload".to_vec())
        .await
        .expect("retained publish");

    // Subscriber joins strictly after the retaining publish.
    let (_handle, mut rx) =
        broker.subscribe("retained/topic").await.expect("subscribe after retain");

    let (topic, payload) = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("did not time out")
        .expect("channel not closed");
    assert_eq!(topic, "retained/topic");
    assert_eq!(payload, b"retained-payload");
}

/// `rumqttd` 0.20.0 exposes no way to stop its own internal router thread
/// (see the module docs on [`MqttBroker`]), so this test validates what
/// actually is true and cancellable: dropping `MqttBroker` cancels its
/// `CancellationToken`, which cascades to every live subscription's
/// forwarding task (a genuine Tokio task this crate owns) and closes its
/// receiver — all within the 1-second bound task.md asks for.
#[tokio::test]
async fn dropping_broker_terminates_subscription_forwarding_tasks_within_one_second() {
    let broker = test_broker();
    let (_handle, mut rx) = broker.subscribe("some/topic").await.expect("subscribe");

    drop(broker);

    let result = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
    assert_eq!(
        result.expect("forwarding task did not stop within 1s"),
        None,
        "receiver should observe closure, not a stray message"
    );
}

/// The publish-side host<->router bridge is rumqttd's own internal event
/// channel (fixed capacity, not the configurable `channel_capacity` --
/// see the module docs), which a fast, un-drained flood of publishes
/// reliably saturates well within this loop's bound. Asserts `publish`
/// degrades to a clean `Internal` error rather than blocking or panicking.
#[tokio::test]
async fn publish_returns_backpressure_error_when_channel_saturated() {
    let broker = test_broker();
    let mut saw_backpressure = false;
    for i in 0..20_000 {
        match broker.publish("flood/topic", format!("msg-{i}").into_bytes()).await {
            Ok(()) => {}
            Err(MessagingError::Internal(msg)) => {
                assert!(msg.contains("backpressure"), "unexpected internal error: {msg}");
                saw_backpressure = true;
                break;
            }
            Err(other) => panic!("unexpected error variant: {other:?}"),
        }
    }
    assert!(saw_backpressure, "expected publish to eventually report backpressure");
}

/// ADR-0010 Finding A5: no MQTT-protocol network listener is ever bound.
/// `MqttBroker::new` never sets `v4`/`v5`/`ws` (no config knob exposes
/// them), so the standard MQTT port must remain free for anything else to
/// bind -- confirmed here by successfully binding it ourselves right after
/// constructing a broker.
#[tokio::test]
async fn no_network_listener_is_bound() {
    let _broker = test_broker();
    let bind_result = tokio::net::TcpListener::bind("127.0.0.1:1883").await;
    assert!(
        bind_result.is_ok(),
        "expected the standard MQTT port to be free (no listener bound by MqttBroker), got: \
         {bind_result:?}"
    );
}

#[test]
fn namespace_topic_disambiguation() {
    assert_eq!(
        namespace_topic("svc-a", "svc/svc-b/orders/new"),
        "svc/svc-b/orders/new",
        "an already-qualified svc/ topic is taken literally"
    );
    assert_eq!(
        namespace_topic("svc-a", "orders/new"),
        "svc/svc-a/orders/new",
        "a bare topic is prefixed with the caller's own namespace"
    );
}

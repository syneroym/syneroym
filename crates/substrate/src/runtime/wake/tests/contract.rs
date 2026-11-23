//! Wake worker reply contract tests (again, warnings, error).

use std::{sync::Arc, time::Duration};

use serde_json::{Value, json};
use syneroym_core::{
    event_bus::EventBus,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
};

use super::*;

#[tokio::test]
async fn wake_reply_contract_again_sets_has_pending() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());
    *proxy.reply.lock().unwrap() = Some(Ok(Value::String(
        r#"{"result":{"again":true,"warnings":["test warning 1"]}}"#.to_string(),
    )));

    let mut spec = sample_spec();
    spec.every_secs = 60;

    registry
        .register(
            "target-svc".to_string(),
            spec.interface.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "target-svc".to_string() },
        )
        .await
        .unwrap();
    registry
        .set_app_context("target-svc".to_string(), "app-1".to_string(), "transaction".to_string())
        .await
        .unwrap();
    registry
        .set_app_context(
            "conversation-svc".to_string(),
            "app-1".to_string(),
            "conversation".to_string(),
        )
        .await
        .unwrap();

    let targets = Arc::new(StaticWakeTargets(vec![("target-svc".to_string(), spec)]));
    let mut worker = WakeWorker::new(targets, bus.clone(), proxy.clone(), registry);

    // Initial tick runs due to boot interval (last_run_at is 0)
    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // Trigger second call via event
    bus.record("conversation-svc", "conversation.message", "conv-1", None, now_ms()).unwrap();
    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 2);

    // Has_pending was set to true by the reply (again: true)
    // On next tick with NO new events and NOT due, it triggers again (3rd call)
    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn wake_reply_contract_plain_object_again_sets_has_pending() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());
    *proxy.reply.lock().unwrap() = Some(Ok(json!({"again": true, "warnings": ["plain warning"]})));

    let mut spec = sample_spec();
    spec.every_secs = 60;

    registry
        .register(
            "target-svc".to_string(),
            spec.interface.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "target-svc".to_string() },
        )
        .await
        .unwrap();

    let targets = Arc::new(StaticWakeTargets(vec![("target-svc".to_string(), spec)]));
    let mut worker = WakeWorker::new(targets, bus.clone(), proxy.clone(), registry);

    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // On next tick without events or interval due, again: true triggers call 2
    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn wake_reply_string_envelope_with_warnings() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());
    *proxy.reply.lock().unwrap() = Some(Ok(Value::String(
        r#"{"result":{"synced":0,"warnings":["warning A","warning B"]}}"#.to_string(),
    )));

    let mut spec = sample_spec();
    spec.every_secs = 60;

    registry
        .register(
            "target-svc".to_string(),
            spec.interface.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "target-svc".to_string() },
        )
        .await
        .unwrap();

    let targets = Arc::new(StaticWakeTargets(vec![("target-svc".to_string(), spec)]));
    let mut worker = WakeWorker::new(targets, bus.clone(), proxy.clone(), registry);

    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // No `again: true`, so next tick does not invoke
    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn wake_reply_string_envelope_with_error() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());
    *proxy.reply.lock().unwrap() = Some(Ok(Value::String(
        r#"{"error":{"code":-32603,"message":"internal error in guest"}}"#.to_string(),
    )));

    let mut spec = sample_spec();
    spec.every_secs = 60;

    registry
        .register(
            "target-svc".to_string(),
            spec.interface.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "target-svc".to_string() },
        )
        .await
        .unwrap();

    let targets = Arc::new(StaticWakeTargets(vec![("target-svc".to_string(), spec)]));
    let mut worker = WakeWorker::new(targets, bus.clone(), proxy.clone(), registry);

    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // Error in reply does not set `again: true`, so next tick does not invoke
    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_reply_without_again_waits_for_the_interval() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());
    *proxy.reply.lock().unwrap() =
        Some(Ok(Value::String(r#"{"result":{"again":false}}"#.to_string())));

    let mut spec = sample_spec();
    spec.every_secs = 30;

    registry
        .register(
            "target-svc".to_string(),
            spec.interface.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "target-svc".to_string() },
        )
        .await
        .unwrap();

    let targets = Arc::new(StaticWakeTargets(vec![("target-svc".to_string(), spec)]));
    let mut worker = WakeWorker::new(targets, bus, proxy.clone(), registry);

    let t0 = 100_000;
    worker.tick_at(t0).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // Next tick without again: false and before interval is due
    worker.tick_at(t0 + 5).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // Once interval expires, call 2 runs
    worker.tick_at(t0 + 30).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 2);
}

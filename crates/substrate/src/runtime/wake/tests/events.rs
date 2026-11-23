//! Wake worker event trigger tests (events, burst, reload, missing endpoint).

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use syneroym_core::{
    event_bus::{Cursor, EventBus},
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
};

use super::*;

#[tokio::test]
async fn wake_worker_triggers_on_event() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

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

    // Initial tick runs boot interval call
    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    let state = worker.states.get("target-svc").unwrap();
    assert!(!state.running.load(Ordering::SeqCst));

    // Emit event
    bus.record("conversation-svc", "conversation.message", "conv-1", None, now_ms()).unwrap();

    // Next tick should trigger on event (invocation 2)
    worker.tick().await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let invocations = proxy.invocations.lock().unwrap();
    assert_eq!(invocations.len(), 2);
}

#[tokio::test]
async fn a_burst_of_events_gives_one_call() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

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

    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // Burst of 10 events
    for i in 1..=10 {
        bus.record(
            "conversation-svc",
            "conversation.message",
            &format!("conv-{i}"),
            None,
            now_ms() + i,
        )
        .unwrap();
    }

    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Only one new invocation is dispatched for the burst
    assert_eq!(proxy.invocations.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn an_event_of_another_kind_gives_no_call() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

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

    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // Record an event of unrelated kind
    bus.record("conversation-svc", "conversation.typing", "conv-1", None, now_ms()).unwrap();

    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // No new invocation
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_reload_of_the_event_window_gives_a_call() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

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

    let targets = Arc::new(StaticWakeTargets(vec![("target-svc".to_string(), spec)]));
    let mut worker = WakeWorker::new(targets, bus, proxy.clone(), registry);

    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // Set a bus cursor from a previous run_id to simulate a node restart / reload
    let state = worker.states.get_mut("target-svc").unwrap();
    state.bus_cursor = Some(Cursor { run_id: "old-run-id".to_string(), seq: 10 });

    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Reload triggers a call
    assert_eq!(proxy.invocations.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_missing_local_endpoint_skips_dispatch_and_preserves_event_trigger() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

    let mut spec = sample_spec();
    spec.every_secs = 60;

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

    let targets = Arc::new(StaticWakeTargets(vec![("target-svc".to_string(), spec.clone())]));
    let mut worker = WakeWorker::new(targets, bus.clone(), proxy.clone(), registry.clone());

    // Record event while endpoint is NOT yet registered
    bus.record("conversation-svc", "conversation.message", "conv-1", None, now_ms()).unwrap();

    // Tick: skips because endpoint is missing, warns once
    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 0);

    let state = worker.states.get("target-svc").unwrap();
    assert!(state.warned_missing_endpoint);

    // Second tick while still missing: skips without error or duplicate call
    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 0);

    // Now register the local endpoint
    registry
        .register(
            "target-svc".to_string(),
            spec.interface.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "target-svc".to_string() },
        )
        .await
        .unwrap();

    // Next tick: endpoint is found, unconsumed event is read, wake call is
    // dispatched
    worker.tick().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    let state = worker.states.get("target-svc").unwrap();
    assert!(!state.warned_missing_endpoint);
}

#[tokio::test]
async fn a_missing_local_endpoint_preserves_has_pending_trigger() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

    let mut spec = sample_spec();
    spec.every_secs = 60;

    let targets = Arc::new(StaticWakeTargets(vec![("target-svc".to_string(), spec.clone())]));
    let mut worker = WakeWorker::new(targets, bus, proxy.clone(), registry.clone());

    // Register state and set has_pending directly (simulating prior again: true)
    let t0 = 100_000;
    worker.tick_at(t0).await; // skips because endpoint missing
    let state = worker.states.get_mut("target-svc").unwrap();
    state.has_pending.store(true, Ordering::SeqCst);
    state.last_run_at = t0; // ensure not due by interval

    // Tick at t0 + 5 while endpoint is still missing: has_pending must NOT be lost
    worker.tick_at(t0 + 5).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 0);
    let state = worker.states.get("target-svc").unwrap();
    assert!(state.has_pending.load(Ordering::SeqCst));

    // Now register the endpoint
    registry
        .register(
            "target-svc".to_string(),
            spec.interface.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "target-svc".to_string() },
        )
        .await
        .unwrap();

    // Tick at t0 + 10: has_pending triggers the call
    worker.tick_at(t0 + 10).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);
}

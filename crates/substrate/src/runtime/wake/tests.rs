//! Unit tests for WakeWorker.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use serde_json::{Value, json};
use syneroym_app_orchestration::{InterfaceName, WakeSpec};
use syneroym_core::{
    event_bus::EventBus,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::{DeployFacts, MockStorage},
};
use syneroym_rpc::{ProxyError, ProxyRequest, ServiceProxy};

use super::*;

#[derive(Debug, Default)]
struct MockProxy {
    invocations: Mutex<Vec<ProxyRequest>>,
    reply: Mutex<Option<Result<Value, String>>>,
}

#[async_trait]
impl ServiceProxy for MockProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        self.invocations.lock().unwrap().push(request);
        match self.reply.lock().unwrap().clone() {
            Some(Ok(v)) => Ok(v),
            Some(Err(e)) => Err(ProxyError::Transport(e)),
            None => Ok(Value::String(
                serde_json::to_string(&json!({
                    "result": {}
                }))
                .unwrap(),
            )),
        }
    }
}

struct StaticWakeTargets(Vec<(String, WakeSpec)>);

impl WakeTargets for StaticWakeTargets {
    fn wake_targets(&self) -> Vec<(String, WakeSpec)> {
        self.0.clone()
    }
}

fn sample_spec() -> WakeSpec {
    WakeSpec {
        interface: InterfaceName::new("test/api@0.1.0"),
        method: "drive".to_string(),
        params: Some(r#"{"test":true}"#.to_string()),
        on_events: vec!["conversation.message".to_string()],
        every_secs: 30,
        timeout_ms: 10_000,
    }
}

#[test]
fn running_guard_resets_flag_on_drop() {
    let flag = Arc::new(AtomicBool::new(true));
    {
        let _guard = RunningGuard(flag.clone());
        assert!(flag.load(Ordering::SeqCst));
    }
    assert!(!flag.load(Ordering::SeqCst));
}

#[test]
fn running_guard_resets_flag_on_panic() {
    let flag = Arc::new(AtomicBool::new(true));
    let flag_clone = flag.clone();
    let result = std::panic::catch_unwind(move || {
        let _guard = RunningGuard(flag_clone);
        panic!("simulated task panic");
    });
    assert!(result.is_err());
    assert!(!flag.load(Ordering::SeqCst));
}

#[tokio::test]
async fn registry_wake_targets_parses_valid_and_ignores_invalid_json() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();

    let spec = sample_spec();
    let valid_json = serde_json::to_string(&spec).unwrap();
    let mut facts1 = DeployFacts::new("wasm");
    facts1.wake_json = Some(valid_json.clone());
    registry.set_deploy_facts("svc-1".to_string(), facts1).await.unwrap();

    let mut facts2 = DeployFacts::new("wasm");
    facts2.wake_json = Some("invalid json".to_string());
    registry.set_deploy_facts("svc-2".to_string(), facts2).await.unwrap();

    let targets = RegistryWakeTargets::new(registry);
    let list = targets.wake_targets();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].0, "svc-1");
    assert_eq!(list[0].1, spec);
}

#[tokio::test]
async fn wake_worker_dispatches_when_due_and_registered() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

    let mut spec = sample_spec();
    spec.every_secs = 0; // immediately due

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

    worker.tick().await;

    // Allow tokio task to run
    tokio::time::sleep(Duration::from_millis(50)).await;

    let invocations = proxy.invocations.lock().unwrap();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].target_service, "target-svc");
    assert_eq!(invocations[0].method, "drive");
    assert_eq!(invocations[0].params, json!({"test": true}));
}

#[tokio::test]
async fn wake_worker_skips_when_endpoint_not_registered() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

    let mut spec = sample_spec();
    spec.every_secs = 0;

    // DO NOT register the endpoint

    let targets = Arc::new(StaticWakeTargets(vec![("unregistered-svc".to_string(), spec)]));
    let mut worker = WakeWorker::new(targets, bus, proxy.clone(), registry);

    worker.tick().await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let invocations = proxy.invocations.lock().unwrap();
    assert_eq!(invocations.len(), 0);
}

#[tokio::test]
async fn wake_worker_skips_when_already_running() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

    let mut spec = sample_spec();
    spec.every_secs = 0;

    registry
        .register(
            "target-svc".to_string(),
            spec.interface.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "target-svc".to_string() },
        )
        .await
        .unwrap();

    let targets = Arc::new(StaticWakeTargets(vec![("target-svc".to_string(), spec.clone())]));
    let mut worker = WakeWorker::new(targets, bus, proxy.clone(), registry);

    // First tick starts it
    worker.tick().await;

    // Manually set running to true to test concurrency suppression
    let state = worker.states.get_mut("target-svc").unwrap();
    state.running.store(true, Ordering::SeqCst);

    worker.tick().await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let invocations = proxy.invocations.lock().unwrap();
    assert_eq!(invocations.len(), 1);
}

#[tokio::test]
async fn wake_worker_triggers_on_event() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

    let mut spec = sample_spec();
    spec.every_secs = 60; // Not due by time

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

    // Initial tick to register target state
    worker.tick().await;

    // Emit event
    bus.record("conversation-svc", "conversation.message", "conv-1", None, 1000).unwrap();

    // Next tick should trigger on event
    worker.tick().await;

    tokio::time::sleep(Duration::from_millis(50)).await;

    let invocations = proxy.invocations.lock().unwrap();
    assert_eq!(invocations.len(), 1);
}

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
    bus.record("conversation-svc", "conversation.message", "conv-1", None, 1000).unwrap();
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

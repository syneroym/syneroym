//! Unit tests for WakeWorker.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
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

mod contract;
mod events;

#[derive(Debug, Default)]
pub(super) struct MockProxy {
    pub(super) invocations: Mutex<Vec<ProxyRequest>>,
    pub(super) reply: Mutex<Option<Result<Value, String>>>,
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

pub(super) struct StaticWakeTargets(pub(super) Vec<(String, WakeSpec)>);

impl WakeTargets for StaticWakeTargets {
    fn wake_targets(&self) -> Vec<(String, WakeSpec)> {
        self.0.clone()
    }
}

pub(super) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

pub(super) fn sample_spec() -> WakeSpec {
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
    spec.every_secs = 30; // due on first tick because last_run_at is 0

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
    spec.every_secs = 30;

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
    spec.every_secs = 30;

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
async fn the_interval_calls_with_no_event() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

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

    // Tick before interval is due
    worker.tick_at(t0 + 10).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // Tick once interval is reached
    worker.tick_at(t0 + 30).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 2);
}

#[derive(Debug)]
struct PanickingProxy;

#[async_trait]
impl ServiceProxy for PanickingProxy {
    async fn invoke(&self, _request: ProxyRequest) -> Result<Value, ProxyError> {
        panic!("simulated proxy panic inside worker");
    }
}

#[tokio::test]
async fn a_panic_in_invoke_resets_running_guard() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(PanickingProxy);

    let spec = sample_spec();

    registry
        .register(
            "target-svc".to_string(),
            spec.interface.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "target-svc".to_string() },
        )
        .await
        .unwrap();

    let targets = Arc::new(StaticWakeTargets(vec![("target-svc".to_string(), spec.clone())]));
    let mut worker = WakeWorker::new(targets, bus, proxy, registry);

    // First tick triggers call which panics in invoke
    worker.tick().await;

    // Wait for the spawned task to panic and drop its guard
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Running flag must be reset to false
    let state = worker.states.get("target-svc").unwrap();
    assert!(!state.running.load(Ordering::SeqCst));
}

#[derive(Debug)]
struct SlowProxy {
    invocations: Mutex<Vec<ProxyRequest>>,
    gate: tokio::sync::Notify,
}

#[async_trait]
impl ServiceProxy for SlowProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        self.invocations.lock().unwrap().push(request);
        self.gate.notified().await;
        Ok(Value::String(r#"{"result":{}}"#.to_string()))
    }
}

#[tokio::test]
async fn a_slow_call_is_never_overlapped() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(SlowProxy {
        invocations: Mutex::new(Vec::new()),
        gate: tokio::sync::Notify::new(),
    });

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

    // Wait until invoke is entered
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // Tick again at t0 + 40 while the call is still blocked in invoke
    worker.tick_at(t0 + 40).await;
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Never overlapped: second call was skipped because running was true
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);

    // Unblock the slow call
    proxy.gate.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Running is reset to false
    let state = worker.states.get("target-svc").unwrap();
    assert!(!state.running.load(Ordering::SeqCst));
}

struct MutableWakeTargets(Mutex<Vec<(String, WakeSpec)>>);

impl WakeTargets for MutableWakeTargets {
    fn wake_targets(&self) -> Vec<(String, WakeSpec)> {
        self.0.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn an_undeployed_service_is_not_called_again() {
    let storage = Arc::new(MockStorage::new());
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let bus = EventBus::new(registry.clone());
    let proxy = Arc::new(MockProxy::default());

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

    let targets = Arc::new(MutableWakeTargets(Mutex::new(vec![("target-svc".to_string(), spec)])));
    let mut worker = WakeWorker::new(targets.clone(), bus, proxy.clone(), registry);

    let t0 = 100_000;
    worker.tick_at(t0).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);
    assert!(worker.states.contains_key("target-svc"));

    // Undeploy service
    *targets.0.lock().unwrap() = Vec::new();

    // Next tick at interval due
    worker.tick_at(t0 + 35).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // No new invocation, and state was cleaned up
    assert_eq!(proxy.invocations.lock().unwrap().len(), 1);
    assert!(!worker.states.contains_key("target-svc"));
}

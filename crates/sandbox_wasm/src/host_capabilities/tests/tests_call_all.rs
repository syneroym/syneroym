use std::{
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use syneroym_app_orchestration::{
    AppInstanceId, AppRegistry, LogicalResolver, LogicalServiceName, ServiceId, StaticInventory,
    TopologyEpoch, TopologyKey, TopologyMode,
};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_rpc::{CallerContext, ProxyError as RpcProxyError, ProxyRequest, ServiceProxy};
use syneroym_wit_interfaces::host::syneroym::proxy::proxy::{
    self, CallOptions, CallRequest, CallTarget,
};

use crate::host_capabilities::{
    CallAll, CallAllLimits, HostState,
    tests::{test_blob_provider, test_messaging_context, test_streaming_context},
};

mod limits;

type NestedHook = Box<
    dyn Fn() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Value, RpcProxyError>> + Send + 'static>,
        > + Send
        + Sync,
>;

#[derive(Default)]
pub(super) struct ScriptedProxy {
    pub(super) invoke_count: AtomicUsize,
    pub(super) in_flight: AtomicUsize,
    pub(super) max_in_flight: AtomicUsize,
    pub(super) dropped: AtomicBool,
    pub(super) last_request: Mutex<Option<ProxyRequest>>,
    pub(super) nested_hook: Mutex<Option<Arc<NestedHook>>>,
}

impl std::fmt::Debug for ScriptedProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedProxy")
            .field("invoke_count", &self.invoke_count.load(Ordering::SeqCst))
            .field("in_flight", &self.in_flight.load(Ordering::SeqCst))
            .field("max_in_flight", &self.max_in_flight.load(Ordering::SeqCst))
            .field("dropped", &self.dropped.load(Ordering::SeqCst))
            .finish()
    }
}

struct InFlightGuard<'a> {
    proxy: &'a ScriptedProxy,
    completed: bool,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        self.proxy.in_flight.fetch_sub(1, Ordering::SeqCst);
        if !self.completed {
            self.proxy.dropped.store(true, Ordering::SeqCst);
        }
    }
}

#[async_trait::async_trait]
impl ServiceProxy for ScriptedProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, RpcProxyError> {
        self.invoke_count.fetch_add(1, Ordering::SeqCst);
        let curr = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(curr, Ordering::SeqCst);
        *self.last_request.lock().unwrap() = Some(request.clone());

        let mut guard = InFlightGuard { proxy: self, completed: false };

        let hook = self.nested_hook.lock().unwrap().clone();
        if let Some(hook_fn) = hook {
            let res = hook_fn().await;
            guard.completed = true;
            return res;
        }

        let res = match request.method.as_str() {
            "echo" => Ok(request.params),
            "sleep" => {
                let ms = request.params.get("ms").and_then(Value::as_u64).unwrap_or(0);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                if let Some(tag) = request.params.get("tag").and_then(Value::as_str) {
                    Ok(Value::String(tag.to_string()))
                } else {
                    Ok(Value::String("slept".to_string()))
                }
            }
            "fail" => Err(RpcProxyError::Callee {
                code: -32000,
                message: "scripted failure".to_string(),
                data: None,
            }),
            "fail_pool" => Err(RpcProxyError::Callee {
                code: -32603,
                message: "pool exhausted".to_string(),
                data: None,
            }),
            "panic" => panic!("scripted host panic"),
            _ => Ok(Value::Null),
        };
        guard.completed = true;
        res
    }

    async fn enqueue(&self, _call: syneroym_rpc::QueuedCall) -> Result<(), RpcProxyError> {
        Ok(())
    }

    async fn saga_begin(&self, _req: syneroym_rpc::SagaBegin) -> Result<String, RpcProxyError> {
        Ok("saga-1".to_string())
    }

    async fn saga_step(&self, _req: syneroym_rpc::SagaStepRequest) -> Result<Value, RpcProxyError> {
        Ok(Value::Null)
    }

    async fn saga_commit(&self, _service_id: &str, _saga_id: &str) -> Result<(), RpcProxyError> {
        Ok(())
    }

    async fn saga_compensate(
        &self,
        _service_id: &str,
        _saga_id: &str,
    ) -> Result<(), RpcProxyError> {
        Ok(())
    }

    async fn saga_status(
        &self,
        _service_id: &str,
        _saga_id: &str,
    ) -> Result<syneroym_rpc::SagaInfo, RpcProxyError> {
        Ok(syneroym_rpc::SagaInfo {
            saga_id: "saga-1".to_string(),
            name: "test".to_string(),
            state: syneroym_rpc::SagaState::Open,
            steps: 0,
            compensated_steps: 0,
            created_at: 0,
            deadline_at: 0,
            last_error: None,
        })
    }
}

fn dependency_topology_entry(members: Vec<&str>) -> syneroym_app_orchestration::TopologyEntry {
    syneroym_app_orchestration::TopologyEntry {
        mode: if members.len() > 1 { TopologyMode::Redundant } else { TopologyMode::Singleton },
        members: members.into_iter().map(ServiceId::new).collect(),
        sharding_strategy: None,
        epoch: TopologyEpoch::default(),
        cache_ttl: Duration::from_secs(60),
        not_after: None,
    }
}

pub(super) fn test_host(
    component_id: &str,
    proxy: &Arc<ScriptedProxy>,
    read_only: bool,
    temp_dir: &Path,
    app_instance_id: Option<String>,
    resolver: Option<Arc<LogicalResolver>>,
) -> HostState {
    let res = resolver
        .unwrap_or_else(|| Arc::new(LogicalResolver::new(Arc::new(StaticInventory::new()))));
    HostState::new(
        component_id.to_string(),
        None,
        Arc::new(KeyStore::new()),
        Arc::new(SqliteStorageProvider::new(temp_dir, false).unwrap()),
        test_blob_provider(),
        CallerContext::service_system(component_id),
        0,
        test_messaging_context(),
        test_streaming_context(),
        Arc::downgrade(proxy) as Weak<dyn ServiceProxy>,
        None,
        read_only,
        syneroym_rpc::empty_row_authorizer(),
        app_instance_id,
        res,
    )
}

pub(super) fn req(
    target: &str,
    method: &str,
    params: Value,
    timeout_ms: Option<u32>,
) -> CallRequest {
    CallRequest {
        target: CallTarget::Service(target.to_string()),
        interface: "scripted".to_string(),
        method: method.to_string(),
        params: params.to_string(),
        options: Some(CallOptions {
            protocol: None,
            idempotent: false,
            timeout_ms,
            routing_key: None,
            idempotency_key: None,
        }),
    }
}

#[tokio::test]
async fn an_empty_batch_returns_an_empty_list_and_runs_nothing() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);

    let res = proxy::Host::call_all(&mut host, vec![]).await.unwrap();
    assert!(res.is_empty());
    assert_eq!(proxy.invoke_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_batch_over_the_maximum_is_refused_before_any_call() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    let limits = CallAllLimits { max_calls: 2, ..Default::default() };
    host = host.with_call_all(CallAll::new(limits, 4));

    let calls = vec![
        req("svc", "echo", json!("1"), None),
        req("svc", "echo", json!("2"), None),
        req("svc", "echo", json!("3"), None),
    ];
    let err = proxy::Host::call_all(&mut host, calls).await.unwrap_err();
    let proxy::ProxyError::UnsupportedTarget(msg) = err else {
        panic!("expected UnsupportedTarget, got {err:?}");
    };
    assert!(
        msg.contains("call-all takes at most 2 calls, got 3"),
        "unexpected error message: {msg}"
    );
    assert_eq!(proxy.invoke_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn results_come_back_in_input_order_whatever_the_finish_order() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);

    let calls = vec![
        req("svc", "sleep", json!({"ms": 50, "tag": "call-1-50ms"}), None),
        req("svc", "sleep", json!({"ms": 20, "tag": "call-2-20ms"}), None),
        req("svc", "sleep", json!({"ms": 5, "tag": "call-3-5ms"}), None),
    ];
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 3);
    assert_eq!(res[0].as_deref().unwrap(), "call-1-50ms");
    assert_eq!(res[1].as_deref().unwrap(), "call-2-20ms");
    assert_eq!(res[2].as_deref().unwrap(), "call-3-5ms");
}

#[tokio::test]
async fn a_failed_call_does_not_change_the_other_entries() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);

    let mut bad_protocol_call = req("svc", "echo", json!("bad proto"), None);
    bad_protocol_call.options.as_mut().unwrap().protocol = Some("unsupported/v9".to_string());

    let bad_dep_call = CallRequest {
        target: CallTarget::Dependency("unbound-dep".to_string()),
        interface: "scripted".to_string(),
        method: "echo".to_string(),
        params: json!("unbound").to_string(),
        options: None,
    };

    let bad_params_call = CallRequest {
        target: CallTarget::Service("svc".to_string()),
        interface: "scripted".to_string(),
        method: "echo".to_string(),
        params: "not json {".to_string(),
        options: None,
    };

    let calls = vec![
        req("svc", "echo", json!("ok"), None),
        req("svc", "fail", json!({}), None),
        bad_params_call,
        bad_protocol_call,
        bad_dep_call,
    ];
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 5);
    assert_eq!(res[0].as_deref().unwrap(), "ok");
    assert!(matches!(res[1], Err(proxy::ProxyError::Callee(_))));
    assert!(matches!(res[2], Err(proxy::ProxyError::Internal(_))));
    assert!(matches!(res[3], Err(proxy::ProxyError::UnsupportedProtocol(_))));
    assert!(matches!(res[4], Err(proxy::ProxyError::DependencyNotBound(_))));
}

#[tokio::test]
async fn at_most_the_concurrency_limit_run_at_once_and_the_rest_wait() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    let limits = CallAllLimits { max_calls: 16, max_concurrency: 4, ..Default::default() };
    host = host.with_call_all(CallAll::new(limits, 4));

    let calls = (0..16).map(|_| req("svc", "sleep", json!({"ms": 30}), None)).collect();
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 16);
    assert_eq!(proxy.max_in_flight.load(Ordering::SeqCst), 4);
    assert_eq!(proxy.invoke_count.load(Ordering::SeqCst), 16);
}

#[tokio::test]
async fn a_call_gets_its_own_time_and_the_host_caps_a_larger_request() {
    let limits = CallAllLimits {
        default_call_timeout: Duration::from_millis(100),
        max_call_timeout: Duration::from_millis(300),
        total_timeout: Duration::from_millis(1000),
        ..Default::default()
    };
    assert_eq!(limits.call_timeout(Some(60_000)), Duration::from_millis(300));
    assert_eq!(limits.call_timeout(None), Duration::from_millis(100));
    assert_eq!(limits.call_timeout(Some(200)), Duration::from_millis(200));

    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    host = host.with_call_all(CallAll::new(limits, 4));

    let calls = vec![req("svc", "echo", json!("ok"), Some(60_000))];
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].as_deref().unwrap(), "ok");

    let last_req = proxy.last_request.lock().unwrap().clone().expect("must have invoked proxy");
    assert_eq!(last_req.timeout, Some(Duration::from_millis(300)));
}

#[tokio::test]
async fn a_slow_call_times_out_and_the_others_keep_their_results() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    let limits = CallAllLimits {
        default_call_timeout: Duration::from_millis(50),
        max_call_timeout: Duration::from_millis(500),
        total_timeout: Duration::from_millis(1000),
        ..Default::default()
    };
    host = host.with_call_all(CallAll::new(limits, 4));

    let calls = vec![
        req("svc", "sleep", json!({"ms": 10}), Some(200)),
        req("svc", "sleep", json!({"ms": 500}), Some(50)),
        req("svc", "sleep", json!({"ms": 10}), Some(200)),
    ];
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 3);
    assert_eq!(res[0].as_deref().unwrap(), "slept");
    assert!(matches!(res[1], Err(proxy::ProxyError::TimedOut)));
    assert_eq!(res[2].as_deref().unwrap(), "slept");
}

#[tokio::test]
async fn the_total_time_ends_the_batch_and_cancels_running_calls() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    let limits = CallAllLimits {
        max_concurrency: 1,
        default_call_timeout: Duration::from_millis(5000),
        max_call_timeout: Duration::from_millis(5000),
        total_timeout: Duration::from_millis(1100),
        ..Default::default()
    };
    host = host.with_call_all(CallAll::new(limits, 4));

    let calls = vec![
        req("svc", "sleep", json!({"ms": 50, "tag": "fast"}), None),
        req("svc", "sleep", json!({"ms": 5000}), None),
        req("svc", "sleep", json!({"ms": 5000}), None),
    ];
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 3);
    assert_eq!(res[0].as_deref().unwrap(), "fast");
    assert!(matches!(res[1], Err(proxy::ProxyError::TimedOut)));
    assert!(matches!(res[2], Err(proxy::ProxyError::TimedOut)));

    for _ in 0..50 {
        if proxy.dropped.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(proxy.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_finished_call_is_not_reported_as_timed_out_behind_a_slow_first_call() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);
    let limits = CallAllLimits {
        max_concurrency: 2,
        default_call_timeout: Duration::from_millis(50),
        max_call_timeout: Duration::from_millis(300),
        total_timeout: Duration::from_millis(2000),
        ..Default::default()
    };
    host = host.with_call_all(CallAll::new(limits, 4));

    let calls = vec![
        req("svc", "sleep", json!({"ms": 500}), Some(50)),
        req("svc", "sleep", json!({"ms": 10}), Some(200)),
    ];
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 2);
    assert!(matches!(res[0], Err(proxy::ProxyError::TimedOut)));
    assert_eq!(res[1].as_deref().unwrap(), "slept");
}

#[tokio::test]
async fn the_same_target_twice_is_two_calls() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);

    let calls = vec![req("svc", "echo", json!("a"), None), req("svc", "echo", json!("b"), None)];
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 2);
    assert_eq!(res[0].as_deref().unwrap(), "a");
    assert_eq!(res[1].as_deref().unwrap(), "b");
    assert_eq!(proxy.invoke_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn each_call_is_built_like_a_single_call() {
    let registry = Arc::new(StaticInventory::new());
    registry.register(
        TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
        dependency_topology_entry(vec!["did:key:zBackend"]),
    );
    let resolver = Arc::new(LogicalResolver::new(registry));
    let temp_dir = tempfile::tempdir().unwrap();

    let cases = vec![
        (CallTarget::Service("did:key:zOther".to_string()), None, None),
        (CallTarget::Service("did:key:zCurrent".to_string()), None, None),
        (
            CallTarget::Dependency("backend".to_string()),
            Some("route-1".to_string()),
            Some("idem-1".to_string()),
        ),
    ];

    for (target, routing_key, idempotency_key) in cases {
        let proxy1 = Arc::new(ScriptedProxy::default());
        let mut host1 = test_host(
            "did:key:zCurrent",
            &proxy1,
            false,
            temp_dir.path(),
            Some("app-1".to_string()),
            Some(resolver.clone()),
        );

        let options1 = Some(CallOptions {
            protocol: None,
            idempotent: true,
            timeout_ms: Some(1500),
            routing_key: routing_key.clone(),
            idempotency_key: idempotency_key.clone(),
        });

        proxy::Host::call(
            &mut host1,
            target.clone(),
            "scripted".to_string(),
            "echo".to_string(),
            json!({"test": 1}).to_string(),
            options1,
        )
        .await
        .unwrap();

        let req1 = proxy1.last_request.lock().unwrap().take().unwrap();

        let proxy2 = Arc::new(ScriptedProxy::default());
        let mut host2 = test_host(
            "did:key:zCurrent",
            &proxy2,
            false,
            temp_dir.path(),
            Some("app-1".to_string()),
            Some(resolver.clone()),
        );

        let call_all_req = CallRequest {
            target,
            interface: "scripted".to_string(),
            method: "echo".to_string(),
            params: json!({"test": 1}).to_string(),
            options: Some(CallOptions {
                protocol: None,
                idempotent: true,
                timeout_ms: Some(1500),
                routing_key,
                idempotency_key,
            }),
        };

        proxy::Host::call_all(&mut host2, vec![call_all_req]).await.unwrap();

        let req2 = proxy2.last_request.lock().unwrap().take().unwrap();

        assert_eq!(req1.target_service, req2.target_service);
        assert_eq!(req1.interface, req2.interface);
        assert_eq!(req1.method, req2.method);
        assert_eq!(req1.params, req2.params);
        assert_eq!(req1.caller.caller_did, req2.caller.caller_did);
        assert_eq!(req1.caller.auth, req2.caller.auth);
        assert_eq!(req1.origin, req2.origin);
        assert_eq!(req1.protocol, req2.protocol);
        assert_eq!(req1.idempotent, req2.idempotent);
        assert_eq!(req1.idempotency_key, req2.idempotency_key);
    }
}

#[tokio::test]
async fn host_call_panic_is_recorded_as_internal_error_and_does_not_abort_batch() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);

    let calls = vec![
        req("svc", "echo", json!("1"), None),
        req("svc", "panic", json!({}), None),
        req("svc", "echo", json!("3"), None),
    ];
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 3);
    assert_eq!(res[0].as_deref().unwrap(), "1");
    let Err(proxy::ProxyError::Internal(msg)) = &res[1] else {
        panic!("expected Internal for panic, got {:?}", res[1]);
    };
    assert!(msg.contains("call failed in the host"));
    assert_eq!(res[2].as_deref().unwrap(), "3");
}

#[tokio::test]
async fn a_callee_that_cannot_get_a_pool_slot_is_one_failed_entry() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);

    let calls =
        vec![req("svc", "fail_pool", json!({}), None), req("svc", "echo", json!("ok"), None)];
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 2);
    let Err(proxy::ProxyError::Callee(err)) = &res[0] else {
        panic!("expected Callee error, got {:?}", res[0]);
    };
    assert_eq!(err.code, -32603);
    assert_eq!(res[1].as_deref().unwrap(), "ok");
}

#[tokio::test]
async fn every_exit_path_preserves_input_order_and_cardinality() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let limits = CallAllLimits {
        default_call_timeout: Duration::from_millis(50),
        max_call_timeout: Duration::from_millis(300),
        total_timeout: Duration::from_millis(1000),
        ..Default::default()
    };
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None)
        .with_call_all(CallAll::new(limits, 4));

    let bad_params_call = CallRequest {
        target: CallTarget::Service("svc".to_string()),
        interface: "scripted".to_string(),
        method: "echo".to_string(),
        params: "invalid json".to_string(),
        options: None,
    };

    let calls = vec![
        req("svc", "echo", json!("first"), None),
        req("svc", "fail", json!({}), None),
        bad_params_call,
        req("svc", "sleep", json!({"ms": 600}), Some(50)),
        req("svc", "echo", json!("last"), None),
    ];
    let res = proxy::Host::call_all(&mut host, calls).await.unwrap();
    assert_eq!(res.len(), 5);
    assert_eq!(res[0].as_deref().unwrap(), "first");
    assert!(matches!(res[1], Err(proxy::ProxyError::Callee(_))));
    assert!(matches!(res[2], Err(proxy::ProxyError::Internal(_))));
    assert!(matches!(res[3], Err(proxy::ProxyError::TimedOut)));
    assert_eq!(res[4].as_deref().unwrap(), "last");
}

#[tokio::test]
async fn never_armed_host_state_allows_batch() {
    let proxy = Arc::new(ScriptedProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = test_host("svc", &proxy, false, temp_dir.path(), None, None);

    assert_eq!(host.epoch.allowance(), CallAllLimits::default().total_timeout);
    let res = proxy::Host::call_all(&mut host, vec![req("svc", "echo", json!("ready"), None)])
        .await
        .unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].as_deref().unwrap(), "ready");
}

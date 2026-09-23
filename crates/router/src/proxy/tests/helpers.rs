pub(crate) use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) use dashmap::DashMap;
pub(crate) use iroh::{EndpointAddr, SecretKey};
pub(crate) use serde_json::Value;
pub(crate) use syneroym_async_queue::QueueConfig;
pub(crate) use syneroym_core::{
    config::RetryPolicy,
    dht_registry::{MasterAnchorPayload, RegistryClient},
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
    util,
};
pub(crate) use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
pub(crate) use syneroym_rpc::{
    AuthLevel, CallOrigin, CallerContext, CallerProof, JsonRpcRequest, NativeDispatchRegistry,
    NativeInvocation, NativeResponse, NativeService, ProxyError, ProxyProtocol,
    ProxyQueueInspector, ProxyRequest, QueuedCall, QueuedTarget, RpcError, RpcResult,
    SERVICE_NOT_FOUND_RPC_CODE, SagaBegin, SagaState as RpcSagaState, SagaStepRequest,
    ServiceProxy, SessionContext,
};
pub(crate) use tokio_util::sync::CancellationToken;

pub(crate) use super::super::{
    ProxyRouter, RemoteHop, merge_forward_result,
    proxy_outbox::{self, Disposition, ProxyOutbox},
    step_call_budget_ms, target_produced,
};
pub(crate) use crate::{
    HandshakeVerifier, MasterAnchorResolver, preamble::RoutePreamble, saga::SagaStore,
};

pub(crate) fn test_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext::default(),
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

pub(crate) fn base_request(target_service: &str, interface: &str) -> ProxyRequest {
    ProxyRequest {
        target_service: target_service.to_string(),
        interface: interface.to_string(),
        method: "get".to_string(),
        params: Value::Null,
        caller: test_caller("did:key:zTestCaller"),
        origin: CallOrigin::Native { service_id: None },
        protocol: ProxyProtocol::JsonRpcV1,
        idempotency_key: None,
        idempotent: false,
        timeout: Some(Duration::from_secs(1)),
    }
}

pub(crate) fn synthetic_addr() -> EndpointAddr {
    let node_id = SecretKey::generate(&mut rand::rng()).public();
    EndpointAddr::new(node_id)
}

pub(crate) fn empty_registry() -> EndpointRegistry {
    EndpointRegistry::new_mock(Arc::new(MockStorage::new()))
}

pub(crate) fn empty_registry_client() -> Arc<RegistryClient> {
    Arc::new(RegistryClient::new(false, None))
}

pub(crate) fn test_router(hop: Arc<dyn RemoteHop>, registry: EndpointRegistry) -> ProxyRouter {
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop,
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 1,
            backoff_multiplier: 2.0,
            max_backoff_ms: 5,
        },
    )
}

pub(crate) struct GuardedNode {
    pub(crate) router: ProxyRouter,
    pub(crate) service: Arc<RecordingNativeService>,
    pub(crate) _native_dispatch: NativeDispatchRegistry,
    pub(crate) _dir: tempfile::TempDir,
}

pub(crate) async fn guarded_node(with_store: bool) -> GuardedNode {
    use syneroym_async_queue::DedupConfig;
    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_data_keystore::KeyStore;

    let registry = empty_registry();
    registry
        .register(
            "svc-a".to_string(),
            "greeter".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-a".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-a".to_string(), service.clone() as Arc<dyn NativeService>);

    let dir = tempfile::tempdir().unwrap();
    let service_dir = dir.path().join("services").join("svc-a");
    std::fs::create_dir_all(&service_dir).unwrap();
    std::fs::write(service_dir.join("state.db"), b"").unwrap();

    let guard_registry = registry.clone();
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );
    let router = if with_store {
        let provider = Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
        router.with_dedup_guard(Arc::new(crate::CallDedupGuard::new(
            provider,
            Arc::new(KeyStore::new()),
            guard_registry,
            DedupConfig {
                ttl_ms: 600_000,
                claim_window_ms: 60_000,
                max_rows: 100,
                max_result_bytes: 64 * 1024,
            },
        )))
    } else {
        router
    };
    GuardedNode { router, service, _native_dispatch: native_dispatch, _dir: dir }
}

pub(crate) struct OutboxNode {
    pub(crate) router: Arc<ProxyRouter>,
    pub(crate) registry: EndpointRegistry,
    pub(crate) resolver: Arc<syneroym_app_orchestration::LogicalResolver>,
    pub(crate) target: Arc<RecordingNativeService>,
    pub(crate) outbox: Arc<ProxyOutbox>,
    pub(crate) provider: Arc<syneroym_data_db::SqliteStorageProvider>,
    pub(crate) _native_dispatch: NativeDispatchRegistry,
    pub(crate) dir: tempfile::TempDir,
}

pub(crate) const CALLER: &str = "did:key:zCaller";

/// `target_reachable` decides whether the immediate attempt succeeds:
/// a registered native endpoint answers, while a WASM endpoint with no
/// engine behind it fails with the retryable "sandbox engine
/// unavailable" -- a shutdown-window state, which is exactly the shape
/// that must queue rather than fail the caller.
#[allow(clippy::too_many_lines)]
pub(crate) async fn outbox_node(target_reachable: bool, max_attempts: u8) -> OutboxNode {
    use syneroym_app_orchestration::{
        AppInstanceId, LogicalResolver, LogicalServiceName, ServiceId, StaticInventory,
        TopologyEntry, TopologyEpoch, TopologyKey, TopologyMode,
    };
    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_data_keystore::KeyStore;

    let registry = empty_registry();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let target = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("did:key:zTarget".to_string(), target.clone() as Arc<dyn NativeService>);
    registry
        .register(
            "did:key:zTarget".to_string(),
            "greeter".to_string(),
            if target_reachable {
                SubstrateEndpoint::NativeHostChannel { service_id: "did:key:zTarget".to_string() }
            } else {
                SubstrateEndpoint::WasmChannel { service_id: "did:key:zTarget".to_string() }
            },
        )
        .await
        .unwrap();

    // The calling service is itself deployed on this node: the worker
    // only drains services the endpoint registry still knows, so
    // without this every queued item would look orphaned.
    registry
        .register(
            CALLER.to_string(),
            "caller-iface".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: CALLER.to_string() },
        )
        .await
        .unwrap();

    // The calling service holds an unexpired instance certificate, so
    // `enqueue`'s certificate refusal does not fire.
    let node_identity = Arc::new(Identity::generate().unwrap());
    let owner = "did:key:zOwner".to_string();
    registry.set_owner(CALLER.to_string(), owner.clone()).await.unwrap();
    let instance = node_identity.derive_service_identity(&owner, CALLER);
    let cert = DelegationCertificate::issue(
        &Identity::generate().unwrap(),
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    registry.set_instance_cert(CALLER.to_string(), cert).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    for service in [CALLER, "did:key:zTarget"] {
        let service_dir = dir.path().join("services").join(service);
        std::fs::create_dir_all(&service_dir).unwrap();
        std::fs::write(service_dir.join("state.db"), b"").unwrap();
    }

    let inventory = Arc::new(StaticInventory::new());
    let resolver = Arc::new(LogicalResolver::new(inventory));
    resolver.register(
        TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
        TopologyEntry {
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new("did:key:zTarget")],
            sharding_strategy: None,
            epoch: TopologyEpoch::default(),
            // No caching, so a test that re-registers a binding sees
            // the change on the very next resolution.
            cache_ttl: Duration::ZERO,
            not_after: None,
        },
    );

    let provider = Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
    let guard_provider = provider.clone();
    let config = QueueConfig {
        retry: RetryPolicy {
            max_attempts,
            initial_backoff_ms: 1,
            backoff_multiplier: 2.0,
            max_backoff_ms: 4,
        },
        visibility_timeout_ms: 0,
        dlq_max_rows: 100,
        max_pending_rows: syneroym_async_queue::DEFAULT_MAX_PENDING_ROWS,
    };
    let outbox = Arc::new(ProxyOutbox::new(
        provider.clone(),
        Arc::new(KeyStore::new()),
        resolver.clone(),
        config,
    ));
    let router = Arc::new(
        ProxyRouter::new(
            registry.clone(),
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            Arc::new(MockHop::default()),
            node_identity,
            RetryPolicy { max_attempts: 1, ..RetryPolicy::default() },
        )
        .with_dedup_guard(Arc::new(crate::CallDedupGuard::new(
            guard_provider,
            Arc::new(KeyStore::new()),
            registry.clone(),
            syneroym_async_queue::DedupConfig {
                ttl_ms: 600_000,
                claim_window_ms: 60_000,
                max_rows: 100,
                max_result_bytes: 64 * 1024,
            },
        )))
        .with_outbox(outbox.clone()),
    );
    OutboxNode {
        router,
        registry,
        resolver,
        target,
        outbox,
        provider,
        _native_dispatch: native_dispatch,
        dir,
    }
}

pub(crate) fn queued_call(target: QueuedTarget, key: &str) -> QueuedCall {
    QueuedCall {
        app_instance_id: Some("app-1".to_string()),
        caller_service_id: CALLER.to_string(),
        target,
        routing_key: None,
        interface: "greeter".to_string(),
        method: "greet".to_string(),
        params: Value::Null,
        idempotency_key: key.to_string(),
        protocol: None,
        timeout_ms: Some(500),
    }
}

impl OutboxNode {
    pub(crate) async fn queued(&self) -> Vec<syneroym_async_queue::QueueItem> {
        self.outbox.queue_for(CALLER).await.unwrap().all().unwrap()
    }

    pub(crate) async fn dead_letters(&self) -> Vec<syneroym_async_queue::DeadLetter> {
        self.outbox.queue_for(CALLER).await.unwrap().dead_letters().unwrap()
    }

    pub(crate) fn queue_file_exists(&self) -> bool {
        self.dir.path().join("services").join(CALLER).join("async.db").exists()
    }
}

/// Polls until `check` holds or the budget runs out.
pub(crate) async fn wait_for<F: FnMut() -> bool>(budget: Duration, mut check: F) -> bool {
    let deadline = std::time::Instant::now() + budget;
    while std::time::Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

pub(crate) fn timing_out_guest_request(key: Option<&str>) -> ProxyRequest {
    let mut req = base_request("did:key:zTarget", "greeter");
    req.origin = CallOrigin::Guest { service_id: CALLER.to_string() };
    req.caller = CallerContext::service_system(CALLER);
    req.idempotency_key = key.map(str::to_string);
    req.idempotent = true;
    req.timeout = Some(Duration::from_millis(100));
    req
}

pub(crate) fn failing_guest_request(key: Option<&str>) -> ProxyRequest {
    let mut req = base_request("did:key:zTarget", "greeter");
    req.origin = CallOrigin::Guest { service_id: CALLER.to_string() };
    req.caller = CallerContext::service_system(CALLER);
    req.idempotency_key = key.map(str::to_string);
    req
}

pub(crate) fn keyed_request(key: &str) -> ProxyRequest {
    let mut req = base_request("svc-a", "greeter");
    req.caller = CallerContext::service_system("svc-caller");
    req.idempotency_key = Some(key.to_string());
    req
}

#[derive(Debug, Default)]
pub(crate) struct RecordingNativeService {
    pub(crate) invoked: AtomicUsize,
    pub(crate) last_caller_did: Mutex<Option<String>>,
    pub(crate) fail_with: std::sync::atomic::AtomicBool,
    pub(crate) hold: Mutex<Option<Arc<tokio::sync::Notify>>>,
    pub(crate) answer_with: Mutex<Option<ProxyError>>,
    pub(crate) last_invocation: Mutex<Option<(String, String, Value)>>,
}

#[async_trait::async_trait]
impl NativeService for RecordingNativeService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        self.invoked.fetch_add(1, Ordering::SeqCst);
        *self.last_caller_did.lock().unwrap() = Some(invocation.caller.caller_did.clone());
        *self.last_invocation.lock().unwrap() = Some((
            invocation.interface.clone(),
            invocation.method.clone(),
            invocation.params.clone(),
        ));
        let hold = self.hold.lock().unwrap().clone();
        if let Some(hold) = hold {
            hold.notified().await;
        }
        if let Some(answer) = self.answer_with.lock().unwrap().as_ref() {
            let (code, message) = match answer {
                ProxyError::Callee { code, message, .. } => (*code, message.clone()),
                other => (-32603, other.to_string()),
            };
            return Err(RpcError::Custom(code, message, None));
        }
        if self.fail_with.load(Ordering::SeqCst) {
            return Err(RpcError::InternalError("the target says no".to_string()));
        }
        Ok(NativeResponse { payload: Value::String("ok".to_string()) })
    }
}

#[derive(Debug, Clone)]
pub(crate) enum MockOutcome {
    Success(Value),
    Transport,
    Callee { code: i32, message: String },
}

#[derive(Debug, Default)]
pub(crate) struct MockHop {
    pub(crate) calls: AtomicUsize,
    pub(crate) last_preamble: Mutex<Option<RoutePreamble>>,
    pub(crate) outcomes: Mutex<std::collections::VecDeque<MockOutcome>>,
}

impl MockHop {
    pub(crate) fn with_outcomes(outcomes: Vec<MockOutcome>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            last_preamble: Mutex::new(None),
            outcomes: Mutex::new(outcomes.into()),
        }
    }

    pub(crate) fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl RemoteHop for MockHop {
    async fn call(
        &self,
        _addr: &EndpointAddr,
        preamble: &RoutePreamble,
        _request: &JsonRpcRequest,
        _timeout: Duration,
    ) -> Result<Value, ProxyError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_preamble.lock().unwrap() = Some(preamble.clone());
        match self.outcomes.lock().unwrap().pop_front() {
            Some(MockOutcome::Success(v)) => Ok(v),
            Some(MockOutcome::Transport) | None => {
                Err(ProxyError::Transport("mock transport failure".to_string()))
            }
            Some(MockOutcome::Callee { code, message }) => {
                Err(ProxyError::Callee { code, message, data: None })
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct EmptyAnchorResolver;
#[async_trait::async_trait]
impl MasterAnchorResolver for EmptyAnchorResolver {
    async fn resolve_master_anchor(
        &self,
        _master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        Ok(MasterAnchorPayload::default())
    }
}

pub(crate) struct SagaNode {
    pub(crate) router: Arc<ProxyRouter>,
    pub(crate) registry: EndpointRegistry,
    pub(crate) target: Arc<RecordingNativeService>,
    pub(crate) sagas: Arc<SagaStore>,
    pub(crate) dedup_guard: Arc<crate::CallDedupGuard>,
    pub(crate) _native_dispatch: NativeDispatchRegistry,
    pub(crate) _dir: tempfile::TempDir,
}

pub(crate) const SAGA_CALLER: &str = "did:key:zSagaCaller";
pub(crate) const SAGA_TARGET: &str = "did:key:zSagaTarget";

pub(crate) fn saga_config(dispatch_epoch_timeout_secs: u64) -> syneroym_async_queue::SagaConfig {
    syneroym_async_queue::SagaConfig::from(&syneroym_core::config::AppSandboxRole {
        dispatch_epoch_timeout_secs,
        ..syneroym_core::config::AppSandboxRole::default()
    })
}

pub(crate) async fn saga_node(dispatch_epoch_timeout_secs: u64) -> SagaNode {
    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_data_keystore::KeyStore;

    let registry = empty_registry();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let target = Arc::new(RecordingNativeService::default());
    native_dispatch.insert(SAGA_TARGET.to_string(), target.clone() as Arc<dyn NativeService>);
    registry
        .register(
            SAGA_TARGET.to_string(),
            "saga-participant".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: SAGA_TARGET.to_string() },
        )
        .await
        .unwrap();
    registry
        .register(
            SAGA_CALLER.to_string(),
            "saga-driver".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: SAGA_CALLER.to_string() },
        )
        .await
        .unwrap();

    let node_identity = Arc::new(Identity::generate().unwrap());
    let owner = "did:key:zSagaOwner".to_string();
    registry.set_owner(SAGA_CALLER.to_string(), owner.clone()).await.unwrap();
    let instance = node_identity.derive_service_identity(&owner, SAGA_CALLER);
    let cert = DelegationCertificate::issue(
        &Identity::generate().unwrap(),
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    registry.set_instance_cert(SAGA_CALLER.to_string(), cert).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    for service in [SAGA_CALLER, SAGA_TARGET] {
        let service_dir = dir.path().join("services").join(service);
        std::fs::create_dir_all(&service_dir).unwrap();
        std::fs::write(service_dir.join("state.db"), b"").unwrap();
    }

    let provider = Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
    let resolver = syneroym_app_orchestration::empty_resolver();
    let sagas = Arc::new(SagaStore::new(
        provider.clone(),
        Arc::new(KeyStore::new()),
        resolver,
        saga_config(dispatch_epoch_timeout_secs),
    ));
    // Every undo the walk sends is keyed, so the receiver needs a real
    // fence behind it -- with no dedup guard configured,
    // `invoke_local_guarded` refuses any keyed call outright.
    let dedup_guard = Arc::new(crate::CallDedupGuard::new(
        provider,
        Arc::new(KeyStore::new()),
        registry.clone(),
        syneroym_async_queue::DedupConfig {
            ttl_ms: 600_000,
            claim_window_ms: 60_000,
            max_rows: 100,
            max_result_bytes: 64 * 1024,
        },
    ));
    let router = Arc::new(
        ProxyRouter::new(
            registry.clone(),
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            Arc::new(MockHop::default()),
            node_identity,
            RetryPolicy { max_attempts: 1, ..RetryPolicy::default() },
        )
        .with_dedup_guard(dedup_guard.clone())
        .with_sagas(sagas.clone()),
    );
    SagaNode {
        router,
        registry,
        target,
        sagas,
        dedup_guard,
        _native_dispatch: native_dispatch,
        _dir: dir,
    }
}

pub(crate) fn saga_step_request(saga_id: &str, target: &str) -> SagaStepRequest {
    SagaStepRequest {
        caller_service_id: SAGA_CALLER.to_string(),
        app_instance_id: None,
        saga_id: saga_id.to_string(),
        target: QueuedTarget::Service(target.to_string()),
        routing_key: None,
        interface: "saga-participant".to_string(),
        method: "reserve".to_string(),
        params: Value::Null,
        idempotency_key: None,
        protocol: None,
        timeout_ms: None,
    }
}

pub(crate) async fn begun_saga(node: &SagaNode) -> String {
    node.router
        .saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf".to_string(),
            deadline_secs: None,
        })
        .await
        .unwrap()
}

pub(crate) async fn add_step(node: &SagaNode, saga_id: &str, item: &str) {
    let mut req = saga_step_request(saga_id, SAGA_TARGET);
    req.params = serde_json::json!({"item": item});
    node.router.saga_step(req).await.unwrap();
}

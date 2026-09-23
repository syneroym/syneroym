pub(super) use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub(super) use dashmap::DashMap;
pub(super) use iroh::{EndpointAddr, SecretKey};
pub(super) use serde_json::Value;
pub(super) use syneroym_async_queue::QueueConfig;
pub(super) use syneroym_core::{
    config::RetryPolicy,
    dht_registry::{MasterAnchorPayload, RegistryClient},
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
    util,
};
pub(super) use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate,
};
pub(super) use syneroym_rpc::{
    AuthLevel, CallOrigin, CallerContext, CallerProof, JsonRpcRequest, NativeDispatchRegistry,
    NativeInvocation, NativeResponse, NativeService, ProxyError, ProxyProtocol,
    ProxyQueueInspector, ProxyRequest, QueuedCall, QueuedTarget, RpcError, RpcResult,
    SERVICE_NOT_FOUND_RPC_CODE, SagaBegin, SagaState as RpcSagaState, SagaStepRequest,
    ServiceProxy, SessionContext,
};
pub(super) use tokio_util::sync::CancellationToken;

pub(super) use super::super::{
    ProxyRouter, RemoteHop, merge_forward_result,
    proxy_outbox::{self, Disposition, ProxyOutbox},
    step_call_budget_ms, target_produced,
};
pub(super) use crate::{
    HandshakeVerifier, MasterAnchorResolver, preamble::RoutePreamble, saga::SagaStore,
};

pub(super) fn test_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext::default(),
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

pub(super) fn base_request(target_service: &str, interface: &str) -> ProxyRequest {
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

pub(super) fn synthetic_addr() -> EndpointAddr {
    let node_id = SecretKey::generate(&mut rand::rng()).public();
    EndpointAddr::new(node_id)
}

pub(super) fn empty_registry() -> EndpointRegistry {
    EndpointRegistry::new_mock(Arc::new(MockStorage::new()))
}

pub(super) fn empty_registry_client() -> Arc<RegistryClient> {
    Arc::new(RegistryClient::new(false, None))
}

pub(super) fn test_router(hop: Arc<dyn RemoteHop>, registry: EndpointRegistry) -> ProxyRouter {
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

/// A node whose registry holds `svc-a` as a native endpoint backed by
/// `service`, with a dedup guard over a real (unencrypted) per-service
/// store. Unencrypted deliberately: the fence's behavior is the
/// subject here, and the SQLCipher half is pinned in the queue crate.
pub(super) struct GuardedNode {
    pub(super) router: ProxyRouter,
    pub(super) service: Arc<RecordingNativeService>,
    pub(super) _native_dispatch: NativeDispatchRegistry,
    pub(super) _dir: tempfile::TempDir,
}

pub(super) async fn guarded_node(with_store: bool) -> GuardedNode {
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

/// A node that can enqueue: a certified calling service, a real
/// per-service store, and a resolver whose bindings a test can change
/// between attempts.
pub(super) struct OutboxNode {
    pub(super) router: Arc<ProxyRouter>,
    pub(super) registry: EndpointRegistry,
    pub(super) resolver: Arc<syneroym_app_orchestration::LogicalResolver>,
    pub(super) target: Arc<RecordingNativeService>,
    pub(super) outbox: Arc<ProxyOutbox>,
    pub(super) provider: Arc<syneroym_data_db::SqliteStorageProvider>,
    pub(super) _native_dispatch: NativeDispatchRegistry,
    pub(super) dir: tempfile::TempDir,
}

pub(super) const CALLER: &str = "did:key:zCaller";

/// `target_reachable` decides whether the immediate attempt succeeds:
/// a registered native endpoint answers, while a WASM endpoint with no
/// engine behind it fails with the retryable "sandbox engine
/// unavailable" -- a shutdown-window state, which is exactly the shape
/// that must queue rather than fail the caller.
#[expect(clippy::too_many_lines, reason = "complex test harness setup helper")]
pub(super) async fn outbox_node(target_reachable: bool, max_attempts: u8) -> OutboxNode {
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

pub(super) fn queued_call(target: QueuedTarget, key: &str) -> QueuedCall {
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
    pub(super) async fn queued(&self) -> Vec<syneroym_async_queue::QueueItem> {
        self.outbox.queue_for(CALLER).await.unwrap().all().unwrap()
    }

    pub(super) async fn dead_letters(&self) -> Vec<syneroym_async_queue::DeadLetter> {
        self.outbox.queue_for(CALLER).await.unwrap().dead_letters().unwrap()
    }

    pub(super) fn queue_file_exists(&self) -> bool {
        self.dir.path().join("services").join(CALLER).join("async.db").exists()
    }
}

/// Polls until `check` holds or the budget runs out.
pub(super) async fn wait_for<F: FnMut() -> bool>(budget: Duration, mut check: F) -> bool {
    let deadline = std::time::Instant::now() + budget;
    while std::time::Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

/// A guest-origin call that failed for good, as the synchronous tier
/// produces it. The target is registered and *answers* -- a refusal it
/// produced itself -- because only a failure the target produced earns
/// a dead letter: this node's own refusals have nothing to replay.
/// A call that runs out of its own deadline against a target that
/// never answers -- a transport-class failure, not a callee one.
///
/// This is the closest a unit fixture gets to "the budget ran out":
/// the retry *loop* itself lives on the remote path, which needs a
/// resolvable address this harness has no registry client for. What
/// matters for the tier rule below is that the failure is one the
/// caller is left holding and that a retry could plausibly have
/// fixed, which a definitive callee refusal is not.
pub(super) fn timing_out_guest_request(key: Option<&str>) -> ProxyRequest {
    let mut req = base_request("did:key:zTarget", "greeter");
    req.origin = CallOrigin::Guest { service_id: CALLER.to_string() };
    req.caller = CallerContext::service_system(CALLER);
    req.idempotency_key = key.map(str::to_string);
    req.idempotent = true;
    req.timeout = Some(Duration::from_millis(100));
    req
}

pub(super) fn failing_guest_request(key: Option<&str>) -> ProxyRequest {
    let mut req = base_request("did:key:zTarget", "greeter");
    req.origin = CallOrigin::Guest { service_id: CALLER.to_string() };
    req.caller = CallerContext::service_system(CALLER);
    req.idempotency_key = key.map(str::to_string);
    req
}

pub(super) fn keyed_request(key: &str) -> ProxyRequest {
    let mut req = base_request("svc-a", "greeter");
    req.caller = CallerContext::service_system("svc-caller");
    req.idempotency_key = Some(key.to_string());
    req
}

#[derive(Debug, Default)]
pub(super) struct RecordingNativeService {
    pub(super) invoked: AtomicUsize,
    pub(super) last_caller_did: Mutex<Option<String>>,
    /// Makes the target answer definitively rather than being absent,
    /// so the queued path's callee-error classification can be driven.
    pub(super) fail_with: std::sync::atomic::AtomicBool,
    /// When set, `dispatch` blocks until this is notified -- a
    /// delivery that genuinely never resolves, which is the only way
    /// to test that shutdown interrupts one.
    pub(super) hold: Mutex<Option<Arc<tokio::sync::Notify>>>,
    /// When set, `dispatch` answers with this exact error, so a test
    /// can drive a specific reserved code the receiver would produce.
    pub(super) answer_with: Mutex<Option<ProxyError>>,
    /// The `(interface, method, params)` of the most recent dispatch --
    /// lets a saga test confirm the walk actually called
    /// `saga-undo-<method>`, not the forward method again.
    pub(super) last_invocation: Mutex<Option<(String, String, Value)>>,
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
pub(super) enum MockOutcome {
    Success(Value),
    Transport,
    Callee { code: i32, message: String },
}

#[derive(Debug, Default)]
pub(super) struct MockHop {
    pub(super) calls: AtomicUsize,
    pub(super) last_preamble: Mutex<Option<RoutePreamble>>,
    pub(super) outcomes: Mutex<std::collections::VecDeque<MockOutcome>>,
}

impl MockHop {
    pub(super) fn with_outcomes(outcomes: Vec<MockOutcome>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            last_preamble: Mutex::new(None),
            outcomes: Mutex::new(outcomes.into()),
        }
    }

    pub(super) fn call_count(&self) -> usize {
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
pub(super) struct EmptyAnchorResolver;
#[async_trait::async_trait]
impl MasterAnchorResolver for EmptyAnchorResolver {
    async fn resolve_master_anchor(
        &self,
        _master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        Ok(MasterAnchorPayload::default())
    }
}

/// A node that can drive a saga: a certified calling service, a real
/// per-service saga log, and a reachable target -- the same shape
/// `outbox_node` builds for `enqueue`, since a saga step is `invoke`
/// plus a log write over the identical wiring.
pub(super) struct SagaNode {
    pub(super) router: Arc<ProxyRouter>,
    pub(super) registry: EndpointRegistry,
    pub(super) target: Arc<RecordingNativeService>,
    pub(super) sagas: Arc<SagaStore>,
    pub(super) dedup_guard: Arc<crate::CallDedupGuard>,
    pub(super) _native_dispatch: NativeDispatchRegistry,
    pub(super) _dir: tempfile::TempDir,
}

pub(super) const SAGA_CALLER: &str = "did:key:zSagaCaller";
pub(super) const SAGA_TARGET: &str = "did:key:zSagaTarget";

pub(super) fn saga_config(dispatch_epoch_timeout_secs: u64) -> syneroym_async_queue::SagaConfig {
    syneroym_async_queue::SagaConfig::from(&syneroym_core::config::AppSandboxRole {
        dispatch_epoch_timeout_secs,
        ..syneroym_core::config::AppSandboxRole::default()
    })
}

pub(super) async fn saga_node(dispatch_epoch_timeout_secs: u64) -> SagaNode {
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

pub(super) fn saga_step_request(saga_id: &str, target: &str) -> SagaStepRequest {
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

pub(super) async fn begun_saga(node: &SagaNode) -> String {
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

pub(super) async fn add_step(node: &SagaNode, saga_id: &str, item: &str) {
    let mut req = saga_step_request(saga_id, SAGA_TARGET);
    req.params = serde_json::json!({"item": item});
    node.router.saga_step(req).await.unwrap();
}

use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde_json::json;
use syneroym_app_orchestration::{ServiceId, StaticInventory, TopologyEpoch, TopologyMode};
use syneroym_core::{local_registry::EndpointRegistry, storage::MockStorage};
use syneroym_data_blob::ObjectStoreBlobProvider;
use syneroym_data_db::SqliteStorageProvider;
use syneroym_fdae::parse_and_validate;
use syneroym_identity::{Identity, substrate};
use syneroym_mqtt_broker::MqttBrokerConfig;
use syneroym_rpc::{Capability, RelationshipProof, SessionContext};

use super::*;

/// Test-only blob provider: in-memory backend, effectively unlimited
/// quota.
pub(crate) fn test_blob_provider() -> Arc<dyn BlobProvider> {
    Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None))
}

/// Test-only messaging context: a real (but throwaway, no network
/// listener) broker with no engine backreference -- sufficient for
/// tests that don't exercise guest-delivery messaging.
pub(crate) fn test_messaging_context() -> MessagingContext {
    MessagingContext {
        broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        engine: Weak::new(),
    }
}

/// Test-only streaming context: a mock in-memory `EndpointRegistry` with
/// no engine backreference -- sufficient for tests that don't exercise
/// stream-protocol registration/routing.
pub(crate) fn test_streaming_context() -> StreamContext {
    StreamContext {
        registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
        engine: Weak::new(),
    }
}

/// Test-only proxy handle: always-unavailable -- sufficient for tests
/// that don't exercise `syneroym:proxy/proxy::call`.
pub(crate) fn test_service_proxy() -> Weak<dyn ServiceProxy> {
    super::empty_service_proxy()
}

fn origin_host_state(
    storage: Arc<dyn StorageProvider>,
    origin: InvocationOrigin,
    auth: AuthLevel,
) -> HostState {
    let caller = CallerContext {
        caller_did: "did:key:zWireCaller".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zWireCaller".to_string(),
            ..Default::default()
        },
        auth,
        proof: None,
    };
    HostState::new(
        "origin-test".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage,
        test_blob_provider(),
        caller,
        0,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    )
    .with_invocation_origin(origin)
}

/// The origin rule: a local dispatch is `internal` whatever identity it
/// carries; a wire dispatch reads the caller's auth, and a
/// substrate-injected level is never `verified`.
#[tokio::test]
async fn invocation_caller_origin_mapping() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());

    for auth in [
        AuthLevel::Delegated,
        AuthLevel::Ucan,
        AuthLevel::System,
        AuthLevel::LocalElevated,
        AuthLevel::LocalReadOnly,
    ] {
        let mut local = origin_host_state(storage.clone(), InvocationOrigin::Local, auth);
        assert_eq!(
            invocation::Host::caller(&mut local).await,
            WitCallerOrigin::Internal,
            "local call must be internal for {auth:?}"
        );
    }

    for auth in [AuthLevel::Delegated, AuthLevel::Ucan] {
        let mut wire = origin_host_state(storage.clone(), InvocationOrigin::Wire, auth);
        assert_eq!(
            invocation::Host::caller(&mut wire).await,
            WitCallerOrigin::Verified("did:key:zWireCaller".to_string()),
        );
    }
    for auth in [AuthLevel::System, AuthLevel::LocalElevated, AuthLevel::LocalReadOnly] {
        let mut wire = origin_host_state(storage.clone(), InvocationOrigin::Wire, auth);
        assert_eq!(
            invocation::Host::caller(&mut wire).await,
            WitCallerOrigin::Anonymous,
            "a substrate-injected level must not read as verified on the wire ({auth:?})"
        );
    }
}

/// Records the last `ProxyRequest` it was invoked with, so a test can
/// inspect what `proxy::Host::call` actually built (in particular
/// `caller`) without needing a real downstream service to answer.
/// `invoke_count` lets a test pin the "no network hop" budget:
/// a dependency resolution that went through the router/supervisor
/// instead of resolving host-side, before the `ProxyRequest` exists,
/// would still land here, but with more than the one `invoke` a single
/// call is supposed to cost.
#[derive(Debug, Default)]
struct RecordingProxy {
    last_request: Mutex<Option<ProxyRequest>>,
    invoke_count: AtomicUsize,
    last_enqueued: Mutex<Option<QueuedCall>>,
    enqueue_count: AtomicUsize,
    last_saga_begin: Mutex<Option<SagaBegin>>,
    last_saga_step: Mutex<Option<SagaStepRequest>>,
    last_saga_commit: Mutex<Option<(String, String)>>,
    last_saga_compensate: Mutex<Option<(String, String)>>,
}

#[async_trait::async_trait]
impl ServiceProxy for RecordingProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, RpcProxyError> {
        self.invoke_count.fetch_add(1, Ordering::SeqCst);
        let recorded = request.clone();
        *self.last_request.lock().unwrap() = Some(recorded);
        Ok(Value::Null)
    }

    async fn enqueue(&self, call: QueuedCall) -> Result<(), RpcProxyError> {
        self.enqueue_count.fetch_add(1, Ordering::SeqCst);
        *self.last_enqueued.lock().unwrap() = Some(call);
        Ok(())
    }

    async fn saga_begin(&self, req: SagaBegin) -> Result<String, RpcProxyError> {
        *self.last_saga_begin.lock().unwrap() = Some(req);
        Ok("saga-1".to_string())
    }

    async fn saga_step(&self, req: SagaStepRequest) -> Result<Value, RpcProxyError> {
        *self.last_saga_step.lock().unwrap() = Some(req);
        Ok(Value::Null)
    }

    async fn saga_commit(&self, service_id: &str, saga_id: &str) -> Result<(), RpcProxyError> {
        *self.last_saga_commit.lock().unwrap() =
            Some((service_id.to_string(), saga_id.to_string()));
        Ok(())
    }

    async fn saga_compensate(&self, service_id: &str, saga_id: &str) -> Result<(), RpcProxyError> {
        *self.last_saga_compensate.lock().unwrap() =
            Some((service_id.to_string(), saga_id.to_string()));
        Ok(())
    }

    async fn saga_status(
        &self,
        _service_id: &str,
        saga_id: &str,
    ) -> Result<syneroym_rpc::SagaInfo, RpcProxyError> {
        Ok(syneroym_rpc::SagaInfo {
            saga_id: saga_id.to_string(),
            name: "wf".to_string(),
            state: RpcSagaState::Open,
            steps: 1,
            compensated_steps: 0,
            created_at: 1_000,
            deadline_at: 4_600_000,
            last_error: None,
        })
    }
}

fn enqueue_options(idempotency_key: Option<&str>) -> Option<CallOptions> {
    Some(CallOptions {
        protocol: None,
        idempotent: false,
        timeout_ms: None,
        routing_key: None,
        idempotency_key: idempotency_key.map(str::to_string),
    })
}

/// A queued call is delivered at least once, so one with no fence
/// would run the target twice on the first retry. Refused before
/// anything is resolved, written, or attempted -- and the refusal
/// names the missing field, so a future change relaxing this has to
/// confront the argument.
#[tokio::test]
async fn an_enqueue_without_an_idempotency_key_is_refused() {
    let resolver = Arc::new(LogicalResolver::new(Arc::new(StaticInventory::new())));
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = dependency_host("frontend", None, resolver, &proxy, temp_dir.path());

    let err = proxy::Host::enqueue(
        &mut host,
        CallTarget::Service("did:key:zBackend".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        enqueue_options(None),
    )
    .await
    .expect_err("an unkeyed enqueue must be refused");

    let proxy::ProxyError::Internal(message) = err else {
        panic!("expected an internal refusal, got {err:?}");
    };
    assert!(
        message.contains("idempotency-key"),
        "the refusal must name the missing field, got: {message}"
    );
    assert_eq!(
        proxy.enqueue_count.load(Ordering::SeqCst),
        0,
        "nothing may reach the outbox for an unfenced call"
    );
}

/// Present is not usable. An empty key becomes this service's queue
/// key, so the first enqueue would take it and every later one would
/// be silently dropped as a duplicate -- the guest seeing success
/// while nothing was queued.
#[tokio::test]
async fn an_enqueue_with_a_blank_idempotency_key_is_refused() {
    let resolver = Arc::new(LogicalResolver::new(Arc::new(StaticInventory::new())));
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = dependency_host("frontend", None, resolver, &proxy, temp_dir.path());

    for blank in ["", "   "] {
        let err = proxy::Host::enqueue(
            &mut host,
            CallTarget::Service("did:key:zBackend".to_string()),
            "greeter".to_string(),
            "greet".to_string(),
            "null".to_string(),
            enqueue_options(Some(blank)),
        )
        .await
        .expect_err("a blank key must be refused");
        assert!(
            matches!(err, proxy::ProxyError::Internal(ref m) if m.contains("non-empty")),
            "unexpected error for {blank:?}: {err:?}"
        );
    }
    assert_eq!(proxy.enqueue_count.load(Ordering::SeqCst), 0);
}

/// The key travels on every attempt and becomes part of a primary key
/// on the receiving node, so it is bounded like any other
/// guest-controlled string that leaves the sandbox.
#[tokio::test]
async fn an_over_long_idempotency_key_is_refused() {
    let resolver = Arc::new(LogicalResolver::new(Arc::new(StaticInventory::new())));
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = dependency_host("frontend", None, resolver, &proxy, temp_dir.path());

    let err = proxy::Host::enqueue(
        &mut host,
        CallTarget::Service("did:key:zBackend".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        enqueue_options(Some(&"k".repeat(MAX_IDEMPOTENCY_KEY_BYTES + 1))),
    )
    .await
    .expect_err("an over-long key must be refused");
    assert!(matches!(err, proxy::ProxyError::Internal(_)), "{err:?}");
    assert_eq!(proxy.enqueue_count.load(Ordering::SeqCst), 0);
}

/// The stored item names the dependency, never a resolved DID: the
/// worker resolves again at every attempt, so a re-pushed binding
/// takes effect (ADR-0021 §2).
#[tokio::test]
async fn an_enqueued_dependency_is_stored_by_name_not_resolved_at_the_host() {
    use syneroym_app_orchestration::AppRegistry;

    let registry = Arc::new(StaticInventory::new());
    registry.register(
        TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
        dependency_topology_entry(vec!["did:key:zBackendMember"]),
    );
    let resolver = Arc::new(LogicalResolver::new(registry));
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host =
        dependency_host("frontend", Some("app-1".to_string()), resolver, &proxy, temp_dir.path());

    proxy::Host::enqueue(
        &mut host,
        CallTarget::Dependency("backend".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        enqueue_options(Some("msg-7")),
    )
    .await
    .unwrap();

    let stored = proxy.last_enqueued.lock().unwrap().take().unwrap();
    assert_eq!(stored.target, QueuedTarget::Dependency("backend".to_string()));
    assert_eq!(stored.idempotency_key, "msg-7");
    assert_eq!(stored.app_instance_id.as_deref(), Some("app-1"));
    assert_eq!(stored.caller_service_id, "frontend");
}

/// The self-proxy caller forwarding in `proxy::Host::call`
/// is scoped to `service == self.component_id` -- a genuinely
/// cross-service proxy call must still synthesize `service_system`, per
/// the function's own doc comment ("does NOT inherit the identity of
/// whoever invoked *this* guest"). Nothing pinned that fact before this
/// test; the whole "cannot escalate to another service's rights"
/// argument in the doc comment rested on it being true, unverified.
#[tokio::test]
async fn self_proxy_forwarding_does_not_extend_to_a_different_target_service() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let proxy = Arc::new(RecordingProxy::default());

    let real_caller = CallerContext {
        caller_did: "did:key:zRealCaller".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zRealCaller".to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::substrate("did:key:zRealCaller"),
                can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };

    let mut host = HostState::new(
        "svc-a".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage,
        test_blob_provider(),
        real_caller,
        0,
        test_messaging_context(),
        test_streaming_context(),
        Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>,
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    proxy::Host::call(
        &mut host,
        CallTarget::Service("svc-b".to_string()),
        "some-interface".to_string(),
        "some-method".to_string(),
        "null".to_string(),
        None,
    )
    .await
    .unwrap();

    let received = proxy.last_request.lock().unwrap().take().unwrap();
    assert_eq!(
        received.caller.auth,
        AuthLevel::System,
        "a proxy call to a *different* service must not carry the guest's real caller identity, \
         capabilities included -- got {:?}",
        received.caller
    );
    assert!(
        received.caller.session.capabilities.is_empty(),
        "a cross-service proxy call must never carry the guest's real capabilities: {:?}",
        received.caller.session.capabilities
    );
}

// ── Dependency resolution through `proxy::Host::call` ──────────────

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

/// Builds a `HostState` naming `component_id` as deployed under
/// `app_instance_id` (or standalone, if `None`), backed by `resolver`
/// and `proxy`. `db_dir` must outlive the returned `HostState`.
fn dependency_host(
    component_id: &str,
    app_instance_id: Option<String>,
    resolver: Arc<LogicalResolver>,
    proxy: &Arc<RecordingProxy>,
    db_dir: &Path,
) -> HostState {
    HostState::new(
        component_id.to_string(),
        None,
        Arc::new(KeyStore::new()),
        Arc::new(SqliteStorageProvider::new(db_dir, false).unwrap()),
        test_blob_provider(),
        CallerContext::service_system(component_id),
        0,
        test_messaging_context(),
        test_streaming_context(),
        Arc::downgrade(proxy) as Weak<dyn ServiceProxy>,
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        app_instance_id,
        resolver,
    )
}

#[tokio::test]
async fn a_dependency_name_resolves_to_its_bound_member_before_the_request_is_built() {
    use syneroym_app_orchestration::AppRegistry;

    let registry = Arc::new(StaticInventory::new());
    registry.register(
        TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
        dependency_topology_entry(vec!["did:key:zBackendMember"]),
    );
    let resolver = Arc::new(LogicalResolver::new(registry));
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host =
        dependency_host("frontend", Some("app-1".to_string()), resolver, &proxy, temp_dir.path());

    proxy::Host::call(
        &mut host,
        CallTarget::Dependency("backend".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        None,
    )
    .await
    .unwrap();

    let received = proxy.last_request.lock().unwrap().take().unwrap();
    assert_eq!(received.target_service, "did:key:zBackendMember");
    // The "no network hop" budget: dependency resolution happens
    // host-side, before the `ProxyRequest` is built, so one dependency
    // call must cost exactly one `invoke` -- never a second hop to ask
    // a supervisor or router to resolve it (ADR-0021 §8 forbids that
    // outright).
    assert_eq!(proxy.invoke_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_unbound_dependency_name_is_dependency_not_bound_and_never_reaches_the_proxy() {
    let resolver = syneroym_app_orchestration::empty_resolver();
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host =
        dependency_host("frontend", Some("app-1".to_string()), resolver, &proxy, temp_dir.path());

    let err = proxy::Host::call(
        &mut host,
        CallTarget::Dependency("backend".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        None,
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, proxy::ProxyError::DependencyNotBound(_)),
        "an unbound dependency name must fail as dependency-not-bound, not service-not-found: \
         {err:?}"
    );
    assert!(
        proxy.last_request.lock().unwrap().is_none(),
        "resolution must fail before a ProxyRequest is ever built"
    );
}

#[tokio::test]
async fn a_component_with_no_app_context_cannot_name_a_dependency() {
    let resolver = syneroym_app_orchestration::empty_resolver();
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = dependency_host("standalone-svc", None, resolver, &proxy, temp_dir.path());

    let err = proxy::Host::call(
        &mut host,
        CallTarget::Dependency("backend".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        None,
    )
    .await
    .unwrap_err();

    assert!(matches!(err, proxy::ProxyError::DependencyNotBound(_)));
}

#[tokio::test]
async fn a_raw_did_target_is_unchanged() {
    let resolver = syneroym_app_orchestration::empty_resolver();
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host =
        dependency_host("frontend", Some("app-1".to_string()), resolver, &proxy, temp_dir.path());

    proxy::Host::call(
        &mut host,
        CallTarget::Service("did:key:zSomeoneElse".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        None,
    )
    .await
    .unwrap();

    let received = proxy.last_request.lock().unwrap().take().unwrap();
    assert_eq!(received.target_service, "did:key:zSomeoneElse");
}

#[tokio::test]
async fn a_routing_key_selects_deterministically_across_a_two_member_binding() {
    use syneroym_app_orchestration::AppRegistry;

    let registry = Arc::new(StaticInventory::new());
    registry.register(
        TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
        dependency_topology_entry(vec!["did:key:zMemberA", "did:key:zMemberB"]),
    );
    let resolver = Arc::new(LogicalResolver::new(registry));
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host =
        dependency_host("frontend", Some("app-1".to_string()), resolver, &proxy, temp_dir.path());

    let options = Some(CallOptions {
        protocol: None,
        idempotent: false,
        timeout_ms: None,
        routing_key: Some("user-42".to_string()),
        idempotency_key: None,
    });
    proxy::Host::call(
        &mut host,
        CallTarget::Dependency("backend".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        options.clone(),
    )
    .await
    .unwrap();
    let first = proxy.last_request.lock().unwrap().take().unwrap().target_service;

    proxy::Host::call(
        &mut host,
        CallTarget::Dependency("backend".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        options,
    )
    .await
    .unwrap();
    let second = proxy.last_request.lock().unwrap().take().unwrap().target_service;

    assert_eq!(first, second, "the same routing key must select the same member every time");
}

/// The guest's fence has to reach the request the router builds, or
/// nothing downstream can put it on the wire for the receiver to
/// dedup on (ADR-0023 §4).
#[tokio::test]
async fn the_host_function_passes_the_guests_key_into_the_proxy_request() {
    let registry = Arc::new(StaticInventory::new());
    let resolver = Arc::new(LogicalResolver::new(registry));
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = dependency_host("frontend", None, resolver, &proxy, temp_dir.path());

    proxy::Host::call(
        &mut host,
        CallTarget::Service("did:key:zBackend".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        Some(CallOptions {
            protocol: None,
            idempotent: false,
            timeout_ms: None,
            routing_key: None,
            idempotency_key: Some("msg-7".to_string()),
        }),
    )
    .await
    .unwrap();

    let received = proxy.last_request.lock().unwrap().take().unwrap();
    assert_eq!(received.idempotency_key.as_deref(), Some("msg-7"));
}

/// The ordinary call is unchanged: no options, no key.
#[tokio::test]
async fn a_call_with_no_options_carries_no_idempotency_key() {
    let registry = Arc::new(StaticInventory::new());
    let resolver = Arc::new(LogicalResolver::new(registry));
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = dependency_host("frontend", None, resolver, &proxy, temp_dir.path());

    proxy::Host::call(
        &mut host,
        CallTarget::Service("did:key:zBackend".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        None,
    )
    .await
    .unwrap();

    let received = proxy.last_request.lock().unwrap().take().unwrap();
    assert_eq!(received.idempotency_key, None);
}

#[tokio::test]
async fn a_dependency_resolving_to_the_components_own_service_still_forwards_the_real_caller() {
    use syneroym_app_orchestration::AppRegistry;

    let registry = Arc::new(StaticInventory::new());
    registry.register(
        TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("self-dep")),
        dependency_topology_entry(vec!["did:key:zSelf"]),
    );
    let resolver = Arc::new(LogicalResolver::new(registry));
    let proxy = Arc::new(RecordingProxy::default());
    let real_caller = CallerContext {
        caller_did: "did:key:zRealCaller".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zRealCaller".to_string(),
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = HostState::new(
        "did:key:zSelf".to_string(),
        None,
        Arc::new(KeyStore::new()),
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap()),
        test_blob_provider(),
        real_caller,
        0,
        test_messaging_context(),
        test_streaming_context(),
        Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>,
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        Some("app-1".to_string()),
        resolver,
    );

    proxy::Host::call(
        &mut host,
        CallTarget::Dependency("self-dep".to_string()),
        "greeter".to_string(),
        "greet".to_string(),
        "null".to_string(),
        None,
    )
    .await
    .unwrap();

    let received = proxy.last_request.lock().unwrap().take().unwrap();
    assert_eq!(received.target_service, "did:key:zSelf");
    assert_eq!(
        received.caller.caller_did, "did:key:zRealCaller",
        "a dependency that resolves to the component's own service is still a self-proxy call, \
         and must forward the real caller"
    );
}

#[tokio::test]
async fn test_config_get_and_get_section() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());

    let config_json =
        r#"{"db_host": "localhost", "db_port": "5432", "db.password": "secret", "db": "mydb"}"#;
    let generation = storage.save_config_generation("test_svc", config_json).await.unwrap();

    let mut host = HostState::new(
        "test_svc".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage,
        test_blob_provider(),
        CallerContext::service_system("test-caller"),
        generation,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    use app_config::Host as ConfigHost;

    // 1. Existing key returns Ok(Some(value))
    let val = ConfigHost::get(&mut host, "db_host".to_string()).await.unwrap().unwrap();
    assert_eq!(val, "localhost");

    // 2. Missing key returns Ok(None)
    let missing = ConfigHost::get(&mut host, "db_user".to_string()).await.unwrap();
    assert!(missing.is_none());

    // get_section returns prefixed values with exact matching boundaries
    let section = ConfigHost::get_section(&mut host, "db".to_string()).await.unwrap();
    let mut section_keys: Vec<String> = section.into_iter().map(|(k, _)| k).collect();
    section_keys.sort();
    // "db" and "db.password" match. "db_host" and "db_port" DO NOT.
    assert_eq!(section_keys, vec!["db", "db.password"]);
}

#[tokio::test]
async fn test_config_isolation_and_generation_pinning() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());

    // Service A Gen 1
    let gen1_a = storage.save_config_generation("svc_a", r#"{"mode": "v1"}"#).await.unwrap();
    // Service A Gen 2
    let gen2_a = storage.save_config_generation("svc_a", r#"{"mode": "v2"}"#).await.unwrap();

    // Service B Gen 1
    let gen1_b = storage.save_config_generation("svc_b", r#"{"mode": "b_mode"}"#).await.unwrap();

    use app_config::Host as ConfigHost;

    // Two WASM components with different configs get isolated values
    let mut host_a_gen2 = HostState::new(
        "svc_a".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage.clone(),
        test_blob_provider(),
        CallerContext::service_system("test-caller"),
        gen2_a,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );
    let mut host_b = HostState::new(
        "svc_b".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage.clone(),
        test_blob_provider(),
        CallerContext::service_system("test-caller"),
        gen1_b,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    let val_a = ConfigHost::get(&mut host_a_gen2, "mode".to_string()).await.unwrap().unwrap();
    let val_b = ConfigHost::get(&mut host_b, "mode".to_string()).await.unwrap().unwrap();
    assert_eq!(val_a, "v2");
    assert_eq!(val_b, "b_mode");

    // Re-deploy bumps generation; in-flight invocations retain prior generation
    let mut host_a_gen1 = HostState::new(
        "svc_a".to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage.clone(),
        test_blob_provider(),
        CallerContext::service_system("test-caller"),
        gen1_a,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );
    let val_a_old = ConfigHost::get(&mut host_a_gen1, "mode".to_string()).await.unwrap().unwrap();
    assert_eq!(val_a_old, "v1");
}

/// M3A failure/security test: `vault/reveal` on a non-existent key
/// returns `vault-error::not-found` at the WIT host-function boundary
/// (not just `Ok(None)` one layer down at `ServiceStore::reveal_secret`,
/// which `syneroym-data-db`'s own tests already cover).
#[tokio::test]
async fn test_vault_reveal_not_found_at_host_boundary() {
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([3u8; 32]).unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), true).unwrap());
    let mut host_state = HostState::new(
        "vault-not-found-svc".to_string(),
        None,
        key_store,
        storage_provider,
        test_blob_provider(),
        CallerContext::service_system("test-caller"),
        0,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        None,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    let result = vault::Host::reveal(&mut host_state, "does-not-exist".to_string()).await;
    assert!(matches!(result, Err(VaultError::NotFound)));
}

// -- FDAE host wiring --------------------------------------------------
//
// Real `QueryAuth` construction from `HostState.fdae_policy`/`caller`,
// `check-access`, and host-side CLS field-stripping, exercised through
// `store::Host` on a `HostState` built with a hand-injected `Policy`
// (`fdae_policy` is `None` for a service with no stored policy).

const FDAE_SERVICE_ID: &str = "svc-fdae-host-test";

fn fdae_resource(collection: &str) -> ResourceUri {
    ResourceUri(format!(
        "{}/collection/{collection}",
        ResourceUri::service(FDAE_SERVICE_ID, FDAE_SERVICE_ID).0
    ))
}

fn fdae_read_cap(collection: &str) -> Capability {
    Capability {
        with: fdae_resource(collection),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: None,
    }
}

fn fdae_caller(subject_did: &str, capabilities: Vec<Capability>) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities,
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    }
}

/// `document` --creator--> `user`, `view` permission reachable only via
/// the creator relation. Mirrors `data_db::tests_fdae::single_hop_policy`.
fn fdae_single_hop_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]}
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

/// Same shape as `fdae_single_hop_policy`, plus a CLS `fields.deny:
/// ["ssn"]`.
fn fdae_cls_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "caller"]],
                            "fields": {"deny": ["ssn"]}
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

/// A `manage` permission covering `data-layer/write`, reachable via the
/// same creator relation -- used to exercise `delete_many`'s write-mode
/// sieve. Mirrors `data_db::tests_fdae::write_policy`.
fn fdae_write_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "manage": {"allows": ["data-layer/write"], "paths": [["creator", "caller"]]}
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

fn fdae_write_cap(collection: &str) -> Capability {
    Capability {
        with: fdae_resource(collection),
        can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
        caveats: None,
    }
}

fn fdae_host_state(
    storage_provider: Arc<dyn StorageProvider>,
    caller: CallerContext,
    fdae_policy: Option<Arc<Policy>>,
) -> HostState {
    HostState::new(
        FDAE_SERVICE_ID.to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage_provider,
        test_blob_provider(),
        caller,
        0,
        test_messaging_context(),
        test_streaming_context(),
        test_service_proxy(),
        fdae_policy,
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    )
}

/// Seeds `users`/`documents` collections: `doc-1` created by alice,
/// `doc-2` created by bob, both carrying an `ssn` field for the CLS
/// tests. Uses a policy-absent `HostState` (`put`/`create_collection`
/// carry no FDAE gate).
async fn fdae_seed_documents(storage_provider: Arc<dyn StorageProvider>) {
    let mut seeder =
        fdae_host_state(storage_provider, CallerContext::service_system(FDAE_SERVICE_ID), None);
    store::Host::create_collection(
        &mut seeder,
        CollectionSchema { name: "users".to_string(), indexes: vec![] },
    )
    .await
    .unwrap();
    store::Host::create_collection(
        &mut seeder,
        CollectionSchema { name: "documents".to_string(), indexes: vec![] },
    )
    .await
    .unwrap();
    store::Host::put(
        &mut seeder,
        "users".to_string(),
        RecordWriteValue {
            id: "u-alice".to_string(),
            payload: json!({"did": "did:key:alice"}).to_string().into_bytes(),
        },
    )
    .await
    .unwrap();
    store::Host::put(
        &mut seeder,
        "users".to_string(),
        RecordWriteValue {
            id: "u-bob".to_string(),
            payload: json!({"did": "did:key:bob"}).to_string().into_bytes(),
        },
    )
    .await
    .unwrap();
    store::Host::put(
        &mut seeder,
        "documents".to_string(),
        RecordWriteValue {
            id: "doc-1".to_string(),
            payload: json!({"creator_uuid": "u-alice", "ssn": "111-11-1111"})
                .to_string()
                .into_bytes(),
        },
    )
    .await
    .unwrap();
    store::Host::put(
        &mut seeder,
        "documents".to_string(),
        RecordWriteValue {
            id: "doc-2".to_string(),
            payload: json!({"creator_uuid": "u-bob", "ssn": "222-22-2222"})
                .to_string()
                .into_bytes(),
        },
    )
    .await
    .unwrap();
}

fn payload_json(record: &RecordReadValue) -> Value {
    serde_json::from_slice(&record.payload).unwrap()
}

/// RLS: `get`/`query` return only alice's own reachable row, and
/// `check_access` matches (reachable -> `true`, unreachable -> `false`).
#[tokio::test]
async fn fdae_rls_filters_get_query_and_check_access() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let policy = Arc::new(fdae_single_hop_policy());
    let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host = fdae_host_state(storage_provider, alice, Some(policy));

    let own =
        store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string()).await.unwrap();
    assert!(own.is_some(), "alice's own document must be reachable");
    let other =
        store::Host::get(&mut host, "documents".to_string(), "doc-2".to_string()).await.unwrap();
    assert!(other.is_none(), "bob's document is unreachable, not an error (ADR-0007)");

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
    let ids: Vec<_> = result.records.iter().map(|r| r.id.clone()).collect();
    assert_eq!(ids, vec!["doc-1"], "bob's document must be excluded from query results");

    assert!(
        store::Host::check_access(
            &mut host,
            "documents".to_string(),
            "doc-1".to_string(),
            Ability::DATA_LAYER_READ.to_string(),
        )
        .await
        .unwrap(),
        "check_access must allow alice's own reachable row"
    );
    assert!(
        !store::Host::check_access(
            &mut host,
            "documents".to_string(),
            "doc-2".to_string(),
            Ability::DATA_LAYER_READ.to_string(),
        )
        .await
        .unwrap(),
        "check_access must deny bob's unreachable row"
    );
}

/// CLS: a policy with `fields.deny: ["ssn"]` strips `ssn` from the
/// payload returned by both `get` and `query` -- host-side projection
/// means a masked value is never returned to the caller.
#[tokio::test]
async fn fdae_cls_strips_masked_field_from_get_and_query() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let policy = Arc::new(fdae_cls_policy());
    let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host = fdae_host_state(storage_provider, alice, Some(policy));

    let own = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
        .await
        .unwrap()
        .unwrap();
    let payload = payload_json(&own);
    assert!(payload.get("ssn").is_none(), "ssn must be stripped from get's payload");
    assert_eq!(payload.get("creator_uuid").and_then(Value::as_str), Some("u-alice"));

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
    assert_eq!(result.records.len(), 1);
    let payload = payload_json(&result.records[0]);
    assert!(payload.get("ssn").is_none(), "ssn must be stripped from query's payload");
}

/// Pass-through: `fdae_policy: None` leaves rows and payloads unchanged
/// -- zero behavior change on the unconfigured (today's production)
/// path.
#[tokio::test]
async fn fdae_policy_absent_is_unfiltered_pass_through() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let caller = CallerContext::service_system(FDAE_SERVICE_ID);
    let mut host = fdae_host_state(storage_provider, caller, None);

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
    assert_eq!(result.records.len(), 2, "no policy means both rows are visible");
    for record in &result.records {
        assert!(
            payload_json(record).get("ssn").is_some(),
            "no policy means no CLS strip -- ssn must survive untouched"
        );
    }
}

/// Lifecycle-hook reads (`init`/`migrate`, which run as
/// `CallerContext::local_elevated`) must stay unfiltered even under a
/// deployed policy. Without `query_auth`'s `LocalElevated` exemption,
/// `local_elevated`'s `data-layer/admin` capability entails
/// `data-layer/read` and covers every collection, so `compile_read`
/// compiles a *real* sieve here -- bound to
/// `"system:local-elevated:<service_id>"`, a DID no principal row can
/// ever hold -- and both documents would silently vanish. A migration
/// that reads its own data to decide how to rewrite it would act on
/// that emptiness instead of erroring.
#[tokio::test]
async fn fdae_local_elevated_lifecycle_reads_stay_unfiltered_under_a_policy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let policy = Arc::new(fdae_single_hop_policy());
    let caller = CallerContext::local_elevated(FDAE_SERVICE_ID);
    let mut host = fdae_host_state(storage_provider, caller, Some(policy));

    let opts = QueryOptions { filter: None, limit: None, cursor: None };
    let result = store::Host::query(&mut host, "documents".to_string(), opts).await.unwrap();
    assert_eq!(
        result.records.len(),
        2,
        "a lifecycle hook must see every row regardless of the deployed policy"
    );

    let doc =
        store::Host::get(&mut host, "documents".to_string(), "doc-2".to_string()).await.unwrap();
    assert!(
        doc.is_some(),
        "get during init/migrate must not be sieved against the synthesized local-elevated \
         identity"
    );
}

/// `aggregate` is row-filtered through the host layer identically to
/// `get`/`query` -- covers the `store::Host::aggregate` wiring seam this
/// phase adds, which no host test previously exercised with a real
/// `Some(policy)` (a dropped or `None`-replaced `query_auth()` call here
/// would have passed every prior test).
#[tokio::test]
async fn fdae_aggregate_is_row_filtered_through_host() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let policy = Arc::new(fdae_single_hop_policy());
    let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host = fdae_host_state(storage_provider, alice, Some(policy));

    let result = store::Host::aggregate(
        &mut host,
        "documents".to_string(),
        r#"{"$group":{"_id":null,"n":{"$sum":1}}}"#.to_string(),
    )
    .await
    .unwrap();
    // `SqlValue` doesn't derive `PartialEq` -- compare via its
    // already-derived `Serialize` impl.
    assert_eq!(
        serde_json::to_value(&result.rows).unwrap(),
        serde_json::to_value(vec![vec![SqlValue::Integer(1)]]).unwrap(),
        "only alice's own doc-1 is counted"
    );
}

/// `delete_many` is filtered as a write operation through the host layer
/// -- same wiring-seam coverage gap as `aggregate` above.
#[tokio::test]
async fn fdae_delete_many_is_write_filtered_through_host() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;
    let policy = Arc::new(fdae_write_policy());

    // A read-only capability must not satisfy the write-mode sieve.
    let alice_read_only = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host_ro =
        fdae_host_state(storage_provider.clone(), alice_read_only, Some(policy.clone()));
    let deleted = store::Host::delete_many(&mut host_ro, "documents".to_string(), String::new())
        .await
        .unwrap();
    assert_eq!(deleted, 0, "a read-only capability must not delete anything");

    // A write capability deletes only alice's own row.
    let alice_write = fdae_caller("did:key:alice", vec![fdae_write_cap("documents")]);
    let mut host_rw = fdae_host_state(storage_provider, alice_write, Some(policy));
    let deleted = store::Host::delete_many(&mut host_rw, "documents".to_string(), String::new())
        .await
        .unwrap();
    assert_eq!(deleted, 1, "only alice's own document is deletable");
}

/// Through the `store::Host` guest boundary: a
/// write-capable caller who cannot reach a row via the compiled sieve
/// is denied `put`/`patch`/`delete` on it; the same caller against a
/// row they do reach succeeds.
#[tokio::test]
async fn fdae_put_patch_delete_deny_an_unreachable_row_and_allow_a_reachable_one() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;
    let policy = Arc::new(fdae_write_policy());
    let alice = fdae_caller("did:key:alice", vec![fdae_write_cap("documents")]);
    let mut host = fdae_host_state(storage_provider, alice, Some(policy));

    // doc-2 belongs to bob -- unreachable to alice under the write sieve.
    let err = store::Host::patch(
        &mut host,
        "documents".to_string(),
        "doc-2".to_string(),
        br#"{"x":1}"#.to_vec(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));

    let err = store::Host::delete(&mut host, "documents".to_string(), "doc-2".to_string())
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));

    let err = store::Host::put(
        &mut host,
        "documents".to_string(),
        RecordWriteValue {
            id: "doc-2".to_string(),
            payload: json!({"creator_uuid": "u-bob", "hijacked": true}).to_string().into_bytes(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));

    // doc-1 belongs to alice -- reachable.
    store::Host::patch(
        &mut host,
        "documents".to_string(),
        "doc-1".to_string(),
        br#"{"nickname":"al"}"#.to_vec(),
    )
    .await
    .unwrap();
    let record = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(payload_json(&record)["nickname"], "al");

    store::Host::delete(&mut host, "documents".to_string(), "doc-1".to_string()).await.unwrap();
    assert!(
        store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
            .await
            .unwrap()
            .is_none()
    );
}

/// `drop_collection` bypasses any per-row policy on the collection
/// entirely, so it must not be reachable through an ordinary write
/// capability: a caller holding only `data-layer/write` on `documents`
/// (able to `put`/`patch`/`delete` rows it can individually reach) is
/// denied `drop_collection("documents")` outright; a caller holding
/// `data-layer/admin` on the service succeeds.
#[tokio::test]
async fn drop_collection_requires_admin_not_an_ordinary_write_capability() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    let writer = fdae_caller("did:key:alice", vec![fdae_write_cap("documents")]);
    let mut host = fdae_host_state(storage_provider.clone(), writer, None);
    let err = store::Host::drop_collection(&mut host, "documents".to_string()).await.unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
    assert!(
        store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
            .await
            .unwrap()
            .is_some(),
        "a denied drop_collection must leave the collection intact"
    );

    let admin = fdae_caller(
        "did:key:admin",
        vec![Capability {
            with: ResourceUri::service(FDAE_SERVICE_ID, FDAE_SERVICE_ID),
            can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
            caveats: None,
        }],
    );
    let mut host = fdae_host_state(storage_provider, admin, None);
    store::Host::drop_collection(&mut host, "documents".to_string()).await.unwrap();
}

/// **Extra-capability CLS-narrowing pin.** The same "an extra
/// capability shouldn't narrow" defect pinned for RLS (a caveated
/// second capability narrowing the result to zero rows) applies to
/// CLS `fields.deny` union across capabilities too. Alice holds both
/// an unrestricted `read` capability and a second `read` capability
/// caveated `fields.deny: ["ssn"]` on the same resource; today's
/// `compile_cls` unions every entitling capability's deny-list, so
/// even the unrestricted grant's payload comes back stripped. When
/// this defect is fixed, this assertion should flip to `ssn` being
/// **present** (the unrestricted capability's caveat-free access
/// should win).
#[tokio::test]
async fn fdae_d04_02_g_extra_caveated_capability_narrows_cls_strip() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    fdae_seed_documents(storage_provider.clone()).await;

    // `fdae_single_hop_policy` carries no policy-level `fields.deny` --
    // the mask below comes entirely from the second capability's caveat.
    let policy = Arc::new(fdae_single_hop_policy());
    let unrestricted_cap = fdae_read_cap("documents");
    let ssn_deny_cap = Capability {
        with: fdae_resource("documents"),
        can: Ability(Ability::DATA_LAYER_READ.to_string()),
        caveats: Some(json!({"fields": {"deny": ["ssn"]}})),
    };
    let alice = fdae_caller("did:key:alice", vec![unrestricted_cap, ssn_deny_cap]);
    let mut host = fdae_host_state(storage_provider, alice, Some(policy));

    let own = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
        .await
        .unwrap()
        .unwrap();
    let payload = payload_json(&own);
    assert!(
        payload.get("ssn").is_none(),
        "D-04-02-g: today, the caveated capability's fields.deny narrows the unrestricted \
         capability's access too, so ssn is stripped even though the unrestricted grant alone \
         should expose it. If this assertion starts failing, D-04-02-g has been fixed -- update \
         this test to assert ssn IS present."
    );
}

// -- Cross-service relationship-proof fetch, wired
// through `HostState::resolve_query_auth` --------------------------

fn fdae_remote_relation_policy(expected_asserter_did: &str) -> Policy {
    parse_and_validate(&format!(
        r#"{{
            "version": "fdae/v1",
            "definitions": {{
                "document": {{
                    "table": "documents",
                    "relations": {{"owner": {{
                        "target": "employee", "service": "hr-svc",
                        "join_column": "owner_uuid",
                        "expected_asserter_did": "{expected_asserter_did}"
                    }}}},
                    "permissions": {{
                        "view": {{"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}}
                    }}
                }}
            }}
        }}"#
    ))
    .unwrap()
}

#[derive(Debug)]
struct StubProxy(Mutex<Option<Result<Value, RpcProxyError>>>);

#[async_trait::async_trait]
impl ServiceProxy for StubProxy {
    async fn invoke(&self, _request: ProxyRequest) -> Result<Value, RpcProxyError> {
        self.0.lock().unwrap().take().expect("StubProxy invoked with no response configured")
    }
}

async fn seed_one_remote_owned_document(storage_provider: Arc<dyn StorageProvider>) {
    let mut seeder =
        fdae_host_state(storage_provider, CallerContext::service_system(FDAE_SERVICE_ID), None);
    store::Host::create_collection(
        &mut seeder,
        CollectionSchema { name: "documents".to_string(), indexes: vec![] },
    )
    .await
    .unwrap();
    store::Host::put(
        &mut seeder,
        "documents".to_string(),
        RecordWriteValue {
            id: "doc-1".to_string(),
            payload: json!({"owner_uuid": "emp-alice"}).to_string().into_bytes(),
        },
    )
    .await
    .unwrap();
}

/// A policy naming a remote relation resolves through `resolve_query_auth`:
/// `get` reaches `HostState.service_proxy`, verifies the returned
/// `RelationshipProof` against the policy's `expected_asserter_did`, and
/// the finalized sieve correctly admits alice's own document.
#[tokio::test]
async fn fdae_remote_relation_fetch_succeeds_through_host_state() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    seed_one_remote_owned_document(storage_provider.clone()).await;

    let identity = Identity::generate().unwrap();
    let asserter_did = substrate::derive_did_key(&identity.public_key());
    let proof = RelationshipProof::sign(
        &identity,
        None,
        "employee",
        "did:key:alice",
        vec!["emp-alice".to_string()],
    )
    .unwrap();
    let stub: Arc<dyn ServiceProxy> =
        Arc::new(StubProxy(Mutex::new(Some(Ok(serde_json::to_value(&proof).unwrap())))));

    let policy = Arc::new(fdae_remote_relation_policy(&asserter_did));
    let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host = HostState::new(
        FDAE_SERVICE_ID.to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage_provider,
        test_blob_provider(),
        alice,
        0,
        test_messaging_context(),
        test_streaming_context(),
        Arc::downgrade(&stub),
        Some(policy),
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    let own =
        store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string()).await.unwrap();
    assert!(own.is_some(), "alice's document must resolve through the real cross-service fetch");
}

/// A fetch failure (the remote proxy call errors) denies the whole read
/// closed rather than falling back to unfiltered or silently empty --
/// `get` must surface an `Err`, not `Ok(None)` masquerading as "not
/// found."
#[tokio::test]
async fn fdae_remote_relation_fetch_failure_denies_closed() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    seed_one_remote_owned_document(storage_provider.clone()).await;

    let stub: Arc<dyn ServiceProxy> =
        Arc::new(StubProxy(Mutex::new(Some(Err(RpcProxyError::Timeout(Duration::from_secs(5)))))));

    let policy = Arc::new(fdae_remote_relation_policy("did:key:zSomeAsserter"));
    let alice = fdae_caller("did:key:alice", vec![fdae_read_cap("documents")]);
    let mut host = HostState::new(
        FDAE_SERVICE_ID.to_string(),
        None,
        Arc::new(KeyStore::new()),
        storage_provider,
        test_blob_provider(),
        alice,
        0,
        test_messaging_context(),
        test_streaming_context(),
        Arc::downgrade(&stub),
        Some(policy),
        false,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    );

    let err = store::Host::get(&mut host, "documents".to_string(), "doc-1".to_string())
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

// -- sagas -----------------------------------------------------------

fn saga_host(read_only: bool, proxy: &Arc<RecordingProxy>, db_dir: &Path) -> HostState {
    HostState::new(
        "driver".to_string(),
        None,
        Arc::new(KeyStore::new()),
        Arc::new(SqliteStorageProvider::new(db_dir, false).unwrap()),
        test_blob_provider(),
        CallerContext::service_system("driver"),
        0,
        test_messaging_context(),
        test_streaming_context(),
        Arc::downgrade(proxy) as Weak<dyn ServiceProxy>,
        None,
        read_only,
        syneroym_rpc::empty_row_authorizer(),
        None,
        syneroym_app_orchestration::empty_resolver(),
    )
}

#[tokio::test]
async fn begin_reaches_the_fake_proxy_with_the_fields_the_wit_carried() {
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = saga_host(false, &proxy, temp_dir.path());

    let saga_id = saga::Host::begin(&mut host, "checkout".to_string(), Some(120)).await.unwrap();
    assert_eq!(saga_id, "saga-1");

    let recorded = proxy.last_saga_begin.lock().unwrap().clone().unwrap();
    assert_eq!(recorded.caller_service_id, "driver");
    assert_eq!(recorded.name, "checkout");
    assert_eq!(recorded.deadline_secs, Some(120));
}

#[tokio::test]
async fn step_reaches_the_fake_proxy_with_the_fields_the_wit_carried() {
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = saga_host(false, &proxy, temp_dir.path());

    let result = saga::Host::step(
        &mut host,
        "saga-1".to_string(),
        CallTarget::Service("did:key:zParticipant".to_string()),
        "saga-participant".to_string(),
        "reserve".to_string(),
        "{\"item\":\"a\"}".to_string(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(result, "null");

    let recorded = proxy.last_saga_step.lock().unwrap().clone().unwrap();
    assert_eq!(recorded.saga_id, "saga-1");
    assert_eq!(recorded.target, QueuedTarget::Service("did:key:zParticipant".to_string()));
    assert_eq!(recorded.interface, "saga-participant");
    assert_eq!(recorded.method, "reserve");
    assert_eq!(recorded.params, serde_json::json!({"item": "a"}));
}

#[tokio::test]
async fn commit_reaches_the_fake_proxy_with_the_saga_id() {
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = saga_host(false, &proxy, temp_dir.path());

    saga::Host::commit(&mut host, "saga-1".to_string()).await.unwrap();

    let recorded = proxy.last_saga_commit.lock().unwrap().clone().unwrap();
    assert_eq!(recorded, ("driver".to_string(), "saga-1".to_string()));
}

#[tokio::test]
async fn compensate_reaches_the_fake_proxy_with_the_saga_id() {
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = saga_host(false, &proxy, temp_dir.path());

    saga::Host::compensate(&mut host, "saga-1".to_string()).await.unwrap();

    let recorded = proxy.last_saga_compensate.lock().unwrap().clone().unwrap();
    assert_eq!(recorded, ("driver".to_string(), "saga-1".to_string()));
}

#[tokio::test]
async fn status_maps_the_fake_proxys_answer_onto_the_wit_record() {
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = saga_host(false, &proxy, temp_dir.path());

    let status = saga::Host::status(&mut host, "saga-1".to_string()).await.unwrap();
    assert_eq!(status.saga_id, "saga-1");
    assert_eq!(status.name, "wf");
    assert_eq!(status.state, WitSagaState::Open);
    assert_eq!(status.steps, 1);
    assert_eq!(status.compensated_steps, 0);
    assert_eq!(status.created_at, 1_000);
    assert_eq!(status.deadline_at, 4_600_000);
    assert!(status.last_error.is_none());
}

/// ADR-0017 §7 is *local* read-only lookups; a stage-4 after-step
/// instance may not originate any proxy call, saga included -- the
/// same refusal `proxy::Host::call`/`enqueue` apply.
#[tokio::test]
async fn a_stage_four_after_step_instance_cannot_open_a_saga() {
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = saga_host(true, &proxy, temp_dir.path());

    let err = saga::Host::begin(&mut host, "checkout".to_string(), None).await.unwrap_err();
    assert!(
        matches!(err, proxy::ProxyError::Internal(_)),
        "expected the stage-4 refusal, got {err:?}"
    );
    assert!(proxy.last_saga_begin.lock().unwrap().is_none(), "the fake proxy must not be reached");
}

#[tokio::test]
async fn a_read_only_host_state_refuses_sign_record() {
    let proxy = Arc::new(RecordingProxy::default());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut host = saga_host(true, &proxy, temp_dir.path());

    let draft = WitRecordDraft {
        version: 1,
        record_type: "listing".to_string(),
        subject: "sub".to_string(),
        payload: "{}".to_string(),
        expires_at_secs: None,
        supersedes: None,
    };

    let err =
        signing::Host::sign_record(&mut host, draft, WitPrincipal::Service).await.unwrap_err();
    assert!(matches!(err, WitSigningError::PermissionDenied));
}

#![allow(clippy::too_many_lines, clippy::cognitive_complexity)]

use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use syneroym_app_orchestration::{ServiceId, StaticInventory, TopologyEpoch, TopologyMode};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_rpc::{Capability, SessionContext};

use super::*;

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

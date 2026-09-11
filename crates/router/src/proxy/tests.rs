use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use dashmap::DashMap;
use iroh::SecretKey;
use syneroym_async_queue::QueueConfig;
use syneroym_core::{dht_registry::MasterAnchorPayload, storage::MockStorage};
use syneroym_identity::{delegation::SCOPE_SERVICE_INSTANCE, substrate};
use syneroym_rpc::{
    AuthLevel, CallerContext, CallerProof, NativeDispatchRegistry, NativeResponse, NativeService,
    QueuedTarget, RpcResult, SERVICE_NOT_FOUND_RPC_CODE, SessionContext,
};

use super::*;
use crate::{HandshakeVerifier, MasterAnchorResolver, proxy_outbox::ProxyOutbox};

fn test_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext::default(),
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

fn base_request(target_service: &str, interface: &str) -> ProxyRequest {
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

fn synthetic_addr() -> EndpointAddr {
    let node_id = SecretKey::generate(&mut rand::rng()).public();
    EndpointAddr::new(node_id)
}

fn empty_registry() -> EndpointRegistry {
    EndpointRegistry::new_mock(Arc::new(MockStorage::new()))
}

fn empty_registry_client() -> Arc<RegistryClient> {
    Arc::new(RegistryClient::new(false, None))
}

fn test_router(hop: Arc<dyn RemoteHop>, registry: EndpointRegistry) -> ProxyRouter {
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
struct GuardedNode {
    router: ProxyRouter,
    service: Arc<RecordingNativeService>,
    _native_dispatch: NativeDispatchRegistry,
    _dir: tempfile::TempDir,
}

async fn guarded_node(with_store: bool) -> GuardedNode {
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

// -- the durable outbox ------------------------------------------------

/// A node that can enqueue: a certified calling service, a real
/// per-service store, and a resolver whose bindings a test can change
/// between attempts.
struct OutboxNode {
    router: Arc<ProxyRouter>,
    registry: EndpointRegistry,
    resolver: Arc<syneroym_app_orchestration::LogicalResolver>,
    target: Arc<RecordingNativeService>,
    outbox: Arc<ProxyOutbox>,
    provider: Arc<syneroym_data_db::SqliteStorageProvider>,
    _native_dispatch: NativeDispatchRegistry,
    dir: tempfile::TempDir,
}

const CALLER: &str = "did:key:zCaller";

/// `target_reachable` decides whether the immediate attempt succeeds:
/// a registered native endpoint answers, while a WASM endpoint with no
/// engine behind it fails with the retryable "sandbox engine
/// unavailable" -- a shutdown-window state, which is exactly the shape
/// that must queue rather than fail the caller.
async fn outbox_node(target_reachable: bool, max_attempts: u8) -> OutboxNode {
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

fn queued_call(target: QueuedTarget, key: &str) -> QueuedCall {
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
    async fn queued(&self) -> Vec<syneroym_async_queue::QueueItem> {
        self.outbox.queue_for(CALLER).await.unwrap().all().unwrap()
    }

    async fn dead_letters(&self) -> Vec<syneroym_async_queue::DeadLetter> {
        self.outbox.queue_for(CALLER).await.unwrap().dead_letters().unwrap()
    }

    fn queue_file_exists(&self) -> bool {
        self.dir.path().join("services").join(CALLER).join("async.db").exists()
    }
}

/// A guest re-enqueueing a key the receiver already ran -- and whose
/// result was too large to retain -- must be told the call succeeded,
/// not handed an error. The fence answers through the error channel
/// because there is no value to return; that is a delivery, and the
/// synchronous probe has to read it the same way the worker does.
#[tokio::test]
async fn an_enqueue_whose_target_already_ran_it_reports_success() {
    let node = outbox_node(true, 50).await;

    // Stand in for the receiver's answer: the fence reports "already
    // ran here, result not retained" through a callee error.
    let already_ran = ProxyError::Callee {
        code: syneroym_async_queue::CALL_RESULT_NOT_RETAINED_RPC_CODE,
        message: "this call already ran here".to_string(),
        data: None,
    };
    assert_eq!(
        proxy_outbox::disposition_of(&already_ran),
        Disposition::Delivered,
        "precondition: this is the code the receiver answers a duplicate with"
    );

    node.target.answer_with.lock().unwrap().replace(already_ran);
    let outcome =
        node.router.enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1")).await;
    assert!(
        outcome.is_ok(),
        "a call the receiver already ran must report success to the guest, got {outcome:?}"
    );
    assert!(!node.queue_file_exists(), "and must not be queued for another delivery attempt");
}

/// The happy path: a reachable target costs one call and zero queue
/// writes. Asserted as "untouched" -- no queue file is created at all.
#[tokio::test]
async fn an_enqueue_to_a_reachable_target_delivers_synchronously_and_never_touches_the_queue() {
    let node = outbox_node(true, 3).await;
    node.router
        .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
        .await
        .unwrap();
    assert_eq!(node.target.invoked.load(Ordering::SeqCst), 1);
    assert!(!node.queue_file_exists(), "a delivered call must not create an outbox");
}

#[tokio::test]
async fn an_enqueue_to_an_unreachable_target_lands_in_that_services_own_outbox() {
    let node = outbox_node(false, 3).await;
    node.router
        .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
        .await
        .unwrap();
    let queued = node.queued().await;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].queue_key, "k1", "the queue key is the idempotency key");
}

/// A failure prevented here by construction: the queue key is the
/// idempotency key, so a caller
/// re-enqueueing the same logical operation gets one item, not two.
#[tokio::test]
async fn a_second_enqueue_for_the_same_key_while_one_is_pending_is_a_no_op() {
    let node = outbox_node(false, 3).await;
    for _ in 0..3 {
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();
    }
    assert_eq!(node.queued().await.len(), 1);
}

/// Without a certificate every delivery attempt would present as
/// anonymous and be refused, so this fails at the call rather than ten
/// hours later at the dead-letter table.
#[tokio::test]
async fn an_enqueue_from_a_service_with_no_unexpired_certificate_is_refused() {
    let node = outbox_node(true, 3).await;
    node.registry.remove_instance_cert(CALLER).await.unwrap();
    let result =
        node.router.enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1")).await;
    assert!(matches!(result, Err(ProxyError::PermissionDenied(_))), "got {result:?}");
    assert!(!node.queue_file_exists());
}

/// The one caller identity that cannot be rebuilt at delivery, and a
/// target that is local and running anyway.
#[tokio::test]
async fn an_enqueue_to_the_calling_service_itself_is_refused() {
    let node = outbox_node(true, 3).await;
    let result =
        node.router.enqueue(queued_call(QueuedTarget::Service(CALLER.to_string()), "k1")).await;
    assert!(matches!(result, Err(ProxyError::UnsupportedTarget(_))), "got {result:?}");
    assert!(!node.queue_file_exists());
}

#[tokio::test]
async fn an_enqueue_to_a_node_level_interface_is_refused() {
    let node = outbox_node(true, 3).await;
    let mut call = queued_call(QueuedTarget::Service("did:key:zNode".into()), "k1");
    call.interface = "orchestrator".to_string();
    let result = node.router.enqueue(call).await;
    assert!(matches!(result, Err(ProxyError::PermissionDenied(_))), "got {result:?}");
}

/// A queued delivery must reach the receiver under the identity the
/// live cross-service path builds, or authorization would silently
/// differ between the immediate attempt and every later one.
#[tokio::test]
async fn a_delivered_call_carries_the_same_caller_identity_the_live_path_builds() {
    let node = outbox_node(true, 3).await;
    node.router
        .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
        .await
        .unwrap();
    let immediate = node.target.last_caller_did.lock().unwrap().clone();
    assert_eq!(immediate.as_deref(), Some("system:did:key:zCaller"));

    // And the worker's own rebuild agrees with it.
    let queued = queued_call(QueuedTarget::Dependency("backend".into()), "k2");
    let rebuilt = node.router.request_from(&queued, "did:key:zTarget".to_string());
    assert_eq!(rebuilt.caller.caller_did, "system:did:key:zCaller");
    assert_eq!(rebuilt.origin, CallOrigin::Guest { service_id: CALLER.to_string() });
    assert_eq!(rebuilt.idempotency_key.as_deref(), Some("k2"));
    assert!(rebuilt.idempotent, "a keyed call is always retry-eligible");
}

/// The entire reason the payload stores intent: a binding re-pushed
/// while the item waited has to take effect on the next attempt.
#[tokio::test]
async fn a_queued_call_resolves_its_dependency_again_at_delivery() {
    use syneroym_app_orchestration::{
        AppInstanceId, LogicalServiceName, ServiceId, TopologyEntry, TopologyEpoch, TopologyKey,
        TopologyMode,
    };
    let node = outbox_node(false, 3).await;
    node.router
        .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
        .await
        .unwrap();
    assert_eq!(node.queued().await.len(), 1);

    // Re-point the dependency at a member that actually answers, the
    // way a re-pushed binding would. It is a deployed service too, so
    // the receiver-side fence can open a store for it.
    let moved_dir = node.dir.path().join("services").join("did:key:zMoved");
    std::fs::create_dir_all(&moved_dir).unwrap();
    std::fs::write(moved_dir.join("state.db"), b"").unwrap();
    let reachable = Arc::new(RecordingNativeService::default());
    node._native_dispatch
        .insert("did:key:zMoved".to_string(), reachable.clone() as Arc<dyn NativeService>);
    node.registry
        .register(
            "did:key:zMoved".to_string(),
            "greeter".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "did:key:zMoved".to_string() },
        )
        .await
        .unwrap();
    node.resolver.register(
        TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
        TopologyEntry {
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new("did:key:zMoved")],
            sharding_strategy: None,
            epoch: TopologyEpoch::default(),
            cache_ttl: Duration::ZERO,
            not_after: None,
        },
    );

    node.router.drain_outboxes_once().await;
    assert_eq!(
        reachable.invoked.load(Ordering::SeqCst),
        1,
        "the re-pushed binding must take effect at delivery"
    );
    assert!(node.queued().await.is_empty(), "a delivered item leaves the outbox");
}

/// A queued call whose dependency no longer resolves is raised by the
/// worker itself before `invoke`, rather than as a proxy error.
#[tokio::test]
async fn a_queued_call_whose_dependency_no_longer_resolves_is_terminal() {
    use syneroym_app_orchestration::{
        AppInstanceId, LogicalServiceName, TopologyEntry, TopologyEpoch, TopologyKey, TopologyMode,
    };
    let node = outbox_node(false, 50).await;
    node.router
        .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
        .await
        .unwrap();

    // The binding goes away entirely.
    node.resolver.register(
        TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
        TopologyEntry {
            mode: TopologyMode::Singleton,
            members: vec![],
            sharding_strategy: None,
            epoch: TopologyEpoch::default(),
            cache_ttl: Duration::ZERO,
            not_after: None,
        },
    );

    node.router.drain_outboxes_once().await;
    let dead = node.dead_letters().await;
    assert_eq!(dead.len(), 1, "an unresolvable dependency is terminal, not retried to budget");
    assert!(node.queued().await.is_empty());
}

/// A callee error has no reader on this path, so recording it is the
/// only way it is ever seen -- the reverse of the synchronous rule.
#[tokio::test]
async fn a_callee_error_on_a_queued_item_dead_letters_rather_than_completing() {
    let node = outbox_node(false, 50).await;
    node.router
        .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
        .await
        .unwrap();
    // Make the target answer definitively rather than being absent.
    node.registry
        .register(
            "did:key:zTarget".to_string(),
            "greeter".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "did:key:zTarget".to_string() },
        )
        .await
        .unwrap();
    node.target.fail_with.store(true, Ordering::SeqCst);

    node.router.drain_outboxes_once().await;
    let dead = node.dead_letters().await;
    assert_eq!(dead.len(), 1);
    assert!(node.queued().await.is_empty());
    // The target refused on its own terms. Pinned explicitly so this
    // test cannot quietly become vacuous: "service not found" is
    // retried rather than dead-lettered, so a scenario that drifted
    // onto that code would assert nothing.
    assert!(
        !dead[0].last_error.contains(&SERVICE_NOT_FOUND_RPC_CODE.to_string()),
        "this case must exercise a genuine callee refusal, not the retried not-found code: {}",
        dead[0].last_error
    );
}

/// The retryable case: the item stays, with an attempt spent.
#[tokio::test]
async fn a_transport_failure_retries_on_the_configured_schedule() {
    let node = outbox_node(false, 50).await;
    node.router
        .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
        .await
        .unwrap();
    node.router.drain_outboxes_once().await;

    let queued = node.queued().await;
    assert_eq!(queued.len(), 1, "a retryable failure keeps the item");
    assert_eq!(queued[0].attempts, 1, "and spends exactly one attempt");
    assert!(node.dead_letters().await.is_empty());
}

/// The poison-pill ceiling: a delivery that never resolves through
/// `fail`/`complete` still consumes a bounded number of claims.
#[tokio::test]
async fn an_item_whose_claim_count_reaches_the_budget_dead_letters() {
    let node = outbox_node(false, 2).await;
    node.router
        .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
        .await
        .unwrap();
    let queue = node.outbox.queue_for(CALLER).await.unwrap();
    // Simulate claims that never resolved -- a crashed worker.
    for _ in 0..3 {
        queue.claim_due(proxy_outbox::now_ms(), 10).unwrap();
    }
    node.router.drain_outboxes_once().await;
    assert_eq!(node.dead_letters().await.len(), 1, "a poison pill must not be handed out forever");
}

/// Nothing removes a service's data directory on undeploy, so an
/// outbox can outlive its service. Delivering would resurrect intent
/// an operator withdrew; dead-lettering would raise noise nobody will
/// act on.
#[tokio::test]
async fn an_item_for_an_undeployed_service_is_completed_not_delivered() {
    let node = outbox_node(false, 50).await;
    node.router
        .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
        .await
        .unwrap();
    assert_eq!(node.queued().await.len(), 1);

    // The calling service is undeployed: its endpoints go away.
    node.registry.remove(CALLER, "greeter").await.ok();
    for (interface, _) in node.registry.lookup_by_service(CALLER) {
        node.registry.remove(CALLER, &interface).await.ok();
    }

    node.router.drain_outboxes_once().await;
    assert!(node.queued().await.is_empty(), "the item must be completed");
    assert!(
        node.dead_letters().await.is_empty(),
        "and silently -- not raised as a dead letter for a service nobody will act on"
    );
    assert_eq!(node.target.invoked.load(Ordering::SeqCst), 0, "and never delivered");
}

/// Cancellation must interrupt a delivery that genuinely never
/// resolves, not merely win a race against the next tick -- a worker
/// waiting out a call to an unreachable peer is exactly the case this
/// queue exists for, so shutdown must not wait for it.
#[tokio::test]
async fn shutdown_abandons_an_in_flight_delivery_rather_than_draining() {
    let node = outbox_node(true, 50).await;

    // Written *straight into the queue*, not through `enqueue`. The
    // probe would claim this key on the way past, and a claim left in
    // flight makes the worker's own delivery hit the fence's
    // "already running here" before it ever reaches the target -- so
    // there would be no in-flight delivery to abandon, structurally,
    // however the timing fell. Bypassing the probe also leaves the
    // target's invocation count at zero, so the wait below can only be
    // satisfied by the worker.
    // A long per-call budget on purpose. With the fixture's usual
    // 500 ms the "blocked" dispatch resolves on its own via the call
    // timeout, the drain returns, and the test passes whether or not
    // cancellation is raced into the delivery -- which is precisely
    // how this test was vacuous twice. At 30 s the only way the worker
    // returns inside the assertion below is by being interrupted.
    let mut item = queued_call(QueuedTarget::Dependency("backend".into()), "worker-only");
    item.timeout_ms = Some(30_000);
    node.outbox.store(&item).await.unwrap();
    assert_eq!(node.queued().await.len(), 1);
    assert_eq!(
        node.target.invoked.load(Ordering::SeqCst),
        0,
        "nothing may have reached the target before the worker starts"
    );

    // The target blocks and is never released during the test, so any
    // delivery the worker starts genuinely never resolves on its own.
    let release = Arc::new(tokio::sync::Notify::new());
    *node.target.hold.lock().unwrap() = Some(release.clone());

    let cancel = CancellationToken::new();
    let worker = {
        let router = node.router.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            router.run_async_worker(Duration::from_millis(5), cancel).await;
        })
    };

    let entered =
        wait_for(Duration::from_secs(5), || node.target.invoked.load(Ordering::SeqCst) >= 1).await;
    assert!(
        entered,
        "the worker never reached the target, so this test would prove nothing about abandoning a \
         delivery"
    );

    // The load-bearing assertion. The delivery in flight right now
    // cannot finish -- nothing will notify `release` before the
    // assertion below. So the worker can only return if cancellation
    // is raced *into* the delivery; a worker that merely checks
    // cancellation between ticks stays inside `drain_outboxes_once`
    // forever and this times out.
    cancel.cancel();
    let stopped = tokio::time::timeout(Duration::from_secs(2), worker).await;
    assert!(
        stopped.is_ok(),
        "shutdown must interrupt a delivery that never resolves, not wait it out"
    );

    // Nothing was lost by not draining: the abandoned item is still
    // on the outbox, and its visibility timeout returns it to a later
    // worker.
    release.notify_waiters();
    assert_eq!(node.queued().await.len(), 1, "the abandoned item must still be on the outbox");
}

/// Polls until `check` holds or the budget runs out.
async fn wait_for<F: FnMut() -> bool>(budget: Duration, mut check: F) -> bool {
    let deadline = std::time::Instant::now() + budget;
    while std::time::Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

// -- the dead-letter tier ----------------------------------------------

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
fn timing_out_guest_request(key: Option<&str>) -> ProxyRequest {
    let mut req = base_request("did:key:zTarget", "greeter");
    req.origin = CallOrigin::Guest { service_id: CALLER.to_string() };
    req.caller = CallerContext::service_system(CALLER);
    req.idempotency_key = key.map(str::to_string);
    req.idempotent = true;
    req.timeout = Some(Duration::from_millis(100));
    req
}

fn failing_guest_request(key: Option<&str>) -> ProxyRequest {
    let mut req = base_request("did:key:zTarget", "greeter");
    req.origin = CallOrigin::Guest { service_id: CALLER.to_string() };
    req.caller = CallerContext::service_system(CALLER);
    req.idempotency_key = key.map(str::to_string);
    req
}

/// The first tier, and the assertion that keeps the whole rule
/// coherent: with no fence there is nothing safe to replay, so there
/// is no row. The caller is alive and holding the error -- this is not
/// silent loss.
///
/// Driven through a real transport-class failure (the call runs out
/// of its deadline against a target that never answers), not a callee
/// refusal -- a callee error is never retried, so a pair built on one
/// would be testing something other than what these names say.
#[tokio::test]
async fn an_unkeyed_call_that_fails_at_the_transport_writes_no_dead_letter() {
    let node = outbox_node(true, 50).await;
    let release = Arc::new(tokio::sync::Notify::new());
    *node.target.hold.lock().unwrap() = Some(release.clone());
    let result = node.router.invoke(timing_out_guest_request(None)).await;
    assert!(
        matches!(result, Err(ProxyError::Transport(_)) | Err(ProxyError::Timeout(_))),
        "the call must have failed at the transport, got {result:?}"
    );
    assert!(
        !node.queue_file_exists(),
        "an unfenced failure must leave no replayable record behind"
    );
    release.notify_waiters();
}

/// The second tier: the row is *additional*, never a substitute for
/// the caller's own error. Same real transport failure as its twin.
#[tokio::test]
async fn a_keyed_call_that_fails_at_the_transport_writes_a_dead_letter_and_still_returns_its_error()
{
    let node = outbox_node(true, 50).await;
    let release = Arc::new(tokio::sync::Notify::new());
    *node.target.hold.lock().unwrap() = Some(release.clone());
    let result = node.router.invoke(timing_out_guest_request(Some("k1"))).await;
    assert!(
        matches!(result, Err(ProxyError::Transport(_)) | Err(ProxyError::Timeout(_))),
        "the caller must still get its own transport error, got {result:?}"
    );
    release.notify_waiters();

    let dead = node.dead_letters().await;
    assert_eq!(dead.len(), 1, "a keyed failure must also be recorded for an operator");
    assert_eq!(dead[0].queue_key, "k1");
    assert!(node.queued().await.is_empty(), "and must not linger in the outbox");
}

/// A keyed call the *target itself* refused. Distinct from exhaustion
/// above: a callee error is never retried, so nothing is exhausted --
/// but it is still an answer the target produced, so it still earns a
/// row an operator can see.
#[tokio::test]
async fn a_keyed_call_a_target_refuses_writes_a_dead_letter_without_retrying() {
    let node = outbox_node(true, 50).await;
    node.target.fail_with.store(true, Ordering::SeqCst);
    let result = node.router.invoke(failing_guest_request(Some("k1"))).await;
    assert!(matches!(result, Err(ProxyError::Callee { .. })), "got {result:?}");
    assert_eq!(
        node.target.invoked.load(Ordering::SeqCst),
        1,
        "a callee error is definitive and must not be retried"
    );
    assert_eq!(node.dead_letters().await.len(), 1);
}

/// The fence's own answers are not delivery failures, so none of them
/// may leave an operator-visible dead letter. "Already running here"
/// is the sharp case: the call is succeeding on another task right
/// now, and a row for it would be indistinguishable from a genuine
/// exhausted delivery and never cleared.
#[test]
fn the_fences_own_answers_never_earn_a_dead_letter() {
    for code in [
        syneroym_async_queue::CALL_ALREADY_RUNNING_RPC_CODE,
        syneroym_async_queue::CALL_RESULT_NOT_RETAINED_RPC_CODE,
    ] {
        let error = ProxyError::Callee { code, message: "fence".to_string(), data: None };
        assert!(!target_produced(&error), "code {code} must not earn a dead letter");
    }
    // Nor may a refusal raised before anything was attempted:
    // replaying it just re-earns the refusal.
    assert!(!target_produced(&ProxyError::PermissionDenied("no store".to_string())));
    assert!(!target_produced(&ProxyError::Internal("no storage provider".to_string())));

    // A not-found target is worth a row, and the two classifiers must
    // agree about that: the queued path retries it as "not yet", so
    // the synchronous path must not silently drop it.
    let not_found = ProxyError::ServiceNotFound("mid-restart".to_string());
    assert!(target_produced(&not_found));
    assert_eq!(
        proxy_outbox::disposition_of(&not_found),
        Disposition::Retry,
        "the two classifiers must not disagree about one error"
    );

    // And a real answer from the target does.
    assert!(target_produced(&ProxyError::Callee {
        code: -32010,
        message: "denied by the target".to_string(),
        data: None,
    }));
    assert!(target_produced(&ProxyError::Transport("peer went away".to_string())));
}

/// Queue growth must be bounded. The
/// dead-letter table was; the *pending* outbox was not, so a guest
/// aimed at an unreachable target could hold unbounded rows for the
/// whole attempt budget. It refuses rather than evicting: a pending
/// item is work somebody still expects to happen.
#[tokio::test]
async fn a_full_outbox_refuses_further_enqueues_rather_than_evicting() {
    let node = outbox_node(false, 50).await;
    let mut cfg = node.outbox.config().clone();
    cfg.max_pending_rows = 2;
    let tight = Arc::new(ProxyOutbox::new(
        node.provider.clone(),
        Arc::new(syneroym_data_keystore::KeyStore::new()),
        node.resolver.clone(),
        cfg,
    ));
    for i in 0..2 {
        tight
            .store(&queued_call(QueuedTarget::Service("did:key:zTarget".into()), &format!("k{i}")))
            .await
            .unwrap();
    }
    let refused =
        tight.store(&queued_call(QueuedTarget::Service("did:key:zTarget".into()), "k2")).await;
    assert!(
        matches!(refused, Err(ProxyError::UnsupportedTarget(_))),
        "a full outbox must refuse, got {refused:?}"
    );
    assert_eq!(
        tight.queue_for(CALLER).await.unwrap().all().unwrap().len(),
        2,
        "and must not have evicted anything to make room"
    );
}

/// One permanently broken recipient must not be able to evict every
/// other conversation's dead letters, which is what scoping the cap by
/// target buys.
#[tokio::test]
async fn the_dlq_cap_is_scoped_per_target() {
    let node = outbox_node(false, 50).await;
    let queue = node.outbox.queue_for(CALLER).await.unwrap();

    // Two targets, and a cap that the first one alone would blow past.
    for i in 0..4 {
        let call = queued_call(QueuedTarget::Service("did:key:zNoisy".into()), &format!("n{i}"));
        node.outbox.record_dead_letter(&call, "unreachable").await.unwrap();
    }
    let quiet = queued_call(QueuedTarget::Service("did:key:zQuiet".into()), "q0");
    node.outbox.record_dead_letter(&quiet, "unreachable").await.unwrap();

    let keys: Vec<String> =
        queue.dead_letters().unwrap().into_iter().map(|d| d.queue_key).collect();
    assert!(
        keys.contains(&"q0".to_string()),
        "the quiet target's dead letter must survive the noisy one's overflow, got {keys:?}"
    );
}

/// The property that makes replay safe at all, and the reason a dead
/// letter needs a key to exist: replaying a call the target already
/// ran must not run it twice. Drives a replay against a target that
/// already executed the original.
#[tokio::test]
async fn a_replayed_call_is_deduplicated_at_the_receiver_if_the_first_one_landed() {
    let node = outbox_node(true, 50).await;

    // The original lands.
    node.router
        .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
        .await
        .unwrap();
    assert_eq!(node.target.invoked.load(Ordering::SeqCst), 1);

    // An operator finds a dead letter for the same logical operation
    // and replays it -- the situation replay exists for, where it is
    // not knowable whether the first attempt landed.
    let call = queued_call(QueuedTarget::Dependency("backend".into()), "k1");
    node.outbox.record_dead_letter(&call, "looked unreachable").await.unwrap();
    let dead = node.outbox.dead_letters(CALLER).await.unwrap();
    assert_eq!(dead.len(), 1);
    node.outbox.replay_dead_letter(CALLER, dead[0].id).await.unwrap();

    node.router.drain_outboxes_once().await;
    assert_eq!(
        node.target.invoked.load(Ordering::SeqCst),
        1,
        "the receiver's record of the first call must stop the replay re-executing it"
    );
}

fn keyed_request(key: &str) -> ProxyRequest {
    let mut req = base_request("svc-a", "greeter");
    req.caller = CallerContext::service_system("svc-caller");
    req.idempotency_key = Some(key.to_string());
    req
}

/// The `invoke_local` entry point. Together with the
/// `dispatch_json_rpc_once` case, this is what makes "one guard, both
/// entry points" a fact rather than an intention.
#[tokio::test]
async fn a_local_call_is_deduplicated_by_the_proxy() {
    let node = guarded_node(true).await;
    let first = node.router.invoke(keyed_request("k1")).await.unwrap();
    assert_eq!(first, Value::String("ok".to_string()));
    assert_eq!(node.service.invoked.load(Ordering::SeqCst), 1);

    let repeat = node.router.invoke(keyed_request("k1")).await.unwrap();
    assert_eq!(repeat, first, "the duplicate must get the first call's own result");
    assert_eq!(
        node.service.invoked.load(Ordering::SeqCst),
        1,
        "the target must not run a second time"
    );
}

/// A different key is a different call, so the target does run again
/// -- the fence must not swallow genuinely distinct work.
#[tokio::test]
async fn a_different_key_is_a_different_call() {
    let node = guarded_node(true).await;
    node.router.invoke(keyed_request("k1")).await.unwrap();
    node.router.invoke(keyed_request("k2")).await.unwrap();
    assert_eq!(node.service.invoked.load(Ordering::SeqCst), 2);
}

/// An unkeyed call is untouched by any of this, including on a node
/// that has a store: it executes every time, exactly as before.
#[tokio::test]
async fn an_unkeyed_call_is_never_deduplicated() {
    let node = guarded_node(true).await;
    let mut req = base_request("svc-a", "greeter");
    req.caller = CallerContext::service_system("svc-caller");
    node.router.invoke(req.clone()).await.unwrap();
    node.router.invoke(req).await.unwrap();
    assert_eq!(node.service.invoked.load(Ordering::SeqCst), 2);
}

/// Coordinator mode: no storage provider at all, so there is nowhere
/// to remember a key. Refused rather than executed unfenced.
#[tokio::test]
async fn a_keyed_call_is_refused_on_a_node_with_no_storage_provider() {
    let node = guarded_node(false).await;
    let result = node.router.invoke(keyed_request("k1")).await;
    assert!(matches!(result, Err(ProxyError::Internal(_))), "got {result:?}");
    assert_eq!(node.service.invoked.load(Ordering::SeqCst), 0);
}

/// A denial or an unknown service happens before the target runs, so
/// it must leave no claim behind -- otherwise a corrected retry is
/// blocked for the whole claim window for something that never ran.
#[tokio::test]
async fn a_failure_before_the_target_ran_leaves_no_claim_behind() {
    let node = guarded_node(true).await;

    // A guest reaching another service's native capability is denied
    // by the capability gate, which runs before any dispatch.
    let mut denied = keyed_request("k1");
    denied.interface = "data-layer".to_string();
    denied.target_service = "svc-a".to_string();
    denied.origin = CallOrigin::Guest { service_id: "svc-caller".to_string() };
    assert!(matches!(node.router.invoke(denied).await, Err(ProxyError::PermissionDenied(_))));

    // The same key, corrected, must still be free to run.
    let corrected = node.router.invoke(keyed_request("k1")).await;
    assert!(corrected.is_ok(), "a corrected retry must not be blocked: {corrected:?}");
    assert_eq!(node.service.invoked.load(Ordering::SeqCst), 1);
}

/// The invariant that makes the local `system:<id>` and the remote DID
/// namespaces safe to keep disjoint: whether a target is local or
/// remote never changes between attempts, so one caller reaches one
/// target under one identity every time.
#[tokio::test]
async fn one_caller_reaches_one_target_under_one_identity_on_every_attempt() {
    let node = guarded_node(true).await;
    node.router.invoke(keyed_request("k1")).await.unwrap();
    let first_identity = node.service.last_caller_did.lock().unwrap().clone();

    // A second, differently-keyed call from the same caller to the
    // same target arrives under the same identity.
    node.router.invoke(keyed_request("k2")).await.unwrap();
    assert_eq!(*node.service.last_caller_did.lock().unwrap(), first_identity);
    assert_eq!(first_identity.as_deref(), Some("system:svc-caller"));
}

#[derive(Debug, Default)]
struct RecordingNativeService {
    invoked: AtomicUsize,
    last_caller_did: Mutex<Option<String>>,
    /// Makes the target answer definitively rather than being absent,
    /// so the queued path's callee-error classification can be driven.
    fail_with: std::sync::atomic::AtomicBool,
    /// When set, `dispatch` blocks until this is notified -- a
    /// delivery that genuinely never resolves, which is the only way
    /// to test that shutdown interrupts one.
    hold: Mutex<Option<Arc<tokio::sync::Notify>>>,
    /// When set, `dispatch` answers with this exact error, so a test
    /// can drive a specific reserved code the receiver would produce.
    answer_with: Mutex<Option<ProxyError>>,
    /// The `(interface, method, params)` of the most recent dispatch --
    /// lets a saga test confirm the walk actually called
    /// `saga-undo-<method>`, not the forward method again.
    last_invocation: Mutex<Option<(String, String, Value)>>,
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
enum MockOutcome {
    Success(Value),
    Transport,
    Callee { code: i32, message: String },
}

#[derive(Debug, Default)]
struct MockHop {
    calls: AtomicUsize,
    last_preamble: Mutex<Option<RoutePreamble>>,
    outcomes: Mutex<std::collections::VecDeque<MockOutcome>>,
}

impl MockHop {
    fn with_outcomes(outcomes: Vec<MockOutcome>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            last_preamble: Mutex::new(None),
            outcomes: Mutex::new(outcomes.into()),
        }
    }

    fn call_count(&self) -> usize {
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

// -- local native dispatch -------------------------------------------

#[tokio::test]
async fn invoke_local_native_reaches_registered_service_with_caller_identity() {
    let registry = empty_registry();
    registry
        .register(
            "svc-a".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-a".to_string() },
        )
        .await
        .unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-a".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-a", "data-layer");
    req.caller = test_caller("did:key:zCallerOne");
    let result = router.invoke(req).await.unwrap();
    assert_eq!(result, Value::String("ok".to_string()));
    assert_eq!(service.invoked.load(Ordering::SeqCst), 1);
    assert_eq!(service.last_caller_did.lock().unwrap().as_deref(), Some("did:key:zCallerOne"));
}

// -- unknown service ---------------------------------------------------

#[tokio::test]
async fn unknown_service_is_service_not_found_and_hop_never_called() {
    let hop = Arc::new(MockHop::default());
    let router = test_router(hop.clone(), empty_registry());

    let result = router.invoke(base_request("no-such-service", "greet")).await;
    assert!(matches!(result, Err(ProxyError::ServiceNotFound(_))));
    assert_eq!(hop.call_count(), 0);
}

// -- native capability gate --------------------------------------------

#[tokio::test]
async fn guest_cross_service_native_capability_is_denied_and_never_dispatched() {
    let registry = empty_registry();
    registry
        .register(
            "svc-b".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-b".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-b", "data-layer");
    req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
    let result = router.invoke(req).await;
    assert!(matches!(result, Err(ProxyError::PermissionDenied(_))));
    assert_eq!(service.invoked.load(Ordering::SeqCst), 0);
}

/// A guest that requests the interface by its `short_hash` (what
/// `EndpointRegistry::lookup` also accepts and canonicalizes back to the
/// literal name) must be denied exactly like the literal-name request
/// above -- `short_hash` is an unsalted SHA-256 prefix, so it's
/// guest-computable and must not bypass the gate.
#[tokio::test]
async fn guest_cross_service_native_capability_is_denied_via_short_hash_too() {
    let registry = empty_registry();
    registry
        .register(
            "svc-b".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-b".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-b", &util::short_hash("data-layer"));
    req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
    let result = router.invoke(req).await;
    assert!(matches!(result, Err(ProxyError::PermissionDenied(_))));
    assert_eq!(service.invoked.load(Ordering::SeqCst), 0);
}

/// A0-01: `orchestrator`/`security` are node-level (registered under the
/// node's own DID, not any deployed service's), so unlike the
/// `NATIVE_CAPABILITY_INTERFACES` gate above there is no same-service
/// exemption -- a guest whose own service is the node's own DID (which
/// cannot legitimately happen, but a guest freely chooses
/// `target_service`) must still be denied. Guards against a guest whose
/// service holds an installed instance certificate (ADR-0020 §1) walking
/// its now-verified identity into `orchestrator` (gated since the
/// deploy-grant admission gate) or `security` (also gated on
/// `substrate/admin` now) -- neither of which a guest could reach at all
/// before that certificate mechanism existed, since a guest-origin call
/// always presented anonymous.
#[tokio::test]
async fn guest_cannot_reach_node_level_orchestrator_or_security_through_the_proxy() {
    let registry = empty_registry();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("node-did".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    for interface in ["orchestrator", "security"] {
        let mut req = base_request("node-did", interface);
        req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
        let result = router.invoke(req).await;
        assert!(
            matches!(result, Err(ProxyError::PermissionDenied(_))),
            "interface '{interface}' must be denied, got {result:?}"
        );
    }
    assert_eq!(service.invoked.load(Ordering::SeqCst), 0);
}

/// The empty-interface convenience ("the destination resolves the
/// caller's one app-declared interface") is for an
/// external caller that cannot know a service's interface names -- the
/// gateway or coordinator resolving a hostname. A WASM guest always
/// names the interface it wants, so an empty one must be denied before
/// `registry.lookup` gets a chance to resolve it -- the target
/// registers exactly one app-declared interface here, so a resolve
/// would otherwise have succeeded.
#[tokio::test]
async fn guest_with_an_empty_interface_is_denied_before_resolution() {
    let registry = empty_registry();
    registry
        .register(
            "svc-b".to_string(),
            "default".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "svc-b".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-b", "");
    req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
    let result = router.invoke(req).await;
    assert!(matches!(result, Err(ProxyError::PermissionDenied(_))), "{result:?}");
    assert_eq!(service.invoked.load(Ordering::SeqCst), 0);
}

/// The case that would fail against a `caller_did`-based comparison
/// instead of the guest's raw `component_id` -- `service_system` puts
/// `"system:svc-a"` in `caller_did`, which would never equal a plain
/// service id.
#[tokio::test]
async fn guest_reaching_its_own_native_capability_is_allowed() {
    let registry = empty_registry();
    registry
        .register(
            "svc-a".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-a".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-a".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-a", "data-layer");
    req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
    req.caller = CallerContext::service_system("svc-a");
    let result = router.invoke(req).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(service.invoked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn guest_reaching_a_non_native_interface_on_another_service_is_allowed() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("svc-b", "some-app-interface");
    req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
    let result = router.invoke_remote_at(&synthetic_addr(), &req).await;
    assert!(result.is_ok());
}

/// The relationship-proof-fetch shape -- a guard against a future
/// tightening of the gate silently re-breaking that fetch.
#[tokio::test]
async fn native_origin_cross_service_data_layer_call_is_allowed_by_the_gate() {
    let registry = empty_registry();
    registry
        .register(
            "svc-b".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "svc-b".to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let service = Arc::new(RecordingNativeService::default());
    native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("svc-b", "data-layer");
    req.origin = CallOrigin::Native { service_id: None };
    let result = router.invoke(req).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(service.invoked.load(Ordering::SeqCst), 1);
}

// -- remote dispatch: retry -------------------------------------------

#[tokio::test]
async fn idempotent_call_retries_transport_failures_up_to_max_attempts() {
    let hop = Arc::new(MockHop::with_outcomes(vec![
        MockOutcome::Transport,
        MockOutcome::Transport,
        MockOutcome::Transport,
    ]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("remote-svc", "greet");
    req.idempotent = true;
    let result = router.invoke_remote_at(&synthetic_addr(), &req).await;
    assert!(matches!(result, Err(ProxyError::Transport(_))));
    assert_eq!(hop.call_count(), 3, "must retry up to max_attempts (3) for an idempotent call");
}

#[tokio::test]
async fn non_idempotent_call_never_retries_transport_failures() {
    let hop =
        Arc::new(MockHop::with_outcomes(vec![MockOutcome::Transport, MockOutcome::Transport]));
    let router = test_router(hop.clone(), empty_registry());

    let req = base_request("remote-svc", "greet"); // idempotent: false (default)
    let result = router.invoke_remote_at(&synthetic_addr(), &req).await;
    assert!(matches!(result, Err(ProxyError::Transport(_))));
    assert_eq!(hop.call_count(), 1, "a non-idempotent call must never be retried");
}

#[tokio::test]
async fn callee_error_is_never_retried_even_when_idempotent() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Callee {
        code: -32010,
        message: "denied".to_string(),
    }]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("remote-svc", "greet");
    req.idempotent = true;
    let result = router.invoke_remote_at(&synthetic_addr(), &req).await;
    assert!(matches!(result, Err(ProxyError::Callee { code: -32010, .. })));
    assert_eq!(hop.call_count(), 1, "a definitive callee error must never be retried");
}

#[tokio::test]
async fn idempotent_call_stops_retrying_once_it_succeeds() {
    let hop = Arc::new(MockHop::with_outcomes(vec![
        MockOutcome::Transport,
        MockOutcome::Success(Value::String("recovered".to_string())),
    ]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("remote-svc", "greet");
    req.idempotent = true;
    let result = router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();
    assert_eq!(result, Value::String("recovered".to_string()));
    assert_eq!(hop.call_count(), 2);
}

// -- remote dispatch: proof forwarding ---------------------------------

#[tokio::test]
async fn caller_with_proof_forwards_it_verbatim_on_the_outbound_preamble() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let router = test_router(hop.clone(), empty_registry());

    let mut req = base_request("remote-svc", "greet");
    req.caller.proof =
        Some(CallerProof { pubkey_hex: "deadbeef".to_string(), delegation_json: None });
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey.as_deref(), Some("deadbeef"));
}

#[tokio::test]
async fn caller_without_proof_presents_the_nodes_own_identity() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let identity = Arc::new(Identity::generate().unwrap());
    let expected_pubkey = hex::encode(identity.public_key().to_bytes());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        identity,
        RetryPolicy::default(),
    );

    let req = base_request("remote-svc", "greet"); // caller.proof: None
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey.as_deref(), Some(expected_pubkey.as_str()));
}

/// A guest never carries a proof (`CallerContext::service_system`), so
/// unlike the `CallOrigin::Native` case above, a cross-node guest call
/// must not launder itself as the node's own identity -- that would let
/// the destination attribute the call to a real, potentially privileged
/// DID (e.g. its `admin_ucan_root`) with no marker that a guest
/// originated it.
#[tokio::test]
async fn guest_without_proof_forwards_as_anonymous_not_node_identity() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let identity = Arc::new(Identity::generate().unwrap());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "greet"); // caller.proof: None
    req.origin = CallOrigin::Guest { service_id: "guest-component".to_string() };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey, None);
}

/// The self-proxy branch (`host_capabilities.rs`) forwards
/// a `CallOrigin::Guest` request that legitimately carries the real
/// caller's proof when the target is the guest's own service. If that
/// request ever falls through `invoke`'s local-registry lookup (an
/// interface the local `EndpointRegistry` hasn't got, even for the
/// guest's raw own `component_id` -- `check_native_capability_gate`
/// only restricts *native-capability* interfaces cross-service, not
/// this fallback), it must not present that proof, or this node's own
/// identity, to a remote destination the guest fully chose the
/// `(interface, method, params)` for. Same invariant as
/// `guest_without_proof_forwards_as_anonymous_not_node_identity`, now
/// pinned for a guest caller that *does* carry a proof.
#[tokio::test]
async fn guest_with_proof_still_forwards_as_anonymous_not_the_real_proof() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let identity = Arc::new(Identity::generate().unwrap());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("guest-component", "some-unregistered-iface");
    req.caller.proof =
        Some(CallerProof { pubkey_hex: "deadbeef".to_string(), delegation_json: None });
    req.origin = CallOrigin::Guest { service_id: "guest-component".to_string() };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey, None,
        "a guest-origin call must never present a proof (its own or the node's) to a remote \
         destination, even when `req.caller.proof` is `Some` -- otherwise a guest could launder a \
         real caller's identity onto the wire by steering a self-proxy call onto an interface the \
         local registry misses"
    );
}

#[derive(Debug)]
struct EmptyAnchorResolver;
#[async_trait::async_trait]
impl MasterAnchorResolver for EmptyAnchorResolver {
    async fn resolve_master_anchor(
        &self,
        _master_id: &str,
    ) -> Result<MasterAnchorPayload, anyhow::Error> {
        Ok(MasterAnchorPayload::default())
    }
}

/// The slice's core claim: a service holding an installed instance
/// certificate makes a guest-origin remote call under its own member
/// master, not anonymous and not the node's identity -- and the
/// destination's handshake (fed the exact preamble this router
/// constructs) resolves that master, matching `HandshakeVerifier`'s
/// contract end to end.
#[tokio::test]
async fn a_guest_call_travels_under_its_services_member_master_not_the_node_identity() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "guest-with-cert".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "greet");
    req.origin = CallOrigin::Guest { service_id: service_id.clone() };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(hex::encode(instance.public_key().to_bytes()).as_str())
    );
    assert!(preamble.delegation.is_some());

    let verified = HandshakeVerifier::verify_preamble(&preamble, &EmptyAnchorResolver)
        .await
        .expect("the destination's handshake must admit a service-instance certificate");
    assert_eq!(verified.master_did, member_master_did);
}

/// The unchanged path (the migration guarantee): a service with no
/// installed certificate presents nothing, exactly like before this arm
/// existed.
#[tokio::test]
async fn a_guest_call_from_a_service_without_a_certificate_is_still_anonymous() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let registry = empty_registry();
    registry.set_owner("no-cert-svc".to_string(), "did:key:zOwner".to_string()).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "greet");
    req.origin = CallOrigin::Guest { service_id: "no-cert-svc".to_string() };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey, None);
    assert!(preamble.delegation.is_none());
}

/// A0-07: an already-expired installed certificate must fall back to
/// anonymous exactly like no certificate at all -- not get attached and
/// then hard-rejected at the destination (`route_handler/io.rs` rejects
/// any connection whose delegation fails to verify), which would turn a
/// missed renewal into an outage for passthrough/relay calls that
/// tolerated anonymous before this certificate mechanism existed.
#[tokio::test]
async fn a_guest_call_from_a_service_with_an_expired_certificate_is_anonymous_not_rejected() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "expired-cert-svc".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

    let member_master = Identity::generate().unwrap();
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        0,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    assert!(cert.is_expired());
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "greet");
    req.origin = CallOrigin::Guest { service_id: service_id.clone() };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(preamble.pubkey, None);
    assert!(preamble.delegation.is_none());
}

/// The transport half: a substrate-internal call made on a deployed
/// service's behalf (the FDAE relationship-proof fetch) presents that
/// service's certified instance key, not the node's -- the same
/// reasoning applied to the guest-origin arm, now at the
/// `(None, Native { service_id: Some(_) })` site.
#[tokio::test]
async fn a_native_origin_call_on_a_services_behalf_presents_that_services_instance_key() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "hr-svc".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

    let member_master = Identity::generate().unwrap();
    let member_master_did = substrate::derive_did_key(&member_master.public_key());
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity,
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "data-layer");
    req.caller.proof = None;
    req.origin = CallOrigin::Native { service_id: Some(service_id.clone()) };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(hex::encode(instance.public_key().to_bytes()).as_str())
    );
    assert!(preamble.delegation.is_some());

    let verified = HandshakeVerifier::verify_preamble(&preamble, &EmptyAnchorResolver)
        .await
        .expect("the destination's handshake must admit a service-instance certificate");
    assert_eq!(verified.master_did, member_master_did);
}

/// When the caller already carries a signed proof (a forwarded chain),
/// a `Native` call must forward that proof verbatim
/// -- never substitute the service's own identity -- so the destination
/// can re-derive `subject_did`/`anchor_did` from the real chain. This is
/// what keeps FDAE's cross-service fetch authorizing the *real* caller
/// rather than the relaying service.
#[tokio::test]
async fn a_native_origin_call_with_a_caller_proof_still_forwards_the_proof_verbatim() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "hr-svc".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();
    let member_master = Identity::generate().unwrap();
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity,
        RetryPolicy::default(),
    );

    let forwarded_pubkey_hex = "aabbccdd".to_string();
    let mut req = base_request("remote-svc", "data-layer");
    req.caller.proof =
        Some(CallerProof { pubkey_hex: forwarded_pubkey_hex.clone(), delegation_json: None });
    req.origin = CallOrigin::Native { service_id: Some(service_id.clone()) };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(forwarded_pubkey_hex.as_str()),
        "an already-present caller proof must be forwarded verbatim, never replaced by the \
         relaying service's own certified identity"
    );
}

/// A native-origin call made on nobody's behalf (`service_id: None`,
/// substrate-internal tooling, tests) keeps presenting the node's own
/// identity, exactly as before this arm existed.
#[tokio::test]
async fn a_native_origin_call_with_no_service_id_still_presents_the_node_identity() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        empty_registry(),
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity.clone(),
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "data-layer");
    req.caller.proof = None;
    req.origin = CallOrigin::Native { service_id: None };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(hex::encode(node_identity.public_key().to_bytes()).as_str()),
        "a node-level native call (no service_id) must still present the node's own identity"
    );
    assert!(preamble.delegation.is_none());
}

/// The same fallback A0 gave the guest arm: an installed-but-expired
/// certificate must fall back to the node identity, not anonymous -- a
/// substrate-internal call has always presented *something*, and
/// dropping to anonymous would break native-dispatch destinations that
/// reject an anonymous caller outright.
#[tokio::test]
async fn a_native_origin_call_with_an_expired_certificate_falls_back_to_the_node_identity() {
    let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
    let node_identity = Arc::new(Identity::generate().unwrap());
    let registry = empty_registry();

    let owner_did = "did:key:zMemberOwner".to_string();
    let service_id = "expired-cert-native-svc".to_string();
    registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

    let member_master = Identity::generate().unwrap();
    let instance = node_identity.derive_service_identity(&owner_did, &service_id);
    let cert = DelegationCertificate::issue(
        &member_master,
        instance.public_key(),
        0,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    assert!(cert.is_expired());
    registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Arc::downgrade(&native_dispatch),
        Weak::new(),
        hop.clone(),
        node_identity.clone(),
        RetryPolicy::default(),
    );

    let mut req = base_request("remote-svc", "data-layer");
    req.caller.proof = None;
    req.origin = CallOrigin::Native { service_id: Some(service_id.clone()) };
    router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

    let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
    assert_eq!(
        preamble.pubkey.as_deref(),
        Some(hex::encode(node_identity.public_key().to_bytes()).as_str()),
        "an expired certificate must fall back to the node identity, not anonymous -- a \
         native-origin call has always presented something"
    );
    assert!(preamble.delegation.is_none());
}

// -- sagas ---------------------------------------------------------

/// A node that can drive a saga: a certified calling service, a real
/// per-service saga log, and a reachable target -- the same shape
/// `outbox_node` builds for `enqueue`, since a saga step is `invoke`
/// plus a log write over the identical wiring.
struct SagaNode {
    router: Arc<ProxyRouter>,
    registry: EndpointRegistry,
    target: Arc<RecordingNativeService>,
    sagas: Arc<SagaStore>,
    dedup_guard: Arc<crate::CallDedupGuard>,
    _native_dispatch: NativeDispatchRegistry,
    _dir: tempfile::TempDir,
}

const SAGA_CALLER: &str = "did:key:zSagaCaller";
const SAGA_TARGET: &str = "did:key:zSagaTarget";

fn saga_config(dispatch_epoch_timeout_secs: u64) -> syneroym_async_queue::SagaConfig {
    syneroym_async_queue::SagaConfig::from(&syneroym_core::config::AppSandboxRole {
        dispatch_epoch_timeout_secs,
        ..syneroym_core::config::AppSandboxRole::default()
    })
}

async fn saga_node(dispatch_epoch_timeout_secs: u64) -> SagaNode {
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

fn saga_step_request(saga_id: &str, target: &str) -> SagaStepRequest {
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

async fn begun_saga(node: &SagaNode) -> String {
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

#[tokio::test]
async fn a_step_records_its_intent_before_the_call_lands() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;

    node.router.saga_step(saga_step_request(&saga_id, SAGA_TARGET)).await.unwrap();

    let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
    let step = log.next_uncompensated_step(&saga_id).unwrap().unwrap();
    assert_eq!(step.idx, 0);
    assert_eq!(step.method, "reserve");
    assert_eq!(node.target.invoked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_step_whose_call_fails_is_recorded_as_failed_and_still_returns_its_error() {
    let node = saga_node(5).await;
    node.target.fail_with.store(true, Ordering::SeqCst);
    let saga_id = begun_saga(&node).await;

    let result = node.router.saga_step(saga_step_request(&saga_id, SAGA_TARGET)).await;
    assert!(result.is_err(), "the caller must still see the failure");

    let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
    assert!(
        log.next_uncompensated_step(&saga_id).unwrap().is_none(),
        "a failed step is never compensated"
    );
}

#[tokio::test]
async fn a_step_on_an_unknown_saga_is_refused() {
    let node = saga_node(5).await;
    let result = node.router.saga_step(saga_step_request("no-such-saga", SAGA_TARGET)).await;
    assert!(result.is_err(), "unexpected success");
}

#[tokio::test]
async fn a_step_against_a_node_level_interface_is_refused() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;

    for interface in ["orchestrator", "security", "supervisor"] {
        let mut req = saga_step_request(&saga_id, SAGA_TARGET);
        req.interface = interface.to_string();
        let result = node.router.saga_step(req).await;
        assert!(
            matches!(result, Err(ProxyError::PermissionDenied(_))),
            "interface '{interface}' must be refused, got {result:?}"
        );
    }
}

#[tokio::test]
async fn a_step_against_the_callers_own_service_is_refused() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;

    let result = node.router.saga_step(saga_step_request(&saga_id, SAGA_CALLER)).await;
    assert!(
        matches!(result, Err(ProxyError::UnsupportedTarget(_))),
        "a self-target must be refused, got {result:?}"
    );
}

#[tokio::test]
async fn a_step_with_no_timeout_takes_a_budget_inside_the_guests_epoch_not_the_proxys_default() {
    // The node's epoch is 5s, so the derived step budget is 4s -- well
    // under `DEFAULT_PROXY_CALL_TIMEOUT` (30s). A target that blocks
    // past 4s but under 30s must still time out, which only holds if
    // the derived budget -- not the proxy default -- was applied.
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    node.target.hold.lock().unwrap().replace(Arc::new(tokio::sync::Notify::new()));

    let started = Instant::now();
    let result = node.router.saga_step(saga_step_request(&saga_id, SAGA_TARGET)).await;
    assert!(matches!(result, Err(ProxyError::Timeout(_))), "got {result:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the step must not wait out the proxy's own 30s default"
    );
}

#[tokio::test]
async fn a_step_with_an_explicit_timeout_keeps_it() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    let mut req = saga_step_request(&saga_id, SAGA_TARGET);
    req.timeout_ms = Some(50);
    node.target.hold.lock().unwrap().replace(Arc::new(tokio::sync::Notify::new()));

    let started = Instant::now();
    let result = node.router.saga_step(req).await;
    assert!(matches!(result, Err(ProxyError::Timeout(_))), "got {result:?}");
    assert!(started.elapsed() < Duration::from_secs(2), "the explicit 50ms budget must apply");
}

#[tokio::test]
async fn begin_is_refused_for_a_service_with_no_unexpired_instance_certificate() {
    let node = saga_node(5).await;
    node.registry.remove_instance_cert(SAGA_CALLER).await.unwrap();

    let result = node
        .router
        .saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf".to_string(),
            deadline_secs: None,
        })
        .await;
    assert!(
        matches!(result, Err(ProxyError::PermissionDenied(_))),
        "an uncertified caller must be refused, got {result:?}"
    );
}

/// A managed instance's certificate is renewed on every supervisor
/// pass, so its own current expiry cannot decide whether a long
/// deadline is sound -- `begin` warns rather than refusing.
#[tokio::test]
async fn begin_warns_but_proceeds_when_the_deadline_outlives_the_certificate() {
    let node = saga_node(5).await;
    let cert = DelegationCertificate::issue(
        &Identity::generate().unwrap(),
        Identity::generate().unwrap().public_key(),
        10,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    node.registry.set_instance_cert(SAGA_CALLER.to_string(), cert).await.unwrap();

    let result = node
        .router
        .saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf".to_string(),
            deadline_secs: Some(60),
        })
        .await;
    assert!(
        result.is_ok(),
        "a deadline outliving the certificate must warn, not refuse: {result:?}"
    );
}

#[tokio::test]
async fn begin_is_refused_above_the_deadline_ceiling() {
    let node = saga_node(5).await;
    let result = node
        .router
        .saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf".to_string(),
            deadline_secs: Some(999_999_999),
        })
        .await;
    assert!(
        matches!(result, Err(ProxyError::PermissionDenied(_))),
        "a deadline above the ceiling must be refused rather than clamped, got {result:?}"
    );
}

#[tokio::test]
async fn a_deadline_of_none_takes_the_node_default() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    let info = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
    let expected = saga_config(5).default_deadline_ms;
    assert_eq!(info.deadline_at - info.created_at, expected);
}

#[tokio::test]
async fn commit_drops_the_log_so_a_later_compensate_is_refused() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;

    node.router.saga_commit(SAGA_CALLER, &saga_id).await.unwrap();

    let status = node.router.saga_status(SAGA_CALLER, &saga_id).await;
    assert!(status.is_err(), "a committed saga must be gone");
    let compensate = node.router.saga_compensate(SAGA_CALLER, &saga_id).await;
    assert!(compensate.is_err(), "compensate must be refused after commit");
}

#[tokio::test]
async fn a_saga_id_is_minted_by_the_host_and_is_unique_per_begin() {
    let node = saga_node(5).await;
    let a = begun_saga(&node).await;
    let b = begun_saga(&node).await;
    assert_ne!(a, b);
}

#[tokio::test]
async fn two_concurrent_begins_through_a_cold_cache_share_one_log_handle() {
    let node = saga_node(5).await;
    let router_a = node.router.clone();
    let router_b = node.router.clone();
    let (a, b) = tokio::join!(
        router_a.saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf-a".to_string(),
            deadline_secs: None,
        }),
        router_b.saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf-b".to_string(),
            deadline_secs: None,
        }),
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a, b);
    // Both sagas are visible through the same log -- if two separate
    // connections had been opened, one write could be invisible to a
    // read through the other handle.
    assert!(node.router.saga_status(SAGA_CALLER, &a).await.is_ok());
    assert!(node.router.saga_status(SAGA_CALLER, &b).await.is_ok());
}

// -- merging the forward result into an undo's parameters -----------

#[test]
fn merge_forward_result_adds_a_member_to_an_object_and_an_element_to_an_array() {
    let object = serde_json::json!({"item": "a"});
    let merged = merge_forward_result(&object, Some(&Value::String("id-1".to_string())));
    assert_eq!(merged, serde_json::json!({"item": "a", "forward-result": "id-1"}));

    let array = serde_json::json!(["a", 1]);
    let merged = merge_forward_result(&array, Some(&Value::String("id-1".to_string())));
    assert_eq!(merged, serde_json::json!(["a", 1, "id-1"]));
}

#[test]
fn merge_forward_result_makes_an_object_when_the_forward_params_were_null() {
    let merged = merge_forward_result(&Value::Null, Some(&Value::String("id-1".to_string())));
    assert_eq!(merged, serde_json::json!({"forward-result": "id-1"}));
}

#[test]
fn a_forward_call_that_returned_nothing_sends_no_forward_result() {
    let object = serde_json::json!({"item": "a"});
    let merged = merge_forward_result(&object, None);
    assert_eq!(merged, object);
}

// -- bookkeeping shortens the step budget ----------------------------

#[test]
fn bookkeeping_before_the_call_shortens_the_step_budget_rather_than_extending_the_epoch() {
    // The whole rule in one number: a slower bookkeeping phase (the log
    // open plus the intent write) must leave the call *less* time, not
    // the same amount tacked on top of it -- otherwise a guest's own
    // epoch could be overrun by exactly the bookkeeping cost this
    // subtraction exists to protect against.
    let fast = step_call_budget_ms(4_000, Duration::from_millis(10));
    let slow = step_call_budget_ms(4_000, Duration::from_millis(1_500));
    assert_eq!(fast, 3_990);
    assert_eq!(slow, 2_500);
    assert!(
        slow < fast,
        "more bookkeeping time must leave a smaller call budget, not a larger one"
    );
}

#[test]
fn the_step_budget_floors_at_the_minimum_rather_than_going_negative() {
    // Bookkeeping that ate the whole epoch (a very cold open, or a tiny
    // configured epoch) must not produce a zero or negative budget --
    // `saturating_sub` alone would floor at zero, which is not a call
    // budget, it is an instant refusal.
    let budget = step_call_budget_ms(4_000, Duration::from_secs(10));
    assert_eq!(budget, syneroym_async_queue::MIN_STEP_CALL_BUDGET_MS);
}

// -- the reverse walk -------------------------------------------------

async fn add_step(node: &SagaNode, saga_id: &str, item: &str) {
    let mut req = saga_step_request(saga_id, SAGA_TARGET);
    req.params = serde_json::json!({"item": item});
    node.router.saga_step(req).await.unwrap();
}

#[tokio::test]
async fn the_walk_undoes_the_newest_step_first() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    add_step(&node, &saga_id, "a").await;
    add_step(&node, &saga_id, "b").await;
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    node.router.sweep_sagas_once().await;
    let (_, method, params) = node.target.last_invocation.lock().unwrap().clone().unwrap();
    assert_eq!(method, "saga-undo-reserve");
    // The forward call's own result ("ok") is merged in as
    // `forward-result`.
    assert_eq!(params, serde_json::json!({"item": "b", "forward-result": "ok"}));

    node.router.sweep_sagas_once().await;
    let (_, method, params) = node.target.last_invocation.lock().unwrap().clone().unwrap();
    assert_eq!(method, "saga-undo-reserve");
    assert_eq!(params, serde_json::json!({"item": "a", "forward-result": "ok"}));

    node.router.sweep_sagas_once().await;
    let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
    assert_eq!(
        status.state,
        RpcSagaState::Compensated,
        "a finished compensation drops the step log but keeps a terminal row an operator can \
         still see"
    );
}

#[tokio::test]
async fn the_walk_undoes_a_pending_step_too() {
    // Never records an outcome for the step -- the crash-mid-call case
    // the walk must compensate anyway.
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    let log = node.sagas.log_for(SAGA_CALLER).await.unwrap();
    log.record_step_intent(
        &saga_id,
        &syneroym_async_queue::StepIntent {
            target: serde_json::to_string(&QueuedTarget::Service(SAGA_TARGET.to_string())).unwrap(),
            routing_key: None,
            interface: "saga-participant".to_string(),
            method: "reserve".to_string(),
            params: b"{}".to_vec(),
        },
        proxy_outbox::now_ms(),
    )
    .unwrap();
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    node.router.sweep_sagas_once().await;
    assert_eq!(node.target.invoked.load(Ordering::SeqCst), 1, "the pending step was undone too");
}

#[tokio::test]
async fn a_late_arriving_forward_result_does_not_revert_an_already_compensated_step() {
    // SAGA-02: intent-before-call means a step row can sit `pending`
    // for the whole forward call. If a deadline (or a concurrent
    // `compensate`) starts the walk before that call returns, and the
    // walk reaches and compensates the step first, the forward call's
    // own late-arriving result must not overwrite `compensated` back
    // to `done` -- that would cost a spurious second undo and make
    // `compensated_steps` count backwards. Reproduced directly against
    // the log rather than by racing two real tasks: the intent is
    // recorded exactly as if the forward call were still in flight
    // (no outcome recorded), which is the same state a real in-flight
    // call leaves it in.
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    let log = node.sagas.log_for(SAGA_CALLER).await.unwrap();
    log.record_step_intent(
        &saga_id,
        &syneroym_async_queue::StepIntent {
            target: serde_json::to_string(&QueuedTarget::Service(SAGA_TARGET.to_string())).unwrap(),
            routing_key: None,
            interface: "saga-participant".to_string(),
            method: "reserve".to_string(),
            params: b"{}".to_vec(),
        },
        proxy_outbox::now_ms(),
    )
    .unwrap();
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    // One sweep: the pending step is undone and marked `compensated`,
    // but the saga itself is not yet finished (that needs a sweep that
    // finds nothing left).
    node.router.sweep_sagas_once().await;
    assert_eq!(
        node.target.invoked.load(Ordering::SeqCst),
        1,
        "the pending step's undo must have run"
    );

    // The forward call's own result finally arrives, after the walk
    // already decided this step's fate.
    log.record_step_outcome(&saga_id, 0, Some(b"late-result"), None, proxy_outbox::now_ms())
        .unwrap();

    // If the guard above did nothing, this write just reset the step
    // back to `done`, and this second sweep would find it again and
    // send a second undo.
    node.router.sweep_sagas_once().await;
    assert_eq!(
        node.target.invoked.load(Ordering::SeqCst),
        1,
        "a late-arriving forward result must not cause a second undo to be sent"
    );
    let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
    assert_eq!(status.state, RpcSagaState::Compensated);
}

#[tokio::test]
async fn an_undo_carries_the_saga_and_step_as_its_idempotency_key() {
    // Every undo's idempotency key is host-minted as
    // `saga:<saga-id>:<idx>`, never guest-set -- this is what makes
    // incrementing attempts before dispatch safe, since a re-dispatch
    // after a crash mid-undo is fenced by the receiver's own record
    // under exactly this key.
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    add_step(&node, &saga_id, "a").await;
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    node.router.sweep_sagas_once().await;
    // 2, not 1: `add_step`'s own forward call already reached the
    // target once, and the undo is the second.
    assert_eq!(node.target.invoked.load(Ordering::SeqCst), 2);

    let caller = format!("system:{SAGA_CALLER}");
    let key = format!("saga:{saga_id}:0");
    assert!(
        node.dedup_guard.debug_has_settled_key(SAGA_TARGET, &caller, &key).await,
        "the undo must have fenced under exactly 'saga:<saga-id>:<idx>', got no settled record \
         for that key"
    );
}

#[tokio::test]
async fn a_dependency_bound_to_nobody_fails_the_compensation_without_retrying() {
    // Row 9's saga counterpart, and the regression test for the
    // bug this review round found: a target that no longer resolves
    // to anybody is "nothing to deliver to", not a failed delivery --
    // it must fail the saga on the very first sweep, never spend the
    // retry budget (`fail_terminal_saga_step`, not `fail_saga_step`).
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    let log = node.sagas.log_for(SAGA_CALLER).await.unwrap();
    log.record_step_intent(
        &saga_id,
        &syneroym_async_queue::StepIntent {
            target: serde_json::to_string(&QueuedTarget::Dependency("gone".to_string())).unwrap(),
            routing_key: None,
            interface: "saga-participant".to_string(),
            method: "reserve".to_string(),
            params: b"{}".to_vec(),
        },
        proxy_outbox::now_ms(),
    )
    .unwrap();
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    node.router.sweep_sagas_once().await;
    let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
    assert_eq!(
        status.state,
        RpcSagaState::Failed,
        "a target bound to nobody must fail the saga at once, not stay compensating"
    );
    assert_eq!(
        node.target.invoked.load(Ordering::SeqCst),
        0,
        "there was nowhere to deliver to, so nothing should ever have been dispatched"
    );
}

#[tokio::test]
async fn an_unopenable_saga_log_settles_nothing_and_loses_nothing() {
    // A locked vault (`KekRequired`, the state of every substrate
    // after a restart until an operator injects the KEK) must not
    // make the sweep act as if the service had no sagas at all.
    // Proven behaviourally: the saga survives the locked sweep
    // untouched, and a fresh sweep against the same file picks it
    // straight back up once the KEK is injected.
    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_data_keystore::KeyStore;

    let dir = tempfile::tempdir().unwrap();
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([7u8; 32]).unwrap();
    // Real encryption, unlike every other saga test in this module --
    // the locked-vault condition only exists in that mode.
    let provider = Arc::new(SqliteStorageProvider::new(dir.path(), true).unwrap());
    let resolver = syneroym_app_orchestration::empty_resolver();

    let sagas1 = Arc::new(SagaStore::new(
        provider.clone(),
        key_store.clone(),
        resolver.clone(),
        saga_config(5),
    ));
    let log1 = sagas1.log_for(SAGA_CALLER).await.unwrap();
    let now = proxy_outbox::now_ms();
    log1.begin("locked-saga", "wf", None, now + 60_000, now).unwrap();

    key_store.clear_kek();

    // A fresh `SagaStore` over the same file with a cold cache --
    // exactly what a restart looks like.
    let sagas2 = Arc::new(SagaStore::new(
        provider.clone(),
        key_store.clone(),
        resolver.clone(),
        saga_config(5),
    ));
    let registry = empty_registry();
    registry
        .register(
            SAGA_CALLER.to_string(),
            "saga-driver".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: SAGA_CALLER.to_string() },
        )
        .await
        .unwrap();
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let router = Arc::new(
        ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            Arc::new(MockHop::default()),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy { max_attempts: 1, ..RetryPolicy::default() },
        )
        .with_sagas(sagas2.clone()),
    );

    let settled = router.sweep_sagas_once().await;
    assert_eq!(settled, 0, "a locked vault must settle nothing, not error out or panic");

    key_store.inject_kek([7u8; 32]).unwrap();
    let log_after_unlock = sagas2.log_for(SAGA_CALLER).await.unwrap();
    assert!(
        log_after_unlock.status("locked-saga").unwrap().is_some(),
        "the saga written before the lock must still be there once it is unlocked"
    );
}

#[tokio::test]
async fn cancellation_interrupts_an_in_flight_undo_and_leaves_the_saga_compensating() {
    // The same property
    // `shutdown_abandons_an_in_flight_delivery_rather_than_draining` proves
    // for the outbox, for the saga sweep: worker shutdown must interrupt an
    // undo that never resolves, not wait out its own
    // budget (`SAGA_UNDO_CALL_BUDGET`). Left otherwise, one
    // unreachable participant could hold node shutdown hostage.
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    add_step(&node, &saga_id, "a").await;
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    // The target blocks and is never released during the test, so any
    // undo the worker starts genuinely never resolves on its own.
    let release = Arc::new(tokio::sync::Notify::new());
    *node.target.hold.lock().unwrap() = Some(release.clone());

    let cancel = CancellationToken::new();
    let worker = {
        let router = node.router.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            router.run_async_worker(Duration::from_millis(5), cancel).await;
        })
    };

    let entered =
        wait_for(Duration::from_secs(5), || node.target.invoked.load(Ordering::SeqCst) >= 1).await;
    assert!(
        entered,
        "the worker never reached the target, so this test would prove nothing about interrupting \
         an in-flight undo"
    );

    // The load-bearing assertion: the undo in flight right now cannot
    // finish on its own, so the worker can only return if cancellation
    // is raced *into* the delivery.
    cancel.cancel();
    let stopped = tokio::time::timeout(Duration::from_secs(2), worker).await;
    assert!(
        stopped.is_ok(),
        "shutdown must interrupt an undo that never resolves, not wait it out"
    );

    release.notify_waiters();
    let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
    assert_eq!(
        status.state,
        RpcSagaState::Compensating,
        "an abandoned undo must leave the saga compensating, not silently finished or failed"
    );
}

#[tokio::test]
async fn a_retryable_undo_failure_schedules_a_backoff_and_keeps_the_saga_compensating() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    add_step(&node, &saga_id, "a").await;
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    // Re-register the target as a WASM channel with no sandbox engine
    // behind this router (`Weak::new()`) -- "sandbox engine
    // unavailable" is a `ProxyError::Internal`, which `disposition_of`
    // classifies `Retry` (a shutdown-window state, not a settled
    // refusal). A definitive `Callee` error -- what `fail_with` would
    // produce -- is terminal by default and does not exercise this
    // path.
    node.registry
        .register(
            SAGA_TARGET.to_string(),
            "saga-participant".to_string(),
            SubstrateEndpoint::WasmChannel { service_id: SAGA_TARGET.to_string() },
        )
        .await
        .unwrap();

    node.router.sweep_sagas_once().await;
    let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
    assert_eq!(status.state, RpcSagaState::Compensating, "a retryable failure keeps compensating");
}

#[tokio::test]
async fn a_terminal_undo_failure_fails_the_saga_immediately() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    add_step(&node, &saga_id, "a").await;
    *node.target.answer_with.lock().unwrap() =
        Some(ProxyError::Callee { code: -32010, message: "denied".to_string(), data: None });
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    node.router.sweep_sagas_once().await;
    let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
    assert_eq!(status.state, RpcSagaState::Failed, "a terminal failure fails the saga at once");
}

#[tokio::test]
async fn an_undo_the_receiver_had_already_run_counts_as_compensated() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    add_step(&node, &saga_id, "a").await;
    // "Already ran, result too large to retain" -- a delivery reported
    // through the error channel, per `disposition_of`'s `Delivered`
    // case. `CALL_ALREADY_RUNNING_RPC_CODE` is a *different*
    // code (still in flight right now) and classifies `Retry`, not
    // `Delivered`.
    *node.target.answer_with.lock().unwrap() = Some(ProxyError::Callee {
        code: syneroym_async_queue::CALL_RESULT_NOT_RETAINED_RPC_CODE,
        message: "already ran; result too large to retain".to_string(),
        data: None,
    });
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    node.router.sweep_sagas_once().await;
    node.router.sweep_sagas_once().await;
    let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
    assert_eq!(
        status.state,
        RpcSagaState::Compensated,
        "the receiver's already-ran answer must count as compensated, not retried forever"
    );
}

#[tokio::test]
async fn a_missing_compensation_is_recorded_with_an_error_that_names_the_convention() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    add_step(&node, &saga_id, "a").await;
    *node.target.answer_with.lock().unwrap() = Some(ProxyError::Callee {
        code: syneroym_rpc::SERVICE_NOT_FOUND_RPC_CODE,
        message: "Method not found: saga-undo-reserve".to_string(),
        data: None,
    });
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    // `SERVICE_NOT_FOUND_RPC_CODE` classifies as retryable, so this is
    // recorded as a `Retry` outcome, not `Failed` -- but the rewritten
    // message must already name the convention on this very first
    // attempt, not only once the saga eventually fails.
    node.router.sweep_sagas_once().await;
    let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
    assert!(
        status.last_error.as_deref().is_some_and(|e| e.contains("saga-undo-reserve")),
        "expected the recorded error to name the convention, got {:?}",
        status.last_error
    );
}

#[tokio::test]
async fn a_saga_past_its_deadline_starts_compensating_without_the_guest() {
    let node = saga_node(5).await;
    let saga_id = node
        .router
        .saga_begin(SagaBegin {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            name: "wf".to_string(),
            deadline_secs: Some(1),
        })
        .await
        .unwrap();
    add_step(&node, &saga_id, "a").await;

    // No guest-driven `compensate` call at all -- the deadline alone
    // must start the walk.
    let past_deadline = proxy_outbox::now_ms() + 2_000;
    let log = node.sagas.log_for(SAGA_CALLER).await.unwrap();
    for head in log.abandoned(past_deadline, 10).unwrap() {
        log.mark_compensating(&head.saga_id, past_deadline).unwrap();
    }

    node.router.sweep_sagas_once().await;
    assert_eq!(node.target.invoked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_undeployed_services_sagas_are_dropped_rather_than_compensated() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    add_step(&node, &saga_id, "a").await;
    // `add_step`'s own forward call already reached the target once.
    let invoked_before = node.target.invoked.load(Ordering::SeqCst);
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    node.registry.remove_instance_cert(SAGA_CALLER).await.unwrap();
    node.registry.remove(SAGA_CALLER, "saga-driver").await.unwrap();

    // Two consecutive absent ticks, not one: a redeploy can leave a
    // service briefly missing from the registry for a single tick, and
    // one miss must not be enough to destroy its saga log (SAGA-03).
    node.router.sweep_sagas_once().await;
    let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
    assert!(
        !log.list().unwrap().is_empty(),
        "one absent tick alone must not drop the saga -- it could be a redeploy in flight"
    );

    node.router.sweep_sagas_once().await;
    assert_eq!(
        node.target.invoked.load(Ordering::SeqCst),
        invoked_before,
        "an undeployed service's saga must never send an undo"
    );
    let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
    assert!(log.list().unwrap().is_empty(), "its sagas must be dropped, not left compensating");
}

#[tokio::test]
async fn a_service_that_reappears_between_ticks_keeps_its_sagas() {
    let node = saga_node(5).await;
    let saga_id = begun_saga(&node).await;
    add_step(&node, &saga_id, "a").await;
    node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

    let endpoint = node.registry.lookup_by_service(SAGA_CALLER)[0].1.clone();
    node.registry.remove_instance_cert(SAGA_CALLER).await.unwrap();
    node.registry.remove(SAGA_CALLER, "saga-driver").await.unwrap();
    node.router.sweep_sagas_once().await;

    // The service comes back before a second consecutive absent tick.
    node.registry
        .register(SAGA_CALLER.to_string(), "saga-driver".to_string(), endpoint)
        .await
        .unwrap();
    node.router.sweep_sagas_once().await;

    // A later real undeploy must still need its own two consecutive
    // ticks, not fire immediately off a stale mark from the first one.
    node.registry.remove(SAGA_CALLER, "saga-driver").await.unwrap();
    node.router.sweep_sagas_once().await;
    let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
    assert!(
        !log.list().unwrap().is_empty(),
        "the absence mark must have been cleared when the service reappeared"
    );
}

/// The control plane downgrades `ProxyState` into a
/// `Weak<dyn ProxyQueueInspector>` it holds in a `OnceLock`. That
/// `Weak` must stay valid for as long as the router does -- which only
/// holds if `ProxyRouter` itself is the bundle's one strong owner. A
/// wrapper built and downgraded with no long-lived owner would drop the
/// moment the constructing function returned, and every operator verb
/// would then answer "this node keeps no durable proxy state".
#[tokio::test]
async fn the_operator_verbs_still_answer_after_the_router_is_the_only_owner_left() {
    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_data_keystore::KeyStore;

    let node = saga_node(5).await;
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
    let outbox = Arc::new(ProxyOutbox::new(
        provider,
        Arc::new(KeyStore::new()),
        syneroym_app_orchestration::empty_resolver(),
        QueueConfig {
            retry: RetryPolicy::default(),
            visibility_timeout_ms: 60_000,
            dlq_max_rows: 100,
            max_pending_rows: syneroym_async_queue::DEFAULT_MAX_PENDING_ROWS,
        },
    ));

    let router = ProxyRouter::new(
        node.registry.clone(),
        empty_registry_client(),
        Weak::new(),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    )
    .with_outbox(outbox)
    .with_sagas(node.sagas.clone());
    let router = Arc::new(router);

    // Mirrors `route_handler.rs`'s own wiring exactly: downgrade from a
    // local binding, then let that binding go out of scope.
    let weak: Weak<dyn ProxyQueueInspector> = {
        let state = router.proxy_state().expect("both outbox and sagas are wired").clone();
        Arc::downgrade(&state) as Weak<dyn ProxyQueueInspector>
    };

    assert!(
        weak.upgrade().is_some(),
        "the router's own Arc<ProxyState> must be what keeps this Weak alive"
    );
}

#[test]
fn check_native_capability_gate_refuses_cross_service_signing_call() {
    let registry = EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));
    let router = ProxyRouter::new(
        registry,
        empty_registry_client(),
        Weak::new(),
        Weak::new(),
        Arc::new(MockHop::default()),
        Arc::new(Identity::generate().unwrap()),
        RetryPolicy::default(),
    );

    let mut self_req = base_request("svc1", "signing");
    self_req.origin = CallOrigin::Guest { service_id: "svc1".to_string() };
    self_req.method = "sign-record".to_string();
    assert!(router.check_native_capability_gate(&self_req).is_ok());

    let mut cross_req = base_request("svc2", "signing");
    cross_req.origin = CallOrigin::Guest { service_id: "svc1".to_string() };
    cross_req.method = "sign-record".to_string();
    let err = router.check_native_capability_gate(&cross_req).unwrap_err();
    assert!(matches!(err, ProxyError::PermissionDenied(msg) if msg.contains("signing")));
}

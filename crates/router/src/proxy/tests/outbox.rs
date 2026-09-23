use super::helpers::*;

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

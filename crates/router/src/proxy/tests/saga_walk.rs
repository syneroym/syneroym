use super::helpers::*;

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

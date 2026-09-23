use super::helpers::*;

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

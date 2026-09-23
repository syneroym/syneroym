use std::{sync::Mutex, time::Instant};

use syneroym_async_queue::{Queue, QueueConfig};
use syneroym_core::config::SupervisorRole;

use super::{super::*, helpers::*};

/// A reachable substrate's `write_bindings` call must never touch the
/// real, wired-up
/// supervisor outbox -- asserted as "the queue is untouched", not as a
/// timing, so it cannot pass by being fast.
#[tokio::test]
async fn a_reachable_substrate_never_touches_the_queue() {
    let s = service();
    let fake = Arc::new(FakeSubstrateClient {
        write_bindings_outcome: Mutex::new(Some(Ok(vec![BindingWriteOutcome::Applied]))),
    });
    let outbox = Arc::new(SupervisorOutbox::new(s.store.queue.clone()));
    let key = QueueKey {
        app_instance_id: "inst-1".to_string(),
        logical_ref: "inst-1/backend".to_string(),
        substrate_did: "did:key:zB".to_string(),
    };
    let actor =
        deploy::build_durable_actor(fake, "did:key:zB".to_string(), key.to_string(), outbox);

    let outcomes =
        actor.write_bindings(test_binding_write("did:key:zSvc", "inst-1")).await.unwrap();
    assert_eq!(outcomes, vec![BindingWriteOutcome::Applied]);
    assert!(s.store.queue.all().unwrap().is_empty());
    assert!(s.store.queue.dead_letters().unwrap().is_empty());
}

/// The worker's replay of an already-applied write must be a no-op
/// from the substrate's own
/// epoch guard's perspective -- scripted here as the fake simply
/// reporting `NoOp` on delivery, which the worker must complete
/// exactly like `Applied`.
#[tokio::test]
async fn applying_a_queued_write_bindings_twice_is_a_no_op() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::NoOp])));
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    let id = enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );

    s.queue_worker_tick().await;

    assert!(s.store.queue.all().unwrap().is_empty(), "the item {id} must be gone from the outbox");
    assert!(s.store.queue.dead_letters().unwrap().is_empty());
}

/// A `stale` delivery means a newer epoch already landed --
/// convergence, not loss -- so it must complete and must not
/// dead-letter.
#[tokio::test]
async fn a_queued_write_delivered_stale_completes_and_does_not_dead_letter() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Stale(7)])));
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );

    s.queue_worker_tick().await;

    assert!(s.store.queue.all().unwrap().is_empty());
    assert!(
        s.store.queue.dead_letters().unwrap().is_empty(),
        "a stale delivery is convergence, not a failure"
    );
}

/// A queued item whose instance was retired between enqueue and
/// delivery must be quietly completed --
/// no delivery attempt (it would resurrect a binding the operator just
/// released) and no `DeliveryExhausted` alert (noise against an
/// instance nobody is going to act on). Unscripted `FakeQueueConnector`
/// is deliberate: reaching `connect` at all would fail the test with
/// "no scripted delivery", so this also proves the retired branch
/// returns before ever attempting one.
#[tokio::test]
async fn a_queued_item_for_a_retired_instance_completes_quietly() {
    let mut s = service();
    s.queue_connector = Arc::new(FakeQueueConnector::default());
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );
    s.store.retire("inst-1").unwrap();

    s.queue_worker_tick().await;

    assert!(s.store.queue.all().unwrap().is_empty(), "the item must be gone from the outbox");
    assert!(
        s.store.queue.dead_letters().unwrap().is_empty(),
        "a retired instance's stale intent is moot, not a failure"
    );
    let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
    assert!(
        !s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::DeliveryExhausted),
        "a retired instance must not gain a fresh DeliveryExhausted alert"
    );
}

/// The `conflict` row -- the case the synchronous coverage misses.
/// Completes **and** raises the same `BindingConflict` alert the
/// synchronous path does.
#[tokio::test]
async fn a_queued_write_delivered_conflicting_raises_the_same_alert_as_the_synchronous_path() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector
        .script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Conflict(9)])));
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );

    s.queue_worker_tick().await;

    assert!(s.store.queue.all().unwrap().is_empty(), "a conflict still completes the item");
    let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
    let active = s.store.alerts.active(&instance_id).unwrap();
    assert!(
        active.iter().any(|a| a.kind == AlertKind::BindingConflict
            && a.logical_ref.as_deref() == Some("inst-1/backend")),
        "{active:?}"
    );
}

/// A transport failure on replay must return the item to the outbox
/// rather than dead-lettering it outright -- the queue's own retry
/// budget governs when it finally gives up, not one failed replay.
#[tokio::test]
async fn a_transport_failure_on_replay_retries_rather_than_dead_lettering() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::ConnectFails);
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );

    s.queue_worker_tick().await;

    let remaining = s.store.queue.all().unwrap();
    assert_eq!(remaining.len(), 1, "a transport failure must stay in the outbox");
    assert!(s.store.queue.dead_letters().unwrap().is_empty());
}

/// A callee error that does *not* name this write's own target as gone
/// -- a transient, reached-and-answered
/// refusal the wire protocol cannot currently distinguish from "gone"
/// by error code -- must stay retryable on the queued path, not
/// dead-letter on its first delivery. Otherwise a queued item that
/// survived a restart specifically to be retried would be given up on
/// by a hiccup a later attempt would have cleared.
#[tokio::test]
async fn an_ambiguous_callee_error_on_replay_retries_rather_than_dead_lettering() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script(
        "did:key:zB",
        FakeDelivery::CalleeError("database is locked, try again".to_string()),
    );
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );

    s.queue_worker_tick().await;

    let remaining = s.store.queue.all().unwrap();
    assert_eq!(remaining.len(), 1, "an ambiguous callee error must stay in the outbox");
    assert!(s.store.queue.dead_letters().unwrap().is_empty());
}

/// A callee error on replay naming this write's own target as gone
/// (the exact wording `control_plane`'s `write-bindings` dispatch uses
/// for that specific refusal) is terminal -- straight to the DLQ, not
/// retried.
#[tokio::test]
async fn a_callee_error_on_replay_dead_letters_immediately() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script(
        "did:key:zB",
        FakeDelivery::CalleeError(
            "'did:key:zSvc' has no app context on this substrate".to_string(),
        ),
    );
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );

    s.queue_worker_tick().await;

    assert!(s.store.queue.all().unwrap().is_empty());
    let dead = s.store.queue.dead_letters().unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].attempts, 1, "a terminal failure dead-letters on its first attempt");
}

/// Without taking `instance_lock`, the worker could interleave with a
/// live pass write for the same instance and race
/// this supervisor into a spurious `BindingConflict`. Proven by
/// holding the lock externally (as a live pass would) and asserting
/// the worker's own attempt to take it blocks until released.
#[tokio::test]
async fn the_worker_and_a_loop_pass_never_write_one_instance_concurrently() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );

    let held = s.instance_lock("inst-1");
    let guard = held.lock().await;

    let s = Arc::new(s);
    let s_clone = s.clone();
    let mut tick = tokio::spawn(async move { s_clone.queue_worker_tick().await });

    tokio::select! {
        _ = &mut tick => panic!("the worker must block on instance_lock while a pass holds it"),
        () = tokio::time::sleep(Duration::from_millis(50)) => {}
    }

    drop(guard);
    tick.await.unwrap();
    assert!(s.store.queue.all().unwrap().is_empty(), "the worker delivers once the lock frees up");
}

/// Cancelling the token must not wait for a delivery *genuinely in
/// flight* -- `FakeDelivery::Blocks` makes `connect` never resolve, so
/// if `shutdown` returned promptly here it is because cancellation
/// actually interrupted a real, ongoing delivery. An earlier version
/// of this test left the substrate unscripted, which fails `connect`
/// immediately and proves nothing about abandoning work in flight.
#[tokio::test]
async fn shutdown_abandons_in_flight_work_rather_than_draining() {
    let mut s = Fixture { queue_tick_secs: Some(1), ..Fixture::default() }.build();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Blocks);
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );

    let s = Arc::new(s);
    let run_handle = {
        let s = s.clone();
        tokio::spawn(async move { s.run_queue_worker().await })
    };
    // Give the 1s interval its first tick a chance to fire, claim the
    // item, and reach `connect` -- which then blocks forever, so by
    // the time this returns a delivery is genuinely in flight.
    tokio::time::sleep(Duration::from_millis(1200)).await;

    let start = Instant::now();
    s.shutdown().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), run_handle)
        .await
        .expect(
            "run_queue_worker must return promptly on shutdown, not wait for the delivery in \
             flight",
        )
        .unwrap()
        .unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));

    // The abandoned item is still claimed (invisible) right after
    // shutdown -- dropped mid-delivery, not completed or failed.
    assert!(
        s.store.queue.claim_due(outbox::now_ms(), 10).unwrap().is_empty(),
        "the item must still be invisible, not silently completed or requeued"
    );
}

/// The in-process analogue of the e2e restart step -- a queued item
/// survives a supervisor restart (a fresh `SupervisorStore` opened
/// against the same database file) and the new process's worker
/// resumes it.
#[tokio::test]
async fn the_worker_resumes_a_queued_item_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
        seed_inventory(&store, "inst-1", "did:key:zB");
        enqueue_test_item(
            &store,
            "inst-1",
            "inst-1/backend",
            "did:key:zB",
            test_binding_write("did:key:zSvc", "inst-1"),
        );
    }

    let store = SupervisorStore::open(dir.path(), "supervisor.db").unwrap();
    let mut s = Fixture::default().build();
    s.store = store;
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
    s.queue_connector = connector;

    s.queue_worker_tick().await;

    assert!(s.store.queue.all().unwrap().is_empty(), "the resumed item must have been delivered");
}

/// Recovery must complete within one `queue_tick_secs`, not one
/// `poll_interval_secs` -- driven against a paused clock with
/// `poll_interval_secs` set far above the worker tick, so passing by
/// accident (both loops running for real) is impossible.
#[tokio::test(start_paused = true)]
async fn recovery_completes_within_one_worker_tick_and_not_one_poll_interval() {
    let mut s =
        Fixture { queue_tick_secs: Some(5), poll_interval_secs: Some(3600), ..Fixture::default() }
            .build();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );

    let s = Arc::new(s);
    let run_handle = {
        let s = s.clone();
        tokio::spawn(async move { s.run_queue_worker().await })
    };

    tokio::time::advance(Duration::from_secs(6)).await;
    // Yield so the worker's now-elapsed tick actually runs.
    tokio::task::yield_now().await;
    tokio::time::sleep(Duration::from_millis(1)).await;

    assert!(
        s.store.queue.all().unwrap().is_empty(),
        "recovery must land within one 5s worker tick, well under the 3600s poll interval"
    );

    s.shutdown().await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(1), run_handle).await;
}

/// `AlertStore`'s unique index cannot express one row per dead letter,
/// and an operator wants the standing fact anyway --
/// a second dead letter for the same key must refresh the existing
/// alert's count, not open a second row.
#[tokio::test]
async fn a_second_dead_letter_for_one_member_refreshes_the_alert_and_raises_its_count() {
    let s = service();
    let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
    let key = QueueKey {
        app_instance_id: "inst-1".to_string(),
        logical_ref: "inst-1/backend".to_string(),
        substrate_did: "did:key:zB".to_string(),
    };
    let id1 = enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc1", "inst-1"),
    );
    let id2 = enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc2", "inst-1"),
    );

    s.fail_queued_item(&instance_id, &key, id1, outbox::now_ms(), "boom", true).await;
    let active = s.store.alerts.active(&instance_id).unwrap();
    assert_eq!(active.iter().filter(|a| a.kind == AlertKind::DeliveryExhausted).count(), 1);
    assert!(
        active.iter().any(|a| a.kind == AlertKind::DeliveryExhausted && a.detail.contains('1'))
    );

    s.fail_queued_item(&instance_id, &key, id2, outbox::now_ms(), "boom again", true).await;
    let active = s.store.alerts.active(&instance_id).unwrap();
    assert_eq!(
        active.iter().filter(|a| a.kind == AlertKind::DeliveryExhausted).count(),
        1,
        "a second dead letter must refresh the existing row, not open a second one"
    );
    assert!(
        active.iter().any(|a| a.kind == AlertKind::DeliveryExhausted && a.detail.contains('2'))
    );
}

/// The clear path -- the same one `RemediationExhausted` already
/// documents. Replaying every dead letter for a key clears its alert;
/// an earlier replay leaving one behind must not.
#[tokio::test]
async fn the_alert_clears_when_the_last_dead_letter_for_that_key_is_gone() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
    connector.script("did:key:zB", FakeDelivery::Attempt(Ok(vec![BindingWriteOutcome::Applied])));
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
    let key = QueueKey {
        app_instance_id: "inst-1".to_string(),
        logical_ref: "inst-1/backend".to_string(),
        substrate_did: "did:key:zB".to_string(),
    };
    let id1 = enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc1", "inst-1"),
    );
    let id2 = enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc2", "inst-1"),
    );
    s.fail_queued_item(&instance_id, &key, id1, outbox::now_ms(), "boom", true).await;
    s.fail_queued_item(&instance_id, &key, id2, outbox::now_ms(), "boom again", true).await;
    assert!(
        s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::DeliveryExhausted)
    );

    let dead = s.store.queue.dead_letters().unwrap();
    assert_eq!(dead.len(), 2);

    dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "replay",
        serde_json::json!(["inst-1", dead[0].id as u64]),
    )
    .await
    .unwrap();
    assert!(
        s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::DeliveryExhausted),
        "one dead letter for this key still remains; the alert must stay active"
    );

    // The first replay's own row must resolve before the second dead
    // letter for the identical key can be replayed too -- `Queue::
    // replay` refuses a second pending row for one key, so this drives
    // the worker to deliver (and complete) the first replay's item
    // before trying the second.
    s.queue_worker_tick().await;
    assert!(s.store.queue.all().unwrap().is_empty(), "the first replay must have landed");

    dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "replay",
        serde_json::json!(["inst-1", dead[1].id as u64]),
    )
    .await
    .unwrap();
    assert!(
        !s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::DeliveryExhausted),
        "the last dead letter for this key is gone; the alert must clear"
    );
}

/// A *pruned* dead letter (the DLQ cap evicting the oldest row on
/// write) must clear its own standing alert exactly the same way an
/// explicit `replay` does -- not just when a human happens to replay
/// it. Distinct from the test above, which only exercises the replay
/// path.
#[tokio::test]
async fn a_pruned_dead_letter_clears_its_own_alert_too() {
    let mut s = service();
    // A cap of 1 so the second key's own dead letter immediately
    // evicts the first key's -- both keys share one group (the app
    // instance), so the cap applies across them.
    s.store.queue = Queue::open_in_memory(QueueConfig {
        dlq_max_rows: 1,
        ..QueueConfig::from(&SupervisorRole::default())
    })
    .unwrap();

    let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
    let key_a = QueueKey {
        app_instance_id: "inst-1".to_string(),
        logical_ref: "inst-1/backend-a".to_string(),
        substrate_did: "did:key:zB".to_string(),
    };
    let key_b = QueueKey {
        app_instance_id: "inst-1".to_string(),
        logical_ref: "inst-1/backend-b".to_string(),
        substrate_did: "did:key:zB".to_string(),
    };
    let id_a = enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend-a",
        "did:key:zB",
        test_binding_write("did:key:zSvcA", "inst-1"),
    );
    s.fail_queued_item(&instance_id, &key_a, id_a, outbox::now_ms(), "boom", true).await;
    assert!(
        s.store
            .alerts
            .active(&instance_id)
            .unwrap()
            .iter()
            .any(|a| a.kind == AlertKind::DeliveryExhausted
                && a.logical_ref.as_deref() == Some("inst-1/backend-a")),
        "key a's dead letter must raise its own alert"
    );

    let id_b = enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend-b",
        "did:key:zB",
        test_binding_write("did:key:zSvcB", "inst-1"),
    );
    // This dead-letters key b, which prunes key a's now-oldest row
    // past the cap of 1 -- with no `replay` in sight.
    s.fail_queued_item(&instance_id, &key_b, id_b, outbox::now_ms(), "boom too", true).await;

    assert_eq!(s.store.queue.dead_letters().unwrap().len(), 1, "the cap must still hold");
    let active = s.store.alerts.active(&instance_id).unwrap();
    assert!(
        !active.iter().any(|a| a.kind == AlertKind::DeliveryExhausted
            && a.logical_ref.as_deref() == Some("inst-1/backend-a")),
        "key a's dead letter was pruned; its alert must clear, not linger forever: {active:?}"
    );
    assert!(
        active.iter().any(|a| a.kind == AlertKind::DeliveryExhausted
            && a.logical_ref.as_deref() == Some("inst-1/backend-b")),
        "key b's own alert must still be active: {active:?}"
    );
}

/// The outbox growth bound -- "at most one row per `(instance,
/// logical_ref, substrate)`" -- is enforced by
/// `SupervisorOutbox::already_pending`, but nothing asserted it as the
/// bound directly.
#[tokio::test]
async fn the_outbox_holds_at_most_one_row_per_key_regardless_of_how_many_enqueue_attempts() {
    let queue = Queue::open_in_memory(QueueConfig::from(&SupervisorRole::default())).unwrap();
    let outbox = SupervisorOutbox::new(queue.clone());
    let key = QueueKey {
        app_instance_id: "inst-1".to_string(),
        logical_ref: "inst-1/backend".to_string(),
        substrate_did: "did:key:zB".to_string(),
    };
    let write = test_binding_write("did:key:zSvc", "inst-1");

    for generation in 0..50u64 {
        outbox
            .enqueue(&key.to_string(), "did:key:zB", &BindingWrite { generation, ..write.clone() })
            .await;
    }

    assert_eq!(
        queue.all().unwrap().len(),
        1,
        "fifty enqueue attempts for one key must still leave exactly one outbox row"
    );
}

/// `dead-letters` lists exactly what the store holds, mapped onto the
/// WIT shape (logical ref and substrate DID pulled back out
/// of the opaque queue key).
#[tokio::test]
async fn dead_letters_lists_what_the_store_holds() {
    let s = service();
    let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
    let key = QueueKey {
        app_instance_id: "inst-1".to_string(),
        logical_ref: "inst-1/backend".to_string(),
        substrate_did: "did:key:zB".to_string(),
    };
    let id = enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );
    s.fail_queued_item(&instance_id, &key, id, outbox::now_ms(), "unreachable", true).await;

    let res = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "dead-letters",
        serde_json::json!(["inst-1"]),
    )
    .await
    .unwrap();
    let rows: Vec<DeadLetter> = serde_json::from_value(res.payload).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].logical_ref, "inst-1/backend");
    assert_eq!(rows[0].substrate_did, "did:key:zB");
    assert_eq!(rows[0].last_error, "unreachable");
    assert_eq!(rows[0].attempts, 1);

    // A different instance's dead letters must not leak through.
    let other = dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "dead-letters",
        serde_json::json!(["inst-2"]),
    )
    .await
    .unwrap();
    let other_rows: Vec<DeadLetter> = serde_json::from_value(other.payload).unwrap();
    assert!(other_rows.is_empty());
}

/// `replay` re-enqueues through the ordinary worker path and does not
/// execute inline -- proven by scripting no delivery
/// at all for the target DID and asserting the RPC call still
/// succeeds, since replay itself never calls the connector.
#[tokio::test]
async fn replay_re_enqueues_and_does_not_execute_inline() {
    let mut s = service();
    s.queue_connector = Arc::new(FakeQueueConnector::default());
    let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
    let key = QueueKey {
        app_instance_id: "inst-1".to_string(),
        logical_ref: "inst-1/backend".to_string(),
        substrate_did: "did:key:zB".to_string(),
    };
    let id = enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );
    s.fail_queued_item(&instance_id, &key, id, outbox::now_ms(), "unreachable", true).await;
    let dead_id = s.store.queue.dead_letters().unwrap()[0].id as u64;

    // No `FakeDelivery` scripted for "did:key:zB" -- if `replay`
    // executed inline it would have to reach the connector and this
    // call would fail closed (`FakeQueueConnector::connect` errors
    // with nothing scripted).
    dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "replay",
        serde_json::json!(["inst-1", dead_id]),
    )
    .await
    .unwrap();

    assert!(s.store.queue.dead_letters().unwrap().is_empty());
    let requeued = s.store.queue.all().unwrap();
    assert_eq!(requeued.len(), 1);
}

/// Over the RPC surface -- a replayed item that fails again returns to
/// the DLQ with its attempt history intact, listable the same way the
/// first one was.
#[tokio::test]
async fn a_replayed_item_that_fails_again_returns_to_the_dlq_with_its_history() {
    let mut s = service();
    let connector = Arc::new(FakeQueueConnector::default());
    connector.script(
        "did:key:zB",
        FakeDelivery::CalleeError(
            "'did:key:zSvc' has no app context on this substrate".to_string(),
        ),
    );
    s.queue_connector = connector;
    seed_inventory(&s.store, "inst-1", "did:key:zB");
    let instance_id = AppInstanceId::try_new("inst-1".to_string()).unwrap();
    let key = QueueKey {
        app_instance_id: "inst-1".to_string(),
        logical_ref: "inst-1/backend".to_string(),
        substrate_did: "did:key:zB".to_string(),
    };
    let id = enqueue_test_item(
        &s.store,
        "inst-1",
        "inst-1/backend",
        "did:key:zB",
        test_binding_write("did:key:zSvc", "inst-1"),
    );
    s.fail_queued_item(&instance_id, &key, id, outbox::now_ms(), "first refusal", true).await;
    let dead_id = s.store.queue.dead_letters().unwrap()[0].id as u64;

    dispatch(
        &s,
        admin_caller("did:key:zSupervisorNode"),
        "replay",
        serde_json::json!(["inst-1", dead_id]),
    )
    .await
    .unwrap();
    s.queue_worker_tick().await;

    let dead = s.store.queue.dead_letters().unwrap();
    assert_eq!(dead.len(), 1, "the replayed item must be back in the DLQ after failing again");
    assert_eq!(dead[0].attempts, 2, "the attempt count must carry over, not reset");
}

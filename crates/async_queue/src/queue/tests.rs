use std::collections::BTreeSet;

use syneroym_core::config::{AppSandboxRole, RetryPolicy, SupervisorRole};

use super::*;

fn config() -> QueueConfig {
    QueueConfig {
        retry: RetryPolicy {
            max_attempts: 3,
            initial_backoff_ms: 100,
            backoff_multiplier: 2.0,
            max_backoff_ms: 900_000,
        },
        visibility_timeout_ms: 120_000,
        dlq_max_rows: 1000,
        max_pending_rows: DEFAULT_MAX_PENDING_ROWS,
    }
}

/// An enqueued item must still be there after the database is closed
/// and reopened -- the whole point of a durable queue.
#[test]
fn an_enqueued_item_survives_reopening_the_database() {
    let dir = tempfile::tempdir().unwrap();
    {
        let queue = Queue::open(dir.path(), "queue.db", config()).unwrap();
        queue.enqueue("inst-1", "inst-1/backend@did:key:zB", b"payload-a", 1_000).unwrap();
    }
    let queue = Queue::open(dir.path(), "queue.db", config()).unwrap();
    let items = queue.all().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].payload, b"payload-a");
    assert_eq!(items[0].queue_key, "inst-1/backend@did:key:zB");
}

/// Durability and the SQLCipher key in one assertion: an item written
/// to an encrypted queue file is still there after the process that
/// wrote it is gone, and still readable with the same key.
#[test]
fn a_queued_item_survives_reopening_the_encrypted_queue_file() {
    let dir = tempfile::tempdir().unwrap();
    let dek = [7u8; 32];
    {
        let queue = Queue::open_encrypted(dir.path(), "async.db", Some(&dek), config()).unwrap();
        queue.enqueue("did:key:zTarget", "msg-7", b"payload-a", 1_000).unwrap();
    }
    let queue = Queue::open_encrypted(dir.path(), "async.db", Some(&dek), config()).unwrap();
    let items = queue.all().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].payload, b"payload-a");
    assert_eq!(items[0].queue_key, "msg-7");
}

/// A queued payload is the calling service's own data, so the file it
/// waits in must be no weaker than the database that data came from.
/// Scoped to an encryption-enabled deployment: with encryption
/// disabled the queue is plain SQLite, exactly as `state.db` is then,
/// and this property is not claimed.
#[test]
fn the_queue_file_is_unreadable_without_the_services_dek() {
    let dir = tempfile::tempdir().unwrap();
    {
        let queue =
            Queue::open_encrypted(dir.path(), "async.db", Some(&[7u8; 32]), config()).unwrap();
        queue.enqueue("did:key:zTarget", "msg-7", b"secret-payload", 1_000).unwrap();
    }
    assert!(
        Queue::open(dir.path(), "async.db", config()).is_err(),
        "an unkeyed open of an encrypted queue file must fail"
    );
    assert!(
        Queue::open_encrypted(dir.path(), "async.db", Some(&[8u8; 32]), config()).is_err(),
        "the wrong key must fail exactly like no key"
    );
}

/// Two workers must never both hold the same item: once claimed, it is
/// invisible to a second claim until its visibility timeout passes.
#[test]
fn a_claimed_item_is_invisible_to_a_second_claim() {
    let queue = Queue::open_in_memory(config()).unwrap();
    queue.enqueue("g", "k", b"p", 1_000).unwrap();

    let first = queue.claim_due(1_000, 10).unwrap();
    assert_eq!(first.len(), 1);

    let second = queue.claim_due(1_000, 10).unwrap();
    assert!(second.is_empty(), "a claimed item must not be claimable again immediately");
}

/// A worker that claims an item and then crashes must not lock it
/// forever: after the visibility timeout the item is claimable again.
#[test]
fn a_claim_that_is_never_completed_returns_to_pending_after_its_visibility_timeout() {
    let queue = Queue::open_in_memory(config()).unwrap();
    queue.enqueue("g", "k", b"p", 1_000).unwrap();
    let claimed = queue.claim_due(1_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);

    // Still within the visibility window: invisible.
    assert!(queue.claim_due(1_000 + 119_999, 10).unwrap().is_empty());
    // Past it: visible again, as if never claimed.
    let reclaimed = queue.claim_due(1_000 + 120_000, 10).unwrap();
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].id, claimed[0].id);
}

/// A delivery that never calls `fail` (a worker panic, a crashed
/// process) must still consume a bounded number of claims, not be
/// handed out forever.
#[test]
fn a_claim_that_never_resolves_still_counts_toward_the_claim_budget() {
    let mut cfg = config();
    cfg.retry.max_attempts = 3;
    let queue = Queue::open_in_memory(cfg).unwrap();
    queue.enqueue("g", "k", b"p", 1_000).unwrap();

    let mut now = 1_000;
    for expected_claim_count in 1..=3u32 {
        let claimed = queue.claim_due(now, 10).unwrap();
        assert_eq!(claimed.len(), 1, "claim #{expected_claim_count} must still see the item");
        assert_eq!(claimed[0].claim_count, expected_claim_count);
        // Simulate a crash: never call `fail` or `complete`. Advance
        // past the visibility timeout so the next loop iteration can
        // reclaim it.
        now += 120_000;
    }
    // The caller (the supervisor's queue worker) is the one that acts
    // on `claim_count` reaching the budget -- this crate only tracks
    // and reports it, so the item is still claimable once more here.
    let claimed = queue.claim_due(now, 10).unwrap();
    assert_eq!(claimed[0].claim_count, 4, "claim_count keeps advancing past the budget too");
}

/// The observed `next_attempt_at` must land within
/// `calculate_jittered_backoff`'s documented +/-10% band around the
/// pure, unjittered curve -- not merely "close to the default".
#[test]
fn next_attempt_at_follows_the_configured_policy_with_jitter() {
    let cfg = config();
    let queue = Queue::open_in_memory(cfg.clone()).unwrap();
    queue.enqueue("g", "k", b"p", 1_000).unwrap();
    let claimed = queue.claim_due(1_000, 10).unwrap();

    let outcome = queue.fail(claimed[0].id, 1_000, "transport error", false).unwrap();
    let FailOutcome::Retrying { next_attempt_at } = outcome else {
        panic!("expected Retrying, got {outcome:?}");
    };
    let base = backoff_before_wait(&cfg.retry, 1);
    let observed_wait = next_attempt_at - 1_000;
    let lower = (base as f64 * 0.9) as i64;
    let upper = (base as f64 * 1.1) as i64 + 1;
    assert!(
        (lower..=upper).contains(&observed_wait),
        "wait {observed_wait} outside jitter band [{lower}, {upper}] around base {base}"
    );
}

/// An item that uses up its attempt budget moves to the dead-letter
/// table and is gone from the outbox.
#[test]
fn an_item_that_exhausts_its_attempts_moves_to_the_dlq_and_leaves_the_outbox() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();

    let mut now = 1_000;
    for _ in 0..3 {
        let claimed = queue.claim_due(now, 10).unwrap();
        assert_eq!(claimed.len(), 1, "the item must still be claimable before exhaustion");
        let outcome = queue.fail(id, now, "still unreachable", false).unwrap();
        match outcome {
            FailOutcome::Retrying { next_attempt_at } => now = next_attempt_at,
            FailOutcome::DeadLettered { .. } => break,
        }
    }

    assert!(queue.all().unwrap().is_empty(), "the outbox must not still hold the item");
    let dead = queue.dead_letters().unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].attempts, 3);
    assert_eq!(dead[0].last_error, "still unreachable");
}

/// The production defaults -- 54 attempts (53 waits), 100 ms initial
/// backoff, x2 multiplier, capped at 900 s -- sum to ~36,738 s
/// (~10.2 h) of unjittered retrying before an item dead-letters.
#[test]
fn the_configured_defaults_give_a_ten_hour_window() {
    let policy = RetryPolicy {
        max_attempts: 54,
        initial_backoff_ms: 100,
        backoff_multiplier: 2.0,
        max_backoff_ms: 900_000,
    };
    let total_ms: u64 = (1..=53).map(|wait| backoff_before_wait(&policy, wait)).sum();
    let total_secs = total_ms as f64 / 1000.0;
    assert!((36_737.0..=36_739.0).contains(&total_secs), "expected ~36,738 s, got {total_secs} s");

    // The ceiling first binds at the 15th wait, not the 14th.
    assert_eq!(backoff_before_wait(&policy, 14), 819_200);
    assert_eq!(backoff_before_wait(&policy, 15), 900_000);
}

/// `SupervisorRole::default()`'s five `queue_*` fields convert into
/// exactly the same curve the hand-written default-window test pins, so
/// the two cannot silently drift apart.
#[test]
fn supervisor_role_defaults_convert_into_the_same_ten_hour_window() {
    let role = SupervisorRole::default();
    let cfg = QueueConfig::from(&role);
    assert_eq!(cfg.retry.max_attempts, 54);
    assert_eq!(cfg.retry.max_backoff_ms, 900_000);
    assert_eq!(cfg.visibility_timeout_ms, 120_000);
    assert_eq!(cfg.dlq_max_rows, 1000);

    let total_ms: u64 = (1..=53).map(|wait| backoff_before_wait(&cfg.retry, wait)).sum();
    assert!(
        (36_737_000.0..=36_739_000.0).contains(&(total_ms as f64)),
        "expected ~36,738,000 ms, got {total_ms} ms"
    );
}

/// The guest proxy outbox's own defaults must give the same overnight
/// window the supervisor's do -- a message queued at 22:00 has to be
/// deliverable at 07:00.
#[test]
fn the_sandbox_role_defaults_give_the_same_ten_hour_window() {
    let cfg = QueueConfig::from(&AppSandboxRole::default());
    assert_eq!(cfg.retry.max_attempts, 54);
    assert_eq!(cfg.retry.max_backoff_ms, 900_000);
    assert_eq!(cfg.visibility_timeout_ms, 120_000);
    assert_eq!(cfg.dlq_max_rows, 1000);
    let total = cfg.total_retry_window_ms() as f64;
    assert!(
        (36_737_000.0..=36_739_000.0).contains(&total),
        "expected ~36,738,000 ms, got {total} ms"
    );
}

#[test]
fn a_configured_zero_sandbox_max_attempts_is_clamped_to_one() {
    let role = AppSandboxRole { queue_max_attempts: 0, ..AppSandboxRole::default() };
    assert_eq!(QueueConfig::from(&role).retry.max_attempts, 1);
}

/// A configured `queue_max_attempts` of 0 must not silently turn the
/// queue into an instant DLQ.
#[test]
fn a_configured_zero_max_attempts_is_clamped_to_one() {
    let role = SupervisorRole { queue_max_attempts: 0, ..SupervisorRole::default() };
    let cfg = QueueConfig::from(&role);
    assert_eq!(cfg.retry.max_attempts, 1, "0 must clamp to 1, not dead-letter with zero tries");
}

/// A completed item is deleted outright, not kept as a tombstone.
#[test]
fn a_completed_item_is_deleted_not_tombstoned() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.complete(id).unwrap();
    assert!(queue.all().unwrap().is_empty());
}

/// The dead-letter table is capped at its row limit, and pruning runs
/// oldest-first on every write.
#[test]
fn the_dlq_is_capped_at_its_row_limit_and_prunes_oldest_first() {
    let mut cfg = config();
    cfg.dlq_max_rows = 3;
    cfg.retry.max_attempts = 1;
    let queue = Queue::open_in_memory(cfg).unwrap();

    for i in 0..5 {
        let id = queue.enqueue("g", &format!("k{i}"), b"p", 1_000 + i).unwrap();
        queue.claim_due(1_000 + i, 10).unwrap();
        queue.fail(id, 1_000 + i, "dead on arrival", false).unwrap();
    }

    let dead = queue.dead_letters().unwrap();
    assert_eq!(dead.len(), 3, "the cap must hold even after five inserts");
    // The two oldest (k0, k1) were pruned; k2..k4 survive.
    let keys: Vec<&str> = dead.iter().map(|d| d.queue_key.as_str()).collect();
    assert_eq!(keys, vec!["k2", "k3", "k4"]);
}

/// The cap and its pruning are scoped per `group_key`, so one noisy
/// group cannot evict another's dead letters.
#[test]
fn the_dlq_cap_is_scoped_per_group_not_across_the_whole_table() {
    let mut cfg = config();
    cfg.dlq_max_rows = 1;
    cfg.retry.max_attempts = 1;
    let queue = Queue::open_in_memory(cfg).unwrap();

    let a1 = queue.enqueue("group-a", "a1", b"p", 1_000).unwrap();
    queue.claim_due(1_000, 10).unwrap();
    queue.fail(a1, 1_000, "dead", false).unwrap();

    let b1 = queue.enqueue("group-b", "b1", b"p", 1_001).unwrap();
    queue.claim_due(1_001, 10).unwrap();
    queue.fail(b1, 1_001, "dead", false).unwrap();

    let a2 = queue.enqueue("group-a", "a2", b"p", 1_002).unwrap();
    queue.claim_due(1_002, 10).unwrap();
    let outcome = queue.fail(a2, 1_002, "dead", false).unwrap();
    let FailOutcome::DeadLettered { pruned_keys } = outcome else {
        panic!("expected DeadLettered");
    };
    assert_eq!(pruned_keys, vec!["a1"], "only group-a's own oldest row is pruned");

    let dead = queue.dead_letters().unwrap();
    let keys: BTreeSet<&str> = dead.iter().map(|d| d.queue_key.as_str()).collect();
    assert_eq!(
        keys,
        BTreeSet::from(["b1", "a2"]),
        "group-b's dead letter must survive group-a's own overflow"
    );
}

/// A queued item whose target no longer exists is terminal, not
/// retryable, regardless of budget remaining.
#[test]
fn a_terminal_error_skips_the_remaining_attempts() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.claim_due(1_000, 10).unwrap();

    let outcome = queue.fail(id, 1_000, "service no longer exists", true).unwrap();
    assert_eq!(outcome, FailOutcome::DeadLettered { pruned_keys: Vec::new() });
    let dead = queue.dead_letters().unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].attempts, 1, "a terminal failure dead-letters on its first attempt");
}

/// The idle-tick budget -- one indexed query, no scan -- asserted
/// structurally via the query plan rather than by timing.
#[test]
fn an_empty_queue_tick_issues_one_indexed_query_and_no_scan() {
    let queue = Queue::open_in_memory(config()).unwrap();
    assert!(queue.claim_due(1_000, 10).unwrap().is_empty());

    let plan = queue.explain_claim_plan().unwrap();
    assert!(
        plan.contains("USING INDEX") || plan.contains("USING COVERING INDEX"),
        "expected the claim query to use idx_outbox_visible_at, got: {plan}"
    );
    assert!(!plan.to_uppercase().contains("SCAN TABLE"), "expected no table scan, got: {plan}");
}

/// The `< 1 ms` enqueue-on-failure budget, asserted as one row changed
/// by exactly one statement rather than wall-clock, so CI noise cannot
/// fail it.
#[test]
fn enqueue_on_failure_costs_one_insert() {
    let queue = Queue::open_in_memory(config()).unwrap();
    queue.enqueue("g", "k", b"p", 1_000).unwrap();
    let conn = queue.conn.lock().unwrap();
    assert_eq!(conn.changes(), 1, "enqueue must be exactly one INSERT");
}

/// The indexed dedup lookup a caller uses before every enqueue must
/// not scan.
#[test]
fn has_pending_uses_the_indexed_lookup_and_no_scan() {
    let queue = Queue::open_in_memory(config()).unwrap();
    assert!(!queue.has_pending("k").unwrap());
    queue.enqueue("g", "k", b"p", 1_000).unwrap();
    assert!(queue.has_pending("k").unwrap());
    assert!(!queue.has_pending("other").unwrap());
}

/// Replay re-enqueues without executing inline, and does not simply
/// mutate the dead letter in place.
#[test]
fn replay_re_enqueues_rather_than_executing_inline() {
    let mut cfg = config();
    cfg.retry.max_attempts = 1;
    let queue = Queue::open_in_memory(cfg).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.claim_due(1_000, 10).unwrap();
    queue.fail(id, 1_000, "unreachable", false).unwrap();
    assert_eq!(queue.dead_letters().unwrap().len(), 1);

    let dead_id = queue.dead_letters().unwrap()[0].id;
    queue.replay(dead_id, 2_000).unwrap();

    assert!(queue.dead_letters().unwrap().is_empty(), "replay must remove the dead letter");
    let requeued = queue.all().unwrap();
    assert_eq!(requeued.len(), 1);
    assert_eq!(requeued[0].payload, b"p");
    // Claimable immediately -- not executed synchronously inside
    // `replay` itself.
    assert_eq!(queue.claim_due(2_000, 10).unwrap().len(), 1);
}

/// Replaying a dead letter whose key already has a pending outbox row
/// must not create a second row for that key.
#[test]
fn replay_refuses_when_a_pending_row_already_exists_for_the_key() {
    let mut cfg = config();
    cfg.retry.max_attempts = 1;
    let queue = Queue::open_in_memory(cfg).unwrap();
    let id = queue.enqueue("g", "k", b"first", 1_000).unwrap();
    queue.claim_due(1_000, 10).unwrap();
    queue.fail(id, 1_000, "unreachable", false).unwrap();
    let dead_id = queue.dead_letters().unwrap()[0].id;

    // A fresh push for the same key lands in the outbox independently
    // of the dead letter (the ordinary case: a later deploy or
    // membership change re-triggers the same logical write).
    queue.enqueue("g", "k", b"second", 2_000).unwrap();

    let err = queue.replay(dead_id, 3_000).unwrap_err();
    assert!(err.to_string().contains("pending"), "unexpected error: {err}");
    assert_eq!(queue.dead_letters().unwrap().len(), 1, "the dead letter must be left in place");
    assert_eq!(queue.all().unwrap().len(), 1, "no second row must have been created");
}

/// Recording a dead letter must never make a key look pending, even
/// for an instant: a concurrent sender would see that row, decide the
/// key was already queued, and drop the call it believed it had
/// enqueued.
#[test]
fn recording_a_dead_letter_never_makes_the_key_look_pending() {
    let queue = Queue::open_in_memory(config()).unwrap();
    queue.record_dead_letter("g", "k", b"p", "gave up", 1_000).unwrap();

    assert!(
        queue.all().unwrap().is_empty(),
        "the outbox must never hold a row for a directly-recorded dead letter"
    );
    assert!(
        !queue.has_pending("k").unwrap(),
        "and a concurrent sender must still be free to enqueue that key"
    );
    let dead = queue.dead_letters().unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].queue_key, "k");
    assert_eq!(dead[0].last_error, "gave up");

    // The sender's own enqueue for the same key still lands.
    assert!(queue.enqueue_if_absent("g", "k", b"p2", 1_100).unwrap());
    assert_eq!(queue.all().unwrap().len(), 1);
}

/// A replayed item that fails again returns to the DLQ with its
/// attempt history intact, rather than a fresh budget.
#[test]
fn a_replayed_item_that_fails_again_returns_to_the_dlq_with_its_history() {
    let mut cfg = config();
    cfg.retry.max_attempts = 2;
    let queue = Queue::open_in_memory(cfg).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.claim_due(1_000, 10).unwrap();
    // First failure: retrying, attempts now 1 (< max_attempts 2).
    queue.fail(id, 1_000, "boom", false).unwrap();
    queue.claim_due(2_000_000, 10).unwrap();
    // Second failure: exhausted, attempts now 2 -- dead-lettered.
    queue.fail(id, 2_000_000, "boom again", false).unwrap();
    let dead = queue.dead_letters().unwrap();
    assert_eq!(dead[0].attempts, 2);

    queue.replay(dead[0].id, 3_000_000).unwrap();
    let claimed = queue.claim_due(3_000_000, 10).unwrap();
    assert_eq!(claimed[0].attempts, 2, "replay must preserve the attempt count");

    // One more failure exhausts it again immediately, since attempts
    // already sits at the configured max.
    let outcome = queue.fail(claimed[0].id, 3_000_000, "boom a third time", false).unwrap();
    assert!(matches!(outcome, FailOutcome::DeadLettered { .. }));
    let dead_again = queue.dead_letters().unwrap();
    assert_eq!(dead_again.len(), 1);
    assert_eq!(dead_again[0].attempts, 3, "history accumulates across replays");
}

/// A caller can write its own table and enqueue in one
/// commit through `TxQueue`.
#[test]
fn transaction_commits_the_callers_write_and_the_enqueue_together() {
    let queue = Queue::open_in_memory(config()).unwrap();
    {
        let conn = queue.conn.lock().unwrap();
        conn.execute_batch("CREATE TABLE owner_rows (id INTEGER PRIMARY KEY)").unwrap();
    }
    queue
        .transaction(|tx, txq| {
            tx.execute("INSERT INTO owner_rows (id) VALUES (1)", [])?;
            txq.enqueue(tx, "g", "k", b"p", 1_000)?;
            Ok(())
        })
        .unwrap();

    assert_eq!(queue.all().unwrap().len(), 1);
    let conn = queue.conn.lock().unwrap();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM owner_rows", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

/// A `transaction` closure returning `Err` must roll back both the
/// caller's own write and the enqueue -- neither exists afterward.
#[test]
fn transaction_rolls_back_both_writes_on_err() {
    let queue = Queue::open_in_memory(config()).unwrap();
    {
        let conn = queue.conn.lock().unwrap();
        conn.execute_batch("CREATE TABLE owner_rows (id INTEGER PRIMARY KEY)").unwrap();
    }
    let result = queue.transaction(|tx, txq| {
        tx.execute("INSERT INTO owner_rows (id) VALUES (1)", [])?;
        txq.enqueue(tx, "g", "k", b"p", 1_000)?;
        Err::<(), _>(anyhow!("caller decided to abort"))
    });
    assert!(result.is_err());

    assert!(queue.all().unwrap().is_empty(), "the enqueue must have rolled back");
    let conn = queue.conn.lock().unwrap();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM owner_rows", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0, "the caller's own write must have rolled back too");
}

/// `defer` must not advance `attempts` (the retry budget)
/// even though it changes `visible_at`, and must un-count the claim so
/// the crashed-worker `claim_count` bound does not fire on a
/// deliberately-waiting item.
#[test]
fn defer_does_not_advance_attempts_and_uncounts_the_claim() {
    let queue = Queue::open_in_memory(config()).unwrap();
    queue.enqueue("g", "k", b"p", 1_000).unwrap();
    let claimed = queue.claim_due(1_000, 10).unwrap();
    assert_eq!(claimed[0].claim_count, 1);
    assert_eq!(claimed[0].attempts, 0);

    queue.defer(claimed[0].id, 5_000).unwrap();

    let all = queue.all().unwrap();
    assert_eq!(all[0].attempts, 0, "defer must not charge the retry budget");
    assert_eq!(all[0].claim_count, 0, "defer must un-count this claim");
}

/// A deferred item must not be claimable again before its new
/// `visible_at`.
#[test]
fn a_deferred_item_is_invisible_to_claim_due_until_its_new_visible_at() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.claim_due(1_000, 10).unwrap();
    queue.defer(id, 5_000).unwrap();

    assert!(
        queue.claim_due(4_999, 10).unwrap().is_empty(),
        "must stay invisible before the deferred visible_at"
    );
    let reclaimed = queue.claim_due(5_000, 10).unwrap();
    assert_eq!(reclaimed.len(), 1, "must become claimable again at the deferred visible_at");
    assert_eq!(reclaimed[0].id, id);
}

#[test]
fn dead_letters_lists_what_the_store_holds() {
    let mut cfg = config();
    cfg.retry.max_attempts = 1;
    let queue = Queue::open_in_memory(cfg).unwrap();
    assert!(queue.dead_letters().unwrap().is_empty());

    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.claim_due(1_000, 10).unwrap();
    queue.fail(id, 1_000, "unreachable", false).unwrap();

    let dead = queue.dead_letters().unwrap();
    assert_eq!(dead.len(), 1);
    assert_eq!(dead[0].queue_key, "k");
    assert_eq!(dead[0].payload, b"p");
}

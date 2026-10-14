use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use rusqlite::{Connection, OptionalExtension};

use super::*;

#[test]
fn a_parked_item_is_never_returned_by_claim_due() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"payload", 1_000).unwrap();
    let claimed = queue.claim_due(1_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);

    queue.park(id, 1_000, 1_000, 100_000, b"receipt").unwrap();
    let due = queue.claim_due(200_000, 10).unwrap();
    assert!(due.is_empty(), "a parked item must never be returned by claim_due");
}

#[test]
fn a_parked_item_is_claimed_by_claim_parked_due_after_its_poll_interval() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"payload", 1_000).unwrap();
    queue.park(id, 1_000, 2_000, 100_000, b"context-data").unwrap();

    // Not due 1 ms before poll interval
    assert!(queue.claim_parked_due(2_999, 10).unwrap().is_empty());

    // Due at parked_at + poll
    let claimed = queue.claim_parked_due(3_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, id);
    let parked = claimed[0].parked.as_ref().expect("expected parked info");
    assert_eq!(parked.parked_at, 1_000);
    assert_eq!(parked.deadline_at, 100_000);
    assert_eq!(parked.poll_interval_ms, 2_000);
    assert_eq!(parked.context, b"context-data");
}

#[test]
fn a_parked_claim_is_hidden_for_the_visibility_timeout() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"payload", 1_000).unwrap();
    queue.park(id, 1_000, 1_000, 100_000, b"ctx").unwrap();

    let claimed = queue.claim_parked_due(2_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);

    let visibility_timeout = config().visibility_timeout_ms as i64;
    assert!(queue.claim_parked_due(2_000 + visibility_timeout - 1, 10).unwrap().is_empty());

    let reclaimed = queue.claim_parked_due(2_000 + visibility_timeout, 10).unwrap();
    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].id, id);
}

#[test]
fn parking_uncounts_the_claim_and_keeps_attempts() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"payload", 1_000).unwrap();
    queue.claim_due(1_000, 10).unwrap();
    queue.fail(id, 1_000, "temp error", false).unwrap();

    // Claim second time
    let claimed = queue.claim_due(100_000, 10).unwrap();
    assert_eq!(claimed[0].claim_count, 2);
    assert_eq!(claimed[0].attempts, 1);

    queue.park(id, 100_000, 1_000, 200_000, b"ctx").unwrap();

    let all = queue.all().unwrap();
    assert_eq!(all[0].claim_count, 1, "first park uncounts the claim");
    assert_eq!(all[0].attempts, 1, "park keeps attempts count");
}

#[test]
fn parking_an_unclaimed_item_keeps_claim_count_at_zero() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"payload", 1_000).unwrap();

    queue.park(id, 1_000, 1_000, 50_000, b"ctx").unwrap();

    let all = queue.all().unwrap();
    assert_eq!(all[0].claim_count, 0, "claim_count must floor at 0");
}

#[test]
fn parked_claims_never_advance_claim_count_or_attempts() {
    let mut cfg = config();
    cfg.retry.max_attempts = 3;
    let queue = Queue::open_in_memory(cfg).unwrap();
    let id = queue.enqueue("g", "k", b"payload", 1_000).unwrap();
    queue.claim_due(1_000, 10).unwrap();
    queue.park(id, 1_000, 1_000, 100_000, b"ctx").unwrap();

    let mut now = 2_000;
    for _ in 0..10 {
        let claimed = queue.claim_parked_due(now, 10).unwrap();
        assert_eq!(claimed.len(), 1);
        queue.park(id, now, 1_000, 100_000, b"ctx").unwrap();
        now += 1_000;
    }

    let all = queue.all().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].claim_count, 0);
    assert_eq!(all[0].attempts, 0);
    assert!(queue.dead_letters().unwrap().is_empty());
}

#[test]
fn each_claim_returns_only_its_own_kind() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id_pending = queue.enqueue("g", "k-pending", b"p", 1_000).unwrap();
    let id_parked = queue.enqueue("g", "k-parked", b"p", 1_000).unwrap();
    queue.park(id_parked, 1_000, 1_000, 10_000, b"ctx").unwrap();

    let due_pending = queue.claim_due(2_000, 10).unwrap();
    assert_eq!(due_pending.len(), 1);
    assert_eq!(due_pending[0].id, id_pending);

    let due_parked = queue.claim_parked_due(2_000, 10).unwrap();
    assert_eq!(due_parked.len(), 1);
    assert_eq!(due_parked[0].id, id_parked);

    // Now test with two fresh rows of each kind, all due, and limit = 1
    let _p1 = queue.enqueue("g", "lim-pending-1", b"p", 3_000).unwrap();
    let _p2 = queue.enqueue("g", "lim-pending-2", b"p", 3_000).unwrap();
    let pk1 = queue.enqueue("g", "lim-parked-1", b"p", 3_000).unwrap();
    queue.park(pk1, 3_000, 500, 10_000, b"ctx").unwrap();
    let pk2 = queue.enqueue("g", "lim-parked-2", b"p", 3_000).unwrap();
    queue.park(pk2, 3_000, 500, 10_000, b"ctx").unwrap();

    let lim_pending_1 = queue.claim_due(4_000, 1).unwrap();
    assert_eq!(lim_pending_1.len(), 1);
    let lim_pending_2 = queue.claim_due(4_000, 1).unwrap();
    assert_eq!(lim_pending_2.len(), 1);
    assert_ne!(lim_pending_1[0].id, lim_pending_2[0].id);
    let remaining_pending = queue.claim_due(4_000, 1).unwrap();
    assert!(remaining_pending.is_empty());

    let lim_parked_1 = queue.claim_parked_due(4_000, 1).unwrap();
    assert_eq!(lim_parked_1.len(), 1);
    let lim_parked_2 = queue.claim_parked_due(4_000, 1).unwrap();
    assert_eq!(lim_parked_2.len(), 1);
    assert_ne!(lim_parked_1[0].id, lim_parked_2[0].id);
    let remaining_parked = queue.claim_parked_due(4_000, 1).unwrap();
    assert!(remaining_parked.is_empty());
}

#[test]
fn a_parked_item_is_claimable_at_its_deadline_even_when_the_poll_is_later() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    // poll = 10_000, deadline = 4_000 (now + 3_000)
    queue.park(id, 1_000, 10_000, 4_000, b"ctx").unwrap();

    // 1 ms before deadline, not claimable
    assert!(queue.claim_parked_due(3_999, 10).unwrap().is_empty());

    // Claimable at deadline
    let claimed = queue.claim_parked_due(4_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);
    let parked = claimed[0].parked.as_ref().unwrap();
    assert!(parked.is_past_deadline(4_000));
    assert!(!parked.is_past_deadline(3_999));
}

#[test]
fn a_re_park_keeps_parked_at_and_replaces_the_context() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.park(id, 1_000, 1_000, 10_000, b"ctx-1").unwrap();

    queue.park(id, 2_000, 2_000, 20_000, b"ctx-2").unwrap();

    let all = queue.all().unwrap();
    let parked = all[0].parked.as_ref().unwrap();
    assert_eq!(parked.parked_at, 1_000, "re-park must keep original parked_at");
    assert_eq!(parked.deadline_at, 20_000);
    assert_eq!(parked.poll_interval_ms, 2_000);
    assert_eq!(parked.context, b"ctx-2");
}

#[test]
fn unpark_returns_the_item_to_claim_due_with_its_counters() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.claim_due(1_000, 10).unwrap();
    queue.fail(id, 1_000, "temp", false).unwrap();
    // Now attempts = 1. Claim again: claim_count = 2
    queue.claim_due(100_000, 10).unwrap();
    queue.park(id, 100_000, 1_000, 200_000, b"ctx").unwrap();
    // After park: attempts = 1, claim_count = 1

    queue.unpark(id, 150_000).unwrap();

    assert!(queue.claim_parked_due(200_000, 10).unwrap().is_empty());
    let claimed = queue.claim_due(150_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, id);
    assert!(claimed[0].parked.is_none());
    assert_eq!(claimed[0].attempts, 1);
    assert_eq!(claimed[0].claim_count, 2, "claim_due advanced claim_count from 1 to 2");
}

#[test]
fn unpark_of_a_pending_item_is_an_error() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    assert!(queue.unpark(id, 2_000).is_err());
}

#[test]
fn park_of_a_missing_item_is_an_error() {
    let queue = Queue::open_in_memory(config()).unwrap();
    assert!(queue.park(999, 1_000, 1_000, 2_000, b"ctx").is_err());
}

#[test]
fn park_refuses_a_zero_poll_interval_and_a_past_deadline() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();

    // Zero poll interval refused
    assert!(queue.park(id, 1_000, 0, 5_000, b"ctx").is_err());

    // Deadline not after now refused
    assert!(queue.park(id, 1_000, 1_000, 1_000, b"ctx").is_err());
    assert!(queue.park(id, 1_000, 1_000, 999, b"ctx").is_err());
}

#[test]
fn a_non_terminal_fail_on_a_parked_item_makes_it_pending() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.park(id, 1_000, 1_000, 10_000, b"ctx").unwrap();
    let claimed = queue.claim_parked_due(2_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);

    let outcome = queue.fail(id, 2_000, "failure", false).unwrap();
    match outcome {
        FailOutcome::Retrying { next_attempt_at } => {
            let all = queue.all().unwrap();
            assert_eq!(all[0].parked, None);
            assert_eq!(all[0].attempts, 1);
            assert_eq!(queue.claim_due(next_attempt_at, 10).unwrap().len(), 1);
        }
        FailOutcome::DeadLettered { .. } => panic!("expected Retrying outcome"),
    }
}

#[test]
fn a_non_terminal_fail_on_a_parked_item_at_budget_dead_letters_it() {
    let mut cfg = config();
    cfg.retry.max_attempts = 2;
    let queue = Queue::open_in_memory(cfg).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();

    // Attempt 1 fails
    queue.claim_due(1_000, 10).unwrap();
    queue.fail(id, 1_000, "err1", false).unwrap();

    // Claim, park, claim parked
    queue.claim_due(100_000, 10).unwrap();
    queue.park(id, 100_000, 1_000, 200_000, b"ctx").unwrap();
    queue.claim_parked_due(101_000, 10).unwrap();

    // Attempt 2 fails (reaches max_attempts = 2)
    let outcome = queue.fail(id, 101_000, "err2", false).unwrap();
    assert_eq!(outcome, FailOutcome::DeadLettered { pruned_keys: Vec::new() });
    assert!(queue.all().unwrap().is_empty());

    let dl = queue.dead_letters().unwrap();
    assert_eq!(dl.len(), 1);
    assert_eq!(dl[0].attempts, 2);
    assert_eq!(dl[0].last_error, "err2");
}

#[test]
fn a_terminal_fail_on_a_parked_item_dead_letters_it() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"original-payload", 1_000).unwrap();
    queue.park(id, 1_000, 1_000, 10_000, b"deposit-context").unwrap();

    let outcome = queue.fail(id, 2_000, "terminal err", true).unwrap();
    assert_eq!(outcome, FailOutcome::DeadLettered { pruned_keys: Vec::new() });

    let dl = queue.dead_letters().unwrap();
    assert_eq!(dl.len(), 1);
    assert_eq!(
        dl[0].payload, b"original-payload",
        "dead letter keeps original payload, not context"
    );
}

#[test]
fn complete_removes_a_parked_item() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.park(id, 1_000, 1_000, 10_000, b"ctx").unwrap();

    queue.complete(id).unwrap();
    assert!(queue.all().unwrap().is_empty());
}

#[test]
fn the_parked_claim_query_uses_its_index() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let plan = queue.explain_claim_plan(ClaimKind::Parked).unwrap();
    assert!(
        plan.contains("idx_outbox_parked_due")
            && (plan.contains("USING INDEX") || plan.contains("USING COVERING INDEX")),
        "expected parked query to use idx_outbox_parked_due, got: {plan}"
    );
    assert!(!plan.to_uppercase().contains("SCAN TABLE"), "expected no table scan, got: {plan}");
}

#[test]
fn defer_has_no_effect_on_a_parked_item() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.park(id, 1_000, 2_000, 10_000, b"ctx").unwrap();

    // Calling defer on a parked item
    queue.defer(id, 50_000).unwrap();

    let all = queue.all().unwrap();
    let parked = all[0].parked.as_ref().unwrap();
    assert_eq!(parked.context, b"ctx");
    assert_eq!(all[0].claim_count, 0);

    // Visible_at must not have moved to 50_000; it is still due at 3_000
    let claimed = queue.claim_parked_due(3_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);
}

#[test]
fn a_parked_item_still_holds_its_key() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.park(id, 1_000, 1_000, 10_000, b"ctx").unwrap();

    assert!(queue.has_pending("k").unwrap());
    assert!(!queue.enqueue_if_absent("g", "k", b"p2", 1_000).unwrap());
    assert_eq!(queue.pending_count().unwrap(), 1);

    // Recording dead letter and replaying with same key
    queue.record_dead_letter("g", "k", b"dl", "err", 1_000).unwrap();
    let dl = queue.dead_letters().unwrap();
    assert!(queue.replay(dl[0].id, 1_000).is_err(), "replay must refuse when key has parked row");
}

#[test]
fn a_parked_item_survives_reopening_the_database() {
    let temp_dir = tempfile::tempdir().unwrap();
    {
        let queue = Queue::open(temp_dir.path(), "test.db", config()).unwrap();
        let id = queue.enqueue("g", "k", b"payload", 1_000).unwrap();
        queue.park(id, 1_000, 2_000, 50_000, b"saved-ctx").unwrap();
    }
    {
        let queue = Queue::open(temp_dir.path(), "test.db", config()).unwrap();
        let all = queue.all().unwrap();
        assert_eq!(all.len(), 1);
        let parked = all[0].parked.as_ref().unwrap();
        assert_eq!(parked.parked_at, 1_000);
        assert_eq!(parked.deadline_at, 50_000);
        assert_eq!(parked.poll_interval_ms, 2_000);
        assert_eq!(parked.context, b"saved-ctx");
    }
}

#[test]
fn park_inside_a_transaction_commits_with_the_owners_write() {
    let queue = Queue::open_in_memory(config()).unwrap();
    {
        let conn = queue.conn.lock().unwrap();
        conn.execute("CREATE TABLE owner_tbl (id INTEGER PRIMARY KEY, note TEXT)", []).unwrap();
    }
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();

    // Successful commit with TxQueue::park
    queue
        .transaction(|tx, txq| {
            tx.execute("INSERT INTO owner_tbl (id, note) VALUES (1, 'deposited')", [])?;
            txq.park(tx, id, 1_000, 2_000, 50_000, b"tx-ctx")?;
            Ok(())
        })
        .unwrap();

    let all = queue.all().unwrap();
    assert!(all[0].parked.is_some());
    {
        let conn = queue.conn.lock().unwrap();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM owner_tbl", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    // Rollback with TxQueue::park
    let id2 = queue.enqueue("g", "k2", b"p", 1_000).unwrap();
    let res: Result<()> = queue.transaction(|tx, txq| {
        tx.execute("INSERT INTO owner_tbl (id, note) VALUES (2, 'aborted')", [])?;
        txq.park(tx, id2, 1_000, 2_000, 50_000, b"tx-ctx")?;
        Err(anyhow!("rollback"))
    });
    assert!(res.is_err());
    let all2 = queue.all().unwrap();
    let item2 = all2.iter().find(|i| i.id == id2).unwrap();
    assert!(item2.parked.is_none(), "rolled back park must leave item pending");
    {
        let conn = queue.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM owner_tbl WHERE id = 2", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    // Successful commit with TxQueue::unpark
    queue
        .transaction(|tx, txq| {
            tx.execute("UPDATE owner_tbl SET note = 'unparked' WHERE id = 1", [])?;
            txq.unpark(tx, id, 5_000)?;
            Ok(())
        })
        .unwrap();
    let all3 = queue.all().unwrap();
    let item1 = all3.iter().find(|i| i.id == id).unwrap();
    assert!(item1.parked.is_none());

    // Rollback with TxQueue::unpark
    queue.park(id, 5_000, 1_000, 20_000, b"ctx").unwrap();
    let res: Result<()> = queue.transaction(|tx, txq| {
        tx.execute("UPDATE owner_tbl SET note = 'bad' WHERE id = 1", [])?;
        txq.unpark(tx, id, 6_000)?;
        Err(anyhow!("rollback"))
    });
    assert!(res.is_err());
    let all4 = queue.all().unwrap();
    let item1_after = all4.iter().find(|i| i.id == id).unwrap();
    assert!(item1_after.parked.is_some(), "rolled back unpark must leave item parked");
}

fn index_exists(conn: &Connection, index_name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1",
        [index_name],
        |_| Ok(()),
    )
    .optional()
    .unwrap()
    .is_some()
}

fn create_pre_park_outbox(conn: &Connection, queue_key: &str) {
    conn.execute_batch(
        "CREATE TABLE outbox (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            group_key   TEXT NOT NULL,
            queue_key   TEXT NOT NULL,
            payload     BLOB NOT NULL,
            attempts    INTEGER NOT NULL DEFAULT 0,
            claim_count INTEGER NOT NULL DEFAULT 0,
            visible_at  INTEGER NOT NULL,
            created_at  INTEGER NOT NULL
         );
         CREATE INDEX idx_outbox_visible_at ON outbox(visible_at);
         CREATE INDEX idx_outbox_queue_key ON outbox(queue_key);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO outbox (group_key, queue_key, payload, attempts, claim_count, visible_at, \
         created_at)
         VALUES ('g', ?1, X'010203', 0, 0, 1000, 1000)",
        [queue_key],
    )
    .unwrap();
}

#[test]
fn an_outbox_created_without_park_columns_gains_them_on_open() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("old.db");

    // Open raw rusqlite connection and create old 8-column schema
    {
        let conn = Connection::open(&db_path).unwrap();
        create_pre_park_outbox(&conn, "k");
    }

    // First Queue::open upgrades schema
    {
        let queue = Queue::open(temp_dir.path(), "old.db", config()).unwrap();
        let claimed = queue.claim_due(1_000, 10).unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].parked, None);
        assert_eq!(claimed[0].payload, vec![1, 2, 3]);

        // It can now be parked
        queue.park(claimed[0].id, 1_000, 1_000, 10_000, b"new-ctx").unwrap();

        // Old index idx_outbox_visible_at was dropped
        let conn = queue.conn.lock().unwrap();
        assert!(!index_exists(&conn, "idx_outbox_visible_at"), "old index must be dropped");

        // Both partial indexes that replaced it must exist.
        assert!(index_exists(&conn, "idx_outbox_due"), "idx_outbox_due must be created on upgrade");
        assert!(
            index_exists(&conn, "idx_outbox_parked_due"),
            "idx_outbox_parked_due must be created on upgrade"
        );
    }

    // Second open proves the ALTER is idempotent
    {
        let queue = Queue::open(temp_dir.path(), "old.db", config()).unwrap();
        let all = queue.all().unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].parked.is_some());
    }
}

#[test]
fn park_context_above_limit_is_refused() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    let big = vec![0u8; MAX_PARK_CONTEXT_BYTES + 1];
    assert!(
        queue.park(id, 1_000, 1_000, 5_000, &big).is_err(),
        "park_context above {MAX_PARK_CONTEXT_BYTES} bytes must be refused"
    );

    // Exactly at the limit is accepted.
    let at_limit = vec![0u8; MAX_PARK_CONTEXT_BYTES];
    queue.park(id, 1_000, 1_000, 5_000, &at_limit).unwrap();
}

#[test]
fn park_after_deadline_is_refused_on_an_already_parked_row() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.park(id, 1_000, 1_000, 5_000, b"ctx").unwrap();

    // Claim the parked item, as a poll worker would.
    let claimed = queue.claim_parked_due(2_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);
    // At or past the deadline, re-parking is refused.
    assert!(queue.park(id, 5_000, 1_000, 5_000, b"ctx").is_err());
    assert!(queue.park(id, 6_000, 1_000, 5_000, b"ctx").is_err());
}

#[test]
fn an_unresolved_parked_claim_past_deadline_returns_after_visibility_timeout() {
    // Documents the current behaviour: the crate does not act at the
    // parked deadline. A caller that leaves a past-deadline item unresolved
    // gets it back after the visibility timeout, not at the deadline itself.
    // The caller must resolve every past-deadline parked claim it receives.
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();
    queue.park(id, 1_000, 1_000, 3_000, b"ctx").unwrap();

    // Claim at deadline -- deadline is 3_000, poll is 1_000, visible_at = 2_000.
    let claimed = queue.claim_parked_due(3_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);
    let parked = claimed[0].parked.as_ref().unwrap();
    assert!(parked.is_past_deadline(3_000));

    // The caller does not resolve it. The item is invisible during the
    // visibility window and comes back after it.
    let visibility_ms = config().visibility_timeout_ms as i64;
    assert!(queue.claim_parked_due(3_000 + visibility_ms - 1, 10).unwrap().is_empty());
    let reclaimed = queue.claim_parked_due(3_000 + visibility_ms, 10).unwrap();
    assert_eq!(
        reclaimed.len(),
        1,
        "unresolved past-deadline item returns after visibility timeout"
    );
    assert_eq!(reclaimed[0].id, id);
}

#[test]
fn a_non_terminal_fail_on_parked_row_gives_correct_claim_count_on_next_claim() {
    // After park: claim_count was reduced by 1 (from the original pending
    // claim that led to park). A non-terminal fail on the parked item
    // clears the park and makes it pending again, without advancing
    // claim_count further.
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"p", 1_000).unwrap();

    // First pending claim: claim_count becomes 1.
    queue.claim_due(1_000, 10).unwrap();
    // Park: claim_count drops back to 0.
    queue.park(id, 1_000, 1_000, 100_000, b"ctx").unwrap();

    // Parked claim: claim_count stays at 0 (parked claims don't advance it).
    queue.claim_parked_due(2_000, 10).unwrap();

    // Non-terminal fail clears park and schedules retry.
    let outcome = queue.fail(id, 2_000, "err", false).unwrap();
    let FailOutcome::Retrying { next_attempt_at } = outcome else {
        panic!("expected Retrying");
    };

    // Next pending claim: attempts = 1, claim_count = 1.
    let claimed = queue.claim_due(next_attempt_at, 10).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].attempts, 1, "fail advanced attempts");
    assert_eq!(claimed[0].claim_count, 1, "pending claim advanced claim_count from 0 to 1");
    assert!(claimed[0].parked.is_none(), "item must be pending after fail cleared the park");
}

#[test]
fn from_connection_on_a_database_with_other_tables() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute("CREATE TABLE unrelated (x INTEGER);", []).unwrap();
    create_pre_park_outbox(&conn, "k-old");

    let queue = Queue::from_connection(Arc::new(Mutex::new(conn)), config()).unwrap();

    // The existing item was upgraded and can be claimed and parked
    let claimed = queue.claim_due(1_000, 10).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].queue_key, "k-old");
    assert!(claimed[0].parked.is_none());

    queue.park(claimed[0].id, 1_000, 1_000, 5_000, b"ctx").unwrap();
    let all = queue.all().unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].parked.is_some());

    // Both new partial indexes exist
    let conn_guard = queue.conn.lock().unwrap();
    assert!(
        index_exists(&conn_guard, "idx_outbox_due"),
        "idx_outbox_due must exist after upgrade via from_connection"
    );
    assert!(
        index_exists(&conn_guard, "idx_outbox_parked_due"),
        "idx_outbox_parked_due must exist after upgrade via from_connection"
    );
}

#[test]
fn deferring_an_unclaimed_item_keeps_claim_count_at_zero() {
    let queue = Queue::open_in_memory(config()).unwrap();
    let id = queue.enqueue("g", "k", b"payload", 1_000).unwrap();

    // Calling defer on an item that was never claimed must floor at 0
    queue.defer(id, 2_000).unwrap();

    let all = queue.all().unwrap();
    assert_eq!(all[0].claim_count, 0, "claim_count must floor at 0 on defer");
    assert_eq!(all[0].attempts, 0);
}

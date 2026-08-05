#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! A durable, owner-local work queue: an `outbox` of items awaiting
//! delivery, and a `dead_letters` table for the ones that never will be
//! (M05B Slice B1,
//! [ADR-0023](../../../docs/decisions/0023-durable-async-primitives.md)).
//!
//! Generic over an opaque payload -- this crate never interprets what it
//! carries, only when to retry it and when to give up. Every queue belongs
//! to one process (ADR-0023 §6): there is no cross-process claim, no
//! distributed lock, and no compare-and-set. Correctness under
//! at-least-once delivery is the caller's own idempotent fence, not
//! anything this crate provides (ADR-0023 §1).
//!
//! **Every timestamp taken or returned by this crate is Unix
//! milliseconds**, not seconds like most of the rest of the tree -- the
//! reused [`RetryPolicy`] backoff curve is specified in milliseconds
//! (100 ms initial backoff), and converting it to second granularity would
//! make the first few retries indistinguishable from each other.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use syneroym_core::{
    config::{RetryPolicy, SupervisorRole},
    retry::calculate_jittered_backoff,
};

/// The retry curve ([`RetryPolicy`], reused per M05B D-B1-13 -- its struct
/// and `calculate_jittered_backoff`, not `retry_with_backoff`, which sleeps
/// in-process where a durable queue must compute a timestamp and forget the
/// item until a worker tick finds it due) plus this queue's own knobs.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    pub retry: RetryPolicy,
    /// How long a claimed item stays invisible to a second claim before a
    /// crashed worker's hold on it is assumed gone (failure-matrix row 1).
    pub visibility_timeout_ms: u64,
    /// Dead letters are pruned oldest-first on every write past this count
    /// (D-B1-9) -- a bound and a trigger, not an adjective.
    pub dlq_max_rows: u32,
}

/// The supervisor's five `queue_*` fields, converted (M05B D-B1-13):
/// initial backoff and multiplier stay `RetryPolicy`'s own defaults (100 ms,
/// x2) since `SupervisorRole` configures only the attempt budget and the
/// ceiling, not the shape of the early curve.
impl From<&SupervisorRole> for QueueConfig {
    fn from(role: &SupervisorRole) -> Self {
        let defaults = RetryPolicy::default();
        Self {
            retry: RetryPolicy {
                max_attempts: role.queue_max_attempts,
                initial_backoff_ms: defaults.initial_backoff_ms,
                backoff_multiplier: defaults.backoff_multiplier,
                max_backoff_ms: role.queue_max_backoff_secs.saturating_mul(1000),
            },
            visibility_timeout_ms: role.queue_visibility_timeout_secs.saturating_mul(1000),
            dlq_max_rows: role.queue_dlq_max_rows,
        }
    }
}

/// One item due for delivery, as [`Queue::claim_due`] hands it to a worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueItem {
    pub id: i64,
    /// The caller's own grouping/dedup key -- opaque to this crate. B1's
    /// supervisor outbox encodes `(instance, logical_ref, substrate)` into
    /// it, since that is the row the DLQ's standing alert groups by
    /// (D-B1-6); this crate does not need to know that shape.
    pub queue_key: String,
    pub payload: Vec<u8>,
    /// How many delivery attempts this item has already used, including
    /// the one that is about to happen -- 0 for one never yet claimed.
    pub attempts: u32,
}

/// A terminally failed item, as [`Queue::dead_letters`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadLetter {
    pub id: i64,
    pub queue_key: String,
    pub payload: Vec<u8>,
    /// Attempts made before this item was dead-lettered.
    pub attempts: u32,
    pub last_error: String,
    pub created_at: i64,
}

/// What [`Queue::fail`] did with an item that did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailOutcome {
    /// Still under budget; back on the outbox, due again at this time.
    Retrying { next_attempt_at: i64 },
    /// Attempts exhausted, or the caller marked this failure terminal;
    /// moved to `dead_letters`.
    DeadLettered,
}

/// One SQLite-backed queue: an `outbox` of pending/in-flight items and a
/// bounded `dead_letters` table. `conn: Arc<Mutex<Connection>>` (matching
/// `AlertStore`/`DeploymentJournal`) so a caller with its own multi-table
/// database -- the supervisor's `supervisor.db` (D-B1-5) -- can hand this
/// queue a clone of the same connection instead of opening a file of its
/// own.
#[derive(Debug, Clone)]
pub struct Queue {
    conn: Arc<Mutex<Connection>>,
    config: QueueConfig,
}

// Lock-poisoning from a panicking holder is a programming error (bug) that
// leaves the data in an inconsistent state; there is no safe recovery path.
// `expect` is therefore the correct idiom here, matching `AlertStore`'s.
#[allow(clippy::expect_used)]
impl Queue {
    pub fn open<P: AsRef<Path>>(dir: P, db_name: &str, config: QueueConfig) -> Result<Self> {
        if db_name.contains('/') || db_name.contains('\\') || db_name.contains("..") {
            return Err(anyhow!("Invalid database name: {}", db_name));
        }
        let path = dir.as_ref().join(db_name);
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::from_connection(Arc::new(Mutex::new(conn)), config)
    }

    pub fn open_in_memory(config: QueueConfig) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(Arc::new(Mutex::new(conn)), config)
    }

    /// Wraps an already-open connection -- the supervisor's own outbox is
    /// the fourth sibling `SupervisorStore::from_connection` constructs
    /// beside the journal and the alert store (D-B1-5), sharing
    /// `supervisor.db` rather than opening a file of its own.
    pub fn from_connection(conn: Arc<Mutex<Connection>>, config: QueueConfig) -> Result<Self> {
        Self::init_schema(&conn.lock().expect("queue connection lock poisoned"))?;
        Ok(Self { conn, config })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        // Unconditional, not gated on `PRAGMA user_version`: pre-release,
        // schema changes are made in place with no version ladder, and
        // `IF NOT EXISTS` is already idempotent.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS outbox (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                queue_key   TEXT NOT NULL,
                payload     BLOB NOT NULL,
                attempts    INTEGER NOT NULL DEFAULT 0,
                visible_at  INTEGER NOT NULL,
                created_at  INTEGER NOT NULL
             );
             -- Drives both scheduling (an item due for its first or next
             -- attempt) and visibility (an in-flight item's lock deadline)
             -- through the one column: an in-flight item is simply one
             -- whose visible_at has been pushed into the future, so
             -- `claim_due`'s single indexed range scan finds both kinds
             -- with no separate state check.
             CREATE INDEX IF NOT EXISTS idx_outbox_visible_at ON outbox(visible_at);

             CREATE TABLE IF NOT EXISTS dead_letters (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                queue_key   TEXT NOT NULL,
                payload     BLOB NOT NULL,
                attempts    INTEGER NOT NULL,
                last_error  TEXT NOT NULL,
                created_at  INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_dead_letters_created_at ON dead_letters(created_at);
             CREATE INDEX IF NOT EXISTS idx_dead_letters_queue_key ON dead_letters(queue_key);",
        )?;
        Ok(())
    }

    /// Writes one item, immediately due. One indexed insert (the `< 1 ms`
    /// enqueue-on-failure budget, task.md).
    pub fn enqueue(&self, queue_key: &str, payload: &[u8], now: i64) -> Result<i64> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        conn.execute(
            "INSERT INTO outbox (queue_key, payload, attempts, visible_at, created_at)
             VALUES (?1, ?2, 0, ?3, ?3)",
            params![queue_key, payload, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Claims up to `limit` items due at or before `now` -- either freshly
    /// due, or in flight past a crashed worker's visibility timeout
    /// (failure-matrix row 1) -- and marks them invisible until `now +
    /// visibility_timeout_ms`. The read and every claiming write run under
    /// one held lock, so two workers in one process cannot claim the same
    /// row (failure-matrix row 6); nothing here claims across processes,
    /// which ADR-0023 §6 rules out by construction.
    pub fn claim_due(&self, now: i64, limit: u32) -> Result<Vec<QueueItem>> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let locked_until = now + self.config.visibility_timeout_ms as i64;
        let items: Vec<QueueItem> = {
            let mut stmt = conn.prepare(
                "SELECT id, queue_key, payload, attempts FROM outbox
                 WHERE visible_at <= ?1 ORDER BY visible_at ASC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![now, limit])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(QueueItem {
                    id: row.get(0)?,
                    queue_key: row.get(1)?,
                    payload: row.get(2)?,
                    attempts: row.get::<_, i64>(3)? as u32,
                });
            }
            out
        };
        for item in &items {
            conn.execute(
                "UPDATE outbox SET visible_at = ?1 WHERE id = ?2",
                params![locked_until, item.id],
            )?;
        }
        Ok(items)
    }

    /// Deletes a completed item (D-B1-9): `applied`/`no-op`/`stale` all
    /// mean the item's intent is satisfied, so nothing is kept once a
    /// worker reaches this call.
    pub fn complete(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        conn.execute("DELETE FROM outbox WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Records a failed delivery attempt.
    ///
    /// `terminal`: the caller already knows this item can never succeed
    /// (failure-matrix row 9 -- e.g. a queued write's target no longer
    /// exists), so it dead-letters immediately regardless of budget
    /// remaining, with a distinguishable reason.
    ///
    /// Otherwise: still under the configured attempt budget computes
    /// `next_attempt_at` from `RetryPolicy` + `calculate_jittered_backoff`
    /// (D-B1-13) and leaves the item due again then; exhausted moves it to
    /// `dead_letters` with `error` and this attempt count, deletes it from
    /// `outbox`, and prunes the oldest dead letter past `dlq_max_rows`
    /// (D-B1-9).
    pub fn fail(&self, id: i64, now: i64, error: &str, terminal: bool) -> Result<FailOutcome> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let (queue_key, payload, prior_attempts): (String, Vec<u8>, i64) = conn
            .query_row(
                "SELECT queue_key, payload, attempts FROM outbox WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow!("no outbox item with id {id}"))?;
        let attempts = prior_attempts as u32 + 1;
        if terminal || attempts >= u32::from(self.config.retry.max_attempts) {
            Self::dead_letter(
                &conn,
                id,
                &queue_key,
                &payload,
                attempts,
                error,
                now,
                self.config.dlq_max_rows,
            )?;
            return Ok(FailOutcome::DeadLettered);
        }
        let base = backoff_before_wait(&self.config.retry, attempts);
        let jittered = calculate_jittered_backoff(base);
        let next_attempt_at = now + jittered as i64;
        conn.execute(
            "UPDATE outbox SET attempts = ?1, visible_at = ?2 WHERE id = ?3",
            params![attempts, next_attempt_at, id],
        )?;
        Ok(FailOutcome::Retrying { next_attempt_at })
    }

    #[allow(clippy::too_many_arguments)]
    fn dead_letter(
        conn: &Connection,
        outbox_id: i64,
        queue_key: &str,
        payload: &[u8],
        attempts: u32,
        error: &str,
        now: i64,
        dlq_max_rows: u32,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO dead_letters (queue_key, payload, attempts, last_error, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![queue_key, payload, attempts, error, now],
        )?;
        conn.execute("DELETE FROM outbox WHERE id = ?1", params![outbox_id])?;
        Self::prune_dead_letters(conn, dlq_max_rows)
    }

    fn prune_dead_letters(conn: &Connection, max_rows: u32) -> Result<()> {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM dead_letters", [], |r| r.get(0))?;
        let excess = count - i64::from(max_rows);
        if excess > 0 {
            conn.execute(
                "DELETE FROM dead_letters WHERE id IN (
                    SELECT id FROM dead_letters ORDER BY created_at ASC, id ASC LIMIT ?1
                 )",
                params![excess],
            )?;
        }
        Ok(())
    }

    /// Every dead letter, oldest first -- `roymctl supervisor
    /// dead-letters`'s own listing (D-B1-6).
    pub fn dead_letters(&self) -> Result<Vec<DeadLetter>> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, queue_key, payload, attempts, last_error, created_at
             FROM dead_letters ORDER BY created_at ASC, id ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(DeadLetter {
                id: row.get(0)?,
                queue_key: row.get(1)?,
                payload: row.get(2)?,
                attempts: row.get::<_, i64>(3)? as u32,
                last_error: row.get(4)?,
                created_at: row.get(5)?,
            });
        }
        Ok(out)
    }

    /// Re-enqueues a dead letter, ready for the very next claim (D-B1-7:
    /// "replay re-enqueues; it never executes inline"). Its attempt count
    /// carries over rather than resetting: a dead letter is already at
    /// budget, so this buys it exactly one more delivery attempt through
    /// the ordinary worker path, and a second failure returns it straight
    /// to `dead_letters` with that history intact.
    pub fn replay(&self, id: i64, now: i64) -> Result<()> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let (queue_key, payload, attempts): (String, Vec<u8>, i64) = conn
            .query_row(
                "SELECT queue_key, payload, attempts FROM dead_letters WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow!("no dead letter with id {id}"))?;
        conn.execute(
            "INSERT INTO outbox (queue_key, payload, attempts, visible_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![queue_key, payload, attempts, now],
        )?;
        conn.execute("DELETE FROM dead_letters WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Every outbox item regardless of due time, oldest first -- test and
    /// operator-inspection use; an ordinary worker always goes through
    /// `claim_due`.
    pub fn all(&self) -> Result<Vec<QueueItem>> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let mut stmt =
            conn.prepare("SELECT id, queue_key, payload, attempts FROM outbox ORDER BY id ASC")?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(QueueItem {
                id: row.get(0)?,
                queue_key: row.get(1)?,
                payload: row.get(2)?,
                attempts: row.get::<_, i64>(3)? as u32,
            });
        }
        Ok(out)
    }

    /// The `EXPLAIN QUERY PLAN` `claim_due`'s own `SELECT` produces --
    /// test-only, so the idle-tick budget (task.md: "one indexed query,
    /// the worker must not scan") can be asserted structurally rather than
    /// by wall-clock.
    #[cfg(test)]
    fn explain_claim_plan(&self) -> Result<String> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let mut stmt = conn.prepare(
            "EXPLAIN QUERY PLAN SELECT id, queue_key, payload, attempts FROM outbox
             WHERE visible_at <= ?1 ORDER BY visible_at ASC LIMIT ?2",
        )?;
        let mut rows = stmt.query(params![0i64, 10u32])?;
        let mut plan = String::new();
        while let Some(row) = rows.next()? {
            let detail: String = row.get(3)?;
            plan.push_str(&detail);
            plan.push('\n');
        }
        Ok(plan)
    }
}

/// The un-jittered backoff before the wait that follows a `wait_number`'th
/// failed attempt (1-indexed), capped at `policy.max_backoff_ms` --
/// `RetryPolicy`'s own curve. [`Queue::fail`] applies
/// `calculate_jittered_backoff` on top of this; tests pin the nominal,
/// unjittered total the configured defaults promise (M05B B1 plan §0.12)
/// by calling this directly.
#[must_use]
pub fn backoff_before_wait(policy: &RetryPolicy, wait_number: u32) -> u64 {
    let exponent = wait_number.saturating_sub(1);
    let raw = policy.initial_backoff_ms as f64 * policy.backoff_multiplier.powi(exponent as i32);
    raw.min(policy.max_backoff_ms as f64) as u64
}

#[cfg(test)]
mod tests {
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
        }
    }

    /// Test 2: the whole point of a durable queue.
    #[test]
    fn an_enqueued_item_survives_reopening_the_database() {
        let dir = tempfile::tempdir().unwrap();
        {
            let queue = Queue::open(dir.path(), "queue.db", config()).unwrap();
            queue.enqueue("inst-1/backend@did:key:zB", b"payload-a", 1_000).unwrap();
        }
        let queue = Queue::open(dir.path(), "queue.db", config()).unwrap();
        let items = queue.all().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].payload, b"payload-a");
        assert_eq!(items[0].queue_key, "inst-1/backend@did:key:zB");
    }

    /// Test 3: failure-matrix row 6.
    #[test]
    fn a_claimed_item_is_invisible_to_a_second_claim() {
        let queue = Queue::open_in_memory(config()).unwrap();
        queue.enqueue("k", b"p", 1_000).unwrap();

        let first = queue.claim_due(1_000, 10).unwrap();
        assert_eq!(first.len(), 1);

        let second = queue.claim_due(1_000, 10).unwrap();
        assert!(second.is_empty(), "a claimed item must not be claimable again immediately");
    }

    /// Test 4: failure-matrix row 1, the crashed-worker case.
    #[test]
    fn a_claim_that_is_never_completed_returns_to_pending_after_its_visibility_timeout() {
        let queue = Queue::open_in_memory(config()).unwrap();
        queue.enqueue("k", b"p", 1_000).unwrap();
        let claimed = queue.claim_due(1_000, 10).unwrap();
        assert_eq!(claimed.len(), 1);

        // Still within the visibility window: invisible.
        assert!(queue.claim_due(1_000 + 119_999, 10).unwrap().is_empty());
        // Past it: visible again, as if never claimed.
        let reclaimed = queue.claim_due(1_000 + 120_000, 10).unwrap();
        assert_eq!(reclaimed.len(), 1);
        assert_eq!(reclaimed[0].id, claimed[0].id);
    }

    /// Test 5: D-B1-13. The observed `next_attempt_at` must land within
    /// `calculate_jittered_backoff`'s documented +/-10% band around the
    /// pure, unjittered curve -- not merely "close to the default".
    #[test]
    fn next_attempt_at_follows_the_configured_policy_with_jitter() {
        let cfg = config();
        let queue = Queue::open_in_memory(cfg.clone()).unwrap();
        queue.enqueue("k", b"p", 1_000).unwrap();
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

    /// Test 6: failure-matrix row 4.
    #[test]
    fn an_item_that_exhausts_its_attempts_moves_to_the_dlq_and_leaves_the_outbox() {
        let queue = Queue::open_in_memory(config()).unwrap();
        let id = queue.enqueue("k", b"p", 1_000).unwrap();

        let mut now = 1_000;
        for _ in 0..3 {
            let claimed = queue.claim_due(now, 10).unwrap();
            assert_eq!(claimed.len(), 1, "the item must still be claimable before exhaustion");
            let outcome = queue.fail(id, now, "still unreachable", false).unwrap();
            match outcome {
                FailOutcome::Retrying { next_attempt_at } => now = next_attempt_at,
                FailOutcome::DeadLettered => break,
            }
        }

        assert!(queue.all().unwrap().is_empty(), "the outbox must not still hold the item");
        let dead = queue.dead_letters().unwrap();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].attempts, 3);
        assert_eq!(dead[0].last_error, "still unreachable");
    }

    /// Test 7: M05B B1 plan §0.12's arithmetic, pinned against the actual
    /// production defaults -- 54 attempts (53 waits), 100 ms initial
    /// backoff, x2 multiplier, capped at 900 s -- summing to ~36,738 s
    /// (~10.2 h), unjittered.
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
        assert!(
            (36_737.0..=36_739.0).contains(&total_secs),
            "expected ~36,738 s, got {total_secs} s"
        );

        // The ceiling first binds at the 15th wait, not the 14th (plan
        // §0.12's own corrected arithmetic).
        assert_eq!(backoff_before_wait(&policy, 14), 819_200);
        assert_eq!(backoff_before_wait(&policy, 15), 900_000);
    }

    /// `SupervisorRole::default()`'s five `queue_*` fields convert into
    /// exactly the curve test 7 pins by hand, so the two cannot silently
    /// drift apart.
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

    /// Test 8: D-B1-9.
    #[test]
    fn a_completed_item_is_deleted_not_tombstoned() {
        let queue = Queue::open_in_memory(config()).unwrap();
        let id = queue.enqueue("k", b"p", 1_000).unwrap();
        queue.complete(id).unwrap();
        assert!(queue.all().unwrap().is_empty());
    }

    /// Test 9: D-B1-9's bound, as a number and a trigger.
    #[test]
    fn the_dlq_is_capped_at_its_row_limit_and_prunes_oldest_first() {
        let mut cfg = config();
        cfg.dlq_max_rows = 3;
        cfg.retry.max_attempts = 1;
        let queue = Queue::open_in_memory(cfg).unwrap();

        for i in 0..5 {
            let id = queue.enqueue(&format!("k{i}"), b"p", 1_000 + i).unwrap();
            queue.claim_due(1_000 + i, 10).unwrap();
            queue.fail(id, 1_000 + i, "dead on arrival", false).unwrap();
        }

        let dead = queue.dead_letters().unwrap();
        assert_eq!(dead.len(), 3, "the cap must hold even after five inserts");
        // The two oldest (k0, k1) were pruned; k2..k4 survive.
        let keys: Vec<&str> = dead.iter().map(|d| d.queue_key.as_str()).collect();
        assert_eq!(keys, vec!["k2", "k3", "k4"]);
    }

    /// Test 10: failure-matrix row 9 -- a queued item whose target no
    /// longer exists is terminal, not retryable, regardless of budget
    /// remaining.
    #[test]
    fn a_terminal_error_skips_the_remaining_attempts() {
        let queue = Queue::open_in_memory(config()).unwrap();
        let id = queue.enqueue("k", b"p", 1_000).unwrap();
        queue.claim_due(1_000, 10).unwrap();

        let outcome = queue.fail(id, 1_000, "service no longer exists", true).unwrap();
        assert_eq!(outcome, FailOutcome::DeadLettered);
        let dead = queue.dead_letters().unwrap();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].attempts, 1, "a terminal failure dead-letters on its first attempt");
    }

    /// Test 11: the idle-tick budget -- "one indexed query, no scan"
    /// (task.md), asserted structurally via the query plan rather than by
    /// timing.
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

    /// Test 12: the `< 1 ms` enqueue-on-failure budget (task.md), asserted
    /// as one row changed by exactly one statement rather than wall-clock,
    /// so CI noise cannot fail it.
    #[test]
    fn enqueue_on_failure_costs_one_insert() {
        let queue = Queue::open_in_memory(config()).unwrap();
        queue.enqueue("k", b"p", 1_000).unwrap();
        let conn = queue.conn.lock().unwrap();
        assert_eq!(conn.changes(), 1, "enqueue must be exactly one INSERT");
    }

    /// D-B1-7: replay re-enqueues without executing inline, and does not
    /// simply mutate the dead letter in place.
    #[test]
    fn replay_re_enqueues_rather_than_executing_inline() {
        let mut cfg = config();
        cfg.retry.max_attempts = 1;
        let queue = Queue::open_in_memory(cfg).unwrap();
        let id = queue.enqueue("k", b"p", 1_000).unwrap();
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

    /// Failure-matrix row 5: a replayed item that fails again returns to
    /// the DLQ with its attempt history intact, rather than a fresh
    /// budget.
    #[test]
    fn a_replayed_item_that_fails_again_returns_to_the_dlq_with_its_history() {
        let mut cfg = config();
        cfg.retry.max_attempts = 2;
        let queue = Queue::open_in_memory(cfg).unwrap();
        let id = queue.enqueue("k", b"p", 1_000).unwrap();
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
        assert_eq!(outcome, FailOutcome::DeadLettered);
        let dead_again = queue.dead_letters().unwrap();
        assert_eq!(dead_again.len(), 1);
        assert_eq!(dead_again[0].attempts, 3, "history accumulates across replays");
    }

    #[test]
    fn dead_letters_lists_what_the_store_holds() {
        let mut cfg = config();
        cfg.retry.max_attempts = 1;
        let queue = Queue::open_in_memory(cfg).unwrap();
        assert!(queue.dead_letters().unwrap().is_empty());

        let id = queue.enqueue("k", b"p", 1_000).unwrap();
        queue.claim_due(1_000, 10).unwrap();
        queue.fail(id, 1_000, "unreachable", false).unwrap();

        let dead = queue.dead_letters().unwrap();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].queue_key, "k");
        assert_eq!(dead[0].payload, b"p");
    }
}

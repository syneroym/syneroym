use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use syneroym_core::{config::RetryPolicy, retry::calculate_jittered_backoff};
use zeroize::Zeroizing;

pub mod types;

pub use types::{DEFAULT_MAX_PENDING_ROWS, DeadLetter, FailOutcome, QueueConfig, QueueItem};

/// One SQLite-backed queue: an `outbox` of pending/in-flight items and a
/// bounded `dead_letters` table. `conn: Arc<Mutex<Connection>>` (matching
/// `AlertStore`/`DeploymentJournal`) so a caller with its own multi-table
/// database -- the supervisor's `supervisor.db` -- can hand this
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
        Self::from_connection(Arc::new(Mutex::new(open_connection(dir, db_name, None)?)), config)
    }

    /// Opens a queue in a SQLCipher-encrypted file, keyed with `dek`.
    ///
    /// A queued payload is the caller's own data, so it must not sit in a
    /// store weaker than the database that data came from. `dek: None`
    /// means encryption is disabled for the whole deployment, in which case
    /// the file is plain SQLite -- matching, not exceeding, the protection
    /// the surrounding data has.
    pub fn open_encrypted<P: AsRef<Path>>(
        dir: P,
        db_name: &str,
        dek: Option<&[u8; 32]>,
        config: QueueConfig,
    ) -> Result<Self> {
        Self::from_connection(Arc::new(Mutex::new(open_connection(dir, db_name, dek)?)), config)
    }

    pub fn open_in_memory(config: QueueConfig) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(Arc::new(Mutex::new(conn)), config)
    }

    /// Wraps an already-open connection -- the supervisor's own outbox is
    /// the fourth sibling `SupervisorStore::from_connection` constructs
    /// beside the journal and the alert store, sharing
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
                group_key   TEXT NOT NULL,
                queue_key   TEXT NOT NULL,
                payload     BLOB NOT NULL,
                attempts    INTEGER NOT NULL DEFAULT 0,
                claim_count INTEGER NOT NULL DEFAULT 0,
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
             -- Queue::has_pending's indexed dedup lookup -- an outbox is
             -- normally small, but a caller should not have to pull every
             -- payload blob into memory just to answer whether a key
             -- already has a row.
             CREATE INDEX IF NOT EXISTS idx_outbox_queue_key ON outbox(queue_key);

             CREATE TABLE IF NOT EXISTS dead_letters (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                group_key   TEXT NOT NULL,
                queue_key   TEXT NOT NULL,
                payload     BLOB NOT NULL,
                attempts    INTEGER NOT NULL,
                last_error  TEXT NOT NULL,
                created_at  INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_dead_letters_created_at ON dead_letters(created_at);
             CREATE INDEX IF NOT EXISTS idx_dead_letters_queue_key ON dead_letters(queue_key);
             CREATE INDEX IF NOT EXISTS idx_dead_letters_group_key ON dead_letters(group_key);",
        )?;
        Ok(())
    }

    /// Writes one item, immediately due. One indexed insert (the `< 1 ms`
    /// enqueue-on-failure budget). `group_key` scopes the dead-letter cap
    /// this item's eventual failure would count against -- opaque to this
    /// crate, same as `queue_key`.
    pub fn enqueue(
        &self,
        group_key: &str,
        queue_key: &str,
        payload: &[u8],
        now: i64,
    ) -> Result<i64> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        conn.execute(
            "INSERT INTO outbox (group_key, queue_key, payload, attempts, claim_count, \
             visible_at, created_at)
             VALUES (?1, ?2, ?3, 0, 0, ?4, ?4)",
            params![group_key, queue_key, payload, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Writes a dead letter directly, for an item that never waited in
    /// the outbox.
    ///
    /// One transaction, and deliberately *not* enqueue-then-fail. Routing
    /// it through the outbox to reuse the capping logic leaves a row for
    /// that key visible between the two writes, and a concurrent
    /// [`Self::enqueue_if_absent`] from a real sender would see it, report
    /// the key already pending, and back off -- then this call would move
    /// that same row into `dead_letters`, and the enqueue the sender
    /// believed had succeeded would be gone.
    pub fn record_dead_letter(
        &self,
        group_key: &str,
        queue_key: &str,
        payload: &[u8],
        error: &str,
        now: i64,
    ) -> Result<Vec<String>> {
        let mut conn = self.conn.lock().expect("queue connection lock poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO dead_letters (group_key, queue_key, payload, attempts, last_error, \
             created_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?5)",
            params![group_key, queue_key, payload, error, now],
        )?;
        let pruned = Self::prune_dead_letters(&tx, self.config.dlq_max_rows, group_key)?;
        tx.commit()?;
        Ok(pruned)
    }

    /// Runs `f` inside one transaction on this queue's own connection,
    /// handing the caller both the transaction and a queue handle that
    /// writes through it. For an owner whose own tables live in the same
    /// file and must commit atomically with the enqueue -- `Queue::enqueue`
    /// itself takes this connection's lock, so an owner already holding a
    /// `Transaction` on it cannot also call `enqueue` (`std::sync::Mutex`
    /// is not reentrant, so it would self-deadlock).
    pub fn transaction<T>(
        &self,
        f: impl FnOnce(&Transaction<'_>, &TxQueue<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut conn = self.conn.lock().expect("queue connection lock poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let txq = TxQueue { _marker: std::marker::PhantomData };
        let result = f(&tx, &txq)?;
        tx.commit()?;
        Ok(result)
    }

    /// Pushes a claimed item back to `visible_at` **without** charging the
    /// attempt budget, and un-counts this claim so the poison-pill bound
    /// (`claim_count > max_attempts`) does not fire on an item that is
    /// deliberately waiting. For a target that is absent rather than
    /// broken; a caller using this owes its own outer bound.
    pub fn defer(&self, id: i64, visible_at: i64) -> Result<()> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        conn.execute(
            "UPDATE outbox SET visible_at = ?1, claim_count = claim_count - 1 WHERE id = ?2",
            params![visible_at, id],
        )?;
        Ok(())
    }

    /// This queue's configured attempt budget -- the same value
    /// [`Queue::fail`] checks internally, exposed so a caller can apply the
    /// identical ceiling to a case this crate cannot see on its own: an
    /// item claimed but never resolved through `fail`/`complete` at all (a
    /// worker panic, a crashed process), which [`QueueItem::claim_count`]
    /// tracks. Reading it here rather than duplicating the number as a
    /// second config field keeps the two budgets from silently drifting
    /// apart.
    #[must_use]
    pub fn max_attempts(&self) -> u8 {
        self.config.retry.max_attempts
    }

    /// Whether `queue_key` already has a row in the outbox -- pending or
    /// claimed, either way already durable and already on its own retry
    /// schedule. An indexed lookup (`idx_outbox_queue_key`), not a scan of
    /// every payload (the caller-side `Queue::all()` scan
    /// `SupervisorOutbox::already_pending` used to pay on every failed
    /// push).
    pub fn has_pending(&self, queue_key: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        Ok(conn
            .query_row(
                "SELECT 1 FROM outbox WHERE queue_key = ?1 LIMIT 1",
                params![queue_key],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// How many items are waiting or in flight. The bound a caller
    /// applies before accepting more work.
    pub fn pending_count(&self) -> Result<u32> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))?;
        Ok(count as u32)
    }

    /// Writes one item unless `queue_key` already has a pending row, and
    /// reports whether it wrote. The check and the insert run in one
    /// immediate transaction, so two callers racing a cold cache -- or two
    /// handles to the same file -- cannot both decide the key is free and
    /// both write. The one-row-per-key invariant every dedup-on-key caller
    /// depends on (including [`Self::replay`]) is the database's here, not
    /// the caller's.
    pub fn enqueue_if_absent(
        &self,
        group_key: &str,
        queue_key: &str,
        payload: &[u8],
        now: i64,
    ) -> Result<bool> {
        let mut conn = self.conn.lock().expect("queue connection lock poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let already: bool = tx
            .query_row(
                "SELECT 1 FROM outbox WHERE queue_key = ?1 LIMIT 1",
                params![queue_key],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if already {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO outbox (group_key, queue_key, payload, attempts, claim_count, \
             visible_at, created_at)
             VALUES (?1, ?2, ?3, 0, 0, ?4, ?4)",
            params![group_key, queue_key, payload, now],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Claims up to `limit` items due at or before `now` -- either freshly
    /// due, or in flight past a crashed worker's visibility timeout -- and
    /// marks them invisible until `now + visibility_timeout_ms`. The read
    /// and every claiming write run under one held lock, so two workers in
    /// one process cannot claim the same row; nothing here claims across
    /// processes, which ADR-0023 §6 rules out by construction. Each claiming
    /// write is itself an `UPDATE ... WHERE id = ? AND visible_at <= ?`, not an
    /// unconditional one, so the guarantee holds even if a future caller
    /// ever shares this table across more than one held lock.
    pub fn claim_due(&self, now: i64, limit: u32) -> Result<Vec<QueueItem>> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let locked_until = now + self.config.visibility_timeout_ms as i64;
        struct Candidate {
            id: i64,
            queue_key: String,
            payload: Vec<u8>,
            attempts: u32,
            claim_count: u32,
        }
        let candidates: Vec<Candidate> = {
            let mut stmt = conn.prepare(
                "SELECT id, queue_key, payload, attempts, claim_count FROM outbox
                 WHERE visible_at <= ?1 ORDER BY visible_at ASC LIMIT ?2",
            )?;
            let mut rows = stmt.query(params![now, limit])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(Candidate {
                    id: row.get(0)?,
                    queue_key: row.get(1)?,
                    payload: row.get(2)?,
                    attempts: row.get::<_, i64>(3)? as u32,
                    claim_count: row.get::<_, i64>(4)? as u32,
                });
            }
            out
        };
        let mut claimed = Vec::with_capacity(candidates.len());
        for c in candidates {
            let claim_count = c.claim_count + 1;
            let updated = conn.execute(
                "UPDATE outbox SET visible_at = ?1, claim_count = ?2 WHERE id = ?3 AND visible_at \
                 <= ?4",
                params![locked_until, claim_count, c.id, now],
            )?;
            if updated == 0 {
                // Lost the row between the read above and this write --
                // cannot happen while both run under the same held lock,
                // but the guard (rather than an unconditional UPDATE) is
                // what makes that a fact about this code, not an assumption
                // a future refactor could silently break.
                continue;
            }
            claimed.push(QueueItem {
                id: c.id,
                queue_key: c.queue_key,
                payload: c.payload,
                attempts: c.attempts,
                claim_count,
            });
        }
        Ok(claimed)
    }

    /// Deletes a completed item: `applied`/`no-op`/`stale` all
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
    /// (e.g. a queued write's target no longer exists, or a claim count
    /// that alone exhausted the budget with `fail` never previously
    /// called), so it dead-letters immediately regardless of budget
    /// remaining, with a distinguishable reason.
    ///
    /// Otherwise: still under the configured attempt budget computes
    /// `next_attempt_at` from `RetryPolicy` + `calculate_jittered_backoff`
    /// and leaves the item due again then; exhausted moves it to
    /// `dead_letters` with `error` and this attempt count, deletes it from
    /// `outbox`, and prunes the oldest dead letter -- within this item's
    /// own `group_key` -- past `dlq_max_rows`.
    pub fn fail(&self, id: i64, now: i64, error: &str, terminal: bool) -> Result<FailOutcome> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let (group_key, queue_key, payload, prior_attempts): (String, String, Vec<u8>, i64) = conn
            .query_row(
                "SELECT group_key, queue_key, payload, attempts FROM outbox WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow!("no outbox item with id {id}"))?;
        let attempts = prior_attempts as u32 + 1;
        if terminal || attempts >= u32::from(self.config.retry.max_attempts) {
            let pruned_keys = Self::dead_letter(
                &conn,
                id,
                &group_key,
                &queue_key,
                &payload,
                attempts,
                error,
                now,
                self.config.dlq_max_rows,
            )?;
            return Ok(FailOutcome::DeadLettered { pruned_keys });
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
        group_key: &str,
        queue_key: &str,
        payload: &[u8],
        attempts: u32,
        error: &str,
        now: i64,
        dlq_max_rows: u32,
    ) -> Result<Vec<String>> {
        conn.execute(
            "INSERT INTO dead_letters (group_key, queue_key, payload, attempts, last_error, \
             created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![group_key, queue_key, payload, attempts, error, now],
        )?;
        conn.execute("DELETE FROM outbox WHERE id = ?1", params![outbox_id])?;
        Self::prune_dead_letters(conn, dlq_max_rows, group_key)
    }

    /// Prunes `dead_letters` oldest-first *within `group_key`* past
    /// `max_rows`, and returns the `queue_key` of every row it
    /// deleted -- so a caller with a standing alert keyed by `queue_key`
    /// can clear it (an unnotified prune left `DeliveryExhausted` alerts
    /// nothing could ever clear). Scoped to one `group_key` rather than the
    /// whole table, so one noisy group cannot silently evict another's dead
    /// letters.
    fn prune_dead_letters(
        conn: &Connection,
        max_rows: u32,
        group_key: &str,
    ) -> Result<Vec<String>> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM dead_letters WHERE group_key = ?1",
            params![group_key],
            |r| r.get(0),
        )?;
        let excess = count - i64::from(max_rows);
        if excess <= 0 {
            return Ok(Vec::new());
        }
        let mut stmt = conn.prepare(
            "SELECT id, queue_key FROM dead_letters WHERE group_key = ?1
             ORDER BY created_at ASC, id ASC LIMIT ?2",
        )?;
        let mut rows = stmt.query(params![group_key, excess])?;
        let mut to_delete = Vec::new();
        let mut pruned_keys = Vec::new();
        while let Some(row) = rows.next()? {
            to_delete.push(row.get::<_, i64>(0)?);
            pruned_keys.push(row.get::<_, String>(1)?);
        }
        drop(rows);
        drop(stmt);
        for id in to_delete {
            conn.execute("DELETE FROM dead_letters WHERE id = ?1", params![id])?;
        }
        Ok(pruned_keys)
    }

    /// Every dead letter, oldest first -- `roymctl supervisor
    /// dead-letters`'s own listing.
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

    /// Re-enqueues a dead letter, ready for the very next claim; it never
    /// executes inline. Its attempt count carries over rather than
    /// resetting: a dead letter is already at budget, so this buys it
    /// exactly one more delivery attempt through the ordinary worker path,
    /// and a second failure returns it straight to `dead_letters` with that
    /// history intact.
    ///
    /// Refuses when the outbox already holds a pending row for the same
    /// `queue_key`: inserting a second row would break the one-row-per-key
    /// invariant every caller that dedupes on `queue_key` depends on -- a
    /// newer, immediately-due duplicate would win every later claim over
    /// the older one waiting out its backoff, and the dead letter's own
    /// history would no longer describe the row actually in flight.
    pub fn replay(&self, id: i64, now: i64) -> Result<()> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let (group_key, queue_key, payload, attempts): (String, String, Vec<u8>, i64) = conn
            .query_row(
                "SELECT group_key, queue_key, payload, attempts FROM dead_letters WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow!("no dead letter with id {id}"))?;
        let already_pending: bool = conn
            .query_row(
                "SELECT 1 FROM outbox WHERE queue_key = ?1 LIMIT 1",
                params![queue_key],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if already_pending {
            return Err(anyhow!(
                "a pending outbox item already exists for this dead letter's key; wait for it to \
                 resolve (or check the outbox) before replaying"
            ));
        }
        conn.execute(
            "INSERT INTO outbox (group_key, queue_key, payload, attempts, claim_count, \
             visible_at, created_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
            params![group_key, queue_key, payload, attempts, now],
        )?;
        conn.execute("DELETE FROM dead_letters WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Every outbox item regardless of due time, oldest first -- test and
    /// operator-inspection use; an ordinary worker always goes through
    /// `claim_due`.
    pub fn all(&self) -> Result<Vec<QueueItem>> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, queue_key, payload, attempts, claim_count FROM outbox ORDER BY id ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(QueueItem {
                id: row.get(0)?,
                queue_key: row.get(1)?,
                payload: row.get(2)?,
                attempts: row.get::<_, i64>(3)? as u32,
                claim_count: row.get::<_, i64>(4)? as u32,
            });
        }
        Ok(out)
    }

    /// The `EXPLAIN QUERY PLAN` `claim_due`'s own `SELECT` produces --
    /// test-only, so the idle-tick budget -- one indexed query, the worker
    /// must not scan -- can be asserted structurally rather than by
    /// wall-clock.
    #[cfg(test)]
    fn explain_claim_plan(&self) -> Result<String> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        let mut stmt = conn.prepare(
            "EXPLAIN QUERY PLAN SELECT id, queue_key, payload, attempts, claim_count FROM outbox
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

/// The transaction-scoped half of [`Queue`], for use inside
/// [`Queue::transaction`]. Same SQL as [`Queue::enqueue`], but written
/// through the caller's own `Transaction` instead of taking `Queue`'s own
/// connection lock, so it composes with the caller's own writes into one
/// atomic commit.
#[derive(Debug)]
pub struct TxQueue<'a> {
    _marker: std::marker::PhantomData<&'a ()>,
}

impl TxQueue<'_> {
    /// Same effect as [`Queue::enqueue`], through `tx` instead of a fresh
    /// lock.
    pub fn enqueue(
        &self,
        tx: &Transaction<'_>,
        group_key: &str,
        queue_key: &str,
        payload: &[u8],
        now: i64,
    ) -> Result<i64> {
        tx.execute(
            "INSERT INTO outbox (group_key, queue_key, payload, attempts, claim_count, \
             visible_at, created_at)
             VALUES (?1, ?2, ?3, 0, 0, ?4, ?4)",
            params![group_key, queue_key, payload, now],
        )?;
        Ok(tx.last_insert_rowid())
    }
}

/// Opens (creating on first use) a WAL-mode SQLite connection at
/// `dir/db_name`, applying `PRAGMA key` before anything else touches the
/// file when `dek` is present. The key pragma must be the first statement
/// on the connection: SQLCipher decides the page cipher from it, so running
/// schema DDL first would create an unencrypted file that later opens
/// refuse.
pub(crate) fn open_connection<P: AsRef<Path>>(
    dir: P,
    db_name: &str,
    dek: Option<&[u8; 32]>,
) -> Result<Connection> {
    if db_name.contains('/') || db_name.contains('\\') || db_name.contains("..") {
        return Err(anyhow!("Invalid database name: {db_name}"));
    }
    let conn = Connection::open(dir.as_ref().join(db_name))?;
    if let Some(dek) = dek {
        let pragma = Zeroizing::new(format!("x'{}'", hex::encode(dek)));
        conn.pragma_update(None, "key", &*pragma)?;
    }
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    Ok(conn)
}

/// The un-jittered backoff before the wait that follows a `wait_number`'th
/// failed attempt (1-indexed), capped at `policy.max_backoff_ms` --
/// `RetryPolicy`'s own curve. [`Queue::fail`] applies
/// `calculate_jittered_backoff` on top of this; tests pin the nominal,
/// unjittered total the configured defaults promise by calling this
/// directly.
#[must_use]
pub fn backoff_before_wait(policy: &RetryPolicy, wait_number: u32) -> u64 {
    let exponent = wait_number.saturating_sub(1);
    let raw = policy.initial_backoff_ms as f64 * policy.backoff_multiplier.powi(exponent as i32);
    raw.min(policy.max_backoff_ms as f64) as u64
}

#[cfg(test)]
mod tests;

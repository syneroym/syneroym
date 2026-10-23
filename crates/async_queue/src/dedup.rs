//! Receiver-side call deduplication: the fence that makes at-least-once
//! delivery safe for a caller that has none of its own
//! ([ADR-0023](../../../docs/decisions/0023-durable-async-primitives.md),
//! sections 1 and 4).
//!
//! A sender that retries -- because its attempt timed out, because its
//! durable outbox came back to the item, or because an operator replayed a
//! dead letter -- must not cause the target to run twice. This store
//! remembers, per `(caller, idempotency key)`, that a call was started and
//! what it eventually answered, so a duplicate is answered from the record
//! instead of executed.
//!
//! **Every timestamp here is Unix milliseconds**, matching the queue's own
//! convention.
//!
//! **Three states, not two.** The obvious design records only finished
//! calls, and then has nothing to say about the case a retry hits most: the
//! first call is *still running*. Re-executing breaks the guarantee, and
//! answering "no result yet" as a success would be a lie. So a record is
//! written when the call starts, carrying a claim expiry, and a duplicate
//! arriving while that claim holds is refused with a reserved code its
//! sender treats as retryable. A claim past its expiry is retaken and the
//! call runs again -- the honest cost of at-least-once, and the only
//! alternative to blocking that key forever after a receiver dies mid-call.

use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use crate::queue::open_connection;

/// Reserved JSON-RPC error code: a call with this idempotency key is
/// already running on this node. Distinguished from every other callee
/// error because it is the one definitive-looking answer that is in fact
/// temporary -- a queued sender retries on this code alone and completes on
/// any other.
pub const CALL_ALREADY_RUNNING_RPC_CODE: i32 = -32094;
/// Reserved JSON-RPC error code: the call already ran here, but its result
/// was larger than this store retains, so it cannot be replayed. Keeps the
/// load-bearing half of the guarantee -- the target is not re-executed --
/// while being truthful about the half it cannot keep.
pub const CALL_RESULT_NOT_RETAINED_RPC_CODE: i32 = -32095;

/// How many claims may pass between row-cap checks. The TTL sweep still
/// runs on every claim; only the `COUNT` behind the cap is amortised.
#[doc(hidden)]
pub const CAP_CHECK_INTERVAL: u32 = 64;

const STATE_IN_FLIGHT: i64 = 0;
const STATE_DONE: i64 = 1;

/// The bounds and windows this store runs under. Derived from the queue's
/// own configuration rather than separately configured: a TTL shorter than
/// the sender's retry window silently converts dedup into no dedup, and a
/// knob whose only distinct setting breaks the guarantee should not exist.
#[derive(Debug, Clone)]
pub struct DedupConfig {
    /// How long a finished record keeps answering duplicates. Must be at
    /// least as long as any sender might still be retrying -- see
    /// [`Self::derive`].
    pub ttl_ms: u64,
    /// How long one attempt may hold a key before a duplicate is allowed
    /// to retake it. The call's own budget, not the TTL: a merely slow
    /// call must never be re-executed underneath itself.
    pub claim_window_ms: u64,
    /// Row ceiling per service, pruned oldest-expiring-first on write. A
    /// TTL is a time bound; without this a receiver taking keyed calls
    /// faster than they expire grows without a ceiling.
    pub max_rows: u32,
    /// Results larger than this are recorded as done *without* their body.
    pub max_result_bytes: usize,
}

/// How long a record answers, and how many one caller may hold. Chosen per
/// target service on each call, so one store can serve both a short and a
/// long window.
///
/// A window longer than the store's own default is a long window. Callers
/// are not limited in number, so a long window also has a total: while the
/// store holds `max_total_rows` live records or more, new claims get the
/// default window instead. Existing records are never evicted for it, so a
/// flood cannot push out a record that still has to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DedupWindow {
    pub ttl_ms: u64,
    /// Rows one caller may hold.
    pub max_rows: u32,
    /// Live rows in the whole store at which a long window closes.
    pub max_total_rows: u32,
}

impl DedupConfig {
    /// The window this config gives every service by default.
    #[must_use]
    pub fn window(&self) -> DedupWindow {
        DedupWindow { ttl_ms: self.ttl_ms, max_rows: self.max_rows, max_total_rows: u32::MAX }
    }

    /// Row ceiling per service. Pruning the oldest-expiring row first
    /// means the record most likely to still matter is the last to go.
    pub const DEFAULT_MAX_ROWS: u32 = 10_000;
    /// Result-body ceiling. A larger result still records "this ran", it
    /// just cannot replay the value.
    pub const DEFAULT_MAX_RESULT_BYTES: usize = 64 * 1024;

    /// Derives the windows from the sender-side budget they must outlast.
    ///
    /// `retry_window_ms` is the nominal total time a sender's queue spends
    /// retrying one item; one visibility timeout of margin covers the last
    /// attempt still being in flight when that window closes.
    /// `call_budget_ms` is one delivery attempt's own deadline, doubled so
    /// a slow call is never re-executed underneath itself.
    #[must_use]
    pub fn derive(retry_window_ms: u64, visibility_timeout_ms: u64, call_budget_ms: u64) -> Self {
        Self {
            ttl_ms: retry_window_ms.saturating_add(visibility_timeout_ms),
            claim_window_ms: call_budget_ms.saturating_mul(2),
            max_rows: Self::DEFAULT_MAX_ROWS,
            max_result_bytes: Self::DEFAULT_MAX_RESULT_BYTES,
        }
    }
}

/// The first outcome a completed call produced, as it is stored and
/// replayed. A callee error is a real answer the target gave, so it
/// replays exactly like a success does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstOutcome {
    /// The success value, as JSON bytes.
    Success(Vec<u8>),
    /// The callee's own JSON-RPC error.
    CalleeError { code: i32, message: String },
    /// The call finished, but its result exceeded
    /// [`DedupConfig::max_result_bytes`] and was not retained.
    SuccessNotRetained,
}

/// What a caller should do with the request that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupDecision {
    /// This attempt owns the key. Run the call, then report back through
    /// [`DedupStore::finish`] or [`DedupStore::release`], quoting this
    /// token.
    ///
    /// The token identifies *this* attempt, not the key. A claim that
    /// outran its window and was retaken by a newer attempt must not be
    /// able to stamp its own outcome as final -- and "is the row still in
    /// flight with an unexpired claim?" cannot tell the two apart, because
    /// the newer attempt's claim satisfies exactly that description.
    Execute(ClaimToken),
    /// The call already finished. Answer with this, do not execute.
    Replay(FirstOutcome),
    /// A call with this key is running here right now. Answer with
    /// [`CALL_ALREADY_RUNNING_RPC_CODE`], do not execute.
    AlreadyRunning,
}

/// Identifies one attempt's hold on a key. Opaque; only equality against
/// the stored value matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimToken(i64);

/// One service's `call_dedup` table. Opened on the **target's** own
/// database with the target's own key: a record is a fact about what that
/// service already did, so it belongs beside that service's data and is
/// protected exactly as strongly.
///
/// `Clone` over a shared connection, so a caller can cache one handle per
/// service and pay the (expensive) SQLCipher key derivation once rather
/// than once per call.
#[derive(Debug, Clone)]
pub struct DedupStore {
    conn: Arc<Mutex<Connection>>,
    config: DedupConfig,
    /// Claims taken since the row cap was last checked. The TTL sweep is
    /// an indexed range delete and runs every time; the cap check is a
    /// `COUNT`, so it is amortised rather than paid on every claim.
    since_cap_check: Arc<AtomicU32>,
    /// Set while the store holds too many live rows to grant a long window.
    /// Refreshed with the cap check, so a `COUNT` stays off the hot path.
    long_window_closed: Arc<AtomicBool>,
}

/// One stored row, as [`DedupStore::begin`]'s lookup reads it.
struct Record {
    state: i64,
    claim_expires_at: i64,
    expires_at: i64,
    result: Option<Vec<u8>>,
    result_retained: i64,
    error_code: Option<i64>,
    error_message: Option<String>,
}

// Lock poisoning from a panicking holder leaves the data inconsistent with
// no safe recovery path, matching the queue's own idiom.
#[allow(clippy::expect_used)]
impl DedupStore {
    pub fn open_encrypted<P: AsRef<Path>>(
        dir: P,
        db_name: &str,
        dek: Option<&[u8; 32]>,
        config: DedupConfig,
    ) -> Result<Self> {
        Self::from_connection(Arc::new(Mutex::new(open_connection(dir, db_name, dek)?)), config)
    }

    pub fn open_in_memory(config: DedupConfig) -> Result<Self> {
        Self::from_connection(Arc::new(Mutex::new(Connection::open_in_memory()?)), config)
    }

    pub fn from_connection(conn: Arc<Mutex<Connection>>, config: DedupConfig) -> Result<Self> {
        Self::init_schema(&conn.lock().expect("dedup connection lock poisoned"))?;
        Ok(Self {
            conn,
            config,
            since_cap_check: Arc::new(AtomicU32::new(0)),
            long_window_closed: Arc::new(AtomicBool::new(false)),
        })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS call_dedup (
                caller           TEXT NOT NULL,
                idem_key         TEXT NOT NULL,
                state            INTEGER NOT NULL,
                claim_expires_at INTEGER NOT NULL,
                claim_token      INTEGER NOT NULL DEFAULT 0,
                expires_at       INTEGER NOT NULL,
                result           BLOB,
                result_retained  INTEGER NOT NULL DEFAULT 1,
                error_code       INTEGER,
                error_message    TEXT,
                PRIMARY KEY (caller, idem_key)
             ) WITHOUT ROWID;
             -- The row bound's oldest-first eviction, and the TTL sweep
             -- that runs beside it, are both ordered by this column.
             CREATE INDEX IF NOT EXISTS idx_call_dedup_expires_at
                ON call_dedup(expires_at);
             -- The row cap counts and evicts within one caller, so both
             -- of its queries are keyed that way.
             CREATE INDEX IF NOT EXISTS idx_call_dedup_caller_expires_at
                ON call_dedup(caller, expires_at);",
        )?;
        Ok(())
    }

    /// Claims `key` for `caller`, or reports what the previous claim
    /// already decided.
    ///
    /// The lookup is a single primary-key probe. Pruning runs on the write
    /// path only -- a lookup that finds a live record does no maintenance
    /// work at all.
    pub fn begin(
        &self,
        caller: &str,
        key: &str,
        now: i64,
        window: DedupWindow,
    ) -> Result<DedupDecision> {
        let mut conn = self.conn.lock().expect("dedup connection lock poisoned");
        // One immediate transaction around the read *and* the claiming
        // write. Holding this struct's own mutex is not enough on its own:
        // two handles to the same file are two connections, and nothing
        // stops both reading "no row" before either writes. Taking the
        // write lock up front makes the guarantee the database's, so it
        // holds however many handles exist -- including across processes.
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| anyhow::anyhow!("could not begin a dedup transaction: {e}"))?;
        let existing: Option<Record> = tx
            .query_row(
                "SELECT state, claim_expires_at, expires_at, result, result_retained, error_code, \
                 error_message
                 FROM call_dedup WHERE caller = ?1 AND idem_key = ?2",
                params![caller, key],
                |row| {
                    Ok(Record {
                        state: row.get(0)?,
                        claim_expires_at: row.get(1)?,
                        expires_at: row.get(2)?,
                        result: row.get(3)?,
                        result_retained: row.get(4)?,
                        error_code: row.get(5)?,
                        error_message: row.get(6)?,
                    })
                },
            )
            .optional()?;

        if let Some(record) = existing
            && record.expires_at > now
        {
            if record.state == STATE_DONE {
                let outcome = match (record.error_code, record.error_message) {
                    (Some(code), Some(message)) => {
                        FirstOutcome::CalleeError { code: code as i32, message }
                    }
                    _ if record.result_retained == 0 => FirstOutcome::SuccessNotRetained,
                    _ => FirstOutcome::Success(record.result.unwrap_or_default()),
                };
                return Ok(DedupDecision::Replay(outcome));
            }
            if record.claim_expires_at > now {
                return Ok(DedupDecision::AlreadyRunning);
            }
        }

        let requested = window;
        let is_long = requested.ttl_ms > self.config.ttl_ms;
        let window = if is_long && self.long_window_closed.load(Ordering::Relaxed) {
            self.config.window()
        } else {
            requested
        };
        let token = ClaimToken(rand::random::<i64>());
        tx.execute(
            "INSERT INTO call_dedup
                (caller, idem_key, state, claim_expires_at, claim_token, expires_at, result,
                 result_retained, error_code, error_message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 1, NULL, NULL)
             ON CONFLICT(caller, idem_key) DO UPDATE SET
                state = excluded.state,
                claim_expires_at = excluded.claim_expires_at,
                claim_token = excluded.claim_token,
                expires_at = excluded.expires_at,
                result = NULL,
                result_retained = 1,
                error_code = NULL,
                error_message = NULL",
            params![
                caller,
                key,
                STATE_IN_FLIGHT,
                now + self.config.claim_window_ms as i64,
                token.0,
                now + window.ttl_ms as i64,
            ],
        )?;
        // After the insert, not before: the count the cap is measured
        // against has to include the row just written, and the new row --
        // holding the furthest expiry -- is never the one evicted.
        // The cap is a ceiling, not a precise limit, so checking it every
        // `CAP_CHECK_INTERVAL` claims is enough -- and keeps a `COUNT` off
        // the hot path. Overshoot is bounded by that interval.
        let check_cap =
            self.since_cap_check.fetch_add(1, Ordering::Relaxed).is_multiple_of(CAP_CHECK_INTERVAL);
        Self::prune(&tx, caller, window.max_rows, now, check_cap)?;
        if check_cap && is_long {
            let live: i64 = tx.query_row(
                "SELECT COUNT(*) FROM (SELECT 1 FROM call_dedup WHERE expires_at > ?1 LIMIT ?2)",
                params![now, requested.max_total_rows],
                |row| row.get(0),
            )?;
            self.long_window_closed
                .store(live >= i64::from(requested.max_total_rows), Ordering::Relaxed);
        }
        tx.commit().map_err(|e| anyhow::anyhow!("could not commit a dedup claim: {e}"))?;
        Ok(DedupDecision::Execute(token))
    }

    /// Records the first outcome of a call this store handed
    /// [`DedupDecision::Execute`] for. A result past
    /// [`DedupConfig::max_result_bytes`] is recorded as done without its
    /// body. The record keeps the window it was claimed with: it lasts the
    /// longer of that and the store's default from now.
    pub fn finish(
        &self,
        caller: &str,
        key: &str,
        claim: ClaimToken,
        outcome: &FirstOutcome,
        now: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("dedup connection lock poisoned");
        let (result, retained, code, message) = match outcome {
            FirstOutcome::Success(bytes) if bytes.len() <= self.config.max_result_bytes => {
                (Some(bytes.clone()), 1i64, None, None)
            }
            FirstOutcome::Success(_) | FirstOutcome::SuccessNotRetained => (None, 0i64, None, None),
            FirstOutcome::CalleeError { code, message } => {
                (None, 1i64, Some(i64::from(*code)), Some(message.clone()))
            }
        };
        // Only the attempt that still holds the claim may record an
        // outcome. Without the state and expiry guard, an attempt that
        // outran its claim window could stamp its own result as final
        // while a *newer* attempt is still running -- a duplicate landing
        // in that gap would be served the stale attempt's answer, and the
        // newer one would then overwrite it. A late attempt therefore
        // records nothing, and a later duplicate re-executes: at-least-once
        // behaving as specified, rather than a wrong answer served
        // confidently.
        conn.execute(
            "UPDATE call_dedup
             SET state = ?1, result = ?2, result_retained = ?3, error_code = ?4,
                 error_message = ?5, expires_at = MAX(expires_at, ?6)
             WHERE caller = ?7 AND idem_key = ?8 AND claim_token = ?9",
            params![
                STATE_DONE,
                result,
                retained,
                code,
                message,
                now + self.config.ttl_ms as i64,
                caller,
                key,
                claim.0,
            ],
        )?;
        Ok(())
    }

    /// Drops a claim whose call never reached the target -- an unknown
    /// service, a denied permission, a transport failure on the way in.
    /// Nothing was executed, so nothing should be remembered: leaving the
    /// claim would block a corrected retry for the whole claim window.
    pub fn release(&self, caller: &str, key: &str, claim: ClaimToken) -> Result<()> {
        let conn = self.conn.lock().expect("dedup connection lock poisoned");
        conn.execute(
            "DELETE FROM call_dedup
             WHERE caller = ?1 AND idem_key = ?2 AND state = ?3 AND claim_token = ?4",
            params![caller, key, STATE_IN_FLIGHT, claim.0],
        )?;
        Ok(())
    }

    /// Deletes every expired record, then evicts oldest-expiring-first
    /// down to `max_rows`. A pruned record no longer answers, so a
    /// duplicate arriving afterwards re-executes -- at-least-once behaving
    /// as specified, not a new failure mode.
    fn prune(
        conn: &Connection,
        caller: &str,
        max_rows: u32,
        now: i64,
        check_cap: bool,
    ) -> Result<()> {
        // Always: an indexed range delete over `expires_at`, which costs
        // nothing when nothing has expired.
        conn.execute("DELETE FROM call_dedup WHERE expires_at <= ?1", params![now])?;
        if !check_cap {
            return Ok(());
        }
        // Counted and evicted **within one caller**. A cap shared across
        // callers is an eviction channel: anyone this node can merely
        // identify could push `max_rows` claims through and flush another
        // caller's records, and a pruned record re-executes on its next
        // duplicate. Per-caller, a noisy peer can only evict its own.
        //
        // The cost is that the table's total size is bounded by callers x
        // `max_rows` rather than by `max_rows` alone. The TTL sweep above
        // runs on every write and bounds it in time regardless, which is
        // the trade this takes deliberately: a larger table beats one
        // caller silently unfencing another.
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM call_dedup WHERE caller = ?1",
            params![caller],
            |r| r.get(0),
        )?;
        let excess = count - i64::from(max_rows);
        if excess > 0 {
            // The table is `WITHOUT ROWID`, so eviction addresses rows by
            // the primary key itself rather than an implicit rowid.
            conn.execute(
                "DELETE FROM call_dedup WHERE (caller, idem_key) IN (
                    SELECT caller, idem_key FROM call_dedup WHERE caller = ?1
                    ORDER BY expires_at ASC LIMIT ?2
                 )",
                params![caller, excess],
            )?;
        }
        Ok(())
    }

    /// Whether the store is too full to grant a long window right now, as
    /// of its last cap check. A caller reports this so the closed state is
    /// seen: while it lasts, a long window protects nothing.
    #[must_use]
    pub fn long_window_closed(&self) -> bool {
        self.long_window_closed.load(Ordering::Relaxed)
    }

    /// How many records this store currently holds -- the row bound's own
    /// assertion, and operator inspection.
    pub fn len(&self) -> Result<u32> {
        let conn = self.conn.lock().expect("dedup connection lock poisoned");
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM call_dedup", [], |r| r.get(0))?;
        Ok(count as u32)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// The token currently stored for `(caller, key)` -- test-only, so a
    /// test can act as "whichever attempt holds the claim right now"
    /// without threading tokens through every call.
    #[cfg(test)]
    fn current_claim(&self, caller: &str, key: &str) -> ClaimToken {
        let conn = self.conn.lock().expect("dedup connection lock poisoned");
        ClaimToken(
            conn.query_row(
                "SELECT claim_token FROM call_dedup WHERE caller = ?1 AND idem_key = ?2",
                params![caller, key],
                |r| r.get(0),
            )
            .expect("no claim row"),
        )
    }

    /// The `EXPLAIN QUERY PLAN` [`Self::begin`]'s own lookup produces --
    /// so the "the lookup does not widen the call's budget" claim can be
    /// asserted structurally rather than by wall-clock, which on a
    /// SQLCipher file would measure the key derivation and let CI noise
    /// decide the result.
    pub fn explain_lookup_plan(&self) -> Result<String> {
        let conn = self.conn.lock().expect("dedup connection lock poisoned");
        let mut stmt = conn.prepare(
            "EXPLAIN QUERY PLAN SELECT state, claim_expires_at, expires_at, result, \
             result_retained, error_code, error_message
             FROM call_dedup WHERE caller = ?1 AND idem_key = ?2",
        )?;
        let mut rows = stmt.query(params!["c", "k"])?;
        let mut plan = String::new();
        while let Some(row) = rows.next()? {
            plan.push_str(&row.get::<_, String>(3)?);
            plan.push('\n');
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests;

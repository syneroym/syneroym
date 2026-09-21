//! The durable step log a saga's reverse walk reads (ADR-0023 §7, as
//! amended). One more table pair beside `outbox`/`dead_letters`/
//! `call_dedup` in a service's own `async.db`, under the same DEK.
//!
//! **Intent is written before the call, outcome after** -- a step row is
//! created `pending` before dispatch and moved to `done` (with the result)
//! or `failed` (with the error) after. The walk compensates `done` *and*
//! `pending` steps, so a step whose result never came back is still undone:
//! the substrate died between the call leaving and its answer arriving is
//! exactly the ambiguous case a saga exists to cover.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use syneroym_core::retry::calculate_jittered_backoff;
use zeroize::Zeroizing;

use crate::queue::backoff_before_wait;

pub mod types;

pub use types::{
    CompensationOutcome, MAX_SAGA_PAYLOAD_BYTES, MIN_STEP_CALL_BUDGET_MS, SagaConfig, SagaHead,
    SagaInfo, SagaState, StepIntent, StepRow, StepState,
};

/// One service's durable saga log: `sagas` (one row per workflow) and
/// `saga_steps` (its recorded, ordered steps). Shares this crate's
/// connection/timestamp conventions with [`crate::Queue`] and
/// [`crate::DedupStore`].
#[derive(Debug, Clone)]
pub struct SagaLog {
    conn: Arc<Mutex<Connection>>,
    config: SagaConfig,
}

#[allow(clippy::expect_used)]
impl SagaLog {
    pub fn open_encrypted<P: AsRef<Path>>(
        dir: P,
        db_name: &str,
        dek: Option<&[u8; 32]>,
        config: SagaConfig,
    ) -> Result<Self> {
        Self::from_connection(Arc::new(Mutex::new(open_connection(dir, db_name, dek)?)), config)
    }

    pub fn open_in_memory(config: SagaConfig) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(Arc::new(Mutex::new(conn)), config)
    }

    pub fn from_connection(conn: Arc<Mutex<Connection>>, config: SagaConfig) -> Result<Self> {
        Self::init_schema(&conn.lock().expect("saga log connection lock poisoned"))?;
        Ok(Self { conn, config })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        // Unconditional, not gated on `PRAGMA user_version`: pre-release,
        // schema changes are made in place with no version ladder, and `IF
        // NOT EXISTS` is already idempotent.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sagas (
                saga_id         TEXT PRIMARY KEY,
                name            TEXT NOT NULL,
                app_instance_id TEXT,
                state           TEXT NOT NULL,
                created_at      INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL,
                deadline_at     INTEGER NOT NULL,
                next_attempt_at INTEGER,
                last_error      TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_sagas_due      ON sagas(state, next_attempt_at);
             CREATE INDEX IF NOT EXISTS idx_sagas_deadline ON sagas(state, deadline_at);
             CREATE INDEX IF NOT EXISTS idx_sagas_updated  ON sagas(state, updated_at);

             CREATE TABLE IF NOT EXISTS saga_steps (
                saga_id     TEXT NOT NULL,
                idx         INTEGER NOT NULL,
                target      TEXT NOT NULL,
                routing_key TEXT,
                interface   TEXT NOT NULL,
                method      TEXT NOT NULL,
                params      BLOB NOT NULL,
                result      BLOB,
                state       TEXT NOT NULL,
                attempts    INTEGER NOT NULL DEFAULT 0,
                last_error  TEXT,
                created_at  INTEGER NOT NULL,
                PRIMARY KEY (saga_id, idx)
             );",
        )?;
        Ok(())
    }

    /// Opens a saga. Refuses when the caller already has `max_open` sagas
    /// open: an open saga is work somebody expects to finish, so the bound
    /// refuses rather than evicts. One immediate transaction, so
    /// the open-count check and the insert cannot race a concurrent
    /// `begin`.
    pub fn begin(
        &self,
        saga_id: &str,
        name: &str,
        app_instance_id: Option<&str>,
        deadline_ms: i64,
        now: i64,
    ) -> Result<()> {
        let mut conn = self.conn.lock().expect("saga log connection lock poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let open_count: i64 =
            tx.query_row("SELECT COUNT(*) FROM sagas WHERE state = 'open'", [], |r| r.get(0))?;
        if open_count >= i64::from(self.config.max_open) {
            return Err(anyhow!(
                "this service already has {open_count} open sagas (the limit is {}); commit or \
                 compensate one before starting another",
                self.config.max_open
            ));
        }
        tx.execute(
            "INSERT INTO sagas (saga_id, name, app_instance_id, state, created_at, updated_at, \
             deadline_at, next_attempt_at, last_error)
             VALUES (?1, ?2, ?3, 'open', ?4, ?4, ?5, NULL, NULL)",
            params![saga_id, name, app_instance_id, now, deadline_ms],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Records one step's intent *before* its forward call is dispatched,
    /// returning its index. The state check and the index allocation share
    /// one transaction, so a `compensate` landing between them cannot add
    /// a step to a saga already walking backwards.
    pub fn record_step_intent(&self, saga_id: &str, intent: &StepIntent, now: i64) -> Result<u32> {
        if intent.params.len() > MAX_SAGA_PAYLOAD_BYTES {
            return Err(anyhow!("step params exceed the {MAX_SAGA_PAYLOAD_BYTES} byte limit"));
        }
        let mut conn = self.conn.lock().expect("saga log connection lock poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let state: Option<String> = tx
            .query_row("SELECT state FROM sagas WHERE saga_id = ?1", params![saga_id], |r| r.get(0))
            .optional()?;
        let Some(state) = state else { return Err(anyhow!("unknown saga {saga_id}")) };
        if state != "open" {
            return Err(anyhow!(
                "saga {saga_id} is {state}; steps may only be added while it is open"
            ));
        }
        let idx: i64 = tx.query_row(
            "SELECT COALESCE(MAX(idx) + 1, 0) FROM saga_steps WHERE saga_id = ?1",
            params![saga_id],
            |r| r.get(0),
        )?;
        if idx >= i64::from(self.config.max_steps) {
            return Err(anyhow!(
                "saga {saga_id} already has {idx} steps (the limit is {})",
                self.config.max_steps
            ));
        }
        tx.execute(
            "INSERT INTO saga_steps (saga_id, idx, target, routing_key, interface, method, \
             params, result, state, attempts, last_error, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 'pending', 0, NULL, ?8)",
            params![
                saga_id,
                idx,
                intent.target,
                intent.routing_key,
                intent.interface,
                intent.method,
                intent.params,
                now
            ],
        )?;
        tx.execute("UPDATE sagas SET updated_at = ?1 WHERE saga_id = ?2", params![now, saga_id])?;
        tx.commit()?;
        Ok(idx as u32)
    }

    /// Records the forward call's outcome. Exactly one of `result`/`error`
    /// must be `Some`. An oversized result is dropped (`NULL`) with a note
    /// in `last_error` rather than refused: the call already happened, and
    /// refusing here would lose the step entirely.
    ///
    /// Guarded to `WHERE state = 'pending'`: the intent-before-call rule
    /// means this row can sit in `pending` for the whole forward call, long
    /// enough for a deadline-triggered walk to reach and compensate it
    /// first. Without the guard, this call's late-arriving result would
    /// overwrite `compensated` back to `done`, costing the next tick a
    /// duplicate undo and making `compensated_steps` count backwards. A
    /// no-op here (0 rows affected) means the walk already decided this
    /// step's fate, which is a fine outcome, not an error.
    pub fn record_step_outcome(
        &self,
        saga_id: &str,
        idx: u32,
        result: Option<&[u8]>,
        error: Option<&str>,
        now: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("saga log connection lock poisoned");
        let (state, stored_result, stored_error): (&str, Option<&[u8]>, Option<&str>) =
            match (result, error) {
                (Some(r), None) if r.len() <= MAX_SAGA_PAYLOAD_BYTES => ("done", Some(r), None),
                (Some(_), None) => {
                    ("done", None, Some("stored result exceeded the saga payload limit"))
                }
                (None, Some(e)) => ("failed", None, Some(e)),
                _ => {
                    return Err(anyhow!(
                        "record_step_outcome requires exactly one of result or error"
                    ));
                }
            };
        let updated = conn.execute(
            "UPDATE saga_steps SET state = ?1, result = ?2, last_error = ?3 WHERE saga_id = ?4 \
             AND idx = ?5 AND state = 'pending'",
            params![state, stored_result, stored_error, saga_id, idx],
        )?;
        if updated > 0 {
            conn.execute(
                "UPDATE sagas SET updated_at = ?1 WHERE saga_id = ?2",
                params![now, saga_id],
            )?;
        }
        Ok(())
    }

    /// The workflow reached its goal: drops the log so it can never be
    /// compensated afterwards. Refuses on anything but `open` -- committing
    /// a compensating saga would silently drop the walk.
    pub fn commit(&self, saga_id: &str) -> Result<()> {
        let mut conn = self.conn.lock().expect("saga log connection lock poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let state: Option<String> = tx
            .query_row("SELECT state FROM sagas WHERE saga_id = ?1", params![saga_id], |r| r.get(0))
            .optional()?;
        let Some(state) = state else { return Err(anyhow!("unknown saga {saga_id}")) };
        if state != "open" {
            return Err(anyhow!("saga {saga_id} is {state}; only an open saga can be committed"));
        }
        tx.execute("DELETE FROM saga_steps WHERE saga_id = ?1", params![saga_id])?;
        tx.execute("DELETE FROM sagas WHERE saga_id = ?1", params![saga_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Transitions an `open` saga to `compensating`, immediately due.
    /// Returns whether it actually transitioned -- `false` for an unknown
    /// saga or one already past `open` (idempotent either way: a second
    /// `compensate` on an already-compensating saga is a no-op, not an
    /// error).
    pub fn mark_compensating(&self, saga_id: &str, now: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("saga log connection lock poisoned");
        let updated = conn.execute(
            "UPDATE sagas SET state = 'compensating', next_attempt_at = ?1, updated_at = ?1 WHERE \
             saga_id = ?2 AND state = 'open'",
            params![now, saga_id],
        )?;
        Ok(updated > 0)
    }

    /// Every saga due for its next compensation attempt, oldest-due first.
    pub fn due_compensations(&self, now: i64, limit: u32) -> Result<Vec<SagaHead>> {
        let conn = self.conn.lock().expect("saga log connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT saga_id, app_instance_id FROM sagas
             WHERE state = 'compensating' AND next_attempt_at <= ?1
             ORDER BY next_attempt_at ASC LIMIT ?2",
        )?;
        Self::query_heads(&mut stmt, params![now, limit])
    }

    /// Every still-`open` saga past its declared deadline -- the crash
    /// case: nothing else can notice, because a guest does not exist
    /// between calls.
    pub fn abandoned(&self, now: i64, limit: u32) -> Result<Vec<SagaHead>> {
        let conn = self.conn.lock().expect("saga log connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT saga_id, app_instance_id FROM sagas
             WHERE state = 'open' AND deadline_at <= ?1
             ORDER BY deadline_at ASC LIMIT ?2",
        )?;
        Self::query_heads(&mut stmt, params![now, limit])
    }

    fn query_heads(
        stmt: &mut rusqlite::Statement<'_>,
        params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<SagaHead>> {
        let mut rows = stmt.query(params)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(SagaHead { saga_id: row.get(0)?, app_instance_id: row.get(1)? });
        }
        Ok(out)
    }

    /// The step the walk should undo next: the highest index still `done`
    /// or `pending` -- a `pending` step counts, since its intent was
    /// written before the call whose result may never have come back.
    /// `None` means nothing left to compensate.
    pub fn next_uncompensated_step(&self, saga_id: &str) -> Result<Option<StepRow>> {
        let conn = self.conn.lock().expect("saga log connection lock poisoned");
        conn.query_row(
            "SELECT idx, target, routing_key, interface, method, params, result, attempts
             FROM saga_steps
             WHERE saga_id = ?1 AND state IN ('done', 'pending')
             ORDER BY idx DESC LIMIT 1",
            params![saga_id],
            |row| {
                Ok(StepRow {
                    idx: row.get::<_, i64>(0)? as u32,
                    target: row.get(1)?,
                    routing_key: row.get(2)?,
                    interface: row.get(3)?,
                    method: row.get(4)?,
                    params: row.get(5)?,
                    result: row.get(6)?,
                    attempts: row.get::<_, i64>(7)? as u32,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    /// Increments a step's attempt count *before* its undo is dispatched:
    /// a crash mid-undo therefore costs an attempt, bounding a poison
    /// undo, and a re-dispatch after that crash is safe because
    /// every undo carries an idempotency key the receiver's own fence
    /// answers a duplicate from. Returns the new attempt count.
    pub fn begin_undo_attempt(&self, saga_id: &str, idx: u32, now: i64) -> Result<u32> {
        let conn = self.conn.lock().expect("saga log connection lock poisoned");
        conn.execute(
            "UPDATE saga_steps SET attempts = attempts + 1 WHERE saga_id = ?1 AND idx = ?2",
            params![saga_id, idx],
        )?;
        conn.execute("UPDATE sagas SET updated_at = ?1 WHERE saga_id = ?2", params![now, saga_id])?;
        let attempts: i64 = conn.query_row(
            "SELECT attempts FROM saga_steps WHERE saga_id = ?1 AND idx = ?2",
            params![saga_id, idx],
            |r| r.get(0),
        )?;
        Ok(attempts as u32)
    }

    pub fn mark_step_compensated(&self, saga_id: &str, idx: u32, now: i64) -> Result<()> {
        let conn = self.conn.lock().expect("saga log connection lock poisoned");
        conn.execute(
            "UPDATE saga_steps SET state = 'compensated', last_error = NULL WHERE saga_id = ?1 \
             AND idx = ?2",
            params![saga_id, idx],
        )?;
        conn.execute("UPDATE sagas SET updated_at = ?1 WHERE saga_id = ?2", params![now, saga_id])?;
        Ok(())
    }

    /// Records a failed undo attempt. `terminal`: the caller already knows
    /// this attempt can never succeed (e.g. a dependency name bound to
    /// nobody), so the saga fails regardless of budget remaining.
    /// Otherwise: still under budget schedules a backoff and stays
    /// `compensating`; exhausted fails the saga, keeping its step history
    /// and pruning terminal rows past the configured cap.
    pub fn fail_compensation(
        &self,
        saga_id: &str,
        idx: u32,
        now: i64,
        error: &str,
        terminal: bool,
    ) -> Result<CompensationOutcome> {
        let mut conn = self.conn.lock().expect("saga log connection lock poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let attempts: i64 = tx.query_row(
            "SELECT attempts FROM saga_steps WHERE saga_id = ?1 AND idx = ?2",
            params![saga_id, idx],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE saga_steps SET last_error = ?1 WHERE saga_id = ?2 AND idx = ?3",
            params![error, saga_id, idx],
        )?;
        let outcome = if terminal || attempts >= i64::from(self.config.retry.max_attempts) {
            tx.execute(
                "UPDATE sagas SET state = 'failed', next_attempt_at = NULL, last_error = ?1, \
                 updated_at = ?2 WHERE saga_id = ?3",
                params![error, now, saga_id],
            )?;
            Self::prune_terminal(&tx, self.config.max_terminal_rows)?;
            CompensationOutcome::Failed
        } else {
            let base = backoff_before_wait(&self.config.retry, attempts as u32);
            let jittered = calculate_jittered_backoff(base);
            let next_attempt_at = now + jittered as i64;
            tx.execute(
                "UPDATE sagas SET next_attempt_at = ?1, last_error = ?2, updated_at = ?3 WHERE \
                 saga_id = ?4",
                params![next_attempt_at, error, now, saga_id],
            )?;
            CompensationOutcome::Retry { next_attempt_at }
        };
        tx.commit()?;
        Ok(outcome)
    }

    /// Everything is compensated: drops the step log and marks the saga
    /// `compensated` -- a committed/compensated saga can never be walked
    /// again, so nothing reads the row.
    pub fn finish_compensation(&self, saga_id: &str, now: i64) -> Result<()> {
        let mut conn = self.conn.lock().expect("saga log connection lock poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM saga_steps WHERE saga_id = ?1", params![saga_id])?;
        tx.execute(
            "UPDATE sagas SET state = 'compensated', next_attempt_at = NULL, updated_at = ?1 \
             WHERE saga_id = ?2",
            params![now, saga_id],
        )?;
        Self::prune_terminal(&tx, self.config.max_terminal_rows)?;
        tx.commit()?;
        Ok(())
    }

    /// Prunes terminal (`compensated`/`failed`) rows oldest-first past
    /// `max_rows`, mirroring `Queue::prune_dead_letters` -- except unscoped
    /// by any group, since this log already belongs to one service.
    fn prune_terminal(conn: &Connection, max_rows: u32) -> Result<Vec<String>> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sagas WHERE state IN ('compensated', 'failed')",
            [],
            |r| r.get(0),
        )?;
        let excess = count - i64::from(max_rows);
        if excess <= 0 {
            return Ok(Vec::new());
        }
        let mut stmt = conn.prepare(
            "SELECT saga_id FROM sagas WHERE state IN ('compensated', 'failed')
             ORDER BY updated_at ASC, saga_id ASC LIMIT ?1",
        )?;
        let ids: Vec<String> = {
            let mut rows = stmt.query(params![excess])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(row.get::<_, String>(0)?);
            }
            out
        };
        drop(stmt);
        for id in &ids {
            conn.execute("DELETE FROM saga_steps WHERE saga_id = ?1", params![id])?;
            conn.execute("DELETE FROM sagas WHERE saga_id = ?1", params![id])?;
        }
        Ok(ids)
    }

    pub fn status(&self, saga_id: &str) -> Result<Option<SagaInfo>> {
        let conn = self.conn.lock().expect("saga log connection lock poisoned");
        Self::load_info(&conn, saga_id)
    }

    /// Every saga this log holds, oldest first -- the operator/guest
    /// listing.
    pub fn list(&self) -> Result<Vec<SagaInfo>> {
        let conn = self.conn.lock().expect("saga log connection lock poisoned");
        let ids: Vec<String> = {
            let mut stmt = conn.prepare("SELECT saga_id FROM sagas ORDER BY created_at ASC")?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(row.get::<_, String>(0)?);
            }
            out
        };
        ids.iter()
            .filter_map(|id| Self::load_info(&conn, id).transpose())
            .collect::<Result<Vec<_>>>()
    }

    fn load_info(conn: &Connection, saga_id: &str) -> Result<Option<SagaInfo>> {
        let row: Option<(String, String, String, i64, i64, Option<String>)> = conn
            .query_row(
                "SELECT saga_id, name, state, created_at, deadline_at, last_error FROM sagas \
                 WHERE saga_id = ?1",
                params![saga_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        let Some((saga_id, name, state_str, created_at, deadline_at, last_error)) = row else {
            return Ok(None);
        };
        // Validate the stored state is one this crate understands, even
        // though `SagaInfo::state` carries the raw string onward -- an
        // unrecognized value here is a schema-level bug worth surfacing at
        // the read, not silently forwarded.
        SagaState::parse(&state_str)?;
        let steps: i64 = conn.query_row(
            "SELECT COUNT(*) FROM saga_steps WHERE saga_id = ?1",
            params![saga_id],
            |r| r.get(0),
        )?;
        let compensated_steps: i64 = conn.query_row(
            "SELECT COUNT(*) FROM saga_steps WHERE saga_id = ?1 AND state = 'compensated'",
            params![saga_id],
            |r| r.get(0),
        )?;
        Ok(Some(SagaInfo {
            saga_id,
            name,
            state: state_str,
            steps: steps as u32,
            compensated_steps: compensated_steps as u32,
            created_at,
            deadline_at,
            last_error,
        }))
    }

    /// The operator's `saga-compensate`: re-arms a `failed` saga back to
    /// `compensating`, with the current (next-to-undo) step's attempts
    /// reset -- the same rule and reason `replay` re-arms a dead letter
    /// (ADR-0023 §5). Never walks inline; the worker picks it up on its
    /// next tick. Returns whether it actually re-armed something.
    pub fn rearm(&self, saga_id: &str, now: i64) -> Result<bool> {
        let mut conn = self.conn.lock().expect("saga log connection lock poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let updated = tx.execute(
            "UPDATE sagas SET state = 'compensating', next_attempt_at = ?1, last_error = NULL, \
             updated_at = ?1 WHERE saga_id = ?2 AND state = 'failed'",
            params![now, saga_id],
        )?;
        if updated == 0 {
            tx.commit()?;
            return Ok(false);
        }
        tx.execute(
            "UPDATE saga_steps SET attempts = 0, last_error = NULL WHERE saga_id = ?1 AND idx = \
             (SELECT idx FROM saga_steps WHERE saga_id = ?1 AND state IN ('done', 'pending')
              ORDER BY idx DESC LIMIT 1)",
            params![saga_id],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// How many sagas are currently `open` -- the bound `begin` enforces.
    pub fn open_count(&self) -> Result<u32> {
        let conn = self.conn.lock().expect("saga log connection lock poisoned");
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM sagas WHERE state = 'open'", [], |r| r.get(0))?;
        Ok(count as u32)
    }

    /// Drops every saga this log holds, unconditionally -- for a service
    /// the sweep finds is no longer deployed. Nothing removes a service's
    /// data directory on undeploy, so its sagas are dropped rather than
    /// compensated: the operator withdrew the whole service, and sending
    /// undos on behalf of something that no longer exists would resurrect
    /// intent the operator withdrew.
    pub fn drop_all_for_undeployed(&self) -> Result<()> {
        let mut conn = self.conn.lock().expect("saga log connection lock poisoned");
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM saga_steps", [])?;
        tx.execute("DELETE FROM sagas", [])?;
        tx.commit()?;
        Ok(())
    }

    /// This log's configured attempt budget, exposed so a caller (the
    /// walk) can apply the identical ceiling to a case this crate cannot
    /// see on its own -- a step whose undo was attempted repeatedly
    /// without ever completing, mirroring `Queue::max_attempts`'s own
    /// reasoning.
    #[must_use]
    pub fn max_attempts(&self) -> u8 {
        self.config.retry.max_attempts
    }
}

/// Opens (creating on first use) a WAL-mode SQLite connection at
/// `dir/db_name`, applying `PRAGMA key` before anything else touches the
/// file when `dek` is present -- identical to [`crate::Queue`]'s own
/// `open_connection`, duplicated rather than shared since it is four lines
/// and this crate has no third module to hang a shared helper off yet.
fn open_connection<P: AsRef<Path>>(
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

#[cfg(test)]
mod tests;

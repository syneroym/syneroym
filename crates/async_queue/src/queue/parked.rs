//! The parked state: an outbox item that waits outside the attempt budget,
//! on its own poll interval, until its own deadline -- for example a
//! message handed to a third party, where the sender now only asks for a
//! receipt and must not deliver directly again.

use anyhow::{Result, ensure};
use rusqlite::{Connection, Transaction, params};

use super::{CLEAR_PARK, ClaimKind, Queue, QueueItem, TxQueue};

/// The maximum size of the `context` blob passed to [`Queue::park`]. The
/// context comes from an external party (for example a mailbox deposit
/// receipt), so a bound here limits how large a hostile or broken node
/// can make each parked row.
pub const MAX_PARK_CONTEXT_BYTES: usize = 4096;

#[allow(clippy::expect_used)] // same lock-poisoning reasoning as queue.rs
impl Queue {
    /// Parks an item, or updates an item that is already parked.
    ///
    /// The item stays in the outbox (it keeps its key and its payload) but
    /// [`Self::claim_due`] no longer returns it. [`Self::claim_parked_due`]
    /// returns it after `poll_interval_ms`, and never later than
    /// `deadline_at`. Parking never changes `attempts`. The first park
    /// un-counts the claim that led to it, as [`Self::defer`] does; a
    /// re-park does not change `claim_count`, since a parked claim never
    /// added to it.
    pub fn park(
        &self,
        id: i64,
        now: i64,
        poll_interval_ms: u64,
        deadline_at: i64,
        context: &[u8],
    ) -> Result<()> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        park_on(&conn, id, now, poll_interval_ms, deadline_at, context)
    }

    /// Makes a parked item pending again, due at `visible_at`. `attempts`
    /// and `claim_count` keep the values they had when it was parked, so
    /// an item that is parked and unparked again and again still uses up
    /// its attempt budget and ends. Errors when the item does not exist or
    /// is not parked.
    pub fn unpark(&self, id: i64, visible_at: i64) -> Result<()> {
        let conn = self.conn.lock().expect("queue connection lock poisoned");
        unpark_on(&conn, id, visible_at)
    }

    /// Claims up to `limit` parked items whose poll time or deadline has
    /// come, and hides them for `visibility_timeout_ms`, as
    /// [`Self::claim_due`] does for pending items. A parked claim does not
    /// advance `claim_count` or `attempts`: the deadline, not the attempt
    /// budget, bounds a parked item. The caller resolves each item with
    /// [`Self::park`] (poll again later), [`Self::complete`],
    /// [`Self::fail`] or [`Self::unpark`]. An item it leaves alone comes
    /// back after the visibility timeout.
    pub fn claim_parked_due(&self, now: i64, limit: u32) -> Result<Vec<QueueItem>> {
        self.claim(now, limit, ClaimKind::Parked)
    }
}

impl TxQueue<'_> {
    /// Same effect as [`Queue::park`], through `tx`, so an owner can park
    /// an item in the same commit as its own state change.
    pub fn park(
        &self,
        tx: &Transaction<'_>,
        id: i64,
        now: i64,
        poll_interval_ms: u64,
        deadline_at: i64,
        context: &[u8],
    ) -> Result<()> {
        park_on(tx, id, now, poll_interval_ms, deadline_at, context)
    }

    /// Same effect as [`Queue::unpark`], through `tx`.
    pub fn unpark(&self, tx: &Transaction<'_>, id: i64, visible_at: i64) -> Result<()> {
        unpark_on(tx, id, visible_at)
    }
}

fn park_on(
    conn: &Connection,
    id: i64,
    now: i64,
    poll_interval_ms: u64,
    deadline_at: i64,
    context: &[u8],
) -> Result<()> {
    ensure!(poll_interval_ms > 0, "park poll interval must be above zero");
    ensure!(deadline_at > now, "park deadline {deadline_at} is not after now {now}");
    ensure!(
        context.len() <= MAX_PARK_CONTEXT_BYTES,
        "park_context is {} bytes, exceeds {MAX_PARK_CONTEXT_BYTES}-byte limit",
        context.len()
    );
    let poll = i64::try_from(poll_interval_ms).unwrap_or(i64::MAX);
    let visible_at = now.saturating_add(poll).min(deadline_at);
    // SQLite evaluates every SET expression against the row as it was
    // before this UPDATE, so the CASE sees the old park_deadline.
    let updated = conn.execute(
        "UPDATE outbox SET
             claim_count = CASE WHEN park_deadline IS NULL
                                THEN MAX(claim_count - 1, 0) ELSE claim_count END,
             parked_at = COALESCE(parked_at, ?1),
             park_deadline = ?2, park_poll_ms = ?3, park_context = ?4, visible_at = ?5
         WHERE id = ?6",
        params![now, deadline_at, poll, context, visible_at, id],
    )?;
    ensure!(updated == 1, "no outbox item with id {id}");
    Ok(())
}

fn unpark_on(conn: &Connection, id: i64, visible_at: i64) -> Result<()> {
    let updated = conn.execute(
        &format!(
            "UPDATE outbox SET {CLEAR_PARK}, visible_at = ?1 WHERE id = ?2 AND park_deadline IS \
             NOT NULL"
        ),
        params![visible_at, id],
    )?;
    ensure!(updated == 1, "no parked outbox item with id {id}");
    Ok(())
}

//! Removing deleted text from the database file itself, and keeping the
//! number of dropped rows bounded.
//!
//! A delete or drop clears the message body and tells the search index to
//! forget it, but neither rewrites the pages that still hold the old text:
//! the index keeps the trigrams of the removed body until its segments are
//! merged, and the write-ahead log keeps the old row until it is truncated.

use std::time::{Duration, Instant};

use anyhow::Result;
use rusqlite::{Connection, params};

use super::ConversationStore;

/// Deletes dropped rows past the cap in direct chats. Both reads are pinned
/// to the partial index on dropped rows: without that SQLite picks the
/// admission index and sorts every dropped row in the store. Only chats over
/// the cap reach the sort.
pub(super) const PRUNE_DROPPED_SQL: &str = "DELETE FROM messages WHERE rowid IN (
     SELECT rowid FROM (
         SELECT rowid, ROW_NUMBER() OVER (
             PARTITION BY conversation_id ORDER BY received_at DESC, rowid DESC
         ) AS n
         FROM messages INDEXED BY idx_messages_dropped
         WHERE admission = 'dropped' AND conversation_id IN (
             SELECT d.conversation_id FROM messages d INDEXED BY idx_messages_dropped
             JOIN conversations c ON c.id = d.conversation_id
             WHERE d.admission = 'dropped' AND c.kind = 'direct'
             GROUP BY d.conversation_id HAVING COUNT(*) > ?1
         )
     ) WHERE n > ?1
 )";

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
    /// Whether enough time has passed since the last finished scrub. The
    /// first scrub after a quiet period is never delayed; only a burst of
    /// deletes or drops is spread out, because rewriting the whole search
    /// index holds the connection lock for as long as the index is large.
    #[must_use]
    pub fn scrub_due(&self) -> bool {
        let min_gap = Duration::from_secs(self.config.scrub_min_interval_secs);
        let last = self.last_scrub.lock().expect("scrub clock lock poisoned");
        last.is_none_or(|at| at.elapsed() >= min_gap)
    }

    /// Records that a scrub finished just now.
    pub fn mark_scrubbed(&self) {
        *self.last_scrub.lock().expect("scrub clock lock poisoned") = Some(Instant::now());
    }

    /// Deletes the oldest dropped rows of each direct conversation beyond
    /// `max_dropped_per_conversation`, and returns how many went. A dropped
    /// row is kept so a repeat delivery is recognised; without a bound, a
    /// blocked sender could add rows for ever. Group conversations are
    /// skipped: their rows are bounded by the group log, and every member
    /// must keep the same rows for the transcript code to match.
    pub fn prune_dropped(&self) -> Result<usize> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let removed =
            conn.execute(PRUNE_DROPPED_SQL, params![self.config.max_dropped_per_conversation])?;
        Ok(removed)
    }

    /// Merges the search index and truncates the log. Returns `true` when
    /// the pass did not finish (a reader held the log, or a statement
    /// failed) and must be tried again on a later tick.
    pub fn scrub_and_checkpoint(&self) -> bool {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        // Do not wait on a reader while every other store call waits on this
        // lock: a blocked pass is simply tried again on the next tick.
        let wait: i64 = conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0)).unwrap_or(0);
        let _ = conn.busy_timeout(Duration::ZERO);
        let retry = Self::scrub(&conn);
        let _ = conn.busy_timeout(Duration::from_millis(wait.max(0) as u64));
        retry
    }

    /// Records, inside the caller's transaction, that a scrub is needed.
    pub(crate) fn mark_scrub_needed(conn: &Connection) -> Result<()> {
        conn.execute("UPDATE store_flags SET needs_scrub = 1 WHERE id = 1", [])?;
        Ok(())
    }

    fn scrub(conn: &Connection) -> bool {
        // Cleared first, under the same lock: a delete cannot slip in between,
        // and a pass that fails sets it again below.
        if conn.execute("UPDATE store_flags SET needs_scrub = 0 WHERE id = 1", []).is_err() {
            return true;
        }
        let retry = Self::merge_and_truncate(conn);
        if retry {
            let _ = Self::mark_scrub_needed(conn);
        }
        retry
    }

    fn merge_and_truncate(conn: &Connection) -> bool {
        if conn.execute_batch("INSERT INTO messages_fts(messages_fts) VALUES('optimize');").is_err()
        {
            return true;
        }
        // The pragma reports a blocked checkpoint as a result row with a
        // non-zero first column, not as an error.
        !conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get::<_, i64>(0))
            .is_ok_and(|busy| busy == 0)
    }
}

//! Removing deleted text from the database file itself.
//!
//! A delete or drop clears the message body and tells the search index to
//! forget it, but neither rewrites the pages that still hold the old text:
//! the index keeps the trigrams of the removed body until its segments are
//! merged, and the write-ahead log keeps the old row until it is truncated.

use std::time::Duration;

use rusqlite::Connection;

use super::ConversationStore;

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
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

    fn scrub(conn: &Connection) -> bool {
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

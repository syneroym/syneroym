//! Full-text search over visible, non-deleted text message bodies using FTS5
//! with the trigram tokenizer and LIKE fallback for short queries.

use anyhow::Result;
use rusqlite::params;

use super::{ConversationStore, StoredMessage, message::row_to_message};

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
    pub fn search(
        &self,
        query: &str,
        conversation: Option<&str>,
        limit: u32,
    ) -> Result<Vec<StoredMessage>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let char_count = query.chars().count();
        if char_count >= 3 {
            // FTS5 phrase query: wrap in double-quotes and double internal quotes
            let escaped = format!("\"{}\"", query.replace('"', "\"\""));
            let mut stmt = conn.prepare(
                "SELECT m.id, m.conversation_id, m.author, m.sender_timestamp, m.received_at, \
                 m.content_type, m.body, m.signature, m.outgoing, m.verified, m.state, \
                 m.last_error, m.system, m.entry_id, m.admission, m.admission_reason, \
                 m.admission_changed_at, m.notify_attempts, m.next_notify_at, m.report_refusal, \
                 m.refused, m.deleted_at, m.restored, m.visible_seq FROM messages m JOIN \
                 messages_fts f ON m.rowid = f.rowid WHERE messages_fts MATCH ?1 AND m.system = 0 \
                 AND (m.outgoing = 1 OR m.admission = 'accepted') AND m.deleted_at IS NULL AND \
                 (?2 IS NULL OR m.conversation_id = ?2) ORDER BY m.sender_timestamp ASC, m.author \
                 ASC, m.id ASC LIMIT ?3",
            )?;
            let mut rows = stmt.query(params![escaped, conversation, limit as i64])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(row_to_message(r)?);
            }
            Ok(out)
        } else {
            // Short query fallback: LIKE scan over text content types
            let pattern = format!("%{query}%");
            let mut stmt = conn.prepare(
                "SELECT m.id, m.conversation_id, m.author, m.sender_timestamp, m.received_at, \
                 m.content_type, m.body, m.signature, m.outgoing, m.verified, m.state, \
                 m.last_error, m.system, m.entry_id, m.admission, m.admission_reason, \
                 m.admission_changed_at, m.notify_attempts, m.next_notify_at, m.report_refusal, \
                 m.refused, m.deleted_at, m.restored, m.visible_seq FROM messages m WHERE \
                 CAST(m.body AS TEXT) LIKE ?1 AND m.system = 0 AND (m.outgoing = 1 OR m.admission \
                 = 'accepted') AND m.deleted_at IS NULL AND (m.content_type LIKE 'text/%' OR \
                 m.content_type = 'application/json' OR m.content_type LIKE '%+json') AND (?2 IS \
                 NULL OR m.conversation_id = ?2) ORDER BY m.sender_timestamp ASC, m.author ASC, \
                 m.id ASC LIMIT ?3",
            )?;
            let mut rows = stmt.query(params![pattern, conversation, limit as i64])?;
            let mut out = Vec::new();
            while let Some(r) = rows.next()? {
                out.push(row_to_message(r)?);
            }
            Ok(out)
        }
    }
}

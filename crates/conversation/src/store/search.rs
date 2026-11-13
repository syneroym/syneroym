//! Full-text search over visible, non-deleted text message bodies using
//! trigram and word tables over search keys.

use anyhow::Result;
use rusqlite::params;
use syneroym_text_lang::{fts, query_terms};

use super::{
    ConversationStore, StoredMessage,
    message::{M_MESSAGE_COLUMNS, row_to_message},
};

/// Maximum rows returned by a single search call.
const MAX_SEARCH_LIMIT: u32 = 500;

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
        let terms = query_terms(query);
        if terms.long.is_empty() && terms.short.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let limit = limit.min(MAX_SEARCH_LIMIT);
        let (tri_match, word_match) = fts::match_exprs("body", &terms);

        let mut out = Vec::new();
        match (tri_match, word_match) {
            (Some(tri), Some(word)) => {
                let sql = format!(
                    "SELECT {M_MESSAGE_COLUMNS} FROM messages m WHERE m.rowid IN (SELECT rowid \
                     FROM messages_tri WHERE messages_tri MATCH ?1) AND m.rowid IN (SELECT rowid \
                     FROM messages_word WHERE messages_word MATCH ?2) AND m.system = 0 AND \
                     (m.outgoing = 1 OR m.admission = 'accepted') AND m.deleted_at IS NULL AND \
                     (?3 IS NULL OR m.conversation_id = ?3) ORDER BY m.sender_timestamp DESC, \
                     m.author DESC, m.id DESC LIMIT ?4"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut rows = stmt.query(params![tri, word, conversation, limit as i64])?;
                while let Some(r) = rows.next()? {
                    out.push(row_to_message(r)?);
                }
            }
            (Some(tri), None) => {
                let sql = format!(
                    "SELECT {M_MESSAGE_COLUMNS} FROM messages m WHERE m.rowid IN (SELECT rowid \
                     FROM messages_tri WHERE messages_tri MATCH ?1) AND m.system = 0 AND \
                     (m.outgoing = 1 OR m.admission = 'accepted') AND m.deleted_at IS NULL AND \
                     (?2 IS NULL OR m.conversation_id = ?2) ORDER BY m.sender_timestamp DESC, \
                     m.author DESC, m.id DESC LIMIT ?3"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut rows = stmt.query(params![tri, conversation, limit as i64])?;
                while let Some(r) = rows.next()? {
                    out.push(row_to_message(r)?);
                }
            }
            (None, Some(word)) => {
                let sql = format!(
                    "SELECT {M_MESSAGE_COLUMNS} FROM messages m WHERE m.rowid IN (SELECT rowid \
                     FROM messages_word WHERE messages_word MATCH ?1) AND m.system = 0 AND \
                     (m.outgoing = 1 OR m.admission = 'accepted') AND m.deleted_at IS NULL AND \
                     (?2 IS NULL OR m.conversation_id = ?2) ORDER BY m.sender_timestamp DESC, \
                     m.author DESC, m.id DESC LIMIT ?3"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut rows = stmt.query(params![word, conversation, limit as i64])?;
                while let Some(r) = rows.next()? {
                    out.push(row_to_message(r)?);
                }
            }
            (None, None) => {}
        }
        Ok(out)
    }
}

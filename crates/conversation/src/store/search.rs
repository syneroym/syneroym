//! Full-text search over visible, non-deleted text message bodies using
//! trigram and word tables over search keys.

use anyhow::Result;
use rusqlite::{params_from_iter, types::Value};
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
        if tri_match.is_none() && word_match.is_none() {
            return Ok(Vec::new());
        }

        let (sql, values) = search_query(tri_match, word_match, conversation, limit);
        let mut out = Vec::new();
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(params_from_iter(values))?;
        while let Some(r) = rows.next()? {
            out.push(row_to_message(r)?);
        }
        Ok(out)
    }
}

/// The one search statement and its positional values. The text clauses are
/// only those that have a match expression; the rest is the same for every
/// shape. Each clause selects rowids from its own index table, so the chat
/// table is searched by rowid and never scanned.
pub(super) fn search_query(
    tri_match: Option<String>,
    word_match: Option<String>,
    conversation: Option<&str>,
    limit: u32,
) -> (String, Vec<Value>) {
    let mut values = Vec::new();
    let mut clauses = Vec::new();
    for (table, expr) in [("messages_tri", tri_match), ("messages_word", word_match)] {
        if let Some(expr) = expr {
            values.push(Value::Text(expr));
            clauses.push(format!(
                "m.rowid IN (SELECT rowid FROM {table} WHERE {table} MATCH ?{})",
                values.len()
            ));
        }
    }
    values.push(conversation.map_or(Value::Null, |c| Value::Text(c.to_string())));
    let conv = values.len();
    values.push(Value::Integer(i64::from(limit)));
    let limit_at = values.len();

    let sql = format!(
        "SELECT {M_MESSAGE_COLUMNS} FROM messages m WHERE {} AND m.system = 0 AND (m.outgoing = 1 \
         OR m.admission = 'accepted') AND m.deleted_at IS NULL AND (?{conv} IS NULL OR \
         m.conversation_id = ?{conv}) ORDER BY m.sender_timestamp DESC, m.author DESC, m.id DESC \
         LIMIT ?{limit_at}",
        clauses.join(" AND ")
    );
    (sql, values)
}

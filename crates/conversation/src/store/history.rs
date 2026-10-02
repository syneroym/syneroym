//! History pagination, changes feed, and transcript digest calculation.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use syneroym_rpc::{
    ConversationChangePage, ConversationHistoryItem, ConversationHistoryPage,
    ConversationMembershipEvent, ConversationNameEvent,
};
use syneroym_signed_record::content_digest;

use super::{ConversationStore, StoredDagEntry, StoredMessage, message::row_to_message};
use crate::dag::EntryKind;

pub const TRANSCRIPT_DIGEST_PREFIX: &str = "roym-transcript:";
pub const MEMBERSHIP_EVENT_CONTENT_TYPE: &str = "application/vnd.roym.membership-event+json";
pub const GROUP_PROFILE_CONTENT_TYPE: &str = "application/vnd.roym.group-profile+json";

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
    pub fn changes(
        &self,
        conversation_id: &str,
        after_seq: u64,
        limit: u32,
    ) -> Result<ConversationChangePage> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, author, sender_timestamp, received_at, content_type, \
             body, signature, outgoing, verified, state, last_error, system, entry_id, admission, \
             admission_reason, admission_changed_at, notify_attempts, next_notify_at, \
             report_refusal, refused, deleted_at, restored, visible_seq FROM messages WHERE \
             conversation_id = ?1 AND visible_seq > ?2 AND system = 0 AND (outgoing = 1 OR \
             admission = 'accepted') ORDER BY visible_seq ASC LIMIT ?3",
        )?;
        let mut rows = stmt.query(params![conversation_id, after_seq as i64, limit as i64])?;
        let mut messages = Vec::new();
        while let Some(r) = rows.next()? {
            let msg: StoredMessage = row_to_message(r)?;
            messages.push(msg.into_wire());
        }
        let last_seq = messages.last().map(|m| m.visible_seq).unwrap_or(after_seq);
        Ok(ConversationChangePage { messages, last_seq })
    }

    pub fn history(
        &self,
        conversation_id: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<ConversationHistoryPage> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let (after_ts, after_author, after_id) = match cursor {
            Some(cid) => self.cursor_sort_key(&conn, conversation_id, cid)?,
            None => (i64::MIN, String::new(), String::new()),
        };
        let fetch_limit = i64::from(limit) + 1;

        // Fetch messages after cursor
        let mut m_stmt = conn.prepare(
            "SELECT id, conversation_id, author, sender_timestamp, received_at, content_type, \
             body, signature, outgoing, verified, state, last_error, system, entry_id, admission, \
             admission_reason, admission_changed_at, notify_attempts, next_notify_at, \
             report_refusal, refused, deleted_at, restored, visible_seq FROM messages WHERE \
             conversation_id = ?1 AND system = 0 AND (outgoing = 1 OR admission = 'accepted') AND \
             (sender_timestamp, author, id) > (?2, ?3, ?4) ORDER BY sender_timestamp ASC, author \
             ASC, id ASC LIMIT ?5",
        )?;
        let mut m_rows = m_stmt.query(params![
            conversation_id,
            after_ts,
            after_author,
            after_id,
            fetch_limit
        ])?;
        let mut msgs: Vec<StoredMessage> = Vec::new();
        while let Some(r) = m_rows.next()? {
            msgs.push(row_to_message(r)?);
        }

        // Fetch DAG entries (membership and profile) after cursor
        let mut d_stmt = conn.prepare(
            "SELECT seq, entry_id, conversation_id, author, sender_timestamp, epoch, kind, \
             header, ciphertext, nonce, payload, signature, applied, relay_pending FROM \
             dag_entries WHERE conversation_id = ?1 AND kind IN ('membership', 'profile') AND \
             applied = 1 AND (sender_timestamp, author, entry_id) > (?2, ?3, ?4) ORDER BY \
             sender_timestamp ASC, author ASC, entry_id ASC LIMIT ?5",
        )?;
        let mut d_rows = d_stmt.query(params![
            conversation_id,
            after_ts,
            after_author,
            after_id,
            fetch_limit
        ])?;
        let mut dags: Vec<StoredDagEntry> = Vec::new();
        while let Some(r) = d_rows.next()? {
            dags.push(Self::row_to_dag_entry(&conn, r)?);
        }

        // Merge two sorted streams
        let (items, next_cursor) = merge_history_items(msgs, dags, limit);
        Ok(ConversationHistoryPage { items, next_cursor })
    }

    fn cursor_sort_key(
        &self,
        conn: &rusqlite::Connection,
        conversation_id: &str,
        cursor_id: &str,
    ) -> Result<(i64, String, String)> {
        if let Some(row) = conn
            .query_row(
                "SELECT sender_timestamp, author FROM messages WHERE id = ?1 AND conversation_id \
                 = ?2",
                params![cursor_id, conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
        {
            return Ok((row.0, row.1, cursor_id.to_string()));
        }
        if let Some(row) = conn
            .query_row(
                "SELECT sender_timestamp, author FROM dag_entries WHERE entry_id = ?1 AND \
                 conversation_id = ?2",
                params![cursor_id, conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
        {
            return Ok((row.0, row.1, cursor_id.to_string()));
        }
        Ok((i64::MIN, String::new(), String::new()))
    }

    pub fn transcript_digest(&self, conversation_id: &str) -> Result<String> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        // All non-system messages (whatever admission, plus deleted)
        let mut m_stmt = conn.prepare(
            "SELECT id, author, sender_timestamp, content_type FROM messages WHERE \
             conversation_id = ?1 AND system = 0",
        )?;
        let mut m_rows = m_stmt.query(params![conversation_id])?;
        let mut entries: Vec<(i64, String, String, String)> = Vec::new();
        while let Some(r) = m_rows.next()? {
            let id: String = r.get(0)?;
            let author: String = r.get(1)?;
            let ts: i64 = r.get(2)?;
            let ct: String = r.get(3)?;
            entries.push((ts, author, id, ct));
        }

        // All DAG entries stored in group log with kind membership or profile
        let mut d_stmt = conn.prepare(
            "SELECT entry_id, author, sender_timestamp, kind FROM dag_entries WHERE \
             conversation_id = ?1 AND kind IN ('membership', 'profile')",
        )?;
        let mut d_rows = d_stmt.query(params![conversation_id])?;
        while let Some(r) = d_rows.next()? {
            let id: String = r.get(0)?;
            let author: String = r.get(1)?;
            let ts: i64 = r.get(2)?;
            let kind_str: String = r.get(3)?;
            let ct = if kind_str == "profile" {
                GROUP_PROFILE_CONTENT_TYPE.to_string()
            } else {
                MEMBERSHIP_EVENT_CONTENT_TYPE.to_string()
            };
            entries.push((ts, author, id, ct));
        }

        // Sort by (sender_timestamp, author, id)
        entries.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
        let lines: Vec<Value> = entries
            .into_iter()
            .map(|(ts, author, id, ct)| {
                serde_json::json!({
                    "id": id,
                    "author": author,
                    "sender_timestamp_ms": ts,
                    "content_type": ct,
                })
            })
            .collect();
        content_digest(TRANSCRIPT_DIGEST_PREFIX, &Value::Array(lines))
            .map_err(|e| anyhow::anyhow!("{e:?}"))
    }
}

fn dag_to_history_item(dag: StoredDagEntry) -> Option<ConversationHistoryItem> {
    match dag.kind {
        EntryKind::Membership => {
            let payload = dag.payload?;
            Some(ConversationHistoryItem::Membership(ConversationMembershipEvent {
                entry: dag.entry_id,
                action: payload.action,
                subject: payload.subject_address,
                epoch: payload.new_epoch,
                sender_timestamp: dag.sender_timestamp_ms,
            }))
        }
        EntryKind::Profile => {
            let profile = dag.profile_payload?;
            Some(ConversationHistoryItem::GroupName(ConversationNameEvent {
                entry: dag.entry_id,
                name: profile.name,
                sender_timestamp: dag.sender_timestamp_ms,
            }))
        }
        EntryKind::Message => None,
    }
}

fn history_item_id(item: &ConversationHistoryItem) -> String {
    match item {
        ConversationHistoryItem::Message(m) => m.id.clone(),
        ConversationHistoryItem::Membership(m) => m.entry.clone(),
        ConversationHistoryItem::GroupName(g) => g.entry.clone(),
    }
}

fn merge_history_items(
    msgs: Vec<StoredMessage>,
    dags: Vec<StoredDagEntry>,
    limit: u32,
) -> (Vec<ConversationHistoryItem>, Option<String>) {
    let mut merged = Vec::new();
    let mut m_iter = msgs.into_iter().peekable();
    let mut d_iter = dags.into_iter().peekable();

    while merged.len() <= limit as usize && (m_iter.peek().is_some() || d_iter.peek().is_some()) {
        let take_msg = match (m_iter.peek(), d_iter.peek()) {
            (Some(m), Some(d)) => {
                let m_key = (m.sender_timestamp_ms, m.author.as_str(), m.id.as_str());
                let d_key = (d.sender_timestamp_ms, d.author.as_str(), d.entry_id.as_str());
                m_key <= d_key
            }
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => unreachable!(),
        };

        if take_msg {
            if let Some(m) = m_iter.next() {
                merged.push(ConversationHistoryItem::Message(m.into_wire()));
            }
        } else if let Some(d) = d_iter.next()
            && let Some(item) = dag_to_history_item(d)
        {
            merged.push(item);
        }
    }

    let next_cursor = if merged.len() > limit as usize {
        merged.pop();
        merged.last().map(history_item_id)
    } else {
        None
    };
    (merged, next_cursor)
}

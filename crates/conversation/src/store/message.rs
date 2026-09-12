use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use syneroym_rpc::ConversationDeliveryState;

use super::{
    ConversationStore, HistoryPage, OutboxItem, StoreError, StoredMessage, state_from_str,
    state_str,
};

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
    pub fn message_count(&self, conversation_id: &str) -> Result<u32> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
            params![conversation_id],
            |r| r.get(0),
        )?;
        Ok(count as u32)
    }

    /// The atomic write for an outgoing `send` — one row in `messages`,
    /// one enqueue, one commit. The per-conversation bounds are enforced
    /// inside this transaction so concurrent `send` calls on the same
    /// conversation cannot both pass the check and both write.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_outgoing_and_enqueue(
        &self,
        conversation_id: &str,
        message_id: &str,
        author: &str,
        sender_timestamp_ms: i64,
        content_type: &str,
        body: &[u8],
        signature: &[u8; 64],
        peer_address: &str,
        now_ms: i64,
        system: bool,
    ) -> Result<()> {
        let payload = serde_json::to_vec(&OutboxItem {
            message_id: message_id.to_string(),
            peer_address: peer_address.to_string(),
            group: None,
        })?;
        let max_pending = self.config.max_pending_per_conversation;
        let max_messages = self.config.max_messages_per_conversation;
        self.queue.transaction(|tx, txq| {
            let pending_count: u32 = tx.query_row(
                "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1 AND state = 'pending'",
                rusqlite::params![conversation_id],
                |r| r.get::<_, i64>(0),
            )? as u32;
            if pending_count >= max_pending {
                return Err(StoreError::PendingQuotaExceeded.into());
            }
            let message_count: u32 = tx.query_row(
                "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
                rusqlite::params![conversation_id],
                |r| r.get::<_, i64>(0),
            )? as u32;
            if message_count >= max_messages {
                return Err(StoreError::MessageQuotaExceeded.into());
            }
            tx.execute(
                "INSERT INTO messages (id, conversation_id, author, sender_timestamp, \
                 received_at, content_type, body, signature, outgoing, verified, state, \
                 last_error, system, entry_id)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, 1, 1, 'pending', NULL, ?8, NULL)",
                params![
                    message_id,
                    conversation_id,
                    author,
                    sender_timestamp_ms,
                    content_type,
                    body,
                    signature.as_slice(),
                    if system { 1i64 } else { 0i64 }
                ],
            )?;
            Self::touch_conversation(tx, conversation_id, now_ms)?;
            txq.enqueue(tx, conversation_id, message_id, &payload, now_ms)?;
            Ok(())
        })?;
        Ok(())
    }

    /// Inbound insert-or-ignore: the whole of receiver-side dedup —
    /// a repeat delivery is a no-op, not an error, which is what makes
    /// at-least-once redelivery safe. Also enforces
    /// `max_messages_per_conversation` inside the same transaction.
    ///
    /// # API note
    /// `&self` is not used in this function body — only `tx` is touched.
    /// Do not reach for `self.conn` inside this function: the queue's mutex
    /// and `self.conn` share the same connection and doing so would
    /// self-deadlock the node.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_incoming_if_absent(
        &self,
        tx: &Transaction<'_>,
        conversation_id: &str,
        message_id: &str,
        author: &str,
        sender_timestamp_ms: i64,
        content_type: &str,
        body: &[u8],
        signature: &[u8; 64],
        now_ms: i64,
        max_messages_per_conversation: u32,
    ) -> Result<bool> {
        tx.execute(
            "INSERT INTO conversations (id, kind, peer_address, created_at, last_activity)
             VALUES (?1, 'direct', ?2, ?3, ?3)
             ON CONFLICT(peer_address) WHERE kind = 'direct' DO UPDATE SET last_activity = ?3",
            params![conversation_id, author, now_ms],
        )?;
        // Enforce the per-conversation message limit on the receive path.
        // Without this check a peer can fill an unbounded number of rows
        // into this service's store.
        let message_count: u32 = tx.query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1",
            params![conversation_id],
            |r| r.get::<_, i64>(0),
        )? as u32;
        if message_count >= max_messages_per_conversation {
            return Err(StoreError::MessageQuotaExceeded.into());
        }
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO messages (id, conversation_id, author, sender_timestamp, \
             received_at, content_type, body, signature, outgoing, verified, state, last_error, \
             system, entry_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 1, 'delivered', NULL, \
             0, NULL)",
            params![
                message_id,
                conversation_id,
                author,
                sender_timestamp_ms,
                now_ms,
                content_type,
                body,
                signature.as_slice()
            ],
        )?;
        Ok(inserted > 0)
    }

    pub fn get_message(&self, id: &str) -> Result<Option<StoredMessage>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        Self::query_message(&conn, id)
    }

    fn query_message(conn: &Connection, id: &str) -> Result<Option<StoredMessage>> {
        conn.query_row(
            "SELECT id, conversation_id, author, sender_timestamp, received_at, content_type, \
             body, signature, outgoing, verified, state, last_error, system, entry_id FROM \
             messages WHERE id = ?1",
            params![id],
            row_to_message,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn set_state(
        &self,
        id: &str,
        state: ConversationDeliveryState,
        last_error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.execute(
            "UPDATE messages SET state = ?1, last_error = ?2 WHERE id = ?3",
            params![state_str(state), last_error, id],
        )?;
        Ok(())
    }

    pub fn history(
        &self,
        conversation_id: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<HistoryPage> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        // Cursor is the last-seen message id from a previous page; since
        // ordering is (sender_timestamp, author, id) and `id` is unique,
        // resuming after that row's own ordering key is sufficient.
        let (after_ts, after_author, after_id) = match cursor {
            Some(id) => {
                let row: Option<(i64, String)> = conn
                    .query_row(
                        "SELECT sender_timestamp, author FROM messages WHERE id = ?1",
                        params![id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                match row {
                    Some((ts, author)) => (ts, author, id.to_string()),
                    None => (i64::MIN, String::new(), String::new()),
                }
            }
            None => (i64::MIN, String::new(), String::new()),
        };
        let fetch_limit = i64::from(limit) + 1;
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, author, sender_timestamp, received_at, content_type, \
             body, signature, outgoing, verified, state, last_error, system, entry_id FROM \
             messages
             WHERE conversation_id = ?1 AND system = 0
             AND (sender_timestamp, author, id) > (?2, ?3, ?4)
             ORDER BY sender_timestamp ASC, author ASC, id ASC
             LIMIT ?5",
        )?;
        let mut rows =
            stmt.query(params![conversation_id, after_ts, after_author, after_id, fetch_limit])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(row_to_message(row)?);
        }
        let next_cursor = if out.len() as u32 > limit {
            out.pop();
            out.last().map(|m: &StoredMessage| m.id.clone())
        } else {
            None
        };
        Ok(HistoryPage { messages: out, next_cursor })
    }

    /// Every message this service still owes delivery for, plus every one
    /// that gave up (`pending`/`failed`) -- the outbox surface (G2).
    pub fn outbox_messages(&self) -> Result<Vec<StoredMessage>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, author, sender_timestamp, received_at, content_type, \
             body, signature, outgoing, verified, state, last_error, system, entry_id FROM \
             messages
             WHERE state IN ('pending', 'failed') AND system = 0 ORDER BY sender_timestamp ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(row_to_message(row)?);
        }
        Ok(out)
    }

    pub fn set_recipient_state(
        &self,
        message_id: &str,
        member_address: &str,
        state: ConversationDeliveryState,
        last_error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.execute(
            "UPDATE message_recipients SET state = ?1, last_error = ?2 WHERE message_id = ?3 AND \
             member_address = ?4",
            params![state_str(state), last_error, message_id, member_address],
        )?;
        Ok(())
    }

    pub fn recipients_remaining(&self, message_id: &str) -> Result<u32> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM message_recipients WHERE message_id = ?1 AND state = 'pending'",
            params![message_id],
            |r| r.get(0),
        )?;
        Ok(count as u32)
    }

    pub fn any_recipient_failed(&self, message_id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM message_recipients WHERE message_id = ?1 AND state = 'failed'",
            params![message_id],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }
}

fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
    let body: Vec<u8> = row.get(6)?;
    let signature_bytes: Vec<u8> = row.get(7)?;
    // A wrong-length blob means the row is corrupt; fail loudly so a
    // caller sees an error rather than a silently malformed signature.
    let signature: [u8; 64] = signature_bytes.as_slice().try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            7,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::other("signature must be exactly 64 bytes")),
        )
    })?;
    let state_str: String = row.get(10)?;
    let system: i64 = row.get(12)?;
    let entry_id: Option<String> = row.get(13)?;
    Ok(StoredMessage {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        author: row.get(2)?,
        sender_timestamp_ms: row.get(3)?,
        received_at_ms: row.get(4)?,
        content_type: row.get(5)?,
        body,
        signature,
        outgoing: row.get::<_, i64>(8)? != 0,
        verified: row.get::<_, i64>(9)? != 0,
        state: state_from_str(&state_str),
        last_error: row.get(11)?,
        system: system != 0,
        entry_id,
    })
}

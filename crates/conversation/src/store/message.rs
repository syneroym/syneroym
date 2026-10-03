//! Read and write operations on the `messages` table: inserting outgoing
//! messages (atomically with the async-queue enqueue), storing incoming
//! deliveries, updating delivery state, and paginating history. Does not
//! touch `dag_entries` — group DAG persistence lives in `dag_store.rs`.

use std::{io, str};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params, types::Type};
use syneroym_rpc::{Admission, ConversationDeliveryState};

use super::{
    ConversationStore, OutboxItem, StoreError, StoredMessage, now_ms, state_from_str, state_str,
};
use crate::dag::{parse_deletion_request, parse_refusal_notice};

pub(crate) fn is_searchable_content_type(ct: &str) -> bool {
    ct.starts_with("text/") || ct == "application/json" || ct.ends_with("+json")
}

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
    pub(crate) fn next_visible_seq(conn: &Connection, conversation_id: &str) -> Result<u64> {
        let seq: i64 = conn.query_row(
            "INSERT INTO conversation_seq (conversation_id, last_seq) VALUES (?1, 1)
             ON CONFLICT(conversation_id) DO UPDATE SET last_seq = last_seq + 1
             RETURNING last_seq",
            params![conversation_id],
            |r| r.get(0),
        )?;
        Ok(seq as u64)
    }

    pub fn message_count(&self, conversation_id: &str) -> Result<u32> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1 AND system = 0 AND \
             (outgoing = 1 OR admission = 'accepted')",
            params![conversation_id],
            |r| r.get(0),
        )?;
        Ok(count as u32)
    }

    /// The atomic write for an outgoing `send` — one row in `messages`,
    /// one enqueue, one commit.
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
                "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1 AND admission != \
                 'dropped'",
                rusqlite::params![conversation_id],
                |r| r.get::<_, i64>(0),
            )? as u32;
            if message_count >= max_messages {
                return Err(StoreError::MessageQuotaExceeded.into());
            }
            let vseq = Self::next_visible_seq(tx, conversation_id)?;
            tx.execute(
                "INSERT INTO messages (id, conversation_id, author, sender_timestamp, \
                 received_at, content_type, body, signature, outgoing, verified, state, \
                 last_error, system, entry_id, admission, admission_reason, admission_changed_at, \
                 notify_attempts, next_notify_at, report_refusal, refused, deleted_at, restored, \
                 visible_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, 1, 'pending', NULL, ?9, NULL, \
                 'accepted', NULL, ?5, 0, NULL, 0, NULL, NULL, 0, ?10)",
                params![
                    message_id,
                    conversation_id,
                    author,
                    sender_timestamp_ms,
                    now_ms,
                    content_type,
                    body,
                    signature.as_slice(),
                    if system { 1i64 } else { 0i64 },
                    vseq as i64,
                ],
            )?;
            let rowid = tx.last_insert_rowid();
            if !system
                && is_searchable_content_type(content_type)
                && let Ok(body_str) = str::from_utf8(body)
            {
                tx.execute(
                    "INSERT INTO messages_fts (rowid, body) VALUES (?1, ?2)",
                    params![rowid, body_str],
                )?;
            }

            if !system {
                Self::touch_conversation(tx, conversation_id, now_ms)?;
            }
            txq.enqueue(tx, conversation_id, message_id, &payload, now_ms)?;
            Ok(())
        })?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(any(test, feature = "test-support"))]
    pub fn insert_outgoing_without_enqueue(
        &self,
        conversation_id: &str,
        message_id: &str,
        author: &str,
        sender_timestamp_ms: i64,
        content_type: &str,
        body: &[u8],
        now_ms: i64,
        state: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let vseq = Self::next_visible_seq(&conn, conversation_id)?;
        conn.execute(
            "INSERT INTO messages (id, conversation_id, author, sender_timestamp, received_at, \
             content_type, body, signature, outgoing, verified, state, last_error, system, \
             entry_id, admission, admission_reason, admission_changed_at, notify_attempts, \
             next_notify_at, report_refusal, refused, deleted_at, restored, visible_seq) VALUES \
             (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, 1, ?9, NULL, 0, NULL, 'accepted', NULL, ?5, 0, \
             NULL, 0, NULL, NULL, 0, ?10)",
            params![
                message_id,
                conversation_id,
                author,
                sender_timestamp_ms,
                now_ms,
                content_type,
                body,
                [0u8; 64].as_slice(),
                state,
                vseq as i64,
            ],
        )?;
        let rowid = conn.last_insert_rowid();
        if is_searchable_content_type(content_type)
            && let Ok(body_str) = str::from_utf8(body)
        {
            let _ = conn.execute(
                "INSERT INTO messages_fts (rowid, body) VALUES (?1, ?2)",
                params![rowid, body_str],
            );
        }
        Self::touch_conversation(&conn, conversation_id, now_ms)?;
        Ok(())
    }

    /// Inbound insert-or-ignore: stores incoming message starting as
    /// `undecided` (or `accepted` if system).
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
            "INSERT OR IGNORE INTO conversations (id, kind, peer_address, opened, restored, \
             created_at, last_activity) VALUES (?1, 'direct', ?2, 0, 0, ?3, ?3)",
            params![conversation_id, author, now_ms],
        )?;
        let conv_exists: bool = tx.query_row(
            "SELECT COUNT(*) FROM conversations WHERE id = ?1",
            params![conversation_id],
            |r| Ok(r.get::<_, i64>(0)? > 0),
        )?;
        if !conv_exists {
            tx.execute(
                "INSERT INTO conversations (id, kind, peer_address, opened, restored, created_at, \
                 last_activity) VALUES (?1, 'direct', NULL, 0, 0, ?2, ?2)",
                params![conversation_id, now_ms],
            )?;
        }
        let message_count: u32 = tx.query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1 AND admission != 'dropped'",
            params![conversation_id],
            |r| r.get::<_, i64>(0),
        )? as u32;
        if message_count >= max_messages_per_conversation {
            return Err(StoreError::MessageQuotaExceeded.into());
        }
        let claim_ms = now_ms + (self.config.admission_claim_secs as i64 * 1000);
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO messages (id, conversation_id, author, sender_timestamp, \
             received_at, content_type, body, signature, outgoing, verified, state, last_error, \
             system, entry_id, admission, admission_reason, admission_changed_at, \
             notify_attempts, next_notify_at, report_refusal, refused, deleted_at, restored, \
             visible_seq) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 1, 'delivered', NULL, 0, \
             NULL, 'undecided', NULL, NULL, 0, ?9, 0, NULL, NULL, 0, 0)",
            params![
                message_id,
                conversation_id,
                author,
                sender_timestamp_ms,
                now_ms,
                content_type,
                body,
                signature.as_slice(),
                claim_ms,
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
             body, signature, outgoing, verified, state, last_error, system, entry_id, admission, \
             admission_reason, admission_changed_at, notify_attempts, next_notify_at, \
             report_refusal, refused, deleted_at, restored, visible_seq FROM messages WHERE id = \
             ?1",
            params![id],
            row_to_message,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn apply_admission(
        &self,
        message_id: &str,
        admission: &Admission,
        now_ms: i64,
    ) -> Result<()> {
        let mut conn = self.conn.lock().expect("conversation connection lock poisoned");
        let tx = conn.transaction()?;
        Self::apply_admission_conn(&tx, message_id, admission, now_ms)?;
        tx.commit()?;
        if matches!(admission, Admission::Drop(_)) {
            self.flag_wal_checkpoint();
        }
        Ok(())
    }

    pub fn apply_admission_conn(
        conn: &Connection,
        message_id: &str,
        admission: &Admission,
        now_ms: i64,
    ) -> Result<()> {
        let msg_info: Option<(i64, String, String, Vec<u8>)> = conn
            .query_row(
                "SELECT rowid, conversation_id, content_type, body FROM messages WHERE id = ?1",
                params![message_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((rowid, conv_id, ct, body)) = msg_info else { return Ok(()) };

        match admission {
            Admission::Accept => {
                let vseq = Self::next_visible_seq(conn, &conv_id)?;
                conn.execute(
                    "UPDATE messages SET admission = 'accepted', admission_changed_at = ?1, \
                     visible_seq = ?2 WHERE id = ?3",
                    params![now_ms, vseq as i64, message_id],
                )?;
                Self::touch_conversation(conn, &conv_id, now_ms)?;
                if is_searchable_content_type(&ct)
                    && let Ok(body_str) = str::from_utf8(&body)
                {
                    conn.execute(
                        "INSERT OR IGNORE INTO messages_fts (rowid, body) VALUES (?1, ?2)",
                        params![rowid, body_str],
                    )?;
                }
            }
            Admission::Hold(reason) => {
                conn.execute(
                    "UPDATE messages SET admission = 'held', admission_reason = ?1, \
                     admission_changed_at = ?2 WHERE id = ?3",
                    params![reason, now_ms, message_id],
                )?;
            }
            Admission::Drop(drop_ans) => {
                let is_group = conn
                    .query_row(
                        "SELECT kind FROM conversations WHERE id = ?1",
                        params![conv_id],
                        |r| Ok(r.get::<_, String>(0)? == "group"),
                    )
                    .unwrap_or(false);
                let report = if is_group { false } else { drop_ans.report };
                if is_searchable_content_type(&ct)
                    && let Ok(body_str) = str::from_utf8(&body)
                {
                    let _ = conn.execute(
                        "INSERT INTO messages_fts (messages_fts, rowid, body) VALUES ('delete', \
                         ?1, ?2)",
                        params![rowid, body_str],
                    );
                }
                conn.execute(
                    "UPDATE messages SET admission = 'dropped', admission_reason = ?1, \
                     report_refusal = ?2, admission_changed_at = ?3, body = zeroblob(0) WHERE id \
                     = ?4",
                    params![drop_ans.reason, if report { 1i64 } else { 0i64 }, now_ms, message_id],
                )?;
            }
        }
        Ok(())
    }

    pub fn delete_message(
        &self,
        conversation_id: &str,
        message_id: &str,
        now_ms: i64,
    ) -> Result<bool> {
        let mut conn = self.conn.lock().expect("conversation connection lock poisoned");
        let tx = conn.transaction()?;
        let deleted = Self::delete_message_conn(&tx, conversation_id, message_id, now_ms)?;
        tx.commit()?;
        if deleted {
            self.flag_wal_checkpoint();
        }
        Ok(deleted)
    }

    pub fn delete_message_conn(
        conn: &Connection,
        conversation_id: &str,
        message_id: &str,
        now_ms: i64,
    ) -> Result<bool> {
        let info: Option<(i64, String, Vec<u8>, String, i64, String)> = conn
            .query_row(
                "SELECT rowid, content_type, body, admission, outgoing, state FROM messages WHERE \
                 id = ?1 AND conversation_id = ?2",
                params![message_id, conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        let Some((rowid, ct, body, admission, outgoing, state)) = info else { return Ok(false) };
        // FTS delete only for rows that were indexed (accepted, non-empty body).
        // Issuing 'delete' for a never-indexed row corrupts the FTS table.
        if admission == "accepted"
            && !body.is_empty()
            && is_searchable_content_type(&ct)
            && let Ok(body_str) = str::from_utf8(&body)
        {
            let _ = conn.execute(
                "INSERT INTO messages_fts (messages_fts, rowid, body) VALUES ('delete', ?1, ?2)",
                params![rowid, body_str],
            );
        }
        if outgoing != 0 && state == "pending" {
            let _ = conn.execute(
                "DELETE FROM outbound_envelopes WHERE message_id = ?1",
                params![message_id],
            );
            conn.execute(
                "UPDATE messages SET body = zeroblob(0), deleted_at = ?1, state = 'failed', \
                 last_error = 'deleted before delivery' WHERE id = ?2",
                params![now_ms, message_id],
            )?;
            let _ = conn.execute(
                "UPDATE message_recipients SET state = 'failed', last_error = 'deleted before \
                 delivery' WHERE message_id = ?1",
                params![message_id],
            );
        } else {
            conn.execute(
                "UPDATE messages SET body = zeroblob(0), deleted_at = ?1 WHERE id = ?2",
                params![now_ms, message_id],
            )?;
        }
        Ok(true)
    }

    pub fn handle_inbound_deletion_request(
        conn: &Connection,
        conversation_id: &str,
        author: &str,
        body: &[u8],
        now_ms: i64,
    ) -> Result<bool> {
        let Some(target_id) = parse_deletion_request(body) else {
            return Ok(false);
        };
        let target_info: Option<(String, String)> = conn
            .query_row(
                "SELECT conversation_id, author FROM messages WHERE id = ?1",
                params![target_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((target_conv, target_author)) = target_info
            && target_conv == conversation_id
            && target_author == author
        {
            return Self::delete_message_conn(conn, &target_conv, &target_id, now_ms);
        }
        Ok(false)
    }

    pub fn handle_inbound_refusal_notice(
        conn: &Connection,
        conversation_id: &str,
        author: &str,
        body: &[u8],
    ) -> Result<bool> {
        let Some((target_id, reason)) = parse_refusal_notice(body) else {
            return Ok(false);
        };
        let conv_info: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT kind, peer_address FROM conversations WHERE id = ?1",
                params![conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((kind, direct_peer)) = conv_info
            && kind == "direct"
            && direct_peer.as_deref() == Some(author)
        {
            conn.execute(
                "UPDATE messages SET refused = ?1 WHERE id = ?2 AND conversation_id = ?3 AND \
                 outgoing = 1",
                params![reason, target_id, conversation_id],
            )?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn readmit(&self, conversation_id: &str, reasons: &[String]) -> Result<u32> {
        let mut conn = self.conn.lock().expect("conversation connection lock poisoned");
        let tx = conn.transaction()?;
        let now = now_ms();
        let mut msg_ids = Vec::new();
        if reasons.is_empty() {
            let mut stmt = tx.prepare(
                "SELECT id FROM messages WHERE conversation_id = ?1 AND admission = 'held'",
            )?;
            let rows = stmt.query_map(params![conversation_id], |r| r.get::<_, String>(0))?;
            for r in rows {
                msg_ids.push(r?);
            }
        } else {
            for r in reasons {
                let mut stmt = tx.prepare(
                    "SELECT id FROM messages WHERE conversation_id = ?1 AND admission = 'held' \
                     AND admission_reason = ?2",
                )?;
                let rows =
                    stmt.query_map(params![conversation_id, r], |row| row.get::<_, String>(0))?;
                for row in rows {
                    msg_ids.push(row?);
                }
            }
        }
        // Reset to undecided so the worker re-asks the app, which runs the
        // live block check. Applying Accept here would bypass it.
        for id in &msg_ids {
            tx.execute(
                "UPDATE messages SET admission = 'undecided', admission_reason = NULL, \
                 admission_changed_at = ?1, next_notify_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
        }
        tx.commit()?;
        Ok(msg_ids.len() as u32)
    }

    pub fn record_refusal(
        &self,
        conversation_id: &str,
        message_id: &str,
        peer_address: &str,
        reason: &str,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let conv: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT kind, peer_address FROM conversations WHERE id = ?1",
                params![conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((kind, direct_peer)) = conv else { return Ok(false) };
        if kind != "direct" || direct_peer.as_deref() != Some(peer_address) {
            return Ok(false);
        }
        let affected = conn.execute(
            "UPDATE messages SET refused = ?1 WHERE id = ?2 AND conversation_id = ?3 AND outgoing \
             = 1",
            params![reason, message_id, conversation_id],
        )?;
        Ok(affected > 0)
    }

    pub fn undecided_messages(&self, now_ms: i64) -> Result<Vec<StoredMessage>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, author, sender_timestamp, received_at, content_type, \
             body, signature, outgoing, verified, state, last_error, system, entry_id, admission, \
             admission_reason, admission_changed_at, notify_attempts, next_notify_at, \
             report_refusal, refused, deleted_at, restored, visible_seq FROM messages WHERE \
             admission = 'undecided' AND system = 0 AND (next_notify_at IS NULL OR next_notify_at \
             <= ?1) ORDER BY next_notify_at ASC LIMIT 64",
        )?;
        let mut rows = stmt.query(params![now_ms])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(row_to_message(r)?);
        }
        Ok(out)
    }

    pub fn update_undecided_retry(
        &self,
        message_id: &str,
        attempts: u32,
        next_notify_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.execute(
            "UPDATE messages SET notify_attempts = ?1, next_notify_at = ?2 WHERE id = ?3",
            params![attempts, next_notify_at, message_id],
        )?;
        Ok(())
    }

    pub fn expire_held_messages(
        &self,
        max_held_age_ms: i64,
        now_ms: i64,
    ) -> Result<Vec<(String, String)>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let cutoff = now_ms.saturating_sub(max_held_age_ms);
        let mut stmt = conn.prepare(
            "SELECT conversation_id, id FROM messages WHERE admission = 'held' AND received_at <= \
             ?1",
        )?;
        let mut rows = stmt.query(params![cutoff])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push((r.get(0)?, r.get(1)?));
        }
        Ok(out)
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

    pub fn restart_pending(&self, id: &str, now_ms: i64) -> Result<()> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.execute(
            "UPDATE messages SET state = 'pending', last_error = NULL, received_at = ?1 WHERE id \
             = ?2",
            params![now_ms, id],
        )?;
        Ok(())
    }

    pub fn outbox_messages(&self) -> Result<Vec<StoredMessage>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, conversation_id, author, sender_timestamp, received_at, content_type, \
             body, signature, outgoing, verified, state, last_error, system, entry_id, admission, \
             admission_reason, admission_changed_at, notify_attempts, next_notify_at, \
             report_refusal, refused, deleted_at, restored, visible_seq FROM messages WHERE state \
             IN ('pending', 'failed') AND system = 0 ORDER BY sender_timestamp ASC",
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

pub(crate) fn row_to_message(row: &Row<'_>) -> rusqlite::Result<StoredMessage> {
    let body: Vec<u8> = row.get(6)?;
    let signature_bytes: Vec<u8> = row.get(7)?;
    let signature: [u8; 64] = signature_bytes.as_slice().try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            7,
            Type::Blob,
            Box::new(io::Error::other("signature must be exactly 64 bytes")),
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
        admission: row.get(14)?,
        admission_reason: row.get(15)?,
        admission_changed_at: row.get(16)?,
        notify_attempts: row.get::<_, i64>(17)? as u32,
        next_notify_at: row.get(18)?,
        report_refusal: row.get::<_, i64>(19)? != 0,
        refused: row.get(20)?,
        deleted_at: row.get(21)?,
        restored: row.get::<_, i64>(22)? != 0,
        visible_seq: row.get::<_, i64>(23)? as u64,
    })
}

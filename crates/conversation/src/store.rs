//! `conversation.db`: one file per service, on one shared connection with
//! its own `async_queue::Queue` — what makes the `send` transaction
//! atomic. Every `BLOB` column here is inside a DEK-opened database,
//! matching the rest of the tree's per-service stores.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use syneroym_async_queue::{Queue, QueueConfig};
use syneroym_rpc::{
    ConversationDeliveryState, ConversationKind, ConversationMembershipEvent, ConversationMessage,
};
use zeroize::Zeroizing;

use crate::dag::{EntryKind, MAX_PARENTS, MembershipPayload, WireEntry, canonical_entry_bytes};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("quota exceeded: pending messages cap reached")]
    PendingQuotaExceeded,
    #[error("quota exceeded: max messages per conversation reached")]
    MessageQuotaExceeded,
    #[error("quota exceeded: max dag entries per conversation reached")]
    DagEntryQuotaExceeded,
}

/// Per-conversation and per-service bounds, plus the clock/age
/// bounds `outbox.rs`/`transport.rs` apply. Converted from
/// `AppSandboxRole`'s `conversation_*` fields by the crate's caller
/// (`crates/substrate/src/runtime.rs`), not read from config directly here
/// — this crate does not depend on `syneroym-substrate`.
#[derive(Debug, Clone)]
pub struct ConversationConfig {
    pub max_body_bytes: u32,
    pub max_pending_per_conversation: u32,
    pub max_messages_per_conversation: u32,
    pub max_pending_age_secs: u64,
    pub max_clock_skew_secs: u64,
    pub prekey_requests_per_peer_per_hour: u32,
    pub conversation_group_sync_secs: u64,
    pub conversation_group_rekey_secs: u64,
    pub conversation_max_group_members: u32,
    pub conversation_max_dag_entries_per_conversation: u32,
    pub conversation_max_sync_entries_per_call: u32,
    pub conversation_relay_fanout: u32,
    pub conversation_sync_now_budget_ms: u64,
    pub conversation_background_sync_budget_ms: u64,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            max_body_bytes: 262_144,
            max_pending_per_conversation: 1_000,
            max_messages_per_conversation: 100_000,
            max_pending_age_secs: 2_592_000,
            max_clock_skew_secs: 86_400,
            prekey_requests_per_peer_per_hour: 20,
            conversation_group_sync_secs: 60,
            conversation_group_rekey_secs: 604_800,
            conversation_max_group_members: 256,
            conversation_max_dag_entries_per_conversation: 100_000,
            conversation_max_sync_entries_per_call: 64,
            conversation_relay_fanout: 3,
            conversation_sync_now_budget_ms: 3_000,
            conversation_background_sync_budget_ms: 160_000,
        }
    }
}

/// A row from `messages`, as `store.rs`'s own callers see it -- close to
/// but not identical to `syneroym_rpc::ConversationMessage` (this one
/// carries `signature`/`outgoing`, which are store-internal).
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: String,
    pub conversation_id: String,
    pub author: String,
    pub sender_timestamp_ms: i64,
    pub received_at_ms: i64,
    pub content_type: String,
    pub body: Vec<u8>,
    pub signature: [u8; 64],
    pub outgoing: bool,
    pub verified: bool,
    pub state: ConversationDeliveryState,
    pub last_error: Option<String>,
    pub system: bool,
    pub entry_id: Option<String>,
}

impl StoredMessage {
    #[must_use]
    pub fn into_wire(self) -> ConversationMessage {
        ConversationMessage {
            id: self.id,
            conversation: self.conversation_id,
            author: self.author,
            sender_timestamp: self.sender_timestamp_ms,
            received_at: self.received_at_ms,
            content_type: self.content_type,
            body: self.body,
            state: self.state,
            verified: self.verified,
            last_error: self.last_error,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConversationRow {
    pub id: String,
    pub kind: ConversationKind,
    pub peer_address: Option<String>,
    pub created_at_ms: i64,
    pub last_activity_ms: i64,
    pub owner_address: Option<String>,
    pub current_epoch: u64,
}

#[derive(Debug, Clone)]
pub struct HistoryPage {
    pub messages: Vec<StoredMessage>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredDagEntry {
    pub seq: i64,
    pub entry_id: String,
    pub conversation_id: String,
    pub author: String,
    pub sender_timestamp_ms: i64,
    pub epoch: u64,
    pub kind: EntryKind,
    pub header: Vec<u8>,
    pub ciphertext: Option<Vec<u8>>,
    pub nonce: Option<[u8; 12]>,
    pub payload: Option<MembershipPayload>,
    pub signature: [u8; 64],
    pub applied: bool,
    pub relay_pending: bool,
    pub parents: Vec<String>,
}

impl StoredDagEntry {
    #[must_use]
    pub fn into_wire(self) -> WireEntry {
        WireEntry {
            entry_id: self.entry_id,
            conversation_id: self.conversation_id,
            author: self.author,
            sender_timestamp_ms: self.sender_timestamp_ms,
            epoch: self.epoch,
            kind: self.kind,
            parents: self.parents,
            ciphertext: self.ciphertext,
            nonce: self.nonce,
            payload: self.payload,
            signature: self.signature,
        }
    }
}

/// A session row, as `crypto.rs` persists and reads it. `state` is opaque
/// to this module -- `crypto.rs`'s own encoding.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub peer_address: String,
    pub pinned_sig_key: [u8; 32],
    pub state: Vec<u8>,
}

/// This service's own long-term conversation keys: generated once, on
/// first use, and never derived from the service's ed25519 node identity.
#[derive(Debug, Clone)]
pub struct LocalIdentityRow {
    pub account_state: Zeroizing<Vec<u8>>,
    pub sig_secret: Zeroizing<Vec<u8>>,
}

pub struct ConversationStore {
    conn: Arc<Mutex<Connection>>,
    queue: Queue,
    config: ConversationConfig,
}

impl std::fmt::Debug for ConversationStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationStore").finish_non_exhaustive()
    }
}

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
    pub fn open_encrypted(
        dir: &Path,
        dek: Option<&[u8; 32]>,
        queue_config: QueueConfig,
        config: ConversationConfig,
    ) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let conn = open_connection(&dir.join("conversation.db"), dek)?;
        Self::init_schema(&conn)?;
        let conn = Arc::new(Mutex::new(conn));
        let queue = Queue::from_connection(conn.clone(), queue_config)?;
        Ok(Self { conn, queue, config })
    }

    #[must_use]
    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    #[must_use]
    pub fn config(&self) -> &ConversationConfig {
        &self.config
    }

    pub fn conn(&self) -> &std::sync::Mutex<Connection> {
        &self.conn
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations (
                id            TEXT PRIMARY KEY,
                kind          TEXT NOT NULL,
                peer_address  TEXT,
                owner_address TEXT,
                current_epoch INTEGER NOT NULL DEFAULT 0,
                system        INTEGER NOT NULL DEFAULT 0,
                created_at    INTEGER NOT NULL,
                last_activity INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_conversations_direct_peer
                 ON conversations(peer_address) WHERE kind = 'direct';

             CREATE TABLE IF NOT EXISTS messages (
                id               TEXT PRIMARY KEY,
                conversation_id  TEXT NOT NULL REFERENCES conversations(id),
                author           TEXT NOT NULL,
                sender_timestamp INTEGER NOT NULL,
                received_at      INTEGER NOT NULL,
                content_type     TEXT NOT NULL,
                body             BLOB NOT NULL,
                signature        BLOB NOT NULL,
                outgoing         INTEGER NOT NULL,
                verified         INTEGER NOT NULL,
                state            TEXT NOT NULL,
                last_error       TEXT,
                system           INTEGER NOT NULL DEFAULT 0,
                entry_id         TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_messages_order
                 ON messages(conversation_id, sender_timestamp, author, id);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_dedup ON messages(author, id);
             CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_id);

             CREATE TABLE IF NOT EXISTS sessions (
                peer_address   TEXT PRIMARY KEY,
                pinned_sig_key BLOB NOT NULL,
                state          BLOB NOT NULL,
                updated_at     INTEGER NOT NULL
             );

             CREATE TABLE IF NOT EXISTS local_identity (
                id         INTEGER PRIMARY KEY CHECK (id = 1),
                account_state  BLOB NOT NULL,
                sig_secret BLOB NOT NULL,
                created_at INTEGER NOT NULL
             );

             CREATE TABLE IF NOT EXISTS prekey_requests (
                caller_did    TEXT NOT NULL,
                window_start  INTEGER NOT NULL,
                count         INTEGER NOT NULL,
                PRIMARY KEY (caller_did, window_start)
             );

             CREATE TABLE IF NOT EXISTS group_members (
                conversation_id TEXT NOT NULL REFERENCES conversations(id),
                member_address  TEXT NOT NULL,
                sig_key         BLOB NOT NULL,
                joined_epoch    INTEGER NOT NULL,
                removed_epoch   INTEGER,
                PRIMARY KEY (conversation_id, member_address)
             );

             CREATE TABLE IF NOT EXISTS group_epochs (
                conversation_id TEXT NOT NULL REFERENCES conversations(id),
                epoch           INTEGER NOT NULL,
                key             BLOB NOT NULL,
                created_at      INTEGER NOT NULL,
                PRIMARY KEY (conversation_id, epoch)
             );

             CREATE TABLE IF NOT EXISTS dag_entries (
                seq              INTEGER PRIMARY KEY AUTOINCREMENT,
                entry_id         TEXT NOT NULL UNIQUE,
                conversation_id  TEXT NOT NULL,
                author           TEXT NOT NULL,
                sender_timestamp INTEGER NOT NULL,
                epoch            INTEGER NOT NULL,
                kind             TEXT NOT NULL,
                header           BLOB NOT NULL,
                ciphertext       BLOB,
                nonce            BLOB,
                payload          TEXT,
                signature        BLOB NOT NULL,
                applied          INTEGER NOT NULL DEFAULT 0,
                relay_pending    INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_dag_order
                 ON dag_entries(conversation_id, sender_timestamp, author, entry_id);
             CREATE INDEX IF NOT EXISTS idx_dag_unapplied
                 ON dag_entries(conversation_id, applied);
             CREATE INDEX IF NOT EXISTS idx_dag_relay
                 ON dag_entries(relay_pending);

             CREATE TABLE IF NOT EXISTS dag_parents (
                child_entry_id  TEXT NOT NULL,
                parent_entry_id TEXT NOT NULL,
                PRIMARY KEY (child_entry_id, parent_entry_id)
             );
             CREATE INDEX IF NOT EXISTS idx_dag_parents_parent
                 ON dag_parents(parent_entry_id);

             CREATE TABLE IF NOT EXISTS sync_cursors (
                conversation_id TEXT NOT NULL,
                peer_address    TEXT NOT NULL,
                last_seq        INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL,
                PRIMARY KEY (conversation_id, peer_address)
             );

             CREATE TABLE IF NOT EXISTS message_recipients (
                message_id     TEXT NOT NULL REFERENCES messages(id),
                member_address TEXT NOT NULL,
                state          TEXT NOT NULL,
                last_error     TEXT,
                PRIMARY KEY (message_id, member_address)
             );
             CREATE INDEX IF NOT EXISTS idx_message_recipients_state
                 ON message_recipients(message_id, state);",
        )?;
        Ok(())
    }

    // -- conversations ----------------------------------------------------

    /// Idempotent: returns the existing direct conversation with
    /// `peer_address`, or creates one.
    pub fn get_or_create_direct(
        &self,
        peer_address: &str,
        id: &str,
        now_ms: i64,
    ) -> Result<String> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM conversations WHERE peer_address = ?1 AND kind = 'direct'",
                params![peer_address],
                |r| r.get(0),
            )
            .optional()?;
        let res_id = if let Some(id) = existing {
            id
        } else {
            conn.execute(
                "INSERT INTO conversations (id, kind, peer_address, owner_address, current_epoch, \
                 system, created_at, last_activity)
                 VALUES (?1, 'direct', ?2, NULL, 0, 0, ?3, ?3)
                 ON CONFLICT(peer_address) WHERE kind = 'direct' DO NOTHING",
                params![id, peer_address, now_ms],
            )?;
            conn.query_row(
                "SELECT id FROM conversations WHERE peer_address = ?1 AND kind = 'direct'",
                params![peer_address],
                |r| r.get(0),
            )
            .map_err(|e| anyhow!("failed to read back created conversation: {e}"))?
        };
        conn.execute(
            "UPDATE conversations SET system = 0 WHERE id = ?1 AND system = 1",
            params![res_id],
        )?;
        Ok(res_id)
    }

    pub fn get_conversation(&self, id: &str) -> Result<Option<ConversationRow>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.query_row(
            "SELECT id, kind, peer_address, created_at, last_activity, owner_address, \
             current_epoch FROM conversations WHERE id = ?1",
            params![id],
            |r| {
                let kind_str: String = r.get(1)?;
                Ok(ConversationRow {
                    id: r.get(0)?,
                    kind: if kind_str == "direct" {
                        ConversationKind::Direct
                    } else {
                        ConversationKind::Group
                    },
                    peer_address: r.get(2)?,
                    created_at_ms: r.get(3)?,
                    last_activity_ms: r.get(4)?,
                    owner_address: r.get(5)?,
                    current_epoch: r.get::<_, i64>(6)? as u64,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_conversations(&self) -> Result<Vec<ConversationRow>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, kind, peer_address, created_at, last_activity, owner_address, \
             current_epoch FROM conversations WHERE system = 0 ORDER BY last_activity DESC",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let kind_str: String = row.get(1)?;
            out.push(ConversationRow {
                id: row.get(0)?,
                kind: if kind_str == "direct" {
                    ConversationKind::Direct
                } else {
                    ConversationKind::Group
                },
                peer_address: row.get(2)?,
                created_at_ms: row.get(3)?,
                last_activity_ms: row.get(4)?,
                owner_address: row.get(5)?,
                current_epoch: row.get::<_, i64>(6)? as u64,
            });
        }
        Ok(out)
    }

    pub fn group_conversations(&self) -> Result<Vec<ConversationRow>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, kind, peer_address, created_at, last_activity, owner_address, \
             current_epoch FROM conversations WHERE kind = 'group' ORDER BY last_activity DESC",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(ConversationRow {
                id: row.get(0)?,
                kind: ConversationKind::Group,
                peer_address: row.get(2)?,
                created_at_ms: row.get(3)?,
                last_activity_ms: row.get(4)?,
                owner_address: row.get(5)?,
                current_epoch: row.get::<_, i64>(6)? as u64,
            });
        }
        Ok(out)
    }

    pub fn get_or_create_group_shell(
        tx: &Transaction<'_>,
        group_id: &str,
        owner_address: &str,
        epoch: u64,
        now_ms: i64,
    ) -> Result<ConversationRow> {
        let existing: Option<ConversationRow> = tx
            .query_row(
                "SELECT id, kind, peer_address, created_at, last_activity, owner_address, \
                 current_epoch FROM conversations WHERE id = ?1",
                params![group_id],
                |r| {
                    let kind_str: String = r.get(1)?;
                    let kind = match kind_str.as_str() {
                        "direct" => ConversationKind::Direct,
                        "group" => ConversationKind::Group,
                        other => {
                            return Err(rusqlite::Error::FromSqlConversionFailure(
                                1,
                                rusqlite::types::Type::Text,
                                Box::new(std::io::Error::other(format!("invalid kind {other}"))),
                            ));
                        }
                    };
                    Ok(ConversationRow {
                        id: r.get(0)?,
                        kind,
                        peer_address: r.get(2)?,
                        created_at_ms: r.get(3)?,
                        last_activity_ms: r.get(4)?,
                        owner_address: r.get(5)?,
                        current_epoch: r.get::<_, i64>(6)? as u64,
                    })
                },
            )
            .optional()?;
        if let Some(row) = existing {
            return Ok(row);
        }
        tx.execute(
            "INSERT INTO conversations (id, kind, peer_address, owner_address, current_epoch, \
             system, created_at, last_activity) VALUES (?1, 'group', NULL, ?2, ?3, 0, ?4, ?4)",
            params![group_id, owner_address, epoch as i64, now_ms],
        )?;
        Ok(ConversationRow {
            id: group_id.to_string(),
            kind: ConversationKind::Group,
            peer_address: None,
            created_at_ms: now_ms,
            last_activity_ms: now_ms,
            owner_address: Some(owner_address.to_string()),
            current_epoch: epoch,
        })
    }

    pub(crate) fn touch_conversation(conn_or_tx: &Connection, id: &str, now_ms: i64) -> Result<()> {
        conn_or_tx.execute(
            "UPDATE conversations SET last_activity = ?1 WHERE id = ?2",
            params![now_ms, id],
        )?;
        Ok(())
    }

    // -- messages -----------------------------------------------------------

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

    // -- group & DAG --------------------------------------------------------

    pub fn current_members(&self, conversation_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT member_address FROM group_members
             WHERE conversation_id = ?1 AND removed_epoch IS NULL
             ORDER BY member_address ASC",
        )?;
        let mut rows = stmt.query(params![conversation_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(row.get(0)?);
        }
        Ok(out)
    }

    pub fn member_sig_key(
        &self,
        conversation_id: &str,
        member_address: &str,
    ) -> Result<Option<[u8; 32]>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT sig_key FROM group_members WHERE conversation_id = ?1 AND member_address \
                 = ?2 AND removed_epoch IS NULL",
                params![conversation_id, member_address],
                |r| r.get(0),
            )
            .optional()?;
        match blob {
            Some(b) => {
                let key: [u8; 32] = b.as_slice().try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(std::io::Error::other("sig_key must be exactly 32 bytes")),
                    )
                })?;
                Ok(Some(key))
            }
            None => Ok(None),
        }
    }

    pub fn member_sig_key_at(
        &self,
        conversation_id: &str,
        member_address: &str,
        epoch: u64,
    ) -> Result<Option<[u8; 32]>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT sig_key FROM group_members
                 WHERE conversation_id = ?1 AND member_address = ?2
                   AND joined_epoch <= ?3 AND (removed_epoch IS NULL OR removed_epoch > ?3)",
                params![conversation_id, member_address, epoch as i64],
                |r| r.get(0),
            )
            .optional()?;
        match blob {
            Some(b) => {
                let key: [u8; 32] = b.as_slice().try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(std::io::Error::other("sig_key must be exactly 32 bytes")),
                    )
                })?;
                Ok(Some(key))
            }
            None => Ok(None),
        }
    }

    /// Pins `key` as `member_address`'s signing key for `conversation_id`,
    /// but only while the row still holds the `zeroblob(32)` placeholder —
    /// group-verb trust-on-first-use for a member this service has no 1:1
    /// session with (so no other source for its real key exists yet, short
    /// of waiting for that member's own DAG membership entry to sync).
    /// Returns `false` (no write) if the row is missing, already removed,
    /// or already pinned to a real key — a second, different key presented
    /// later is never silently re-pinned, only refused by the caller's own
    /// signature check against the key already on file.
    pub fn pin_member_sig_key_if_placeholder(
        &self,
        conversation_id: &str,
        member_address: &str,
        key: &[u8; 32],
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let affected = conn.execute(
            "UPDATE group_members SET sig_key = ?3
             WHERE conversation_id = ?1 AND member_address = ?2
               AND removed_epoch IS NULL AND sig_key = zeroblob(32)",
            params![conversation_id, member_address, key.as_slice()],
        )?;
        Ok(affected > 0)
    }

    /// The moment `member_address`'s removal took effect, i.e. when its
    /// `removed_epoch` began — `None` if the member was never removed (or
    /// never joined). A member's last legitimate epoch is the one before
    /// this; the epoch key it holds for that last epoch still verifies
    /// its signature, so `member_sig_key_at` alone cannot fence out a
    /// backdated entry claiming that epoch. Callers combine this with
    /// `sender_timestamp_ms` to make removal a hard cutoff in time, not
    /// only in epoch number.
    pub fn removed_epoch_created_at(
        &self,
        conversation_id: &str,
        member_address: &str,
    ) -> Result<Option<i64>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.query_row(
            "SELECT ge.created_at FROM group_members gm
             JOIN group_epochs ge ON ge.conversation_id = gm.conversation_id
                                  AND ge.epoch = gm.removed_epoch
             WHERE gm.conversation_id = ?1 AND gm.member_address = ?2
               AND gm.removed_epoch IS NOT NULL",
            params![conversation_id, member_address],
            |r| r.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn heads(&self, conversation_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT d.entry_id FROM dag_entries d
             WHERE d.conversation_id = ?1
               AND NOT EXISTS (SELECT 1 FROM dag_parents p WHERE p.parent_entry_id = d.entry_id)
             ORDER BY d.sender_timestamp DESC, d.author DESC, d.entry_id DESC
             LIMIT ?2",
        )?;
        let mut rows = stmt.query(params![conversation_id, MAX_PARENTS as i64])?;
        let mut heads = Vec::new();
        while let Some(row) = rows.next()? {
            heads.push(row.get(0)?);
        }
        heads.sort();
        Ok(heads)
    }

    pub fn epoch_key(&self, conversation_id: &str, epoch: u64) -> Result<Option<[u8; 32]>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let blob: Option<Vec<u8>> = conn
            .query_row(
                "SELECT key FROM group_epochs WHERE conversation_id = ?1 AND epoch = ?2",
                params![conversation_id, epoch as i64],
                |r| r.get(0),
            )
            .optional()?;
        match blob {
            Some(b) => {
                let key: [u8; 32] = b.as_slice().try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(std::io::Error::other("epoch key must be exactly 32 bytes")),
                    )
                })?;
                Ok(Some(key))
            }
            None => Ok(None),
        }
    }

    pub fn epoch_key_in(
        tx: &Transaction<'_>,
        conversation_id: &str,
        epoch: u64,
    ) -> Result<Option<[u8; 32]>> {
        let blob: Option<Vec<u8>> = tx
            .query_row(
                "SELECT key FROM group_epochs WHERE conversation_id = ?1 AND epoch = ?2",
                params![conversation_id, epoch as i64],
                |r| r.get(0),
            )
            .optional()?;
        match blob {
            Some(b) => {
                let key: [u8; 32] = b.as_slice().try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(std::io::Error::other("epoch key must be exactly 32 bytes")),
                    )
                })?;
                Ok(Some(key))
            }
            None => Ok(None),
        }
    }

    pub fn current_epoch_row(&self, conversation_id: &str) -> Result<Option<(u64, i64)>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.query_row(
            "SELECT epoch, created_at FROM group_epochs WHERE conversation_id = ?1 ORDER BY epoch \
             DESC LIMIT 1",
            params![conversation_id],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn current_epoch_in(tx: &Transaction<'_>, conversation_id: &str) -> Result<u64> {
        let epoch: Option<i64> = tx
            .query_row(
                "SELECT MAX(epoch) FROM group_epochs WHERE conversation_id = ?1",
                params![conversation_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(epoch.unwrap_or(0) as u64)
    }

    pub fn dag_entry_count(&self, conversation_id: &str) -> Result<u32> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM dag_entries WHERE conversation_id = ?1",
            params![conversation_id],
            |r| r.get(0),
        )?;
        Ok(count as u32)
    }

    pub fn insert_entry_if_absent(
        tx: &Transaction<'_>,
        conversation_id: &str,
        entry: &WireEntry,
        applied: bool,
        relay_pending: bool,
    ) -> Result<bool> {
        let kind_str = match entry.kind {
            EntryKind::Message => "message",
            EntryKind::Membership => "membership",
        };
        let header = canonical_entry_bytes(entry);
        let payload_json = entry.payload.as_ref().map(serde_json::to_string).transpose()?;
        let nonce_slice = entry.nonce.as_ref().map(|n| n.as_slice());
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO dag_entries (
                entry_id, conversation_id, author, sender_timestamp, epoch, kind, header, \
             ciphertext, nonce, payload, signature, applied, relay_pending
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                entry.entry_id,
                conversation_id,
                entry.author,
                entry.sender_timestamp_ms,
                entry.epoch as i64,
                kind_str,
                header.as_slice(),
                entry.ciphertext.as_deref(),
                nonce_slice,
                payload_json,
                entry.signature.as_slice(),
                if applied { 1i64 } else { 0i64 },
                if relay_pending { 1i64 } else { 0i64 },
            ],
        )?;
        if inserted > 0 {
            for parent in &entry.parents {
                tx.execute(
                    "INSERT OR IGNORE INTO dag_parents (child_entry_id, parent_entry_id) VALUES \
                     (?1, ?2)",
                    params![entry.entry_id, parent],
                )?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn apply_membership(
        tx: &Transaction<'_>,
        conversation_id: &str,
        payload: &MembershipPayload,
    ) -> Result<()> {
        let existing: Option<(i64, Option<i64>, Vec<u8>)> = tx
            .query_row(
                "SELECT joined_epoch, removed_epoch, sig_key FROM group_members WHERE \
                 conversation_id = ?1 AND member_address = ?2",
                params![conversation_id, payload.subject_address],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;

        let new_epoch = payload.new_epoch as i64;
        if payload.action == "add" {
            match existing {
                Some((joined, removed, sig_key)) => {
                    // A row seeded from a `GroupKeyPayload` (see
                    // `peer_deliver_impl`) is a best-effort guess: the payload
                    // carries only member *addresses*, not each bystander's
                    // true join epoch, so a member first learned about
                    // through a later key message can end up placeholder-
                    // seeded at a *later* epoch than it actually joined.
                    // `sig_key` still holding the `zeroblob(32)` placeholder
                    // is exactly the signal that this row was never confirmed
                    // by a real, owner-signed DAG entry — the entry being
                    // applied right now always outranks a guess like that,
                    // regardless of epoch ordering. The ordering guard below
                    // (`new_epoch >= prior_epoch`) is only meaningful between
                    // two *real* entries, guarding against a stale replay.
                    let unconfirmed = sig_key.as_slice() == [0u8; 32].as_slice();
                    let prior_epoch = removed.unwrap_or(joined);
                    if unconfirmed || new_epoch >= prior_epoch {
                        tx.execute(
                            "UPDATE group_members SET sig_key = ?3, joined_epoch = ?4, \
                             removed_epoch = NULL WHERE conversation_id = ?1 AND member_address = \
                             ?2",
                            params![
                                conversation_id,
                                payload.subject_address,
                                payload.subject_sig_key.as_slice(),
                                new_epoch,
                            ],
                        )?;
                    }
                }
                None => {
                    tx.execute(
                        "INSERT INTO group_members (conversation_id, member_address, sig_key, \
                         joined_epoch, removed_epoch) VALUES (?1, ?2, ?3, ?4, NULL)",
                        params![
                            conversation_id,
                            payload.subject_address,
                            payload.subject_sig_key.as_slice(),
                            new_epoch,
                        ],
                    )?;
                }
            }
        } else if payload.action == "remove" {
            match existing {
                Some((joined, removed, sig_key)) => {
                    let unconfirmed = sig_key.as_slice() == [0u8; 32].as_slice();
                    let prior_epoch = removed.unwrap_or(joined);
                    if unconfirmed || new_epoch > prior_epoch {
                        tx.execute(
                            "UPDATE group_members SET removed_epoch = ?3 WHERE conversation_id = \
                             ?1 AND member_address = ?2",
                            params![conversation_id, payload.subject_address, new_epoch,],
                        )?;
                    }
                }
                None => {
                    // Relay push is per-entry and unordered by design, so a `remove`
                    // can arrive before the `add` it removes. Record a tombstone —
                    // joined and removed at the same epoch — rather than dropping
                    // the removal: a later `add` at or before this epoch then loses
                    // the epoch-ordering check below and does not resurrect the
                    // member, while a genuinely later `add` (higher epoch) still
                    // succeeds.
                    tx.execute(
                        "INSERT INTO group_members (conversation_id, member_address, sig_key, \
                         joined_epoch, removed_epoch) VALUES (?1, ?2, ?3, ?4, ?4)",
                        params![
                            conversation_id,
                            payload.subject_address,
                            payload.subject_sig_key.as_slice(),
                            new_epoch,
                        ],
                    )?;
                }
            }
        }
        Ok(())
    }

    pub fn mark_dag_applied(tx: &Transaction<'_>, entry_id: &str) -> Result<()> {
        tx.execute("UPDATE dag_entries SET applied = 1 WHERE entry_id = ?1", params![entry_id])?;
        Ok(())
    }

    pub fn unapplied_dag_entries(&self, conversation_id: &str) -> Result<Vec<StoredDagEntry>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT seq, entry_id, conversation_id, author, sender_timestamp, epoch, kind, \
             header, ciphertext, nonce, payload, signature, applied, relay_pending
             FROM dag_entries WHERE conversation_id = ?1 AND applied = 0 ORDER BY seq ASC",
        )?;
        let mut rows = stmt.query(params![conversation_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(Self::row_to_dag_entry(&conn, row)?);
        }
        Ok(out)
    }

    pub fn membership_history(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationMembershipEvent>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT entry_id, payload, sender_timestamp, author FROM dag_entries
             WHERE conversation_id = ?1 AND kind = 'membership'
             ORDER BY sender_timestamp ASC, author ASC, entry_id ASC",
        )?;
        let mut rows = stmt.query(params![conversation_id])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let entry: String = row.get(0)?;
            let payload_str: Option<String> = row.get(1)?;
            let sender_timestamp: i64 = row.get(2)?;
            if let Some(str) = payload_str
                && let Ok(payload) = serde_json::from_str::<MembershipPayload>(&str)
            {
                out.push(ConversationMembershipEvent {
                    entry,
                    action: payload.action,
                    subject: payload.subject_address,
                    epoch: payload.new_epoch,
                    sender_timestamp,
                });
            }
        }
        Ok(out)
    }

    pub fn entries_after_seq(
        &self,
        conversation_id: &str,
        after_seq: i64,
        limit: u32,
    ) -> Result<Vec<StoredDagEntry>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT seq, entry_id, conversation_id, author, sender_timestamp, epoch, kind, \
             header, ciphertext, nonce, payload, signature, applied, relay_pending
             FROM dag_entries WHERE conversation_id = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT ?3",
        )?;
        let mut rows = stmt.query(params![conversation_id, after_seq, limit as i64])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(Self::row_to_dag_entry(&conn, row)?);
        }
        Ok(out)
    }

    pub fn sync_cursor(&self, conversation_id: &str, peer_address: &str) -> Result<i64> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let cursor: Option<i64> = conn
            .query_row(
                "SELECT last_seq FROM sync_cursors WHERE conversation_id = ?1 AND peer_address = \
                 ?2",
                params![conversation_id, peer_address],
                |r| r.get(0),
            )
            .optional()?;
        Ok(cursor.unwrap_or(0))
    }

    pub fn set_sync_cursor(
        &self,
        conversation_id: &str,
        peer_address: &str,
        last_seq: i64,
        now_ms: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.execute(
            "INSERT INTO sync_cursors (conversation_id, peer_address, last_seq, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(conversation_id, peer_address) DO UPDATE SET last_seq = ?3, updated_at = \
             ?4",
            params![conversation_id, peer_address, last_seq, now_ms],
        )?;
        Ok(())
    }

    pub fn claim_relay_pending(&self, limit: u32) -> Result<Vec<StoredDagEntry>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT seq, entry_id, conversation_id, author, sender_timestamp, epoch, kind, \
             header, ciphertext, nonce, payload, signature, applied, relay_pending
             FROM dag_entries WHERE relay_pending = 1 LIMIT ?1",
        )?;
        let mut rows = stmt.query(params![limit as i64])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            entries.push(Self::row_to_dag_entry(&conn, row)?);
        }
        for entry in &entries {
            conn.execute(
                "UPDATE dag_entries SET relay_pending = 0 WHERE entry_id = ?1",
                params![entry.entry_id],
            )?;
        }
        Ok(entries)
    }

    pub fn wire_entry(&self, entry_id: &str) -> Result<Option<WireEntry>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let entry = conn
            .query_row(
                "SELECT seq, entry_id, conversation_id, author, sender_timestamp, epoch, kind, \
                 header, ciphertext, nonce, payload, signature, applied, relay_pending
                 FROM dag_entries WHERE entry_id = ?1",
                params![entry_id],
                |r| Self::row_to_dag_entry(&conn, r),
            )
            .optional()?;
        Ok(entry.map(|e| e.into_wire()))
    }

    fn row_to_dag_entry(
        conn: &Connection,
        row: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<StoredDagEntry> {
        let entry_id: String = row.get(1)?;
        let kind_str: String = row.get(6)?;
        let kind =
            if kind_str == "membership" { EntryKind::Membership } else { EntryKind::Message };
        let header: Vec<u8> = row.get(7)?;
        let ciphertext: Option<Vec<u8>> = row.get(8)?;
        let nonce_blob: Option<Vec<u8>> = row.get(9)?;
        let nonce = if let Some(nb) = nonce_blob {
            let arr: [u8; 12] = nb.as_slice().try_into().map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    9,
                    rusqlite::types::Type::Blob,
                    Box::new(std::io::Error::other("nonce must be 12 bytes")),
                )
            })?;
            Some(arr)
        } else {
            None
        };
        let payload_str: Option<String> = row.get(10)?;
        let payload = payload_str.and_then(|s| serde_json::from_str::<MembershipPayload>(&s).ok());
        let sig_bytes: Vec<u8> = row.get(11)?;
        let signature: [u8; 64] = sig_bytes.as_slice().try_into().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                11,
                rusqlite::types::Type::Blob,
                Box::new(std::io::Error::other("signature must be 64 bytes")),
            )
        })?;
        let applied: i64 = row.get(12)?;
        let relay_pending: i64 = row.get(13)?;

        let mut p_stmt = conn.prepare(
            "SELECT parent_entry_id FROM dag_parents WHERE child_entry_id = ?1 ORDER BY \
             parent_entry_id ASC",
        )?;
        let mut p_rows = p_stmt.query(params![entry_id])?;
        let mut parents = Vec::new();
        while let Some(pr) = p_rows.next()? {
            parents.push(pr.get(0)?);
        }

        Ok(StoredDagEntry {
            seq: row.get(0)?,
            entry_id,
            conversation_id: row.get(2)?,
            author: row.get(3)?,
            sender_timestamp_ms: row.get(4)?,
            epoch: row.get::<_, i64>(5)? as u64,
            kind,
            header,
            ciphertext,
            nonce,
            payload,
            signature,
            applied: applied != 0,
            relay_pending: relay_pending != 0,
            parents,
        })
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

    // -- sessions -----------------------------------------------------------

    pub fn session(&self, peer_address: &str) -> Result<Option<SessionRow>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.query_row(
            "SELECT peer_address, pinned_sig_key, state FROM sessions WHERE peer_address = ?1",
            params![peer_address],
            |r| {
                let sig_key: Vec<u8> = r.get(1)?;
                // A wrong-length blob means the row is corrupt. Fail
                // loudly: a silently zero-padded key would compare as a
                // partially-zero array, which is incorrect key material.
                let pinned: [u8; 32] = sig_key.as_slice().try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Blob,
                        Box::new(std::io::Error::other("pinned_sig_key must be exactly 32 bytes")),
                    )
                })?;
                Ok(SessionRow { peer_address: r.get(0)?, pinned_sig_key: pinned, state: r.get(2)? })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn upsert_session(&self, row: &SessionRow, now_ms: i64) -> Result<()> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        Self::upsert_session_conn(&conn, row, now_ms)
    }

    pub fn upsert_session_in(
        &self,
        tx: &Transaction<'_>,
        row: &SessionRow,
        now_ms: i64,
    ) -> Result<()> {
        Self::upsert_session_conn(tx, row, now_ms)
    }

    /// `pub(crate)`: `crypto.rs`'s `SessionCrypto::commit_in` calls this
    /// directly, since it has a `&Transaction` but no `&ConversationStore`.
    pub(crate) fn upsert_session_conn(
        conn: &Connection,
        row: &SessionRow,
        now_ms: i64,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO sessions (peer_address, pinned_sig_key, state, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(peer_address) DO UPDATE SET state = ?3, updated_at = ?4",
            params![row.peer_address, row.pinned_sig_key.as_slice(), row.state, now_ms],
        )?;
        Ok(())
    }

    // -- local identity -------------------------------------------------

    /// Loads this service's own conversation identity, generating one on
    /// first use.
    pub fn local_identity_or_generate(
        &self,
        generate: impl FnOnce() -> (Vec<u8>, Vec<u8>),
    ) -> Result<LocalIdentityRow> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let existing: Option<(Vec<u8>, Vec<u8>)> = conn
            .query_row(
                "SELECT account_state, sig_secret FROM local_identity WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (account_state, sig_secret) = match existing {
            Some(pair) => pair,
            None => {
                let (dh, sig) = generate();
                conn.execute(
                    "INSERT INTO local_identity (id, account_state, sig_secret, created_at) \
                     VALUES (1, ?1, ?2, ?3)",
                    params![dh, sig, now_ms()],
                )?;
                (dh, sig)
            }
        };
        Ok(LocalIdentityRow {
            account_state: Zeroizing::new(account_state),
            sig_secret: Zeroizing::new(sig_secret),
        })
    }

    /// Persists a mutated ratchet account (`crypto.rs`'s `save_account`) --
    /// the row a one-time-key consumption or a fresh batch must land in
    /// before anything else can rely on it surviving a restart.
    pub fn save_local_account(&self, account_state: &[u8]) -> Result<()> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.execute(
            "UPDATE local_identity SET account_state = ?1 WHERE id = 1",
            params![account_state],
        )?;
        Ok(())
    }

    // -- prekey rate limiting -------------------------------------------------

    /// Increments this hour's request count for `caller_did` and returns
    /// whether it is still within budget. `window_start` is the request's
    /// own hour bucket. Rows older than 24 hours are pruned on every call
    /// to keep the table bounded for long-running services.
    pub fn record_prekey_request(&self, caller_did: &str, now_ms: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let window_start = now_ms - (now_ms % 3_600_000);
        let cutoff = now_ms - 86_400_000; // 24 hours
        conn.execute("DELETE FROM prekey_requests WHERE window_start < ?1", params![cutoff])?;
        conn.execute(
            "INSERT INTO prekey_requests (caller_did, window_start, count) VALUES (?1, ?2, 1)
             ON CONFLICT(caller_did, window_start) DO UPDATE SET count = count + 1",
            params![caller_did, window_start],
        )?;
        let count: i64 = conn.query_row(
            "SELECT count FROM prekey_requests WHERE caller_did = ?1 AND window_start = ?2",
            params![caller_did, window_start],
            |r| r.get(0),
        )?;
        Ok(count as u32 <= self.config.prekey_requests_per_peer_per_hour)
    }
}

/// The queue payload for one outgoing delivery -- shared between the
/// `send` write (this module) and the outbox worker's read (`outbox.rs`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct OutboxItem {
    pub message_id: String,
    pub peer_address: String,
    #[serde(default)]
    pub group: Option<String>,
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

#[must_use]
pub fn state_str(state: ConversationDeliveryState) -> &'static str {
    match state {
        ConversationDeliveryState::Pending => "pending",
        ConversationDeliveryState::Delivered => "delivered",
        ConversationDeliveryState::Failed => "failed",
    }
}

fn state_from_str(s: &str) -> ConversationDeliveryState {
    match s {
        "delivered" => ConversationDeliveryState::Delivered,
        "failed" => ConversationDeliveryState::Failed,
        _ => ConversationDeliveryState::Pending,
    }
}

#[must_use]
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Opens (creating on first use) a WAL-mode SQLite connection, applying
/// `PRAGMA key` before anything else touches the file when `dek` is
/// present -- mirrors `syneroym-async-queue`'s own `open_connection`
/// exactly, duplicated rather than shared since that one is private to its
/// crate.
fn open_connection(path: &Path, dek: Option<&[u8; 32]>) -> Result<Connection> {
    let conn = Connection::open(path)?;
    if let Some(dek) = dek {
        let pragma = Zeroizing::new(format!("x'{}'", hex::encode(dek)));
        conn.pragma_update(None, "key", &*pragma)?;
    }
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use syneroym_core::config::RetryPolicy;

    use super::*;

    fn store() -> ConversationStore {
        let dir = tempfile::tempdir().unwrap();
        // Leak the tempdir so the file lives for the test's duration; each
        // test gets its own directory so this is bounded.
        let path = Box::leak(Box::new(dir)).path();
        ConversationStore::open_encrypted(
            path,
            None,
            QueueConfig {
                retry: RetryPolicy {
                    max_attempts: 5,
                    initial_backoff_ms: 10,
                    backoff_multiplier: 2.0,
                    max_backoff_ms: 1000,
                },
                visibility_timeout_ms: 5000,
                dlq_max_rows: 100,
                max_pending_rows: 1000,
            },
            ConversationConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn get_or_create_direct_is_idempotent() {
        let s = store();
        let id1 = s.get_or_create_direct("did:key:zPeer", "conv:precomputed", 1_000).unwrap();
        let id2 = s.get_or_create_direct("did:key:zPeer", "conv:precomputed-again", 2_000).unwrap();
        assert_eq!(id1, id2, "a second open-direct for the same peer must return the same id");
    }

    /// The send transaction is atomic: an injected failure between the two
    /// writes leaves neither behind. Simulated here by a closure that
    /// writes the message row then deliberately errors before enqueueing.
    #[test]
    fn the_send_transaction_is_atomic_under_injected_failure() {
        let s = store();
        let conv_id = s.get_or_create_direct("did:key:zPeer", "conv:1", 1_000).unwrap();
        let result = s.queue.transaction(|tx, _txq| {
            tx.execute(
                "INSERT INTO messages (id, conversation_id, author, sender_timestamp, \
                 received_at, content_type, body, signature, outgoing, verified, state, \
                 last_error) VALUES ('msg:1', ?1, 'a', 0, 0, 'text', X'00', X'00', 1, 1, \
                 'pending', NULL)",
                params![conv_id],
            )?;
            Err::<(), _>(anyhow!("simulated failure before enqueue"))
        });
        assert!(result.is_err());
        assert!(s.get_message("msg:1").unwrap().is_none(), "the message row must have rolled back");
        assert!(s.queue.all().unwrap().is_empty(), "no enqueue must have landed either");
    }

    #[test]
    fn the_author_id_index_rejects_a_repeat() {
        let s = store();
        let author = "did:key:zPeer";
        let conv_id = crate::ids::derive_conversation_id("did:key:zMe", author);
        // First delivery: a genuine insert.
        let first = {
            let conn = s.conn.lock().unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            let inserted = s
                .insert_incoming_if_absent(
                    &tx,
                    &conv_id,
                    "msg:1",
                    author,
                    1_000,
                    "text/plain",
                    b"hi",
                    &[0u8; 64],
                    1_000,
                    100,
                )
                .unwrap();
            tx.commit().unwrap();
            inserted
        };
        assert!(first, "the first delivery of (author, id) must insert");

        // A second insert for the exact same (author, id) must not create
        // a second row -- proven directly against the underlying
        // constraint via the incoming-insert path's INSERT OR IGNORE.
        let second = {
            let conn = s.conn.lock().unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            let inserted = s
                .insert_incoming_if_absent(
                    &tx,
                    &conv_id,
                    "msg:1",
                    author,
                    1_000,
                    "text/plain",
                    b"hi",
                    &[0u8; 64],
                    2_000,
                    100,
                )
                .unwrap();
            tx.commit().unwrap();
            inserted
        };
        assert!(!second, "a repeat (author, id) must be ignored, not error");
    }

    #[test]
    fn exhausting_one_conversation_quota_leaves_another_conversation_unaffected() {
        let s = store();
        let conv_1 = "conv:1";
        let conv_2 = "conv:2";
        let author_1 = "did:key:zA";
        let author_2 = "did:key:zB";

        // Insert 2 messages into conv_1 with a cap of 2.
        {
            let conn = s.conn.lock().unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            s.insert_incoming_if_absent(
                &tx,
                conv_1,
                "msg:1",
                author_1,
                1_000,
                "text/plain",
                b"1",
                &[0u8; 64],
                1_000,
                2,
            )
            .unwrap();
            s.insert_incoming_if_absent(
                &tx,
                conv_1,
                "msg:2",
                author_1,
                1_001,
                "text/plain",
                b"2",
                &[0u8; 64],
                1_001,
                2,
            )
            .unwrap();
            tx.commit().unwrap();
        }

        // 3rd message into conv_1 fails with quota exceeded.
        {
            let conn = s.conn.lock().unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            let res = s.insert_incoming_if_absent(
                &tx,
                conv_1,
                "msg:3",
                author_1,
                1_002,
                "text/plain",
                b"3",
                &[0u8; 64],
                1_002,
                2,
            );
            assert!(res.is_err(), "exceeding max_messages_per_conversation must error");
        }

        // conv_2 is unaffected and accepts messages.
        {
            let conn = s.conn.lock().unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            let res = s.insert_incoming_if_absent(
                &tx,
                conv_2,
                "msg:c2_1",
                author_2,
                1_000,
                "text/plain",
                b"hello",
                &[0u8; 64],
                1_000,
                2,
            );
            assert!(res.is_ok(), "other conversation must not be affected by conv_1's exhaustion");
            tx.commit().unwrap();
        }
    }

    #[test]
    fn history_returns_the_documented_order_under_a_skewed_clock() {
        let s = store();
        let conv_id = s.get_or_create_direct("did:key:zPeer", "conv:1", 1_000).unwrap();
        // Out-of-order timestamps, as a skewed-clock sender would produce.
        for (id, ts, author) in [
            ("msg:b", 500, "did:key:zA"),
            ("msg:a", 500, "did:key:zA"),
            ("msg:c", 100, "did:key:zA"),
        ] {
            s.insert_outgoing_and_enqueue(
                &conv_id,
                id,
                author,
                ts,
                "text/plain",
                b"x",
                &[0u8; 64],
                "did:key:zPeer",
                ts,
                false,
            )
            .unwrap();
        }
        let page = s.history(&conv_id, 10, None).unwrap();
        let ids: Vec<&str> = page.messages.iter().map(|m| m.id.as_str()).collect();
        // (sender_timestamp, author, id): msg:c (100) first, then msg:a
        // before msg:b at the same timestamp (id tiebreak).
        assert_eq!(ids, vec!["msg:c", "msg:a", "msg:b"]);
    }

    #[test]
    fn history_pages_and_reports_a_next_cursor() {
        let s = store();
        let conv_id = s.get_or_create_direct("did:key:zPeer", "conv:1", 1_000).unwrap();
        for i in 0..5 {
            s.insert_outgoing_and_enqueue(
                &conv_id,
                &format!("msg:{i}"),
                "did:key:zA",
                i,
                "text/plain",
                b"x",
                &[0u8; 64],
                "did:key:zPeer",
                i,
                false,
            )
            .unwrap();
        }
        let page1 = s.history(&conv_id, 2, None).unwrap();
        assert_eq!(page1.messages.len(), 2);
        assert_eq!(page1.messages[0].id, "msg:0");
        assert_eq!(page1.messages[1].id, "msg:1");
        assert!(page1.next_cursor.is_some());

        let page2 = s.history(&conv_id, 2, page1.next_cursor.as_deref()).unwrap();
        assert_eq!(page2.messages[0].id, "msg:2");
        assert_eq!(page2.messages[1].id, "msg:3");
    }

    #[test]
    fn local_identity_is_generated_once_and_persists() {
        let s = store();
        let first = s.local_identity_or_generate(|| (vec![1, 2, 3], vec![4, 5, 6])).unwrap();
        let second = s.local_identity_or_generate(|| (vec![9, 9, 9], vec![9, 9, 9])).unwrap();
        assert_eq!(
            &*first.account_state, &*second.account_state,
            "must not regenerate on a second call"
        );
        assert_eq!(&*first.sig_secret, &*second.sig_secret);
    }

    #[test]
    fn prekey_rate_limit_refuses_past_the_configured_ceiling() {
        let cfg = ConversationConfig { prekey_requests_per_peer_per_hour: 2, ..Default::default() };
        let dir = tempfile::tempdir().unwrap();
        let path = Box::leak(Box::new(dir)).path();
        let s = ConversationStore::open_encrypted(
            path,
            None,
            QueueConfig {
                retry: RetryPolicy {
                    max_attempts: 5,
                    initial_backoff_ms: 10,
                    backoff_multiplier: 2.0,
                    max_backoff_ms: 1000,
                },
                visibility_timeout_ms: 5000,
                dlq_max_rows: 100,
                max_pending_rows: 1000,
            },
            cfg,
        )
        .unwrap();
        let now = 1_000_000;
        assert!(s.record_prekey_request("did:key:zPeer", now).unwrap());
        assert!(s.record_prekey_request("did:key:zPeer", now + 1).unwrap());
        assert!(
            !s.record_prekey_request("did:key:zPeer", now + 2).unwrap(),
            "the third request in the same hour must be refused"
        );
    }

    #[test]
    fn heads_are_the_entries_with_no_child() {
        let s = store();
        let conv_id = "conv:g1";
        // Create conversation
        {
            let conn = s.conn.lock().unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 1, 1000)
                .unwrap();
            let entry1 = WireEntry {
                entry_id: "ent:1".to_string(),
                conversation_id: conv_id.to_string(),
                author: "svc:owner".to_string(),
                sender_timestamp_ms: 1000,
                epoch: 1,
                kind: EntryKind::Message,
                parents: vec![],
                ciphertext: Some(vec![1]),
                nonce: Some([0u8; 12]),
                payload: None,
                signature: [0u8; 64],
            };
            ConversationStore::insert_entry_if_absent(&tx, conv_id, &entry1, true, false).unwrap();

            let entry2 = WireEntry {
                entry_id: "ent:2".to_string(),
                conversation_id: conv_id.to_string(),
                author: "svc:owner".to_string(),
                sender_timestamp_ms: 1001,
                epoch: 1,
                kind: EntryKind::Message,
                parents: vec!["ent:1".to_string()],
                ciphertext: Some(vec![2]),
                nonce: Some([0u8; 12]),
                payload: None,
                signature: [0u8; 64],
            };
            ConversationStore::insert_entry_if_absent(&tx, conv_id, &entry2, true, false).unwrap();
            tx.commit().unwrap();
        }

        let heads = s.heads(conv_id).unwrap();
        assert_eq!(heads, vec!["ent:2"]);
    }

    #[test]
    fn the_sync_cursor_never_skips_an_entry_inserted_out_of_timestamp_order() {
        let s = store();
        let conv_id = "conv:g1";
        {
            let conn = s.conn.lock().unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 1, 1000)
                .unwrap();
            for (id, ts) in [("ent:3", 3000), ("ent:2", 2000), ("ent:1", 1000)] {
                let entry = WireEntry {
                    entry_id: id.to_string(),
                    conversation_id: conv_id.to_string(),
                    author: "svc:owner".to_string(),
                    sender_timestamp_ms: ts,
                    epoch: 1,
                    kind: EntryKind::Message,
                    parents: vec![],
                    ciphertext: Some(vec![1]),
                    nonce: Some([0u8; 12]),
                    payload: None,
                    signature: [0u8; 64],
                };
                ConversationStore::insert_entry_if_absent(&tx, conv_id, &entry, true, false)
                    .unwrap();
            }
            tx.commit().unwrap();
        }

        let entries = s.entries_after_seq(conv_id, 0, 10).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].entry_id, "ent:3");
        assert_eq!(entries[1].entry_id, "ent:2");
        assert_eq!(entries[2].entry_id, "ent:1");
    }

    #[test]
    fn dag_entry_quota_is_per_conversation() {
        let s = store();
        let conv1 = "conv:g1";
        let conv2 = "conv:g2";
        {
            let conn = s.conn.lock().unwrap();
            let tx = conn.unchecked_transaction().unwrap();
            ConversationStore::get_or_create_group_shell(&tx, conv1, "svc:owner", 1, 1000).unwrap();
            ConversationStore::get_or_create_group_shell(&tx, conv2, "svc:owner", 1, 1000).unwrap();

            let entry1 = WireEntry {
                entry_id: "ent:1".to_string(),
                conversation_id: conv1.to_string(),
                author: "svc:owner".to_string(),
                sender_timestamp_ms: 1000,
                epoch: 1,
                kind: EntryKind::Message,
                parents: vec![],
                ciphertext: Some(vec![1]),
                nonce: Some([0u8; 12]),
                payload: None,
                signature: [0u8; 64],
            };
            ConversationStore::insert_entry_if_absent(&tx, conv1, &entry1, true, false).unwrap();
            tx.commit().unwrap();
        }

        assert_eq!(s.dag_entry_count(conv1).unwrap(), 1);
        assert_eq!(s.dag_entry_count(conv2).unwrap(), 0);
    }

    #[test]
    fn recipients_remaining_reaches_zero_only_when_every_member_settles() {
        let s = store();
        let conv_id = s.get_or_create_direct("did:key:zPeer", "conv:1", 1000).unwrap();
        let msg_id = "msg:1";
        {
            let conn = s.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO messages (id, conversation_id, author, sender_timestamp, \
                 received_at, content_type, body, signature, outgoing, verified, state, \
                 last_error, system, entry_id)
                 VALUES (?1, ?2, 'svc:me', 1000, 1000, 'text/plain', X'00', X'00', 1, 1, \
                 'pending', NULL, 0, ?1)",
                params![msg_id, conv_id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO message_recipients (message_id, member_address, state, last_error) \
                 VALUES (?1, 'peer1', 'pending', NULL)",
                params![msg_id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO message_recipients (message_id, member_address, state, last_error) \
                 VALUES (?1, 'peer2', 'pending', NULL)",
                params![msg_id],
            )
            .unwrap();
        }

        assert_eq!(s.recipients_remaining(msg_id).unwrap(), 2);
        assert!(!s.any_recipient_failed(msg_id).unwrap());

        s.set_recipient_state(msg_id, "peer1", ConversationDeliveryState::Delivered, None).unwrap();
        assert_eq!(s.recipients_remaining(msg_id).unwrap(), 1);

        s.set_recipient_state(msg_id, "peer2", ConversationDeliveryState::Delivered, None).unwrap();
        assert_eq!(s.recipients_remaining(msg_id).unwrap(), 0);
    }

    #[test]
    fn history_and_outbox_exclude_system_messages() {
        let s = store();
        let conv_id = s.get_or_create_direct("did:key:zPeer", "conv:1", 1000).unwrap();
        // Insert one regular message and one system message
        s.insert_outgoing_and_enqueue(
            &conv_id,
            "msg:regular",
            "svc:me",
            1000,
            "text/plain",
            b"hello",
            &[0u8; 64],
            "did:key:zPeer",
            1000,
            false,
        )
        .unwrap();
        s.insert_outgoing_and_enqueue(
            &conv_id,
            "msg:system",
            "svc:me",
            1001,
            "application/vnd.syneroym.group-key+json",
            b"{}",
            &[0u8; 64],
            "did:key:zPeer",
            1001,
            true,
        )
        .unwrap();

        let hist = s.history(&conv_id, 10, None).unwrap();
        assert_eq!(hist.messages.len(), 1);
        assert_eq!(hist.messages[0].id, "msg:regular");

        let outbox = s.outbox_messages().unwrap();
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].id, "msg:regular");
    }

    #[test]
    fn list_conversations_excludes_system_conversations() {
        let s = store();
        let conv_id = "conv:sys";
        {
            let conn = s.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO conversations (id, kind, peer_address, owner_address, current_epoch, \
                 system, created_at, last_activity)
                 VALUES (?1, 'direct', 'peer_sys', NULL, 0, 1, 1000, 1000)",
                params![conv_id],
            )
            .unwrap();
        }
        let list = s.list_conversations().unwrap();
        assert!(list.is_empty(), "system conversation must be excluded from list_conversations");
    }

    #[test]
    fn get_or_create_direct_clears_the_system_flag_on_an_existing_row() {
        let s = store();
        let peer = "peer_sys";
        let conv_id = "conv:sys";
        {
            let conn = s.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO conversations (id, kind, peer_address, owner_address, current_epoch, \
                 system, created_at, last_activity)
                 VALUES (?1, 'direct', ?2, NULL, 0, 1, 1000, 1000)",
                params![conv_id, peer],
            )
            .unwrap();
        }

        // Verify initially excluded
        assert!(s.list_conversations().unwrap().is_empty());

        // Now open_direct on the same peer
        let returned_id = s.get_or_create_direct(peer, "conv:ignored", 2000).unwrap();
        assert_eq!(returned_id, conv_id);

        let list = s.list_conversations().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, conv_id);
    }
}

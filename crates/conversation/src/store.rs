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
use syneroym_rpc::{ConversationDeliveryState, ConversationKind, ConversationMessage};
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("quota exceeded: pending messages cap reached")]
    PendingQuotaExceeded,
    #[error("quota exceeded: max messages per conversation reached")]
    MessageQuotaExceeded,
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
}

#[derive(Debug, Clone)]
pub struct HistoryPage {
    pub messages: Vec<StoredMessage>,
    pub next_cursor: Option<String>,
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

    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations (
                id            TEXT PRIMARY KEY,
                kind          TEXT NOT NULL,
                peer_address  TEXT,
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
                last_error       TEXT
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
             );",
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
        if let Some(id) = existing {
            return Ok(id);
        }
        conn.execute(
            "INSERT INTO conversations (id, kind, peer_address, created_at, last_activity)
             VALUES (?1, 'direct', ?2, ?3, ?3)
             ON CONFLICT(peer_address) WHERE kind = 'direct' DO NOTHING",
            params![id, peer_address, now_ms],
        )?;
        conn.query_row(
            "SELECT id FROM conversations WHERE peer_address = ?1 AND kind = 'direct'",
            params![peer_address],
            |r| r.get(0),
        )
        .map_err(|e| anyhow!("failed to read back created conversation: {e}"))
    }

    pub fn get_conversation(&self, id: &str) -> Result<Option<ConversationRow>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        conn.query_row(
            "SELECT id, kind, peer_address, created_at, last_activity FROM conversations WHERE id \
             = ?1",
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
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_conversations(&self) -> Result<Vec<ConversationRow>> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, kind, peer_address, created_at, last_activity FROM conversations ORDER BY \
             last_activity DESC",
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
            });
        }
        Ok(out)
    }

    fn touch_conversation(conn_or_tx: &Connection, id: &str, now_ms: i64) -> Result<()> {
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
    ) -> Result<()> {
        let payload = serde_json::to_vec(&OutboxItem {
            message_id: message_id.to_string(),
            peer_address: peer_address.to_string(),
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
                 last_error)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, 1, 1, 'pending', NULL)",
                params![
                    message_id,
                    conversation_id,
                    author,
                    sender_timestamp_ms,
                    content_type,
                    body,
                    signature.as_slice()
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
             received_at, content_type, body, signature, outgoing, verified, state, last_error) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 1, 'delivered', NULL)",
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
             body, signature, outgoing, verified, state, last_error FROM messages WHERE id = ?1",
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
             body, signature, outgoing, verified, state, last_error FROM messages
             WHERE conversation_id = ?1
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
             body, signature, outgoing, verified, state, last_error FROM messages
             WHERE state IN ('pending', 'failed') ORDER BY sender_timestamp ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(row_to_message(row)?);
        }
        Ok(out)
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
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct OutboxItem {
    pub message_id: String,
    pub peer_address: String,
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
}

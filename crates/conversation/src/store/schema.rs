use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use rusqlite::Connection;
use syneroym_async_queue::{Queue, QueueConfig};
use zeroize::Zeroizing;

use super::{ConversationConfig, ConversationStore};

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
                epoch_confirmed INTEGER NOT NULL DEFAULT 0,
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

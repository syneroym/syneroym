//! Backup export and restore for conversation capability data.

use std::str;

use anyhow::Result;
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};
use syneroym_rpc::ConversationExportChunk;

use super::{ConversationStore, StoredDagEntry, message::is_searchable_content_type};
use crate::dag::WireEntry;

pub const BACKUP_VERSION: u32 = 1;
const EXPORT_PAGE_SIZE: i64 = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConversation {
    pub id: String,
    pub kind: String,
    pub peer_address: Option<String>,
    pub owner_address: Option<String>,
    pub current_epoch: u64,
    pub name: Option<String>,
    pub created_at: i64,
    pub last_activity: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMessage {
    pub id: String,
    pub conversation_id: String,
    pub author: String,
    pub sender_timestamp: i64,
    pub received_at: i64,
    pub content_type: String,
    pub body: Vec<u8>,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub signature: [u8; 64],
    pub outgoing: bool,
    pub state: String,
    pub last_error: Option<String>,
    pub system: bool,
    pub entry_id: Option<String>,
    pub admission: String,
    pub admission_reason: Option<String>,
    pub deleted_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupGroupMember {
    pub conversation_id: String,
    pub member_address: String,
    #[serde(with = "crate::wire::fixed_bytes")]
    pub sig_key: [u8; 32],
    pub joined_epoch: u64,
    pub removed_epoch: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupBundle {
    pub version: u32,
    pub conversations: Vec<BackupConversation>,
    pub messages: Vec<BackupMessage>,
    pub dag_entries: Vec<WireEntry>,
    pub group_members: Vec<BackupGroupMember>,
}

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
    pub fn export_history(&self, cursor: Option<String>) -> Result<ConversationExportChunk> {
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let offset: i64 = match cursor.as_deref() {
            Some(c) => {
                c.parse::<i64>().map_err(|_| anyhow::anyhow!("invalid export cursor: {c}"))?
            }
            None => 0,
        };

        let (conversations, dag_entries, group_members) = if offset == 0 {
            (
                Self::export_conversations(&conn)?,
                Self::export_dag_entries(&conn)?,
                Self::export_group_members(&conn)?,
            )
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };

        let (messages, next_cursor) = Self::export_messages_page(&conn, offset)?;

        let bundle = BackupBundle {
            version: BACKUP_VERSION,
            conversations,
            messages,
            dag_entries,
            group_members,
        };
        let data = serde_json::to_vec(&bundle)?;
        Ok(ConversationExportChunk { data, next_cursor })
    }

    fn export_conversations(conn: &Connection) -> Result<Vec<BackupConversation>> {
        let mut stmt = conn.prepare(
            "SELECT id, kind, peer_address, owner_address, current_epoch, name, created_at, \
             last_activity FROM conversations WHERE system = 0",
        )?;
        let mut rows = stmt.query([])?;
        let mut convs = Vec::new();
        while let Some(r) = rows.next()? {
            convs.push(BackupConversation {
                id: r.get(0)?,
                kind: r.get(1)?,
                peer_address: r.get(2)?,
                owner_address: r.get(3)?,
                current_epoch: r.get::<_, i64>(4)? as u64,
                name: r.get(5)?,
                created_at: r.get(6)?,
                last_activity: r.get(7)?,
            });
        }
        Ok(convs)
    }

    fn export_dag_entries(conn: &Connection) -> Result<Vec<WireEntry>> {
        let mut stmt = conn.prepare(
            "SELECT seq, entry_id, conversation_id, author, sender_timestamp, epoch, kind, \
             header, ciphertext, nonce, payload, signature, applied, relay_pending FROM \
             dag_entries ORDER BY seq ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut dags = Vec::new();
        while let Some(r) = rows.next()? {
            let entry: StoredDagEntry = Self::row_to_dag_entry(conn, r)?;
            dags.push(entry.into_wire());
        }
        Ok(dags)
    }

    fn export_group_members(conn: &Connection) -> Result<Vec<BackupGroupMember>> {
        let mut stmt = conn.prepare(
            "SELECT conversation_id, member_address, sig_key, joined_epoch, removed_epoch FROM \
             group_members",
        )?;
        let mut rows = stmt.query([])?;
        let mut members = Vec::new();
        while let Some(r) = rows.next()? {
            let sig_blob: Vec<u8> = r.get(2)?;
            let sig_key: [u8; 32] = sig_blob.as_slice().try_into().unwrap_or([0u8; 32]);
            let rem: Option<i64> = r.get(4)?;
            members.push(BackupGroupMember {
                conversation_id: r.get(0)?,
                member_address: r.get(1)?,
                sig_key,
                joined_epoch: r.get::<_, i64>(3)? as u64,
                removed_epoch: rem.map(|e| e as u64),
            });
        }
        Ok(members)
    }

    fn export_messages_page(
        conn: &Connection,
        offset: i64,
    ) -> Result<(Vec<BackupMessage>, Option<String>)> {
        let mut m_stmt = conn.prepare(
            "SELECT id, conversation_id, author, sender_timestamp, received_at, content_type, \
             body, signature, outgoing, state, last_error, system, entry_id, admission, \
             admission_reason, deleted_at FROM messages WHERE system = 0 ORDER BY \
             sender_timestamp ASC, author ASC, id ASC LIMIT ?1 OFFSET ?2",
        )?;
        let mut m_rows = m_stmt.query(params![EXPORT_PAGE_SIZE + 1, offset])?;
        let mut messages = Vec::new();
        while let Some(r) = m_rows.next()? {
            let sig_blob: Vec<u8> = r.get(7)?;
            let signature: [u8; 64] = sig_blob.as_slice().try_into().unwrap_or([0u8; 64]);
            messages.push(BackupMessage {
                id: r.get(0)?,
                conversation_id: r.get(1)?,
                author: r.get(2)?,
                sender_timestamp: r.get(3)?,
                received_at: r.get(4)?,
                content_type: r.get(5)?,
                body: r.get(6)?,
                signature,
                outgoing: r.get::<_, i64>(8)? != 0,
                state: r.get(9)?,
                last_error: r.get(10)?,
                system: r.get::<_, i64>(11)? != 0,
                entry_id: r.get(12)?,
                admission: r.get(13)?,
                admission_reason: r.get(14)?,
                deleted_at: r.get(15)?,
            });
        }

        let next_cursor = if messages.len() as i64 > EXPORT_PAGE_SIZE {
            messages.pop();
            Some((offset + EXPORT_PAGE_SIZE).to_string())
        } else {
            None
        };
        Ok((messages, next_cursor))
    }

    pub fn import_history(&self, data: &[u8]) -> Result<u32> {
        let bundle: BackupBundle = serde_json::from_slice(data)?;
        if bundle.version != BACKUP_VERSION {
            anyhow::bail!("unsupported backup version: {}", bundle.version);
        }
        let mut conn = self.conn.lock().expect("conversation connection lock poisoned");
        let tx = conn.transaction()?;

        Self::import_conversations(&tx, bundle.conversations)?;
        Self::import_group_members(&tx, bundle.group_members)?;

        for de in bundle.dag_entries {
            let is_live: bool = tx
                .query_row(
                    "SELECT COUNT(*) FROM group_epochs WHERE conversation_id = ?1",
                    params![de.conversation_id],
                    |r| r.get::<_, i64>(0),
                )
                .map(|c| c > 0)
                .unwrap_or(false);
            if !is_live {
                Self::insert_entry_if_absent(&tx, &de.conversation_id, &de, true, false)?;
            }
        }

        let count = Self::import_messages(&tx, bundle.messages)?;
        tx.commit()?;
        Ok(count)
    }

    fn import_conversations(tx: &Transaction<'_>, convs: Vec<BackupConversation>) -> Result<()> {
        for c in convs {
            let is_group = c.kind == "group";
            let restored = if is_group {
                let has_keys: bool = tx
                    .query_row(
                        "SELECT COUNT(*) FROM group_epochs WHERE conversation_id = ?1",
                        params![c.id],
                        |r| r.get::<_, i64>(0),
                    )
                    .map(|count| count > 0)
                    .unwrap_or(false);
                !has_keys
            } else {
                // For direct chats: mark restored only when the id is new
                // on this node. A same-address restore produces the same id,
                // so the chat stays live (restored=0) and open_direct works.
                let id_exists: bool = tx
                    .query_row(
                        "SELECT COUNT(*) FROM conversations WHERE id = ?1",
                        params![c.id],
                        |r| r.get::<_, i64>(0),
                    )
                    .map(|count| count > 0)
                    .unwrap_or(false);
                !id_exists
            };

            tx.execute(
                "INSERT INTO conversations (id, kind, peer_address, owner_address, current_epoch, \
                 system, opened, restored, name, created_at, last_activity) VALUES (?1, ?2, ?3, \
                 ?4, ?5, 0, 0, ?6, ?7, ?8, ?9) ON CONFLICT(id) DO UPDATE SET restored = \
                 MIN(conversations.restored, ?6), name = COALESCE(conversations.name, \
                 excluded.name)",
                params![
                    c.id,
                    c.kind,
                    c.peer_address,
                    c.owner_address,
                    c.current_epoch as i64,
                    if restored { 1i64 } else { 0i64 },
                    c.name,
                    c.created_at,
                    c.last_activity,
                ],
            )?;
        }
        Ok(())
    }

    fn import_group_members(tx: &Transaction<'_>, members: Vec<BackupGroupMember>) -> Result<()> {
        for gm in members {
            let is_live: bool = tx
                .query_row(
                    "SELECT COUNT(*) FROM group_epochs WHERE conversation_id = ?1",
                    params![gm.conversation_id],
                    |r| r.get::<_, i64>(0),
                )
                .map(|count| count > 0)
                .unwrap_or(false);
            if is_live {
                continue;
            }
            tx.execute(
                "INSERT OR IGNORE INTO group_members (conversation_id, member_address, sig_key, \
                 joined_epoch, removed_epoch, epoch_confirmed) VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                params![
                    gm.conversation_id,
                    gm.member_address,
                    gm.sig_key.as_slice(),
                    gm.joined_epoch as i64,
                    gm.removed_epoch.map(|e| e as i64),
                ],
            )?;
        }
        Ok(())
    }

    fn import_messages(tx: &Transaction<'_>, messages: Vec<BackupMessage>) -> Result<u32> {
        let mut count = 0u32;
        for m in messages {
            if !matches!(m.admission.as_str(), "accepted" | "held" | "dropped" | "undecided") {
                anyhow::bail!("invalid admission value in backup: {}", m.admission);
            }
            if !matches!(m.state.as_str(), "pending" | "delivered" | "failed") {
                anyhow::bail!("invalid state value in backup: {}", m.state);
            }

            // Skip messages that already exist; the plan says duplicate ids are
            // ignored on import.
            let already_exists: bool = tx
                .query_row("SELECT COUNT(*) FROM messages WHERE id = ?1", params![m.id], |r| {
                    r.get::<_, i64>(0)
                })
                .map(|c| c > 0)
                .unwrap_or(false);
            if already_exists {
                continue;
            }

            let (state, last_error) = if m.outgoing && m.state == "pending" {
                ("failed".to_string(), Some("restored from a backup".to_string()))
            } else {
                (m.state, m.last_error)
            };

            // Restored flag follows the conversation: a message in a live chat
            // is not restored, even if it came from the bundle.
            let conv_restored: bool = tx
                .query_row(
                    "SELECT restored FROM conversations WHERE id = ?1",
                    params![m.conversation_id],
                    |r| r.get::<_, i64>(0),
                )
                .map(|v| v != 0)
                .unwrap_or(true);

            let vseq = Self::next_visible_seq(tx, &m.conversation_id)?;
            tx.execute(
                "INSERT INTO messages (id, conversation_id, author, sender_timestamp, \
                 received_at, content_type, body, signature, outgoing, verified, state, \
                 last_error, system, entry_id, admission, admission_reason, admission_changed_at, \
                 notify_attempts, next_notify_at, report_refusal, refused, deleted_at, restored, \
                 visible_seq) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?11, ?12, ?13, \
                 ?14, ?15, NULL, 0, NULL, 0, NULL, ?16, ?17, ?18)",
                params![
                    m.id,
                    m.conversation_id,
                    m.author,
                    m.sender_timestamp,
                    m.received_at,
                    m.content_type,
                    m.body,
                    m.signature.as_slice(),
                    if m.outgoing { 1i64 } else { 0i64 },
                    state,
                    last_error,
                    if m.system { 1i64 } else { 0i64 },
                    m.entry_id,
                    m.admission,
                    m.admission_reason,
                    m.deleted_at,
                    if conv_restored { 1i64 } else { 0i64 },
                    vseq as i64,
                ],
            )?;
            count += 1;

            // Index for search only when the message is accepted and not deleted.
            if m.deleted_at.is_none()
                && m.admission == "accepted"
                && is_searchable_content_type(&m.content_type)
                && let Ok(body_str) = str::from_utf8(&m.body)
            {
                let rowid = tx.last_insert_rowid();
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO messages_fts (rowid, body) VALUES (?1, ?2)",
                    params![rowid, body_str],
                );
            }
        }
        Ok(count)
    }
}

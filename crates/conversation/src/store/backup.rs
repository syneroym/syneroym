//! Backup export and restore for conversation capability data.

use std::str;

use anyhow::Result;
use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};
use syneroym_rpc::ConversationExportChunk;

use super::{ConversationStore, StoreError, StoredDagEntry, message::is_searchable_content_type};
use crate::{dag::WireEntry, ids::derive_conversation_id};

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

/// Where an export call resumes. Conversations and members come first,
/// then log entries, then messages, so an importer always sees a parent
/// before its children.
enum ExportCursor {
    Start,
    Entries(i64),
    Messages(i64),
}

impl ExportCursor {
    fn parse(raw: Option<&str>) -> Result<Self> {
        let Some(raw) = raw else { return Ok(Self::Start) };
        let bad = || StoreError::InvalidInput(format!("invalid export cursor: {raw}"));
        let (kind, n) = raw.split_once(':').ok_or_else(bad)?;
        let n: i64 = n.parse().map_err(|_| bad())?;
        match kind {
            "d" => Ok(Self::Entries(n)),
            "m" => Ok(Self::Messages(n)),
            _ => Err(bad().into()),
        }
    }
}

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
    /// One page of the history bundle. The cursor is opaque to callers:
    /// `d:<seq>` continues the log entries, `m:<rowid>` continues the
    /// messages. The first chunk also carries the conversations and group
    /// members, which every later chunk needs to exist first.
    pub fn export_history(&self, cursor: Option<String>) -> Result<ConversationExportChunk> {
        let cursor = ExportCursor::parse(cursor.as_deref())?;
        let conn = self.conn.lock().expect("conversation connection lock poisoned");
        let mut bundle = BackupBundle {
            version: BACKUP_VERSION,
            conversations: Vec::new(),
            messages: Vec::new(),
            dag_entries: Vec::new(),
            group_members: Vec::new(),
        };
        let next_cursor = match cursor {
            ExportCursor::Start => {
                bundle.conversations = Self::export_conversations(&conn)?;
                bundle.group_members = Self::export_group_members(&conn)?;
                Self::export_dag_page(&conn, 0, &mut bundle)?
            }
            ExportCursor::Entries(after_seq) => {
                Self::export_dag_page(&conn, after_seq, &mut bundle)?
            }
            ExportCursor::Messages(after_rowid) => {
                Self::export_messages_page(&conn, after_rowid, &mut bundle)?
            }
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

    /// Adds the next page of log entries to `bundle` and returns the cursor
    /// for what follows: more entries, or the first message page.
    fn export_dag_page(
        conn: &Connection,
        after_seq: i64,
        bundle: &mut BackupBundle,
    ) -> Result<Option<String>> {
        let mut stmt = conn.prepare(
            "SELECT seq, entry_id, conversation_id, author, sender_timestamp, epoch, kind, \
             header, ciphertext, nonce, payload, signature, applied, relay_pending FROM \
             dag_entries WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2",
        )?;
        let mut rows = stmt.query(params![after_seq, EXPORT_PAGE_SIZE + 1])?;
        let mut entries = Vec::new();
        while let Some(r) = rows.next()? {
            entries.push(Self::row_to_dag_entry(conn, r)?);
        }
        let more = entries.len() as i64 > EXPORT_PAGE_SIZE;
        if more {
            entries.pop();
        }
        let last_seq = entries.last().map_or(after_seq, |e| e.seq);
        bundle.dag_entries = entries.into_iter().map(StoredDagEntry::into_wire).collect();
        Ok(Some(if more { format!("d:{last_seq}") } else { "m:0".to_string() }))
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

    /// Adds the next page of messages (in row order, so a message that
    /// arrives mid-export is picked up by a later page, never skipped) to
    /// `bundle`.
    fn export_messages_page(
        conn: &Connection,
        after_rowid: i64,
        bundle: &mut BackupBundle,
    ) -> Result<Option<String>> {
        let mut m_stmt = conn.prepare(
            "SELECT id, conversation_id, author, sender_timestamp, received_at, content_type, \
             body, signature, outgoing, state, last_error, system, entry_id, admission, \
             admission_reason, deleted_at, rowid FROM messages WHERE system = 0 AND rowid > ?1 \
             ORDER BY rowid ASC LIMIT ?2",
        )?;
        let mut m_rows = m_stmt.query(params![after_rowid, EXPORT_PAGE_SIZE + 1])?;
        let mut rowids = Vec::new();
        while let Some(r) = m_rows.next()? {
            let sig_blob: Vec<u8> = r.get(7)?;
            let signature: [u8; 64] = sig_blob.as_slice().try_into().unwrap_or([0u8; 64]);
            bundle.messages.push(BackupMessage {
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
            rowids.push(r.get::<_, i64>(16)?);
        }
        if bundle.messages.len() as i64 > EXPORT_PAGE_SIZE {
            bundle.messages.pop();
            rowids.pop();
            return Ok(rowids.last().map(|id| format!("m:{id}")));
        }
        Ok(None)
    }

    /// Imports one chunk. Safe to repeat: existing messages, log entries and
    /// members are skipped, so a failed import is resumed by running the
    /// whole bundle again. `service_id` is this node's own address; it decides
    /// whether a direct chat in the bundle is still live here.
    pub fn import_history(&self, service_id: &str, data: &[u8]) -> Result<u32> {
        let bundle: BackupBundle = serde_json::from_slice(data)?;
        if bundle.version != BACKUP_VERSION {
            return Err(StoreError::InvalidInput(format!(
                "unsupported backup version: {}",
                bundle.version
            ))
            .into());
        }
        let mut conn = self.conn.lock().expect("conversation connection lock poisoned");
        let tx = conn.transaction()?;

        Self::import_conversations(&tx, service_id, bundle.conversations)?;
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

    fn import_conversations(
        tx: &Transaction<'_>,
        service_id: &str,
        convs: Vec<BackupConversation>,
    ) -> Result<()> {
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
                // A direct chat is live when its id is the one this node derives
                // for that peer, which is the case after a restore at the same
                // address. Any other id came from another node's address, so the
                // chat is read-only history here.
                c.peer_address
                    .as_deref()
                    .is_none_or(|peer| c.id != derive_conversation_id(service_id, peer))
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
                return Err(StoreError::InvalidInput(format!(
                    "invalid admission value in backup: {}",
                    m.admission
                ))
                .into());
            }
            if !matches!(m.state.as_str(), "pending" | "delivered" | "failed") {
                return Err(StoreError::InvalidInput(format!(
                    "invalid state value in backup: {}",
                    m.state
                ))
                .into());
            }

            // A message that is already stored is skipped, so importing the same
            // bundle twice changes nothing and an interrupted import can be rerun.
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

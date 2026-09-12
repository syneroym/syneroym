//! CRUD operations on the `conversations` table: looking up or creating
//! direct conversations, creating group conversations, reading metadata
//! (`ConversationRow`), and listing all conversations. Sibling files
//! `message.rs` and `dag_store.rs` own the per-conversation payload tables.

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use syneroym_rpc::ConversationKind;

use super::{ConversationRow, ConversationStore};

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
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
}

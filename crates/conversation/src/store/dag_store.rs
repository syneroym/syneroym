//! Persistence for the group DAG: `dag_entries`, `group_members`, and
//! `group_epochs` tables. Handles appending and reading `StoredDagEntry`
//! rows, tracking current membership and epoch keys, and walking parent
//! chains. Distinct from `message.rs`, which owns the plain `messages` table.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use syneroym_rpc::ConversationMembershipEvent;

use super::{ConversationStore, StoredDagEntry};
use crate::dag::{EntryKind, MAX_PARENTS, MembershipPayload, WireEntry, canonical_entry_bytes};

// Lock-poisoning from a panicking holder is a programming error; there is
// no safe recovery path, matching `syneroym-async-queue`'s own precedent.
#[allow(clippy::expect_used)]
impl ConversationStore {
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
        let existing: Option<(i64, Option<i64>, bool)> = tx
            .query_row(
                "SELECT joined_epoch, removed_epoch, epoch_confirmed FROM group_members WHERE \
                 conversation_id = ?1 AND member_address = ?2",
                params![conversation_id, payload.subject_address],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)),
            )
            .optional()?;

        let new_epoch = payload.new_epoch as i64;
        if payload.action == "add" {
            match existing {
                Some((joined, removed, epoch_confirmed)) => {
                    // A row seeded from a `GroupKeyPayload` (see
                    // `peer_deliver_impl`) is a best-effort guess: the payload
                    // carries only member *addresses*, not each bystander's
                    // true join epoch, so a member first learned about
                    // through a later key message can end up placeholder-
                    // seeded at a *later* epoch than it actually joined.
                    // `epoch_confirmed` staying false is exactly the signal
                    // that this row's `joined_epoch` was never confirmed by a
                    // real, owner-signed DAG entry — the entry being applied
                    // right now always outranks a guess like that, regardless
                    // of epoch ordering. This must be a signal separate from
                    // `sig_key`: TOFU-pinning a peer's signing key (see
                    // `pin_member_sig_key_if_placeholder`) confirms the *key*
                    // the moment that peer is first talked to directly, well
                    // before the real DAG entry naming their true join epoch
                    // has necessarily arrived — treating that as "confirmed"
                    // let a lower, correct epoch from the real entry lose to
                    // the ordering guard below forever. That guard
                    // (`new_epoch >= prior_epoch`) is only meaningful between
                    // two *real* entries, guarding against a stale replay.
                    let unconfirmed = !epoch_confirmed;
                    let prior_epoch = removed.unwrap_or(joined);
                    if unconfirmed || new_epoch >= prior_epoch {
                        tx.execute(
                            "UPDATE group_members SET sig_key = ?3, joined_epoch = ?4, \
                             removed_epoch = NULL, epoch_confirmed = 1 WHERE conversation_id = ?1 \
                             AND member_address = ?2",
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
                         joined_epoch, removed_epoch, epoch_confirmed) VALUES (?1, ?2, ?3, ?4, \
                         NULL, 1)",
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
                Some((joined, removed, epoch_confirmed)) => {
                    let unconfirmed = !epoch_confirmed;
                    let prior_epoch = removed.unwrap_or(joined);
                    if unconfirmed || new_epoch > prior_epoch {
                        tx.execute(
                            "UPDATE group_members SET removed_epoch = ?3, epoch_confirmed = 1 \
                             WHERE conversation_id = ?1 AND member_address = ?2",
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
                         joined_epoch, removed_epoch, epoch_confirmed) VALUES (?1, ?2, ?3, ?4, \
                         ?4, 1)",
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
}

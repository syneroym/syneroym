//! SQLite storage for queues, messages, and receipts.

use std::{
    path::Path,
    sync::{Mutex, MutexGuard, PoisonError},
};

use rusqlite::{Connection, Transaction};
use syneroym_core::mailbox::QueueLimits;
use syneroym_mailbox_format::Receipt;

use crate::{error::MailboxError, limits::Limits};

pub(crate) mod claim;
pub(crate) mod deposit;
pub(crate) mod queue;
pub(crate) mod sweep;

#[cfg(test)]
mod tests;

pub(crate) struct MailboxStore {
    conn: Mutex<Connection>,
    limits: Limits,
}

impl MailboxStore {
    pub(crate) fn open(path: &Path, limits: Limits) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        init_schema(&conn)?;
        Ok(Self { conn: Mutex::new(conn), limits })
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory(limits: Limits) -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        init_schema(&conn)?;
        Ok(Self { conn: Mutex::new(conn), limits })
    }

    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn init_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS queues (
            service_id          TEXT PRIMARY KEY,
            record_json         TEXT NOT NULL,
            record_signed_at    INTEGER NOT NULL,
            record_not_after    INTEGER NOT NULL,
            newest_issued_at    INTEGER NOT NULL,
            newest_instance_did TEXT NOT NULL,
            max_bytes           INTEGER NOT NULL,
            max_messages        INTEGER NOT NULL,
            max_ttl_secs        INTEGER NOT NULL,
            bytes               INTEGER NOT NULL DEFAULT 0,
            messages            INTEGER NOT NULL DEFAULT 0,
            last_active_at      INTEGER NOT NULL,
            next_publish_at     INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS messages (
            seq           INTEGER PRIMARY KEY AUTOINCREMENT,
            service_id    TEXT NOT NULL,
            sender        TEXT NOT NULL,
            deposit_id    TEXT NOT NULL,
            sealed        TEXT NOT NULL,
            size          INTEGER NOT NULL,
            deposited_at  INTEGER NOT NULL,
            expires_at    INTEGER NOT NULL,
            claimed_until INTEGER NOT NULL DEFAULT 0,
            claim_count   INTEGER NOT NULL DEFAULT 0,
            UNIQUE (sender, deposit_id)
        );
        CREATE INDEX IF NOT EXISTS messages_queue ON messages (service_id, seq);
        CREATE INDEX IF NOT EXISTS messages_queue_sender ON messages (service_id, sender);
        CREATE INDEX IF NOT EXISTS messages_expiry ON messages (expires_at);
        CREATE TABLE IF NOT EXISTS receipts (
            sender       TEXT NOT NULL,
            deposit_id   TEXT NOT NULL,
            receipt_json TEXT NOT NULL,
            expires_at   INTEGER NOT NULL,
            PRIMARY KEY (sender, deposit_id)
        );
        CREATE INDEX IF NOT EXISTS receipts_expiry ON receipts (expires_at);",
    )
}

pub(super) fn to_sql(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(super) fn from_sql(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(0)
}

#[derive(Debug, Clone)]
pub(super) struct QueueRow {
    pub(super) service_id: String,
    pub(super) record_signed_at: u64,
    pub(super) newest_issued_at: u64,
    pub(super) newest_instance_did: String,
    pub(super) limits: QueueLimits,
    pub(super) bytes: u64,
    pub(super) messages: u64,
    #[allow(dead_code)]
    pub(super) last_active_at: u64,
}

pub(super) fn queue_row(
    tx: &Transaction,
    service_id: &str,
) -> Result<Option<QueueRow>, MailboxError> {
    let mut stmt = tx.prepare(
        "SELECT service_id, record_signed_at, newest_issued_at, newest_instance_did, max_bytes, \
         max_messages, max_ttl_secs, bytes, messages, last_active_at FROM queues WHERE service_id \
         = ?1",
    )?;
    let mut rows = stmt.query([service_id])?;
    if let Some(r) = rows.next()? {
        let sid: String = r.get(0)?;
        let record_signed_at: i64 = r.get(1)?;
        let newest_issued_at: i64 = r.get(2)?;
        let newest_instance_did: String = r.get(3)?;
        let max_bytes: i64 = r.get(4)?;
        let max_messages: i64 = r.get(5)?;
        let max_ttl_secs: i64 = r.get(6)?;
        let bytes: i64 = r.get(7)?;
        let messages: i64 = r.get(8)?;
        let last_active_at: i64 = r.get(9)?;
        Ok(Some(QueueRow {
            service_id: sid,
            record_signed_at: from_sql(record_signed_at),
            newest_issued_at: from_sql(newest_issued_at),
            newest_instance_did,
            limits: QueueLimits {
                max_bytes: from_sql(max_bytes),
                max_messages: from_sql(max_messages) as u32,
                max_ttl_secs: from_sql(max_ttl_secs),
            },
            bytes: from_sql(bytes),
            messages: from_sql(messages),
            last_active_at: from_sql(last_active_at),
        }))
    } else {
        Ok(None)
    }
}

pub(super) fn remove_message(
    tx: &Transaction,
    seq: i64,
    receipt: &Receipt,
    keep_secs: u64,
    now: u64,
) -> Result<(), MailboxError> {
    let mut stmt =
        tx.prepare("SELECT service_id, sender, deposit_id, size FROM messages WHERE seq = ?1")?;
    let mut rows = stmt.query([seq])?;
    let info: Option<(String, String, String, i64)> = if let Some(r) = rows.next()? {
        Some((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    } else {
        None
    };
    drop(rows);
    drop(stmt);

    let Some((service_id, sender, deposit_id, size)) = info else {
        return Ok(());
    };

    tx.execute("DELETE FROM messages WHERE seq = ?1", [seq])?;
    tx.execute(
        "UPDATE queues SET bytes = MAX(0, bytes - ?1), messages = MAX(0, messages - 1) WHERE \
         service_id = ?2",
        rusqlite::params![size, service_id],
    )?;
    put_receipt(tx, &sender, &deposit_id, receipt, now.saturating_add(keep_secs))?;
    Ok(())
}

pub(super) fn put_receipt(
    tx: &Transaction,
    sender: &str,
    deposit_id: &str,
    receipt: &Receipt,
    expires_at: u64,
) -> Result<(), MailboxError> {
    let json = serde_json::to_string(receipt)
        .map_err(|e| MailboxError::Internal(format!("failed to serialize receipt: {e}")))?;
    tx.execute(
        "INSERT INTO receipts (sender, deposit_id, receipt_json, expires_at) VALUES (?1, ?2, ?3, \
         ?4) ON CONFLICT (sender, deposit_id) DO UPDATE SET receipt_json = excluded.receipt_json, \
         expires_at = excluded.expires_at",
        rusqlite::params![sender, deposit_id, json, to_sql(expires_at)],
    )?;
    Ok(())
}

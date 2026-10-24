//! SQLite storage for queues, messages, and receipts.

use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use blake3::Hasher;
use rusqlite::{Connection, Transaction};
use syneroym_core::mailbox::QueueLimits;
use syneroym_mailbox_format::{DEDICATED_MAX_TTL_SECS, Receipt};
use syneroym_passport::{Lease, LeaseStore, facts};
use tokio::task;

use crate::{error::MailboxError, limits::Limits};

pub(crate) mod claim;
pub(crate) mod deposit;
pub(crate) mod queue;
pub(crate) mod settled;
pub(crate) mod sweep;

#[cfg(test)]
mod tests;

pub(crate) struct MailboxStore {
    conn: Mutex<Connection>,
    limits: Limits,
    /// The passport leases. Without them every queue has the open limits.
    leases: Option<LeaseStore>,
}

impl MailboxStore {
    pub(crate) fn open(
        path: &Path,
        limits: Limits,
        leases: Option<LeaseStore>,
    ) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        init_schema(&conn)?;
        settled::trim_at_open(&mut conn, limits.max_settled_receipts)?;
        Ok(Self { conn: Mutex::new(conn), limits, leases })
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory(limits: Limits) -> anyhow::Result<Self> {
        Self::open_in_memory_with_leases(limits, None)
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory_with_leases(
        limits: Limits,
        leases: Option<LeaseStore>,
    ) -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        init_schema(&conn)?;
        Ok(Self { conn: Mutex::new(conn), limits, leases })
    }

    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The limits of `service_id`'s queue now: the open limits, raised by
    /// the facts of a running lease for that master. A fact below the open
    /// limit is ignored, so a passport never makes a queue worse than none,
    /// and `max_ttl_secs` is cut to what a dedicated mailbox may give.
    pub(crate) fn queue_limits(&self, service_id: &str, now: u64) -> QueueLimits {
        let open = self.limits.queue;
        let Some(lease) = self.lease_for(service_id, now) else { return open };
        let raise = |open: u64, fact: Option<u64>| fact.map_or(open, |f| f.max(open));
        QueueLimits {
            max_bytes: raise(open.max_bytes, lease.limit(facts::MAX_BYTES)),
            max_messages: u32::try_from(raise(
                u64::from(open.max_messages),
                lease.limit(facts::MAX_MESSAGES),
            ))
            .unwrap_or(u32::MAX),
            max_ttl_secs: raise(
                open.max_ttl_secs,
                lease.limit(facts::MAX_TTL_SECS).map(|t| t.min(DEDICATED_MAX_TTL_SECS)),
            ),
        }
    }

    /// The running lease of `service_id`, from memory. The lease store never
    /// takes the mailbox lock, so the two locks cannot wait on each other.
    pub(crate) fn lease_for(&self, service_id: &str, now: u64) -> Option<Lease> {
        self.leases.as_ref()?.lease_for(service_id, now)
    }

    pub(super) fn receipt_policy(&self) -> ReceiptPolicy {
        ReceiptPolicy {
            keep_secs: self.limits.receipt_keep_secs,
            max_settled: self.limits.max_settled_receipts,
        }
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
            destination  TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            deposited_at INTEGER NOT NULL,
            ttl_secs     INTEGER NOT NULL,
            receipt_json TEXT NOT NULL,
            expires_at   INTEGER NOT NULL,
            settled      INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (sender, deposit_id)
        );
        CREATE INDEX IF NOT EXISTS receipts_expiry ON receipts (expires_at);
        CREATE INDEX IF NOT EXISTS receipts_settled ON receipts (settled, sender, expires_at);
        CREATE TABLE IF NOT EXISTS mailbox_stats (
            id               INTEGER PRIMARY KEY CHECK (id = 1),
            total_bytes      INTEGER NOT NULL DEFAULT 0,
            settled_receipts INTEGER NOT NULL DEFAULT 0
        );
        INSERT OR IGNORE INTO mailbox_stats (id, total_bytes, settled_receipts)
            SELECT 1, COALESCE(SUM(bytes), 0),
                (SELECT COUNT(*) FROM receipts WHERE settled = 1) FROM queues;
        UPDATE mailbox_stats SET
            total_bytes = (SELECT COALESCE(SUM(bytes), 0) FROM queues),
            settled_receipts = (SELECT COUNT(*) FROM receipts WHERE settled = 1)
            WHERE id = 1;",
    )
}

/// Runs a blocking closure against the mailbox store on Tokio's blocking thread
/// pool.
pub(crate) async fn blocking<T: Send + 'static>(
    store: &Arc<MailboxStore>,
    f: impl FnOnce(&MailboxStore) -> Result<T, MailboxError> + Send + 'static,
) -> Result<T, MailboxError> {
    let s = store.clone();
    task::spawn_blocking(move || f(&s))
        .await
        .map_err(|e| MailboxError::Internal(format!("blocking storage task failed: {e}")))?
}

/// Computes a deterministic content hash of the destination DID and sealed
/// ciphertext.
pub(super) fn compute_content_hash(destination: &str, sealed: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(destination.as_bytes());
    hasher.update(b"\x00");
    hasher.update(sealed.as_bytes());
    hasher.finalize().to_hex().to_string()
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
    pub(super) bytes: u64,
    pub(super) messages: u64,
}

pub(super) fn queue_row(
    tx: &Transaction,
    service_id: &str,
) -> Result<Option<QueueRow>, MailboxError> {
    let mut stmt = tx.prepare(
        "SELECT service_id, record_signed_at, newest_issued_at, newest_instance_did, bytes, \
         messages FROM queues WHERE service_id = ?1",
    )?;
    let mut rows = stmt.query([service_id])?;
    if let Some(r) = rows.next()? {
        Ok(Some(QueueRow {
            service_id: r.get(0)?,
            record_signed_at: from_sql(r.get(1)?),
            newest_issued_at: from_sql(r.get(2)?),
            newest_instance_did: r.get(3)?,
            bytes: from_sql(r.get(4)?),
            messages: from_sql(r.get(5)?),
        }))
    } else {
        Ok(None)
    }
}

/// How long settled receipts stay, and how many may stay.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReceiptPolicy {
    pub(crate) keep_secs: u64,
    pub(crate) max_settled: u64,
}

/// Deletes the message at `seq` and settles its receipt: the body is gone, the
/// receipt stays for the sender. Drops settled receipts, from the senders
/// that hold the most, when this leaves more than `policy.max_settled`.
pub(super) fn remove_message(
    tx: &Transaction,
    seq: i64,
    receipt: &Receipt,
    policy: ReceiptPolicy,
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
    tx.execute(
        "UPDATE mailbox_stats SET total_bytes = MAX(0, total_bytes - ?1) WHERE id = 1",
        rusqlite::params![size],
    )?;
    let json = serde_json::to_string(receipt)
        .map_err(|e| MailboxError::Internal(format!("failed to serialize receipt: {e}")))?;
    let expires = to_sql(now.saturating_add(policy.keep_secs));
    let updated = tx.execute(
        "UPDATE receipts SET receipt_json = ?1, expires_at = ?2, settled = 1 WHERE sender = ?3 \
         AND deposit_id = ?4",
        rusqlite::params![json, expires, sender, deposit_id],
    )?;
    debug_assert_eq!(updated, 1, "receipt row must exist when removing message");
    if updated == 1 {
        tx.execute(
            "UPDATE mailbox_stats SET settled_receipts = settled_receipts + 1 WHERE id = 1",
            [],
        )?;
        settled::drop_settled_over_cap(tx, policy.max_settled)?;
    }
    Ok(())
}

pub(super) struct ReceiptEntry<'a> {
    pub(super) sender: &'a str,
    pub(super) deposit_id: &'a str,
    pub(super) destination: &'a str,
    pub(super) content_hash: &'a str,
    pub(super) deposited_at: u64,
    pub(super) ttl_secs: u64,
    pub(super) receipt: &'a Receipt,
    pub(super) expires_at: u64,
}

/// Stores the receipt of a new deposit. It is a plain insert: `deposit`
/// answers from an existing receipt before it gets here, and a receipt that
/// already exists would otherwise change what the settled count means.
pub(super) fn put_receipt(tx: &Transaction, entry: ReceiptEntry<'_>) -> Result<(), MailboxError> {
    let json = serde_json::to_string(entry.receipt)
        .map_err(|e| MailboxError::Internal(format!("failed to serialize receipt: {e}")))?;
    tx.execute(
        "INSERT INTO receipts (sender, deposit_id, destination, content_hash, deposited_at, \
         ttl_secs, receipt_json, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            entry.sender,
            entry.deposit_id,
            entry.destination,
            entry.content_hash,
            to_sql(entry.deposited_at),
            to_sql(entry.ttl_secs),
            json,
            to_sql(entry.expires_at)
        ],
    )?;
    Ok(())
}

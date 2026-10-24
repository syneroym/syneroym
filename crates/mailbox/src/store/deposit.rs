//! Storage operations for deposits and receipt status.

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};
use syneroym_core::mailbox::{MAX_STATUS_BATCH, QueueLimits, StatusEntry};
use syneroym_mailbox_format::{Deposit, DepositError, DepositId, Receipt};
use tracing::warn;

use crate::{
    error::MailboxError,
    store::{
        MailboxStore, QueueRow, ReceiptEntry, ReceiptPolicy, compute_content_hash, from_sql,
        put_receipt, queue_row, remove_message, to_sql,
    },
};

/// The outcome of storing a deposit: the receipt and how many messages were
/// evicted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DepositOutcome {
    pub receipt: Receipt,
    pub evicted: u32,
}

impl MailboxStore {
    /// Stores a deposit in the destination's queue, evicting older messages if
    /// needed.
    pub(crate) fn deposit(
        &self,
        sender: &str,
        deposit: &Deposit,
        now: u64,
    ) -> Result<DepositOutcome, MailboxError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let row = queue_row(&tx, &deposit.destination)?
            .ok_or_else(|| MailboxError::UnknownDestination(deposit.destination.clone()))?;

        let limits = self.queue_limits(&deposit.destination, now);
        deposit.check(limits.max_ttl_secs).map_err(|e| match e {
            DepositError::TooLarge { bytes, max } => MailboxError::TooLarge { bytes, max },
            DepositError::TtlTooLong { ttl_secs, max_secs } => {
                MailboxError::TtlTooLong { ttl_secs, max_secs }
            }
            other => MailboxError::InvalidParams(other.to_string()),
        })?;

        let id = deposit.deposit_id.to_string();
        let size = deposit.sealed.len();
        let sealed_text = String::from(deposit.sealed.clone());
        let content_hash = compute_content_hash(&deposit.destination, &sealed_text);

        if let Some(existing) = check_existing_message(&tx, sender, &id)? {
            if existing.destination == deposit.destination
                && existing.sealed == sealed_text
                && existing.ttl_secs == deposit.ttl_secs
            {
                return Ok(DepositOutcome {
                    receipt: Receipt::Stored { at_secs: existing.at },
                    evicted: 0,
                });
            }
            return Err(MailboxError::DuplicateDeposit(id));
        }

        if let Some(receipt_row) = check_existing_receipt(&tx, sender, &id)? {
            if receipt_row.destination == deposit.destination
                && receipt_row.content_hash == content_hash
                && receipt_row.ttl_secs == deposit.ttl_secs
            {
                return Ok(DepositOutcome { receipt: receipt_row.receipt, evicted: 0 });
            }
            return Err(MailboxError::DuplicateDeposit(id));
        }

        let evicted = make_room(&tx, &row, limits, sender, size, now, self.receipt_policy())?;

        let total: i64 =
            tx.query_row("SELECT total_bytes FROM mailbox_stats WHERE id = 1", [], |r| r.get(0))?;
        if from_sql(total).saturating_add(size as u64) > self.limits.max_total_bytes {
            return Err(MailboxError::MailboxFull);
        }

        let expires = now.saturating_add(deposit.ttl_secs);
        tx.execute(
            "INSERT INTO messages (service_id, sender, deposit_id, sealed, size, deposited_at, \
             expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                deposit.destination,
                sender,
                id,
                sealed_text,
                to_sql(size as u64),
                to_sql(now),
                to_sql(expires),
            ],
        )?;
        tx.execute(
            "UPDATE queues SET bytes = bytes + ?1, messages = messages + 1 WHERE service_id = ?2",
            rusqlite::params![to_sql(size as u64), deposit.destination],
        )?;
        tx.execute(
            "UPDATE mailbox_stats SET total_bytes = total_bytes + ?1 WHERE id = 1",
            rusqlite::params![to_sql(size as u64)],
        )?;
        put_receipt(
            &tx,
            ReceiptEntry {
                sender,
                deposit_id: &id,
                destination: &deposit.destination,
                content_hash: &content_hash,
                deposited_at: now,
                ttl_secs: deposit.ttl_secs,
                receipt: &Receipt::Stored { at_secs: now },
                expires_at: expires.saturating_add(self.limits.receipt_keep_secs),
            },
        )?;

        tx.commit()?;
        Ok(DepositOutcome { receipt: Receipt::Stored { at_secs: now }, evicted })
    }

    /// Reads back delivery receipts for previously deposited messages.
    pub(crate) fn status(
        &self,
        sender: &str,
        ids: &[DepositId],
        now: u64,
    ) -> Result<Vec<StatusEntry>, MailboxError> {
        if ids.len() > MAX_STATUS_BATCH {
            return Err(MailboxError::InvalidParams(format!(
                "status batch size {} exceeds maximum of {}",
                ids.len(),
                MAX_STATUS_BATCH
            )));
        }

        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT receipt_json FROM receipts WHERE sender = ?1 AND deposit_id = ?2 AND \
             expires_at > ?3",
        )?;

        let mut entries = Vec::with_capacity(ids.len());
        for &id in ids {
            let id_str = id.to_string();
            let mut rows = stmt.query(rusqlite::params![sender, id_str, to_sql(now)])?;
            let receipt = if let Some(r) = rows.next()? {
                let json: String = r.get(0)?;
                serde_json::from_str::<Receipt>(&json).unwrap_or_else(|e| {
                    warn!(error = %e, sender, deposit_id = %id_str, "corrupt receipt in store");
                    Receipt::Unknown
                })
            } else {
                Receipt::Unknown
            };
            entries.push(StatusEntry { deposit_id: id, receipt });
        }

        Ok(entries)
    }
}

struct ExistingMessage {
    destination: String,
    sealed: String,
    ttl_secs: u64,
    at: u64,
}

fn check_existing_message(
    tx: &Transaction,
    sender: &str,
    deposit_id: &str,
) -> Result<Option<ExistingMessage>, MailboxError> {
    let mut stmt = tx.prepare(
        "SELECT service_id, sealed, deposited_at, expires_at FROM messages WHERE sender = ?1 AND \
         deposit_id = ?2",
    )?;
    let mut rows = stmt.query(rusqlite::params![sender, deposit_id])?;
    if let Some(r) = rows.next()? {
        let dest: String = r.get(0)?;
        let sealed: String = r.get(1)?;
        let at: i64 = r.get(2)?;
        let exp: i64 = r.get(3)?;
        let ttl_secs = from_sql(exp).saturating_sub(from_sql(at));
        Ok(Some(ExistingMessage { destination: dest, sealed, ttl_secs, at: from_sql(at) }))
    } else {
        Ok(None)
    }
}

struct ExistingReceipt {
    destination: String,
    content_hash: String,
    ttl_secs: u64,
    receipt: Receipt,
}

fn check_existing_receipt(
    tx: &Transaction,
    sender: &str,
    deposit_id: &str,
) -> Result<Option<ExistingReceipt>, MailboxError> {
    let res = tx
        .query_row(
            "SELECT destination, content_hash, deposited_at, ttl_secs, receipt_json FROM receipts \
             WHERE sender = ?1 AND deposit_id = ?2",
            rusqlite::params![sender, deposit_id],
            |r| {
                let destination: String = r.get(0)?;
                let content_hash: String = r.get(1)?;
                let deposited_at = from_sql(r.get(2)?);
                let ttl_secs = from_sql(r.get(3)?);
                let json: String = r.get(4)?;
                let receipt = serde_json::from_str::<Receipt>(&json)
                    .unwrap_or(Receipt::Stored { at_secs: deposited_at });
                Ok(ExistingReceipt { destination, content_hash, ttl_secs, receipt })
            },
        )
        .optional()?;
    Ok(res)
}

fn make_room(
    tx: &Transaction,
    row: &QueueRow,
    limits: QueueLimits,
    depositor: &str,
    size: usize,
    now: u64,
    policy: ReceiptPolicy,
) -> Result<u32, MailboxError> {
    let mut bytes = row.bytes;
    let mut messages = row.messages;
    let mut evicted = 0;
    let size_u64 = size as u64;

    while messages.saturating_add(1) > u64::from(limits.max_messages)
        || bytes.saturating_add(size_u64) > limits.max_bytes
    {
        let top = select_eviction_candidate(tx, &row.service_id)?;
        match top {
            None => return Err(MailboxError::QueueFull(row.service_id.clone())),
            Some((s, _)) if s == depositor => {
                return Err(MailboxError::QueueFull(row.service_id.clone()));
            }
            Some((s, _)) => {
                let (seq, vsize) = select_oldest_message_from_sender(tx, &row.service_id, &s, now)?
                    .ok_or_else(|| MailboxError::QueueFull(row.service_id.clone()))?;

                remove_message(tx, seq, &Receipt::Expired { at_secs: now }, policy, now)?;
                bytes = bytes.saturating_sub(from_sql(vsize));
                messages = messages.saturating_sub(1);
                evicted += 1;
            }
        }
    }
    Ok(evicted)
}

fn select_eviction_candidate(
    tx: &Transaction,
    service_id: &str,
) -> Result<Option<(String, i64)>, MailboxError> {
    let mut stmt = tx.prepare(
        "SELECT sender, SUM(size) AS b FROM messages WHERE service_id = ?1 GROUP BY sender ORDER \
         BY b DESC, sender ASC LIMIT 1",
    )?;
    let mut rows = stmt.query([service_id])?;
    if let Some(r) = rows.next()? { Ok(Some((r.get(0)?, r.get(1)?))) } else { Ok(None) }
}

fn select_oldest_message_from_sender(
    tx: &Transaction,
    service_id: &str,
    sender: &str,
    now: u64,
) -> Result<Option<(i64, i64)>, MailboxError> {
    let mut stmt = tx.prepare(
        "SELECT seq, size FROM messages WHERE service_id = ?1 AND sender = ?2 ORDER BY \
         (claimed_until > ?3) ASC, seq ASC LIMIT 1",
    )?;
    let mut rows = stmt.query(rusqlite::params![service_id, sender, to_sql(now)])?;
    if let Some(r) = rows.next()? { Ok(Some((r.get(0)?, r.get(1)?))) } else { Ok(None) }
}

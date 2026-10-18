//! Storage operations for deposits and receipt status.

use rusqlite::Transaction;
use syneroym_core::mailbox::{MAX_STATUS_BATCH, StatusEntry};
use syneroym_mailbox_format::{Deposit, DepositError, DepositId, Receipt};
use tracing::warn;

use crate::{
    error::MailboxError,
    store::{MailboxStore, QueueRow, from_sql, put_receipt, queue_row, remove_message, to_sql},
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
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let row = queue_row(&tx, &deposit.destination)?
            .ok_or_else(|| MailboxError::UnknownDestination(deposit.destination.clone()))?;

        deposit.check(row.limits.max_ttl_secs).map_err(|e| match e {
            DepositError::TooLarge { bytes, max } => MailboxError::TooLarge { bytes, max },
            DepositError::TtlTooLong { ttl_secs, max_secs } => {
                MailboxError::TtlTooLong { ttl_secs, max_secs }
            }
            other => MailboxError::InvalidParams(other.to_string()),
        })?;

        let id = deposit.deposit_id.to_string();
        let size = deposit.sealed.len();

        if let Some(existing) = check_existing_message(&tx, sender, &id)? {
            if existing.destination == deposit.destination
                && existing.sealed == String::from(deposit.sealed.clone())
            {
                return Ok(DepositOutcome {
                    receipt: Receipt::Stored { at_secs: existing.at },
                    evicted: 0,
                });
            }
            return Err(MailboxError::DuplicateDeposit(id));
        }

        if check_existing_receipt(&tx, sender, &id)? {
            return Err(MailboxError::DuplicateDeposit(id));
        }

        let evicted = make_room(&tx, &row, sender, size, now, self.limits.receipt_keep_secs)?;

        let total: i64 =
            tx.query_row("SELECT COALESCE(SUM(bytes), 0) FROM queues", [], |r| r.get(0))?;
        if from_sql(total).saturating_add(size as u64) > self.limits.max_total_bytes {
            return Err(MailboxError::MailboxFull);
        }

        let expires = now.saturating_add(deposit.ttl_secs);
        let sealed_text = String::from(deposit.sealed.clone());
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
        put_receipt(
            &tx,
            sender,
            &id,
            &Receipt::Stored { at_secs: now },
            expires.saturating_add(self.limits.receipt_keep_secs),
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
    at: u64,
}

fn check_existing_message(
    tx: &Transaction,
    sender: &str,
    deposit_id: &str,
) -> Result<Option<ExistingMessage>, MailboxError> {
    let mut stmt = tx.prepare(
        "SELECT service_id, sealed, deposited_at FROM messages WHERE sender = ?1 AND deposit_id = \
         ?2",
    )?;
    let mut rows = stmt.query(rusqlite::params![sender, deposit_id])?;
    if let Some(r) = rows.next()? {
        let dest: String = r.get(0)?;
        let sealed: String = r.get(1)?;
        let at: i64 = r.get(2)?;
        Ok(Some(ExistingMessage { destination: dest, sealed, at: from_sql(at) }))
    } else {
        Ok(None)
    }
}

fn check_existing_receipt(
    tx: &Transaction,
    sender: &str,
    deposit_id: &str,
) -> Result<bool, MailboxError> {
    let in_receipts = tx
        .query_row(
            "SELECT 1 FROM receipts WHERE sender = ?1 AND deposit_id = ?2",
            rusqlite::params![sender, deposit_id],
            |_| Ok(()),
        )
        .is_ok();
    Ok(in_receipts)
}

fn make_room(
    tx: &Transaction,
    row: &QueueRow,
    depositor: &str,
    size: usize,
    now: u64,
    keep_secs: u64,
) -> Result<u32, MailboxError> {
    let mut bytes = row.bytes;
    let mut messages = row.messages;
    let mut evicted = 0;
    let size_u64 = size as u64;

    while messages.saturating_add(1) > u64::from(row.limits.max_messages)
        || bytes.saturating_add(size_u64) > row.limits.max_bytes
    {
        let top = select_eviction_candidate(tx, &row.service_id)?;
        match top {
            None => return Err(MailboxError::QueueFull(row.service_id.clone())),
            Some((s, _)) if s == depositor => {
                return Err(MailboxError::QueueFull(row.service_id.clone()));
            }
            Some((s, _)) => {
                let (seq, vsize) = select_oldest_message_from_sender(tx, &row.service_id, &s)?
                    .ok_or_else(|| MailboxError::QueueFull(row.service_id.clone()))?;

                remove_message(tx, seq, &Receipt::Expired { at_secs: now }, keep_secs, now)?;
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
) -> Result<Option<(i64, i64)>, MailboxError> {
    let mut stmt = tx.prepare(
        "SELECT seq, size FROM messages WHERE service_id = ?1 AND sender = ?2 ORDER BY seq LIMIT 1",
    )?;
    let mut rows = stmt.query(rusqlite::params![service_id, sender])?;
    if let Some(r) = rows.next()? { Ok(Some((r.get(0)?, r.get(1)?))) } else { Ok(None) }
}

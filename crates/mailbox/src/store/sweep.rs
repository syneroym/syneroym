//! Sweep passes for expired messages, dead queues, and record publishing.

use rusqlite::{Transaction, TransactionBehavior};
use syneroym_core::dht_registry::SignedEndpointInfo;
use syneroym_mailbox_format::Receipt;
use tracing::warn;

use crate::{
    error::MailboxError,
    store::{
        MailboxStore, ReceiptPolicy, claim::close_over_claimed, from_sql, remove_message, to_sql,
    },
};

/// The summary of removals performed by a sweep pass.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SweepReport {
    pub expired: u32,
    pub over_claimed: u32,
    pub queues_removed: u32,
    pub receipts_removed: u32,
}

/// A service endpoint record that is due to be published to the registry.
pub(crate) struct DueRecord {
    pub service_id: String,
    pub record: SignedEndpointInfo,
}

/// Workspace-wide resource totals currently held in the mailbox.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Totals {
    pub queues: u64,
    pub messages: u64,
    pub bytes: u64,
    pub settled_receipts: u64,
}

impl MailboxStore {
    /// Sweeps expired messages, over-claimed messages, dead queues, and expired
    /// receipts.
    pub(crate) fn sweep(&self, now: u64) -> Result<SweepReport, MailboxError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let policy = self.receipt_policy();
        let expired = sweep_expired_messages(&tx, policy, now)?;
        let over_claimed = close_over_claimed(&tx, None, policy, now)?;
        let queues_removed = sweep_dead_queues(&tx, self.limits.queue_lifetime_secs, policy, now)?;
        let receipts_removed = remove_expired_receipts(&tx, now)?;

        tx.commit()?;
        Ok(SweepReport { expired, over_claimed, queues_removed, receipts_removed })
    }

    /// Selects registered service records that are due to be republished to the
    /// registry.
    pub(crate) fn records_due(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<DueRecord>, MailboxError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT service_id, record_json FROM queues WHERE next_publish_at <= ?1 AND \
             record_not_after > ?1 ORDER BY next_publish_at LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![to_sql(now), to_sql(limit as u64)], |r| {
            let sid: String = r.get(0)?;
            let json: String = r.get(1)?;
            Ok((sid, json))
        })?;

        let mut records = Vec::new();
        for row in rows {
            let (sid, json) = row?;
            match serde_json::from_str::<SignedEndpointInfo>(&json) {
                Ok(record) => records.push(DueRecord { service_id: sid, record }),
                Err(e) => {
                    warn!(error = %e, service_id = sid, "corrupt record_json in database");
                }
            }
        }
        Ok(records)
    }

    /// Records the next republish deadline for a service record.
    pub(crate) fn mark_published(
        &self,
        service_id: &str,
        next_at: u64,
    ) -> Result<(), MailboxError> {
        let conn = self.lock();
        conn.execute(
            "UPDATE queues SET next_publish_at = ?1 WHERE service_id = ?2",
            rusqlite::params![to_sql(next_at), service_id],
        )?;
        Ok(())
    }

    /// Returns aggregate storage counts across all active queues.
    pub(crate) fn totals(&self) -> Result<Totals, MailboxError> {
        let conn = self.lock();
        let (q, m, b): (i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(messages), 0), COALESCE(SUM(bytes), 0) FROM queues",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let settled: i64 =
            conn.query_row("SELECT settled_receipts FROM mailbox_stats WHERE id = 1", [], |r| {
                r.get(0)
            })?;
        Ok(Totals {
            queues: from_sql(q),
            messages: from_sql(m),
            bytes: from_sql(b),
            settled_receipts: from_sql(settled),
        })
    }
}

/// Deletes receipts whose keep time ended, and keeps the settled count exact.
fn remove_expired_receipts(tx: &Transaction, now: u64) -> Result<u32, MailboxError> {
    let settled =
        tx.execute("DELETE FROM receipts WHERE expires_at <= ?1 AND settled = 1", [to_sql(now)])?;
    tx.execute(
        "UPDATE mailbox_stats SET settled_receipts = MAX(0, settled_receipts - ?1) WHERE id = 1",
        [to_sql(settled as u64)],
    )?;
    // A held message expires before its receipt does, so this is normally 0.
    let held =
        tx.execute("DELETE FROM receipts WHERE expires_at <= ?1 AND settled = 0", [to_sql(now)])?;
    Ok((settled + held) as u32)
}

fn sweep_expired_messages(
    tx: &Transaction,
    policy: ReceiptPolicy,
    now: u64,
) -> Result<u32, MailboxError> {
    let mut stmt = tx.prepare("SELECT seq FROM messages WHERE expires_at <= ?1 LIMIT 1000")?;
    let seqs: Vec<i64> = stmt.query_map([to_sql(now)], |r| r.get(0))?.collect::<Result<_, _>>()?;
    drop(stmt);

    let count = seqs.len() as u32;
    for seq in seqs {
        remove_message(tx, seq, &Receipt::Expired { at_secs: now }, policy, now)?;
    }
    Ok(count)
}

fn sweep_dead_queues(
    tx: &Transaction,
    lifetime_secs: u64,
    policy: ReceiptPolicy,
    now: u64,
) -> Result<u32, MailboxError> {
    let cutoff = now.saturating_sub(lifetime_secs);
    let mut stmt =
        tx.prepare("SELECT service_id FROM queues WHERE last_active_at <= ?1 LIMIT 100")?;
    let dead_services: Vec<String> =
        stmt.query_map([to_sql(cutoff)], |r| r.get(0))?.collect::<Result<_, _>>()?;
    drop(stmt);

    let mut deleted_queues = 0u32;
    let mut message_budget = 1000usize;

    for sid in dead_services {
        if message_budget > 0 {
            let mut stmt =
                tx.prepare("SELECT seq FROM messages WHERE service_id = ?1 ORDER BY seq LIMIT ?2")?;
            let seqs: Vec<i64> = stmt
                .query_map(rusqlite::params![&sid, message_budget as i64], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            drop(stmt);

            for seq in &seqs {
                remove_message(tx, *seq, &Receipt::Expired { at_secs: now }, policy, now)?;
            }
            message_budget = message_budget.saturating_sub(seqs.len());
        }

        let mut check_stmt = tx.prepare("SELECT 1 FROM messages WHERE service_id = ?1 LIMIT 1")?;
        let has_remaining = check_stmt.exists([&sid])?;
        drop(check_stmt);

        if !has_remaining {
            tx.execute("DELETE FROM queues WHERE service_id = ?1", [&sid])?;
            deleted_queues = deleted_queues.saturating_add(1);
        }
    }
    Ok(deleted_queues)
}

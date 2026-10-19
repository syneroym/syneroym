//! Queue registration and instance admission.

use rusqlite::Transaction;
use syneroym_core::mailbox::RegisterAnswer;
use syneroym_mailbox_format::Receipt;

use crate::{
    caller::Instance,
    error::MailboxError,
    record::CheckedRecord,
    store::{MailboxStore, QueueRow, from_sql, queue_row, remove_message, to_sql},
};

/// What a successful registration answers to the caller and to the background
/// loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Registered {
    pub answer: RegisterAnswer,
    pub publish_now: bool,
}

/// The newest-certificate rule: returns the queue row, recording any newer
/// certificate.
pub(super) fn admit_instance(tx: &Transaction, who: &Instance) -> Result<QueueRow, MailboxError> {
    let mut row = queue_row(tx, &who.master_did)?
        .ok_or_else(|| MailboxError::UnknownDestination(who.master_did.clone()))?;

    if who.instance_did == row.newest_instance_did {
        if who.issued_at_secs > row.newest_issued_at {
            tx.execute(
                "UPDATE queues SET newest_issued_at = ?1 WHERE service_id = ?2",
                rusqlite::params![to_sql(who.issued_at_secs), who.master_did],
            )?;
            row.newest_issued_at = who.issued_at_secs;
        }
        return Ok(row);
    }

    if who.issued_at_secs > row.newest_issued_at {
        tx.execute(
            "UPDATE queues SET newest_issued_at = ?1, newest_instance_did = ?2 WHERE service_id = \
             ?3",
            rusqlite::params![to_sql(who.issued_at_secs), who.instance_did, who.master_did],
        )?;
        row.newest_issued_at = who.issued_at_secs;
        row.newest_instance_did = who.instance_did.clone();
        return Ok(row);
    }

    Err(MailboxError::StaleInstance(who.master_did.clone()))
}

impl MailboxStore {
    /// Registers or updates a queue for the instance's master service.
    pub(crate) fn register(
        &self,
        who: &Instance,
        record: &CheckedRecord,
        now: u64,
    ) -> Result<Registered, MailboxError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let (publish_now, queue_limits) = match queue_row(&tx, &who.master_did)? {
            None => {
                let count: i64 = tx.query_row("SELECT COUNT(*) FROM queues", [], |r| r.get(0))?;
                if from_sql(count) >= u64::from(self.limits.max_queues) {
                    return Err(MailboxError::MailboxFull);
                }
                tx.execute(
                    "INSERT INTO queues (service_id, record_json, record_signed_at, \
                     record_not_after, newest_issued_at, newest_instance_did, max_bytes, \
                     max_messages, max_ttl_secs, last_active_at, next_publish_at) VALUES (?1, ?2, \
                     ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        who.master_did,
                        record.json,
                        to_sql(record.signed_at),
                        to_sql(record.not_after),
                        to_sql(who.issued_at_secs),
                        who.instance_did,
                        to_sql(self.limits.queue.max_bytes),
                        to_sql(u64::from(self.limits.queue.max_messages)),
                        to_sql(self.limits.queue.max_ttl_secs),
                        to_sql(now),
                        to_sql(now),
                    ],
                )?;
                (true, self.limits.queue)
            }
            Some(existing) => {
                let row = admit_instance(&tx, who)?;
                let publish = record.signed_at > existing.record_signed_at;
                if publish {
                    tx.execute(
                        "UPDATE queues SET record_json = ?1, record_signed_at = ?2, \
                         record_not_after = ?3, next_publish_at = ?4 WHERE service_id = ?5",
                        rusqlite::params![
                            record.json,
                            to_sql(record.signed_at),
                            to_sql(record.not_after),
                            to_sql(now),
                            who.master_did,
                        ],
                    )?;
                }
                tx.execute(
                    "UPDATE queues SET last_active_at = ?1 WHERE service_id = ?2",
                    rusqlite::params![to_sql(now), who.master_did],
                )?;
                (publish, row.limits)
            }
        };

        tx.commit()?;
        Ok(Registered {
            answer: RegisterAnswer {
                limits: queue_limits,
                queue_ends_at_secs: now.saturating_add(self.limits.queue_lifetime_secs),
            },
            publish_now,
        })
    }

    /// Removes a queue and all remaining messages in it.
    pub(crate) fn unregister(&self, who: &Instance, now: u64) -> Result<bool, MailboxError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        if queue_row(&tx, &who.master_did)?.is_none() {
            return Ok(false);
        }
        admit_instance(&tx, who)?;

        let mut stmt = tx.prepare("SELECT seq FROM messages WHERE service_id = ?1")?;
        let seqs: Vec<i64> =
            stmt.query_map([&who.master_did], |r| r.get(0))?.collect::<Result<_, _>>()?;
        drop(stmt);

        for seq in seqs {
            remove_message(
                &tx,
                seq,
                &Receipt::Expired { at_secs: now },
                self.limits.receipt_keep_secs,
                now,
            )?;
        }

        tx.execute("DELETE FROM queues WHERE service_id = ?1", [&who.master_did])?;
        tx.commit()?;
        Ok(true)
    }
}

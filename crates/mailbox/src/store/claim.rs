//! Storage operations for message claims, acks, and visibility timers.

use rusqlite::Transaction;
use syneroym_core::mailbox::{
    AckEntry, AckOutcome, ClaimAnswer, ClaimedMessage, MAX_ACK_BATCH, MAX_CLAIM_BATCH,
    MAX_CLAIM_BYTES, MAX_CLAIMS, MAX_RECEIPT_JSON_BYTES,
};
use syneroym_mailbox_format::{DepositId, Receipt, RejectCode, SealedBytes};
use tracing::warn;

use crate::{
    caller::Instance,
    error::MailboxError,
    store::{MailboxStore, from_sql, queue::admit_instance, remove_message, to_sql},
};

/// Closes messages that were claimed too many times without an acknowledgement.
pub(super) fn close_over_claimed(
    tx: &Transaction,
    queue: Option<&str>,
    keep_secs: u64,
    now: u64,
) -> Result<u32, MailboxError> {
    let sql = if queue.is_some() {
        "SELECT seq FROM messages WHERE claim_count >= ?1 AND claimed_until <= ?2 AND service_id = \
         ?3 LIMIT 1000"
    } else {
        "SELECT seq FROM messages WHERE claim_count >= ?1 AND claimed_until <= ?2 LIMIT 1000"
    };
    let mut stmt = tx.prepare(sql)?;
    let seqs: Vec<i64> = if let Some(q) = queue {
        stmt.query_map(rusqlite::params![to_sql(u64::from(MAX_CLAIMS)), to_sql(now), q], |r| {
            r.get(0)
        })?
        .collect::<Result<_, _>>()?
    } else {
        stmt.query_map(rusqlite::params![to_sql(u64::from(MAX_CLAIMS)), to_sql(now)], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    drop(stmt);

    let count = seqs.len() as u32;
    for seq in seqs {
        remove_message(
            tx,
            seq,
            &Receipt::unsigned_rejection(RejectCode::TooManyClaims, now),
            keep_secs,
            now,
        )?;
    }
    Ok(count)
}

struct MessageRow {
    seq: i64,
    sender: String,
    deposit_id: DepositId,
    sealed: String,
    deposited_at: u64,
    expires_at: u64,
    claim_count: u32,
}

impl MailboxStore {
    /// Claims a batch of visible messages for the instance's service.
    pub(crate) fn claim(
        &self,
        who: &Instance,
        max: u32,
        now: u64,
    ) -> Result<ClaimAnswer, MailboxError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let row = admit_instance(&tx, who)?;
        close_over_claimed(&tx, Some(&row.service_id), self.limits.receipt_keep_secs, now)?;

        let limit = max.clamp(1, MAX_CLAIM_BATCH);
        let candidates = select_claim_candidates(&tx, &row.service_id, now, limit)?;

        let mut taken = Vec::new();
        let mut total_bytes: usize = 0;

        for candidate in candidates {
            let sealed = match SealedBytes::try_from(candidate.sealed.clone()) {
                Ok(s) => s,
                Err(_) => {
                    warn!(
                        seq = candidate.seq,
                        "stored sealed text does not parse; closing as malformed"
                    );
                    remove_message(
                        &tx,
                        candidate.seq,
                        &Receipt::unsigned_rejection(RejectCode::Malformed, now),
                        self.limits.receipt_keep_secs,
                        now,
                    )?;
                    continue;
                }
            };

            let byte_len = sealed.len();
            if !taken.is_empty() && total_bytes.saturating_add(byte_len) > MAX_CLAIM_BYTES {
                break;
            }
            total_bytes = total_bytes.saturating_add(byte_len);
            taken.push((candidate, sealed));
        }

        let until = now.saturating_add(self.limits.visibility_secs);
        let mut messages = Vec::with_capacity(taken.len());

        for (cand, sealed) in taken {
            let new_count = cand.claim_count.saturating_add(1);
            tx.execute(
                "UPDATE messages SET claimed_until = ?1, claim_count = ?2 WHERE seq = ?3",
                rusqlite::params![to_sql(until), to_sql(u64::from(new_count)), cand.seq],
            )?;
            messages.push(ClaimedMessage {
                sender: cand.sender,
                deposit_id: cand.deposit_id,
                sealed,
                deposited_at_secs: cand.deposited_at,
                expires_at_secs: cand.expires_at,
                claim_count: new_count,
            });
        }

        tx.execute(
            "UPDATE queues SET last_active_at = ?1 WHERE service_id = ?2",
            rusqlite::params![to_sql(now), &row.service_id],
        )?;
        tx.commit()?;

        Ok(ClaimAnswer { messages, visible_again_at_secs: until })
    }

    /// Records receipts for messages processed by the receiving instance.
    pub(crate) fn ack(
        &self,
        who: &Instance,
        acks: &[AckEntry],
        now: u64,
    ) -> Result<Vec<AckOutcome>, MailboxError> {
        if acks.len() > MAX_ACK_BATCH {
            return Err(MailboxError::InvalidParams(format!(
                "ack batch size {} exceeds maximum of {}",
                acks.len(),
                MAX_ACK_BATCH
            )));
        }

        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row = admit_instance(&tx, who)?;

        let mut outcomes = Vec::with_capacity(acks.len());
        for entry in acks {
            if !matches!(entry.receipt, Receipt::Delivered { .. } | Receipt::Rejected { .. }) {
                outcomes.push(AckOutcome::Refused);
                continue;
            }
            let json = serde_json::to_string(&entry.receipt)
                .map_err(|e| MailboxError::Internal(format!("receipt serialize error: {e}")))?;
            if json.len() > MAX_RECEIPT_JSON_BYTES {
                outcomes.push(AckOutcome::Refused);
                continue;
            }

            let deposit_id_str = entry.deposit_id.to_string();
            let mut stmt = tx.prepare(
                "SELECT seq FROM messages WHERE service_id = ?1 AND sender = ?2 AND deposit_id = \
                 ?3",
            )?;
            let mut rows =
                stmt.query(rusqlite::params![&row.service_id, &entry.sender, &deposit_id_str,])?;
            let seq: Option<i64> = if let Some(r) = rows.next()? { Some(r.get(0)?) } else { None };
            drop(rows);
            drop(stmt);

            match seq {
                None => outcomes.push(AckOutcome::NotFound),
                Some(s) => {
                    remove_message(&tx, s, &entry.receipt, self.limits.receipt_keep_secs, now)?;
                    outcomes.push(AckOutcome::Stored);
                }
            }
        }

        tx.commit()?;
        Ok(outcomes)
    }
}

fn select_claim_candidates(
    tx: &Transaction,
    service_id: &str,
    now: u64,
    limit: u32,
) -> Result<Vec<MessageRow>, MailboxError> {
    let mut stmt = tx.prepare(
        "SELECT seq, sender, deposit_id, sealed, deposited_at, expires_at, claim_count FROM \
         messages WHERE service_id = ?1 AND claimed_until <= ?2 AND expires_at > ?2 AND \
         claim_count < ?3 ORDER BY seq LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![
            service_id,
            to_sql(now),
            to_sql(u64::from(MAX_CLAIMS)),
            to_sql(u64::from(limit)),
        ],
        |r| {
            let seq: i64 = r.get(0)?;
            let sender: String = r.get(1)?;
            let dep_str: String = r.get(2)?;
            let sealed: String = r.get(3)?;
            let deposited_at: i64 = r.get(4)?;
            let expires_at: i64 = r.get(5)?;
            let claim_count: i64 = r.get(6)?;
            let deposit_id = dep_str.parse::<DepositId>().map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid deposit id in database",
                    )),
                )
            })?;
            Ok(MessageRow {
                seq,
                sender,
                deposit_id,
                sealed,
                deposited_at: from_sql(deposited_at),
                expires_at: from_sql(expires_at),
                claim_count: from_sql(claim_count) as u32,
            })
        },
    )?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

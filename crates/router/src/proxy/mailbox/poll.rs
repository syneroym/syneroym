//! Asking a mailbox what became of a deposit, and judging the answer.

use std::collections::{BTreeMap, HashMap};

use syneroym_core::mailbox::{self as wire, MAX_STATUS_BATCH, StatusAnswer, StatusParams};
use syneroym_mailbox_format::{
    DepositId, MAX_CLOCK_SKEW_SECS, Receipt, SentDeposit, SignedOutcome,
};
use syneroym_rpc::DepositStatus;

use super::*;

/// Reading the receiver master's anchor before a receipt is accepted.
const RECEIVER_ANCHOR_TIMEOUT: Duration = Duration::from_secs(5);

fn retry(reason: &str) -> DepositStatus {
    DepositStatus::Retry { reason: reason.to_string() }
}

fn count_polled(result: &'static str) {
    metrics::counter!("substrate.proxy.mailbox.polled", "result" => result).increment(1);
}

impl ProxyRouter {
    /// One status for each stored deposit context, in the same order.
    pub(in crate::proxy) async fn poll_deposits_inner(
        &self,
        sender_service_id: &str,
        contexts: &[Vec<u8>],
    ) -> Vec<DepositStatus> {
        let mut out = vec![DepositStatus::Waiting; contexts.len()];
        // A sender with no valid certificate cannot ask now. The supervisor
        // renews it, and the deadline bounds the wait.
        let Some(sender) = self.sender_identity(sender_service_id) else { return out };

        let mut by_mailbox: BTreeMap<String, Vec<(usize, SentDeposit)>> = BTreeMap::new();
        for (i, bytes) in contexts.iter().enumerate() {
            match serde_json::from_slice::<DepositContext>(bytes) {
                Ok(ctx) => by_mailbox.entry(ctx.mailbox).or_default().push((i, ctx.sent)),
                Err(_) => out[i] = retry("the stored deposit context is unreadable"),
            }
        }
        for (mailbox, entries) in by_mailbox {
            for chunk in entries.chunks(MAX_STATUS_BATCH) {
                self.poll_chunk(&sender, &mailbox, chunk, &mut out).await;
            }
        }
        for status in &out {
            count_polled(match status {
                DepositStatus::Waiting => "waiting",
                DepositStatus::Delivered => "delivered",
                DepositStatus::Rejected { .. } => "rejected",
                DepositStatus::Retry { .. } => "retry",
            });
        }
        out
    }

    async fn poll_chunk(
        &self,
        sender: &DepositSender,
        mailbox: &str,
        chunk: &[(usize, SentDeposit)],
        out: &mut [DepositStatus],
    ) {
        let params =
            StatusParams { deposit_ids: chunk.iter().map(|(_, s)| s.deposit_id).collect() };
        let Ok(params) = serde_json::to_value(params) else { return };
        let call = Self::mailbox_request(sender, mailbox, wire::METHOD_STATUS, params);
        let answer = match self.invoke_inner(&call).await {
            Ok(value) => value,
            // The mailbox is down or refused us now: ask again later.
            Err(e) => {
                debug!(mailbox, error = %e, "could not ask the mailbox about deposits");
                return;
            }
        };
        let Ok(answer) = serde_json::from_value::<StatusAnswer>(answer) else {
            for (i, _) in chunk {
                out[*i] = retry("the mailbox status answer does not parse");
            }
            return;
        };
        let receipts: HashMap<DepositId, Receipt> =
            answer.receipts.into_iter().map(|e| (e.deposit_id, e.receipt)).collect();
        let now = now_secs();
        for (i, sent) in chunk {
            out[*i] = self.judge(receipts.get(&sent.deposit_id), sent, now).await;
        }
    }

    /// Turns one receipt into one status. Only a receipt the receiver
    /// signed, for exactly this message, ends the message.
    pub(in crate::proxy) async fn judge(
        &self,
        receipt: Option<&Receipt>,
        sent: &SentDeposit,
        now: u64,
    ) -> DepositStatus {
        let receipt = match receipt {
            None => return retry("the mailbox has no receipt for this deposit"),
            Some(Receipt::Stored { .. }) => return DepositStatus::Waiting,
            Some(receipt) => receipt,
        };
        let outcome = match receipt.verify_signed(sent, MAX_CLOCK_SKEW_SECS, now) {
            Ok(outcome) => outcome,
            Err(e) => return retry(&format!("receipt not accepted: {e}")),
        };
        match self.receiver_key_revoked(receipt, sent).await {
            Ok(false) => {}
            Ok(true) => return retry("the receiver key that signed the receipt is revoked"),
            Err(_) => return DepositStatus::Waiting,
        }
        match outcome {
            SignedOutcome::Delivered => DepositStatus::Delivered,
            SignedOutcome::Rejected(code) => {
                DepositStatus::Rejected { code: code.as_str().to_string() }
            }
        }
    }

    /// Whether the instance key that signed `receipt` is in the receiver
    /// master's revoked keys. `verify_signed` checks only the receipt time
    /// against the certificate window.
    async fn receiver_key_revoked(
        &self,
        receipt: &Receipt,
        sent: &SentDeposit,
    ) -> anyhow::Result<bool> {
        let (Receipt::Delivered { proof, .. } | Receipt::Rejected { proof: Some(proof), .. }) =
            receipt
        else {
            return Ok(false);
        };
        let anchor = time::timeout(
            RECEIVER_ANCHOR_TIMEOUT,
            self.anchor_resolver.resolve_master_anchor(&sent.receiver_service_id),
        )
        .await??;
        Ok(anchor.revoked_keys.contains(&proof.receiver_certificate.temporary_did))
    }
}

//! The outbox rows that wait at a peer's mailbox: one poll for all of them
//! on each tick, and what each answer means for the message.

use syneroym_async_queue::{Parked, QueueItem};
use syneroym_rpc::{ConversationDeliveryState, DepositStatus};

use super::Row;
use crate::{
    ConversationService,
    store::{ConversationStore, OutboxItem, StoredMessage, now_ms},
};

/// A claimed parked row that still waits for a receipt.
struct ParkedRow {
    item: QueueItem,
    parked: Parked,
    out: OutboxItem,
    msg: StoredMessage,
}

impl ParkedRow {
    fn as_row<'a>(&'a self, svc: &'a str, store: &'a ConversationStore) -> Row<'a> {
        Row { svc, store, item: &self.item, msg: &self.msg, out: &self.out }
    }
}

impl ConversationService {
    pub(super) async fn drain_parked_once(&self) {
        for svc in self.candidate_service_ids() {
            let still_deployed = self.is_deployed(&svc);
            if let Ok(store) = self.store_for(&svc).await {
                self.drain_parked_one(&svc, &store, still_deployed).await;
            }
        }
    }

    async fn drain_parked_one(&self, svc: &str, store: &ConversationStore, still_deployed: bool) {
        let now = now_ms();
        let Ok(items) = store.queue().claim_parked_due(now, super::CLAIM_LIMIT_PER_TICK) else {
            return;
        };
        let mut waiting = Vec::new();
        for item in items {
            if let Some(row) = self.check_parked(svc, store, still_deployed, item, now).await {
                waiting.push(row);
            }
        }
        if waiting.is_empty() {
            return;
        }
        let Some(proxy) = self.current_service_proxy().upgrade() else { return };
        let contexts = waiting.iter().map(|r| r.parked.context.clone()).collect();
        let statuses = proxy.poll_deposits(svc, contexts).await;
        for (row, status) in waiting.iter().zip(statuses) {
            self.settle_parked(&row.as_row(svc, store), &row.parked, status, now).await;
        }
    }

    /// The row, when it still waits for a receipt. Settles the row itself,
    /// and returns `None`, when it needs no poll.
    async fn check_parked(
        &self,
        svc: &str,
        store: &ConversationStore,
        still_deployed: bool,
        item: QueueItem,
        now: i64,
    ) -> Option<ParkedRow> {
        let Some(parked) = item.parked.clone() else {
            let _ = store.queue().complete(item.id);
            return None;
        };
        if !still_deployed {
            let _ = store.queue().complete(item.id);
            return None;
        }
        let Ok(out) = serde_json::from_slice::<OutboxItem>(&item.payload) else {
            let _ = store.queue().fail(item.id, now, "queued payload is unreadable", true);
            return None;
        };
        let is_group = out.group.is_some();
        let msg = store.get_message(&out.message_id).ok().flatten();
        let Some(msg) = msg.filter(|m| !m.restored && is_deposited(store, m, &out)) else {
            // Deleted, restored, or settled some other way while it waited.
            if !is_group {
                let _ = store.delete_outbound_envelope(&out.message_id);
            }
            let _ = store.queue().complete(item.id);
            return None;
        };
        let max_age_ms = (store.config().max_pending_age_secs as i64).saturating_mul(1000);
        if now.saturating_sub(msg.received_at_ms) > max_age_ms {
            self.settle_failed(
                svc,
                store,
                &item,
                "recipient never became reachable",
                is_group,
                &out.peer_address,
            )
            .await;
            return None;
        }
        Some(ParkedRow { item, parked, out, msg })
    }

    async fn settle_parked(&self, row: &Row<'_>, parked: &Parked, status: DepositStatus, now: i64) {
        let Row { svc, store, item, msg, out } = row;
        let (is_group, peer) = (row.is_group(), out.peer_address.as_str());
        match status {
            DepositStatus::Delivered => {
                self.settle_delivered(svc, store, item, msg, is_group, peer).await;
            }
            DepositStatus::Rejected { code } => {
                let reason = format!("the receiver refused the message: {code}");
                self.settle_failed(svc, store, item, &reason, is_group, peer).await;
            }
            DepositStatus::Retry { .. } => self.back_to_direct(row, now).await,
            DepositStatus::Waiting if parked.is_past_deadline(now) => {
                self.back_to_direct(row, now).await;
            }
            DepositStatus::Waiting => {
                let _ = store.queue().park(
                    item.id,
                    now,
                    parked.poll_interval_ms,
                    parked.deadline_at,
                    &parked.context,
                );
            }
        }
    }

    /// The mailbox did not take the message after all: deliver it directly
    /// again, with the envelope already made.
    async fn back_to_direct(&self, row: &Row<'_>, now: i64) {
        match row.store.unpark_to_pending(row.item.id, &row.msg.id, row.member(), now) {
            Ok(true) if !row.is_group() => {
                self.notify_state(row.svc, row.msg.id.clone(), ConversationDeliveryState::Pending)
                    .await;
            }
            // Deleted or restored while the poll ran: it must not be sent.
            Ok(false) => self.drop_row(row),
            Ok(true) | Err(_) => {}
        }
    }
}

/// Whether the message (1:1) or this member's copy (group) is still
/// `deposited`.
fn is_deposited(store: &ConversationStore, msg: &StoredMessage, out: &OutboxItem) -> bool {
    if out.group.is_some() {
        matches!(
            store.recipient_state(&msg.id, &out.peer_address),
            Ok(Some(ConversationDeliveryState::Deposited))
        )
    } else {
        msg.state == ConversationDeliveryState::Deposited
    }
}

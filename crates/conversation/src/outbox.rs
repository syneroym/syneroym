//! The delivery worker: drains every open service's conversation outbox on
//! a tick, mirroring `syneroym_router::proxy_outbox`'s three subtleties —
//! a claim that never resolves is bounded by `claim_count`, not
//! `attempts`; an unreadable payload is terminal; a target that no longer
//! resolves is terminal on its own terms.

use std::time::Duration;

use syneroym_async_queue::{FailOutcome, QueueItem};
use syneroym_rpc::ConversationDeliveryState;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    ConversationService,
    dag::EntryKind,
    store::{ConversationStore, OutboxItem, now_ms},
    transport::Disposition,
};

/// How many items one service may have claimed in a single worker tick —
/// raised to 64 per D-B5-13 because group fan-out multiplies outbox rows by
/// member count.
const CLAIM_LIMIT_PER_TICK: u32 = 64;

impl ConversationService {
    /// Runs until `cancel` fires. Spawned once, beside
    /// `proxy_outbox_join`.
    pub async fn run_worker(self: std::sync::Arc<Self>, tick: Duration, cancel: CancellationToken) {
        let mut sync_tick_count = 0u64;
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(tick) => {
                    self.drain_once().await;
                    self.drain_relay_pending_once().await;
                    self.scheduled_rekey_once().await;
                    sync_tick_count = sync_tick_count.wrapping_add(1);
                    self.periodic_group_sync_once(tick, sync_tick_count).await;
                }
            }
        }
    }

    async fn periodic_group_sync_once(&self, tick: Duration, tick_count: u64) {
        let services = self.candidate_service_ids();
        for svc in services {
            if let Ok(store) = self.store_for(&svc).await {
                let interval_secs = store.config().conversation_group_sync_secs;
                if interval_secs == 0 {
                    continue;
                }
                let tick_secs = tick.as_secs().max(1);
                let ticks_per_sync = (interval_secs / tick_secs).max(1);
                if !tick_count.is_multiple_of(ticks_per_sync) {
                    continue;
                }
                if let Ok(convs) = store.group_conversations() {
                    for conv in convs {
                        let _ = self
                            .periodic_group_sync_pass(&svc, &conv.id, tick_count as usize)
                            .await;
                    }
                }
            }
        }
    }

    async fn drain_once(&self) {
        let services = self.candidate_service_ids();
        for svc in services {
            let still_deployed = self.registry.owner_of(&svc).is_some()
                || self.registry.instance_cert(&svc).is_some();
            if let Ok(store) = self.store_for(&svc).await {
                self.drain_one(&svc, &store, still_deployed).await;
            }
        }
    }

    async fn drain_relay_pending_once(&self) {
        let services = self.candidate_service_ids();
        for svc in services {
            if let Ok(store) = self.store_for(&svc).await
                && let Ok(entries) = store.claim_relay_pending(CLAIM_LIMIT_PER_TICK)
            {
                let fanout = store.config().conversation_relay_fanout.max(1) as usize;
                for entry in entries {
                    if let Ok(members) = store.current_members(&entry.conversation_id) {
                        let wire = entry.into_wire();
                        let mut targets: Vec<String> = members
                            .into_iter()
                            .filter(|m| m != &svc && m != &wire.author)
                            .collect();
                        if targets.len() > fanout {
                            use rand::seq::SliceRandom;
                            let mut rng = rand::rng();
                            targets.shuffle(&mut rng);
                            targets.truncate(fanout);
                        }
                        // A `remove` membership entry is relayed to the fanout
                        // sample above *plus* the member it just removed —
                        // added after truncation, not before, so it is never
                        // the truncated-away entry it would otherwise often be
                        // on a group larger than the fanout. Otherwise the
                        // removed member never learns of their own removal
                        // (relay targets are `current_members` evaluated at
                        // push time, which already excludes them by then) and
                        // keeps using the last epoch key it was ever handed.
                        // This still gives it no way to decrypt anything past
                        // that point — the owner never sends the new epoch's
                        // key to a removed member — only the membership fact
                        // itself, so its own client can update local state and
                        // refuse to keep posting.
                        if let (EntryKind::Membership, Some(payload)) = (wire.kind, &wire.payload)
                            && payload.action == "remove"
                            && payload.subject_address != svc
                            && payload.subject_address != wire.author
                            && !targets.contains(&payload.subject_address)
                        {
                            targets.push(payload.subject_address.clone());
                        }
                        for m in targets {
                            let _ = self.push_group_entry(&store, &svc, &m, &wire).await;
                        }
                    }
                }
            }
        }
    }

    async fn drain_one(&self, svc: &str, store: &ConversationStore, still_deployed: bool) {
        let now = now_ms();
        let Ok(items) = store.queue().claim_due(now, CLAIM_LIMIT_PER_TICK) else { return };
        for item in items {
            self.drain_item(svc, store, still_deployed, item, now).await;
        }
    }

    async fn drain_item(
        &self,
        svc: &str,
        store: &ConversationStore,
        still_deployed: bool,
        item: QueueItem,
        now: i64,
    ) {
        if !still_deployed {
            let _ = store.queue().complete(item.id);
            return;
        }
        let Ok(parsed) = serde_json::from_slice::<OutboxItem>(&item.payload) else {
            let _ = store.queue().fail(item.id, now, "queued payload is unreadable", true);
            return;
        };
        let is_group = parsed.group.is_some();
        if item.claim_count > u32::from(store.queue().max_attempts()) {
            self.settle_failed(
                svc,
                store,
                &item,
                "claimed repeatedly without ever completing",
                is_group,
                &parsed.peer_address,
            )
            .await;
            return;
        }
        let Ok(Some(msg)) = store.get_message(&parsed.message_id) else {
            let _ = store.queue().complete(item.id);
            return;
        };
        if msg.state != ConversationDeliveryState::Pending {
            let _ = store.queue().complete(item.id);
            return;
        }
        let max_age_ms = (store.config().max_pending_age_secs as i64).saturating_mul(1000);
        if now.saturating_sub(msg.received_at_ms) > max_age_ms {
            self.settle_failed(
                svc,
                store,
                &item,
                "recipient never became reachable",
                is_group,
                &parsed.peer_address,
            )
            .await;
            return;
        }

        let delivery_result = if is_group {
            self.deliver_group_one(svc, &parsed.peer_address, &msg).await
        } else {
            self.deliver_one(svc, &parsed.peer_address, &msg).await
        };

        match delivery_result {
            Ok(()) | Err(Disposition::Delivered) => {
                if is_group {
                    let _ = store.set_recipient_state(
                        &msg.id,
                        &parsed.peer_address,
                        ConversationDeliveryState::Delivered,
                        None,
                    );
                    let _ = store.queue().complete(item.id);
                    if store.recipients_remaining(&msg.id).unwrap_or(0) == 0 {
                        if store.any_recipient_failed(&msg.id).unwrap_or(false) {
                            let _ = store.set_state(
                                &msg.id,
                                ConversationDeliveryState::Failed,
                                Some("one or more recipients failed"),
                            );
                            self.notify_state(
                                svc,
                                msg.id.clone(),
                                ConversationDeliveryState::Failed,
                            )
                            .await;
                        } else {
                            let _ = store.set_state(
                                &msg.id,
                                ConversationDeliveryState::Delivered,
                                None,
                            );
                            self.notify_state(
                                svc,
                                msg.id.clone(),
                                ConversationDeliveryState::Delivered,
                            )
                            .await;
                        }
                    }
                } else {
                    let _ = store.set_state(&msg.id, ConversationDeliveryState::Delivered, None);
                    self.notify_state(svc, msg.id.clone(), ConversationDeliveryState::Delivered)
                        .await;
                    let _ = store.queue().complete(item.id);
                }
            }
            Err(Disposition::Unreachable) => {
                // `defer` un-counts the claim and does not advance
                // `attempts` by design, so the queue has no counter that
                // survives a defer. We derive the backoff exponentially
                // from the message's elapsed age since creation.
                let age_ms = now.saturating_sub(msg.received_at_ms);
                let backoff_ms = backoff_for_age(age_ms);
                let _ = store.queue().defer(item.id, now + backoff_ms);
            }
            Err(Disposition::Terminal(reason)) => {
                self.settle_failed(svc, store, &item, &reason, is_group, &parsed.peer_address)
                    .await;
            }
            Err(Disposition::Retry) => {
                match store.queue().fail(item.id, now, "transport error", false) {
                    Ok(FailOutcome::DeadLettered { .. }) => {
                        self.finish_item_failed(
                            svc,
                            store,
                            &msg.id,
                            "delivery attempts exhausted",
                            is_group,
                            &parsed.peer_address,
                        )
                        .await;
                    }
                    Ok(FailOutcome::Retrying { .. }) | Err(_) => {}
                }
            }
        }
    }

    async fn settle_failed(
        &self,
        svc: &str,
        store: &ConversationStore,
        item: &QueueItem,
        reason: &str,
        is_group: bool,
        peer_address: &str,
    ) {
        let _ = store.queue().fail(item.id, now_ms(), reason, true);
        if let Ok(parsed) = serde_json::from_slice::<OutboxItem>(&item.payload) {
            self.finish_item_failed(svc, store, &parsed.message_id, reason, is_group, peer_address)
                .await;
        }
    }

    async fn finish_item_failed(
        &self,
        svc: &str,
        store: &ConversationStore,
        message_id: &str,
        reason: &str,
        is_group: bool,
        peer_address: &str,
    ) {
        if is_group {
            let _ = store.set_recipient_state(
                message_id,
                peer_address,
                ConversationDeliveryState::Failed,
                Some(reason),
            );
            if store.recipients_remaining(message_id).unwrap_or(0) == 0 {
                let _ = store.set_state(
                    message_id,
                    ConversationDeliveryState::Failed,
                    Some("one or more recipients failed"),
                );
                self.notify_state(svc, message_id.to_string(), ConversationDeliveryState::Failed)
                    .await;
                metrics::counter!("substrate.conversation.outbox.dead_lettered").increment(1);
                warn!(
                    service = svc,
                    message = message_id,
                    error = reason,
                    "group conversation delivery gave up"
                );
            }
        } else {
            let _ = store.set_state(message_id, ConversationDeliveryState::Failed, Some(reason));
            self.notify_state(svc, message_id.to_string(), ConversationDeliveryState::Failed).await;
            metrics::counter!("substrate.conversation.outbox.dead_lettered").increment(1);
            warn!(
                service = svc,
                message = message_id,
                error = reason,
                "conversation delivery gave up"
            );
        }
    }
}

#[must_use]
fn backoff_for_age(age_ms: i64) -> i64 {
    // Start at 1s, double with elapsed time, capped at 300s (5 minutes).
    // Steps every 1s, 2s, 4s, 8s, 16s, 32s, 64s, 128s, 256s, 512s of age.
    let step = if age_ms < 1_000 {
        0
    } else if age_ms < 3_000 {
        1
    } else if age_ms < 7_000 {
        2
    } else if age_ms < 15_000 {
        3
    } else if age_ms < 31_000 {
        4
    } else if age_ms < 63_000 {
        5
    } else if age_ms < 127_000 {
        6
    } else if age_ms < 255_000 {
        7
    } else if age_ms < 511_000 {
        8
    } else {
        9
    };
    let base = 1_000i64.saturating_mul(1i64 << step);
    base.min(300_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_curve_grows_with_age_and_caps() {
        assert_eq!(backoff_for_age(0), 1_000);
        assert_eq!(backoff_for_age(500), 1_000);
        assert_eq!(backoff_for_age(1_500), 2_000);
        assert_eq!(backoff_for_age(4_000), 4_000);
        assert_eq!(backoff_for_age(10_000), 8_000);
        assert_eq!(backoff_for_age(20_000), 16_000);
        assert_eq!(backoff_for_age(100_000), 64_000);
        assert_eq!(backoff_for_age(300_000), 256_000);
        assert_eq!(backoff_for_age(600_000), 300_000);
        assert_eq!(backoff_for_age(1_000_000), 300_000);
    }
}

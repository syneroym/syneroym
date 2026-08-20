//! The delivery worker: drains every open service's conversation outbox on
//! a tick, mirroring `syneroym_router::proxy_outbox`'s three subtleties
//! (F1) -- a claim that never resolves is bounded by `claim_count`, not
//! `attempts`; an unreadable payload is terminal; a target that no longer
//! resolves is terminal on its own terms.

use std::time::Duration;

use syneroym_async_queue::{FailOutcome, QueueItem};
use syneroym_rpc::ConversationDeliveryState;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    ConversationService,
    store::{ConversationStore, OutboxItem, now_ms},
    transport::Disposition,
};

/// How many items one service may have claimed in a single worker tick --
/// matches `syneroym_router::proxy_outbox::CLAIM_LIMIT_PER_TICK`'s
/// reasoning: one service with a permanently unreachable target must not
/// spend the whole node's tick budget on its own backlog.
const CLAIM_LIMIT_PER_TICK: u32 = 16;

impl ConversationService {
    /// Runs until `cancel` fires. Spawned once, beside
    /// `proxy_outbox_join` (`D-B4-19`).
    pub async fn run_worker(self: std::sync::Arc<Self>, tick: Duration, cancel: CancellationToken) {
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(tick) => {
                    self.drain_once().await;
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
        if item.claim_count > u32::from(store.queue().max_attempts()) {
            self.settle_failed(svc, store, &item, "claimed repeatedly without ever completing")
                .await;
            return;
        }
        let Ok(parsed) = serde_json::from_slice::<OutboxItem>(&item.payload) else {
            let _ = store.queue().fail(item.id, now, "queued payload is unreadable", true);
            return;
        };
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
            self.settle_failed(svc, store, &item, "recipient never became reachable").await;
            return;
        }

        match self.deliver_one(svc, &parsed.peer_address, &msg).await {
            Ok(()) => {
                let _ = store.set_state(&msg.id, ConversationDeliveryState::Delivered, None);
                self.notify_state(svc, msg.id.clone(), ConversationDeliveryState::Delivered).await;
                let _ = store.queue().complete(item.id);
            }
            Err(Disposition::Unreachable) => {
                let backoff_ms = backoff_for(item.attempts);
                let _ = store.queue().defer(item.id, now + backoff_ms);
            }
            Err(Disposition::Terminal) => {
                self.settle_failed(svc, store, &item, "delivery refused or the target is invalid")
                    .await;
            }
            Err(Disposition::Retry) => {
                match store.queue().fail(item.id, now, "transport error", false) {
                    Ok(FailOutcome::DeadLettered { .. }) => {
                        self.finish_failed(svc, store, &msg.id, "delivery attempts exhausted")
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
    ) {
        let _ = store.queue().fail(item.id, now_ms(), reason, true);
        if let Ok(parsed) = serde_json::from_slice::<OutboxItem>(&item.payload) {
            self.finish_failed(svc, store, &parsed.message_id, reason).await;
        }
    }

    async fn finish_failed(
        &self,
        svc: &str,
        store: &ConversationStore,
        message_id: &str,
        reason: &str,
    ) {
        let _ = store.set_state(message_id, ConversationDeliveryState::Failed, Some(reason));
        self.notify_state(svc, message_id.to_string(), ConversationDeliveryState::Failed).await;
        metrics::counter!("substrate.conversation.outbox.dead_lettered").increment(1);
        warn!(service = svc, message = message_id, error = reason, "conversation delivery gave up");
        // No `AlertStore` write -- `D-B4-25`: the existing proxy outbox
        // gives a dead letter the same treatment (metric + log), and a
        // node running a conversation service and no supervisor role has
        // no `AlertStore` to write to at all.
    }
}

#[must_use]
fn backoff_for(attempts: u32) -> i64 {
    let base = 1_000i64.saturating_mul(1i64 << attempts.min(10));
    base.min(300_000)
}

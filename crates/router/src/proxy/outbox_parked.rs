//! The outbox rows that wait at a receiver's mailbox: the poll pass, and
//! the small row operations both passes share.

use syneroym_async_queue::Parked;
use syneroym_rpc::DepositTicket;

use super::*;

pub(super) async fn complete_row(queue: &Queue, id: i64) {
    let queue = queue.clone();
    let _ = task::spawn_blocking(move || queue.complete(id)).await;
}

pub(super) async fn delivered_row(queue: &Queue, id: i64) {
    metrics::counter!("substrate.proxy.outbox.delivered").increment(1);
    complete_row(queue, id).await;
}

pub(super) async fn fail_row(queue: &Queue, id: i64, now: i64, message: &str, terminal: bool) {
    let queue = queue.clone();
    let message = message.to_string();
    let outcome = task::spawn_blocking(move || queue.fail(id, now, &message, terminal)).await;
    if let Ok(Ok(FailOutcome::DeadLettered { .. })) = outcome {
        metrics::counter!("substrate.proxy.outbox.dead_lettered").increment(1);
    }
}

/// Parks `item` for the deposit `ticket` describes. The deadline never goes
/// past the row's age limit. When the row cannot be parked it stays claimed
/// and comes back after the visibility timeout; a second deposit is safe,
/// because the receiver fence runs the call once.
pub(super) async fn park_row(queue: &Queue, item: &QueueItem, now: i64, ticket: &DepositTicket) {
    let age_limit = item.created_at.saturating_add(proxy_outbox::MAX_ROW_AGE_MS);
    let deadline_at = ticket.deadline_at_ms.min(age_limit).max(now + 1);
    let poll_ms = u64::try_from(ticket.poll_interval.as_millis()).unwrap_or(u64::MAX);
    let (queue, id, context) = (queue.clone(), item.id, ticket.context.clone());
    let parked =
        task::spawn_blocking(move || queue.park(id, now, poll_ms, deadline_at, &context)).await;
    if !matches!(parked, Ok(Ok(()))) {
        warn!(id, "could not park a deposited call; it will be tried again");
    }
}

async fn unpark_row(queue: &Queue, id: i64, now: i64) {
    let queue = queue.clone();
    let _ = task::spawn_blocking(move || queue.unpark(id, now)).await;
}

async fn repark_row(queue: &Queue, id: i64, now: i64, parked: &Parked) {
    let (queue, parked) = (queue.clone(), parked.clone());
    let _ = task::spawn_blocking(move || {
        queue.park(id, now, parked.poll_interval_ms, parked.deadline_at, &parked.context)
    })
    .await;
}

impl ProxyRouter {
    /// Asks the mailboxes about every parked row that is due. Returns how
    /// many rows it looked at.
    pub(super) async fn drain_parked_outbox(
        &self,
        queue: &Queue,
        service_id: &str,
        still_deployed: bool,
    ) -> usize {
        let now = proxy_outbox::now_ms();
        let claimed = {
            let queue = queue.clone();
            match task::spawn_blocking(move || {
                queue.claim_parked_due(now, proxy_outbox::CLAIM_LIMIT_PER_TICK)
            })
            .await
            {
                Ok(Ok(items)) => items,
                _ => return 0,
            }
        };
        let count = claimed.len();
        if !still_deployed {
            for item in claimed {
                complete_row(queue, item.id).await;
            }
            return count;
        }
        let rows: Vec<(QueueItem, Parked)> = claimed
            .into_iter()
            .filter_map(|item| item.parked.clone().map(|parked| (item, parked)))
            .collect();
        let contexts = rows.iter().map(|(_, parked)| parked.context.clone()).collect::<Vec<_>>();
        let statuses = self.poll_deposits_inner(service_id, &contexts).await;
        for ((item, parked), status) in rows.into_iter().zip(statuses) {
            Self::settle_parked(queue, &item, &parked, status, now).await;
        }
        count
    }

    /// One parked row and what its mailbox said. Never calls a non-terminal
    /// `fail` on a parked row: that would clear the parked state.
    async fn settle_parked(
        queue: &Queue,
        item: &QueueItem,
        parked: &Parked,
        status: DepositStatus,
        now: i64,
    ) {
        let too_old = now - item.created_at > proxy_outbox::MAX_ROW_AGE_MS;
        match status {
            DepositStatus::Delivered => delivered_row(queue, item.id).await,
            DepositStatus::Rejected { code } => {
                let message = format!("the receiver refused the call: {code}");
                fail_row(queue, item.id, now, &message, true).await;
            }
            _ if too_old => {
                fail_row(queue, item.id, now, "the receiver never took this call", true).await;
            }
            DepositStatus::Retry { reason } => {
                debug!(id = item.id, reason, "a deposit goes back to direct attempts");
                unpark_row(queue, item.id, now).await;
            }
            DepositStatus::Waiting if parked.is_past_deadline(now) => {
                unpark_row(queue, item.id, now).await;
            }
            DepositStatus::Waiting => repark_row(queue, item.id, now, parked).await,
        }
    }
}

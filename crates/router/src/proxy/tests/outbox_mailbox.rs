//! The proxy outbox with a receiver that has a mailbox and no address.

use serde_json::json;
use syneroym_async_queue::{DeadLetter, Queue, QueueItem};
use syneroym_core::mailbox::codes;
use tokio::time;

use super::{helpers::*, mailbox_helpers::*};
use crate::proxy_outbox::{MAX_ROW_AGE_MS, now_ms};

const THIRTY_ONE_DAYS_MS: i64 = 31 * 24 * 3600 * 1000;

fn call_for(node: &SenderNode, key: &str) -> QueuedCall {
    QueuedCall {
        app_instance_id: None,
        caller_service_id: SENDER.to_string(),
        target: QueuedTarget::Service(node.receiver.service_id()),
        routing_key: None,
        interface: "greeter".to_string(),
        method: "greet".to_string(),
        params: json!({ "who": "world" }),
        idempotency_key: key.to_string(),
        protocol: None,
        timeout_ms: Some(500),
    }
}

async fn queue_of(node: &SenderNode) -> Queue {
    node.outbox.as_ref().unwrap().queue_for(SENDER).await.unwrap()
}

async fn rows(node: &SenderNode) -> Vec<QueueItem> {
    queue_of(node).await.all().unwrap()
}

async fn dead_letters(node: &SenderNode) -> Vec<DeadLetter> {
    queue_of(node).await.dead_letters().unwrap()
}

/// Enqueues one call and runs the worker once, so the call is deposited.
async fn deposited_call(node: &SenderNode) {
    node.router.enqueue(call_for(node, "k1")).await.unwrap();
    node.router.drain_outboxes_once().await;
}

/// The poll time of every parked row has come.
async fn make_parked_rows_due(node: &SenderNode) {
    let queue = queue_of(node).await;
    for item in queue.all().unwrap() {
        if let Some(parked) = item.parked {
            queue.park(item.id, now_ms(), 1, parked.deadline_at, &parked.context).unwrap();
        }
    }
    time::sleep(Duration::from_millis(5)).await;
}

#[tokio::test]
async fn a_queued_call_to_an_unreachable_receiver_with_a_mailbox_is_parked_as_deposited() {
    let node = outbox_sender_node(3).await;

    deposited_call(&node).await;

    let rows = rows(&node).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].parked.is_some(), "the row waits at the mailbox");
    assert_eq!(node.mailbox.count("deposit"), 1);
    let listed = node.outbox.as_ref().unwrap().queued_calls(SENDER).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].deposited);
}

#[tokio::test]
async fn a_deposited_row_stays_deposited_past_its_attempt_budget() {
    let node = outbox_sender_node(2).await;
    deposited_call(&node).await;

    for _ in 0..6 {
        node.router.drain_outboxes_once().await;
        make_parked_rows_due(&node).await;
    }

    let rows = rows(&node).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].parked.is_some());
    assert_eq!(rows[0].attempts, 0, "waiting at a mailbox uses no attempt");
    assert!(dead_letters(&node).await.is_empty());
    assert_eq!(node.mailbox.count("deposit"), 1, "no second deposit while it waits");
}

#[tokio::test]
async fn a_signed_delivery_completes_the_row_without_a_dead_letter() {
    let node = outbox_sender_node(3).await;
    deposited_call(&node).await;
    node.receiver_delivers().await;
    make_parked_rows_due(&node).await;

    node.router.drain_outboxes_once().await;

    assert!(rows(&node).await.is_empty());
    assert!(dead_letters(&node).await.is_empty());
}

#[tokio::test]
async fn a_signed_rejection_dead_letters_the_row_with_its_code() {
    let node = outbox_sender_node(3).await;
    deposited_call(&node).await;
    node.receiver_rejects_as_not_eligible().await;
    make_parked_rows_due(&node).await;

    node.router.drain_outboxes_once().await;

    assert!(rows(&node).await.is_empty());
    let letters = dead_letters(&node).await;
    assert_eq!(letters.len(), 1);
    assert!(letters[0].last_error.contains("not_eligible"), "{}", letters[0].last_error);
}

#[tokio::test]
async fn a_receipt_that_cannot_be_trusted_sends_the_row_back_to_direct_attempts() {
    let node = outbox_sender_node(3).await;
    deposited_call(&node).await;
    node.receiver_delivers().await;
    node.router_anchors.revoked.lock().unwrap().push(node.receiver.cert.temporary_did.clone());
    make_parked_rows_due(&node).await;

    node.router.drain_outboxes_once().await;

    let pending = rows(&node).await;
    assert_eq!(pending.len(), 1);
    assert!(pending[0].parked.is_none(), "the row is pending again");
    assert!(dead_letters(&node).await.is_empty());

    // The next pass delivers again, which here is a new deposit.
    node.router.drain_outboxes_once().await;
    assert_eq!(node.mailbox.count("deposit"), 2);
    assert!(rows(&node).await[0].parked.is_some());
}

#[tokio::test]
async fn a_deposit_that_is_still_waiting_past_its_deadline_goes_back_to_direct_attempts() {
    let node = outbox_sender_node(3).await;
    deposited_call(&node).await;
    let queue = queue_of(&node).await;
    let item = queue.all().unwrap().remove(0);
    let parked = item.parked.unwrap();
    queue.park(item.id, now_ms(), 1, now_ms() + 3, &parked.context).unwrap();
    time::sleep(Duration::from_millis(10)).await;

    node.router.drain_outboxes_once().await;

    assert!(rows(&node).await[0].parked.is_none());
}

#[tokio::test]
async fn a_pending_row_older_than_the_age_limit_is_dead_lettered() {
    let node = outbox_sender_node(3).await;
    let payload = serde_json::to_vec(&call_for(&node, "old")).unwrap();
    let queue = queue_of(&node).await;
    queue.enqueue("group", "old", &payload, now_ms() - THIRTY_ONE_DAYS_MS).unwrap();

    node.router.drain_outboxes_once().await;

    assert!(rows(&node).await.is_empty());
    let letters = dead_letters(&node).await;
    assert_eq!(letters.len(), 1);
    assert!(letters[0].last_error.contains("never took"), "{}", letters[0].last_error);
}

#[tokio::test]
async fn a_parked_row_older_than_the_age_limit_is_dead_lettered() {
    let node = outbox_sender_node(3).await;
    let payload = serde_json::to_vec(&call_for(&node, "old")).unwrap();
    let queue = queue_of(&node).await;
    let id = queue.enqueue("group", "old", &payload, now_ms() - THIRTY_ONE_DAYS_MS).unwrap();
    queue.park(id, now_ms(), 1, now_ms() + MAX_ROW_AGE_MS, b"not a context").unwrap();
    time::sleep(Duration::from_millis(5)).await;

    node.router.drain_outboxes_once().await;

    assert!(rows(&node).await.is_empty());
    assert_eq!(dead_letters(&node).await.len(), 1);
}

#[tokio::test]
async fn a_refused_deposit_leaves_the_row_pending_and_spends_an_attempt() {
    let node = outbox_sender_node(3).await;
    node.mailbox.refuse_deposits_with(codes::QUEUE_FULL);

    deposited_call(&node).await;

    let rows = rows(&node).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].parked.is_none(), "nothing waits at the mailbox");
    assert_eq!(rows[0].attempts, 1, "the failed delivery counts like any other");
    assert!(dead_letters(&node).await.is_empty());
}

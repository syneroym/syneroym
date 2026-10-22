use syneroym_rpc::ConversationDeliveryState;

use super::{store, store_with_config};
use crate::{
    ids::derive_conversation_id,
    store::{ConversationConfig, ConversationStore, StoreError, deposit::Park},
};

const ME: &str = "svc:me";
const PEER: &str = "svc:peer";

fn park(now: i64) -> Park<'static> {
    Park { now, poll_interval_ms: 60_000, deadline_at: now + 600_000, context: b"deposit" }
}

fn conversation(s: &ConversationStore) -> String {
    s.get_or_create_direct(PEER, &derive_conversation_id(ME, PEER), 1_000).unwrap()
}

/// A sent message and its claimed outbox row.
fn sent(s: &ConversationStore, conv: &str, id: &str) -> i64 {
    s.insert_outgoing_and_enqueue(
        conv,
        id,
        ME,
        1_000,
        "text/plain",
        b"hello",
        &[0u8; 64],
        PEER,
        1_000,
        false,
    )
    .unwrap();
    let claimed = s.queue().claim_due(1_000, 10).unwrap();
    claimed.iter().find(|item| item.queue_key.contains(id)).or(claimed.first()).unwrap().id
}

fn state_of(s: &ConversationStore, id: &str) -> ConversationDeliveryState {
    s.get_message(id).unwrap().unwrap().state
}

#[test]
fn parking_a_deposit_sets_the_state_and_the_row_in_one_commit() {
    let s = store();
    let conv = conversation(&s);
    let item = sent(&s, &conv, "m:1");

    s.park_deposited(item, "m:1", None, &park(2_000)).unwrap();

    assert_eq!(state_of(&s, "m:1"), ConversationDeliveryState::Deposited);
    let rows = s.queue().all().unwrap();
    assert!(rows[0].parked.is_some());
}

#[test]
fn a_park_that_fails_leaves_the_message_state_unchanged() {
    let s = store();
    let conv = conversation(&s);
    let item = sent(&s, &conv, "m:1");
    let past_deadline = Park { deadline_at: 1_500, ..park(2_000) };

    assert!(s.park_deposited(item, "m:1", None, &past_deadline).is_err());

    assert_eq!(state_of(&s, "m:1"), ConversationDeliveryState::Pending);
    assert!(s.queue().all().unwrap()[0].parked.is_none());
}

#[test]
fn unparking_sets_the_message_back_to_pending_and_the_row_back_to_direct() {
    let s = store();
    let conv = conversation(&s);
    let item = sent(&s, &conv, "m:1");
    s.park_deposited(item, "m:1", None, &park(2_000)).unwrap();

    s.unpark_to_pending(item, "m:1", None, 3_000).unwrap();

    assert_eq!(state_of(&s, "m:1"), ConversationDeliveryState::Pending);
    assert!(s.queue().all().unwrap()[0].parked.is_none());
}

#[test]
fn a_deposited_message_still_counts_against_the_pending_quota() {
    let (s, _) = store_with_config(ConversationConfig {
        max_pending_per_conversation: 1,
        ..ConversationConfig::default()
    });
    let conv = conversation(&s);
    let item = sent(&s, &conv, "m:1");
    s.park_deposited(item, "m:1", None, &park(2_000)).unwrap();

    let second = s.insert_outgoing_and_enqueue(
        &conv,
        "m:2",
        ME,
        1_100,
        "text/plain",
        b"again",
        &[0u8; 64],
        PEER,
        1_100,
        false,
    );

    let error = second.unwrap_err();
    assert!(matches!(error.downcast_ref::<StoreError>(), Some(StoreError::PendingQuotaExceeded)));
}

#[test]
fn a_deposited_member_still_counts_as_remaining() {
    let s = store();
    let conv = conversation(&s);
    let item = sent(&s, &conv, "m:1");
    {
        let conn = s.conn.lock().unwrap();
        for member in ["peer1", "peer2"] {
            conn.execute(
                "INSERT INTO message_recipients (message_id, member_address, state, last_error) \
                 VALUES ('m:1', ?1, 'pending', NULL)",
                [member],
            )
            .unwrap();
        }
    }

    s.park_deposited(item, "m:1", Some("peer1"), &park(2_000)).unwrap();

    assert_eq!(
        s.recipient_state("m:1", "peer1").unwrap(),
        Some(ConversationDeliveryState::Deposited)
    );
    assert_eq!(s.recipients_remaining("m:1").unwrap(), 2);
    assert_eq!(
        state_of(&s, "m:1"),
        ConversationDeliveryState::Pending,
        "a group message stays pending"
    );
    s.set_recipient_state("m:1", "peer1", ConversationDeliveryState::Delivered, None).unwrap();
    assert_eq!(s.recipients_remaining("m:1").unwrap(), 1);
}

#[test]
fn a_deposited_message_deleted_before_delivery_is_failed() {
    let s = store();
    let conv = conversation(&s);
    let item = sent(&s, &conv, "m:1");
    s.park_deposited(item, "m:1", None, &park(2_000)).unwrap();

    assert!(s.delete_message(&conv, "m:1", 3_000).unwrap());

    let message = s.get_message("m:1").unwrap().unwrap();
    assert_eq!(message.state, ConversationDeliveryState::Failed);
    assert_eq!(message.last_error.as_deref(), Some("deleted before delivery"));
}

#[test]
fn the_outbox_lists_a_deposited_message() {
    let s = store();
    let conv = conversation(&s);
    let item = sent(&s, &conv, "m:1");
    s.park_deposited(item, "m:1", None, &park(2_000)).unwrap();

    let listed = s.outbox_messages().unwrap();

    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, ConversationDeliveryState::Deposited);
}

#[test]
fn a_backed_up_deposited_message_comes_back_failed() {
    let s = store();
    let conv = s.get_or_create_direct(PEER, &derive_conversation_id(ME, PEER), 1_000).unwrap();
    s.insert_outgoing_without_enqueue(
        &conv,
        "m:0",
        ME,
        1_000,
        "text/plain",
        b"hello",
        1_000,
        "deposited",
    )
    .unwrap();
    let mut chunks = Vec::new();
    let mut cursor = None;
    loop {
        let chunk = s.export_history(cursor).unwrap();
        chunks.push(chunk.data);
        cursor = chunk.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    let fresh = store();
    for chunk in chunks {
        fresh.import_history(ME, &chunk).unwrap();
    }

    let restored = fresh.get_message("m:0").unwrap().unwrap();
    assert_eq!(restored.state, ConversationDeliveryState::Failed);
    assert_eq!(restored.last_error.as_deref(), Some("restored from a backup"));
}

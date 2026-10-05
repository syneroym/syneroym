//! A group deletion request can reach a member before the message it names,
//! for example when the message is only pulled by a later sync.

use std::sync::{
    Arc, Weak,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use syneroym_rpc::{
    Admission, ConversationDeliveryState, ConversationMessage, ConversationNotifier, NotifyOutcome,
};

use super::*;
use crate::dag::{DELETION_REQUEST_CONTENT_TYPE, MembershipPayload, deletion_request_body};

const CONV: &str = "conv:withdrawn";
const KEY: [u8; 32] = [9u8; 32];

/// A group this node is in, with `members` added and the epoch key known.
fn group_with(s: &ConversationStore, members: &[(&str, &SigningKey)]) -> ConversationRow {
    let conv = group_without_key(s, members);
    give_key(s);
    conv
}

/// The epoch key arrives; entries stored before it can now be applied.
fn give_key(s: &ConversationStore) {
    s.conn()
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) VALUES (?1, 1, \
             ?2, 1000)",
            rusqlite::params![CONV, KEY.as_slice()],
        )
        .unwrap();
}

/// A group with `members`, whose epoch key has not arrived yet: entries are
/// stored but not applied.
fn group_without_key(s: &ConversationStore, members: &[(&str, &SigningKey)]) -> ConversationRow {
    let conn = s.conn().lock().unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    let conv =
        ConversationStore::get_or_create_group_shell(&tx, CONV, "svc:owner", 1, 1_000).unwrap();
    for (address, sk) in members {
        let payload = MembershipPayload {
            action: "add".to_string(),
            subject_address: (*address).to_string(),
            subject_sig_key: sk.verifying_key().to_bytes(),
            new_epoch: 1,
            member_list_hash: "hash".to_string(),
        };
        ConversationStore::apply_membership(&tx, CONV, &payload).unwrap();
    }
    tx.commit().unwrap();
    conv
}

fn entry(sk: &SigningKey, author: &str, at: i64, content_type: &str, body: &[u8]) -> WireEntry {
    build_message_entry(sk, CONV, author, at, 1, vec![], &KEY, &encode_body(content_type, body))
        .unwrap()
}

#[test]
fn a_request_that_arrives_first_is_applied_when_the_message_arrives() {
    let s = store();
    let alice = SigningKey::generate(&mut rand_core::OsRng);
    let conv = group_with(&s, &[("svc:alice", &alice)]);
    let message = entry(&alice, "svc:alice", 1_000, "text/plain", b"take this back");
    let request = entry(
        &alice,
        "svc:alice",
        1_001,
        DELETION_REQUEST_CONTENT_TYPE,
        &deletion_request_body(&message.entry_id),
    );

    validate_and_insert(&s, "svc:me", &conv, &request).unwrap();
    let (_, for_app) = validate_and_insert(&s, "svc:me", &conv, &message).unwrap();

    assert!(for_app.is_none(), "the app is never asked about a withdrawn message");
    let row = s.get_message(&message.entry_id).unwrap().expect("the row is kept for the digest");
    assert!(row.body.is_empty());
    assert!(row.deleted_at.is_some());
    assert_eq!(row.admission, "dropped");
    assert!(s.history(CONV, 10, None).unwrap().items.is_empty());
}

#[test]
fn a_request_from_someone_else_does_not_withdraw_the_message() {
    let s = store();
    let alice = SigningKey::generate(&mut rand_core::OsRng);
    let mallory = SigningKey::generate(&mut rand_core::OsRng);
    let conv = group_with(&s, &[("svc:alice", &alice), ("svc:mallory", &mallory)]);
    let message = entry(&alice, "svc:alice", 1_000, "text/plain", b"mine to keep");
    let forged = entry(
        &mallory,
        "svc:mallory",
        1_001,
        DELETION_REQUEST_CONTENT_TYPE,
        &deletion_request_body(&message.entry_id),
    );

    validate_and_insert(&s, "svc:me", &conv, &forged).unwrap();
    let (_, for_app) = validate_and_insert(&s, "svc:me", &conv, &message).unwrap();

    let asked = for_app.expect("the message goes to the app as usual");
    assert_eq!(asked.body, b"mine to keep");
    let row = s.get_message(&message.entry_id).unwrap().unwrap();
    assert!(row.deleted_at.is_none());
}

/// The message and its deletion request, signed by the same author.
fn message_and_request(alice: &SigningKey) -> (WireEntry, WireEntry) {
    let message = entry(alice, "svc:alice", 1_000, "text/plain", b"take this back");
    let request = entry(
        alice,
        "svc:alice",
        1_001,
        DELETION_REQUEST_CONTENT_TYPE,
        &deletion_request_body(&message.entry_id),
    );
    (message, request)
}

#[test]
fn every_order_of_arrival_gives_the_same_transcript_code() {
    let alice = SigningKey::generate(&mut rand_core::OsRng);
    let (message, request) = message_and_request(&alice);

    // Request first: the message is stored already withdrawn.
    let first = store();
    let conv = group_with(&first, &[("svc:alice", &alice)]);
    validate_and_insert(&first, "svc:me", &conv, &request).unwrap();
    validate_and_insert(&first, "svc:me", &conv, &message).unwrap();

    // Message first and accepted by the app, then deleted.
    let seen = store();
    let conv = group_with(&seen, &[("svc:alice", &alice)]);
    validate_and_insert(&seen, "svc:me", &conv, &message).unwrap();
    seen.apply_admission(&message.entry_id, &Admission::Accept, 1_000).unwrap();
    validate_and_insert(&seen, "svc:me", &conv, &request).unwrap();

    // Message first while the app has not answered yet, then deleted.
    let waiting = store();
    let conv = group_with(&waiting, &[("svc:alice", &alice)]);
    validate_and_insert(&waiting, "svc:me", &conv, &message).unwrap();
    validate_and_insert(&waiting, "svc:me", &conv, &request).unwrap();

    let code = first.transcript_digest(CONV).unwrap();
    assert_eq!(seen.transcript_digest(CONV).unwrap(), code);
    assert_eq!(waiting.transcript_digest(CONV).unwrap(), code);
    let row = waiting.get_message(&message.entry_id).unwrap().unwrap();
    assert_eq!(
        (row.admission.as_str(), row.admission_reason.as_deref()),
        ("dropped", Some("deleted-by-author")),
        "a re-ask can never accept an empty, withdrawn message"
    );
}

#[derive(Debug, Default)]
struct Counting {
    asked: AtomicUsize,
}

#[async_trait]
impl ConversationNotifier for Counting {
    async fn notify_message(&self, _: &str, _: ConversationMessage) -> NotifyOutcome {
        self.asked.fetch_add(1, Ordering::SeqCst);
        NotifyOutcome::Answered(Admission::Accept)
    }

    async fn notify_delivery_state(&self, _: &str, _: String, _: ConversationDeliveryState) {}
}

#[tokio::test]
async fn a_sync_that_brings_a_message_and_its_withdrawal_never_shows_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let service = service_for_rekey_test(dir.path(), 3600).await;
    let store = service.store_for("svc:me").await.unwrap();
    let alice = SigningKey::generate(&mut rand_core::OsRng);
    let conv = group_without_key(&store, &[("svc:alice", &alice)]);
    let (message, request) = message_and_request(&alice);
    // Both arrive in one sync, in the author's order, before the key.
    validate_and_insert(&store, "svc:me", &conv, &message).unwrap();
    validate_and_insert(&store, "svc:me", &conv, &request).unwrap();
    let app = Arc::new(Counting::default());
    service.register_service_notifier(
        "svc:me".to_string(),
        Arc::downgrade(&app) as Weak<dyn ConversationNotifier>,
    );

    give_key(&store);
    service.apply_pending_entries(&store, "svc:me", CONV).await;

    assert_eq!(app.asked.load(Ordering::SeqCst), 0, "the app never sees the withdrawn text");
    let row = store.get_message(&message.entry_id).unwrap().unwrap();
    assert!(row.body.is_empty());
    assert_eq!(row.admission, "dropped");
}

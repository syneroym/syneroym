//! A group deletion request can reach a member before the message it names,
//! for example when the message is only pulled by a later sync.

use ed25519_dalek::SigningKey;

use super::*;
use crate::dag::{DELETION_REQUEST_CONTENT_TYPE, MembershipPayload, deletion_request_body};

const CONV: &str = "conv:withdrawn";
const KEY: [u8; 32] = [9u8; 32];

/// A group this node is in, with `members` added and the epoch key known.
fn group_with(s: &ConversationStore, members: &[(&str, &SigningKey)]) -> ConversationRow {
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
    tx.execute(
        "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) VALUES (?1, 1, ?2, \
         1000)",
        rusqlite::params![CONV, KEY.as_slice()],
    )
    .unwrap();
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

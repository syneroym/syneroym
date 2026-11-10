use ed25519_dalek::SigningKey;
use rusqlite::params;

use super::*;
use crate::dag::{MembershipPayload, encode_body};

#[test]
fn group_message_with_languages_is_stored_in_db_and_read_back() {
    let s = store();
    let conv_id = "conv:g_lang";
    let now = 1000;
    let sk = SigningKey::from_bytes(&[2u8; 32]);
    let conv = {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let c = ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 1, now)
            .unwrap();
        let payload = MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:alice".to_string(),
            subject_sig_key: sk.verifying_key().to_bytes(),
            new_epoch: 1,
            member_list_hash: "hash1".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &payload).unwrap();
        tx.commit().unwrap();
        c
    };

    let epoch_key = [7u8; 32];
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) VALUES (?1, 1, \
             ?2, ?3)",
            params![conv_id, epoch_key.as_slice(), now],
        )
        .unwrap();
        tx.commit().unwrap();
    }

    let langs = vec!["hi".to_string(), "en".to_string()];
    let body_bytes = encode_body("text/plain", &langs, b"namaste");
    let entry =
        build_message_entry(&sk, conv_id, "svc:alice", now, 1, vec![], &epoch_key, &body_bytes)
            .unwrap();

    let (ins, msg_opt, _) = validate_and_insert(&s, "svc:me", &conv, &entry).unwrap();
    assert!(ins);
    let msg_row = msg_opt.expect("message row emitted");
    assert_eq!(msg_row.languages, langs);

    // Verify stored row via get_message
    let stored = s.get_message(&msg_row.id).unwrap().expect("stored message exists");
    assert_eq!(stored.languages, langs);

    // Verify column directly in SQLite
    let raw_lang: String = s
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT languages FROM messages WHERE conversation_id = ?1 AND id = ?2",
            params![conv_id, &msg_row.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw_lang, "hi en");
}

#[test]
fn group_message_with_non_canonical_language_tag_is_refused_and_not_stored() {
    let s = store();
    let conv_id = "conv:g_bad_lang";
    let now = 1000;
    let sk = SigningKey::from_bytes(&[3u8; 32]);
    let conv = {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let c = ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 1, now)
            .unwrap();
        let payload = MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:bob".to_string(),
            subject_sig_key: sk.verifying_key().to_bytes(),
            new_epoch: 1,
            member_list_hash: "hash_bad".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &payload).unwrap();
        tx.commit().unwrap();
        c
    };

    let epoch_key = [8u8; 32];
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) VALUES (?1, 1, \
             ?2, ?3)",
            params![conv_id, epoch_key.as_slice(), now],
        )
        .unwrap();
        tx.commit().unwrap();
    }

    // Uppercase or invalid tag
    let bad_langs = vec!["EN".to_string()];
    let body_bytes = encode_body("text/plain", &bad_langs, b"hello");
    let entry =
        build_message_entry(&sk, conv_id, "svc:bob", now, 1, vec![], &epoch_key, &body_bytes)
            .unwrap();

    let (ins, msg_opt, _) = validate_and_insert(&s, "svc:me", &conv, &entry).unwrap();
    assert!(ins);
    // Bad tag causes apply_message_entry to log warn and return Ok(()), msg_opt is
    // None
    assert!(msg_opt.is_none());

    // Messages table has no row for this conversation
    let count: i64 = s
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM messages WHERE conversation_id = ?1",
            params![conv_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

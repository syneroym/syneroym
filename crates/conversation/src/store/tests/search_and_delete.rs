use syneroym_rpc::{Admission, ConversationDeliveryState};

use super::store;

#[test]
fn delete_message_removes_from_store_and_fts() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    let now = 1_000;

    s.insert_outgoing_and_enqueue(
        &conv,
        "m:to_delete",
        "did:key:zMe",
        now,
        "text/plain",
        b"unique_keyword_to_find",
        &[0u8; 64],
        "did:key:zPeer",
        now,
        false,
    )
    .unwrap();

    // Verify found in search
    let hits = s.search("unique_keyword_to_find", Some(&conv), 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m:to_delete");

    // Delete message: delete_message(conv_id, msg_id, now_ms)
    assert!(s.delete_message(&conv, "m:to_delete", now + 10).unwrap());

    // No longer returned by search
    let hits_after = s.search("unique_keyword_to_find", Some(&conv), 10).unwrap();
    assert_eq!(hits_after.len(), 0);

    // Message is marked deleted
    let msg = s.get_message("m:to_delete").unwrap().unwrap();
    assert!(msg.deleted_at.is_some());
}

#[test]
fn delete_pending_outgoing_marks_failed_and_cleans_outbox() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    let now = 1_000;

    s.insert_outgoing_and_enqueue(
        &conv,
        "m:pending_del",
        "did:key:zMe",
        now,
        "text/plain",
        b"pending text",
        &[0u8; 64],
        "did:key:zPeer",
        now,
        false,
    )
    .unwrap();

    // Verify queue item exists
    assert_eq!(s.queue().all().unwrap().len(), 1);

    // Delete pending message
    s.delete_message(&conv, "m:pending_del", now + 10).unwrap();

    // State is failed
    let msg = s.get_message("m:pending_del").unwrap().unwrap();
    assert_eq!(msg.state, ConversationDeliveryState::Failed);
}

#[test]
fn delete_held_incoming_zeros_body_but_fts_is_not_corrupted() {
    // The host layer (host_impl.rs) blocks deleting held incoming rows before
    // reaching the store. At the store level, delete_message succeeds — it
    // zeros the body — but does NOT touch the FTS index (there is no FTS
    // entry for unaccepted rows) so the trigram table stays consistent.
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    let now = 1_000;

    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            &conv,
            "m:held_in",
            "did:key:zPeer",
            now,
            "text/plain",
            b"incoming held",
            &[0u8; 64],
            now,
            100,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    s.apply_admission("m:held_in", &Admission::Hold("test".into()), now).unwrap();

    // At the store level delete_message succeeds; the FTS guard (admission ==
    // "accepted") prevents touching the trigram index for this never-indexed row.
    let deleted = s.delete_message(&conv, "m:held_in", now + 10).unwrap();
    assert!(deleted);
    let msg = s.get_message("m:held_in").unwrap().unwrap();
    assert!(msg.deleted_at.is_some(), "body must be zeroed even for held incoming");
}

#[test]
fn search_escapes_like_special_characters() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    let now = 1_000;

    s.insert_outgoing_and_enqueue(
        &conv,
        "m:pct",
        "did:key:zMe",
        now,
        "text/plain",
        b"discount is 100% off today",
        &[0u8; 64],
        "did:key:zPeer",
        now,
        false,
    )
    .unwrap();

    s.insert_outgoing_and_enqueue(
        &conv,
        "m:other",
        "did:key:zMe",
        now + 1,
        "text/plain",
        b"discount is 1000 off today",
        &[0u8; 64],
        "did:key:zPeer",
        now + 1,
        false,
    )
    .unwrap();

    // Query is short (< 3 chars for FTS), uses LIKE path with escape.
    // "%" is a LIKE wildcard; escaped it should only match the literal "%".
    let hits = s.search("100%", Some(&conv), 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m:pct");
}

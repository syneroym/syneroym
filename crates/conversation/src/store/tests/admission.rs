use syneroym_rpc::{Admission, DropAnswer};

use super::{store, store_with_config};
use crate::{
    dag::DELETION_REQUEST_CONTENT_TYPE,
    store::{ConversationConfig, ConversationStore},
};

#[test]
fn admission_visibility_filters_unaccepted_rows() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    let now = 1_000;

    // Incoming message 1: inserted undecided
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            &conv,
            "m:1",
            "did:key:zPeer",
            1_000,
            "text/plain",
            b"hello",
            &[0u8; 64],
            now,
            100,
        )
        .unwrap();
        tx.commit().unwrap();
    }

    // Undecided messages are not returned by history or changes
    let hist = s.history(&conv, 10, None).unwrap();
    assert_eq!(hist.items.len(), 0);
    let chg = s.changes(&conv, 0, 10).unwrap();
    assert_eq!(chg.messages.len(), 0);

    // Apply hold: still hidden
    s.apply_admission("m:1", &Admission::Hold("stranger".into()), now).unwrap();
    let hist = s.history(&conv, 10, None).unwrap();
    assert_eq!(hist.items.len(), 0);
    let chg = s.changes(&conv, 0, 10).unwrap();
    assert_eq!(chg.messages.len(), 0);

    // Incoming message 2: accepted
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            &conv,
            "m:2",
            "did:key:zPeer",
            1_001,
            "text/plain",
            b"world",
            &[0u8; 64],
            now + 1,
            100,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    s.apply_admission("m:2", &Admission::Accept, now + 1).unwrap();

    // Accepted message is visible
    let hist = s.history(&conv, 10, None).unwrap();
    assert_eq!(hist.items.len(), 1);
    let chg = s.changes(&conv, 0, 10).unwrap();
    assert_eq!(chg.messages.len(), 1);
    assert_eq!(chg.messages[0].id, "m:2");
}

fn insert_undecided(s: &ConversationStore, conv: &str, id: &str, body: &[u8]) {
    let conn = s.conn().lock().unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    s.insert_incoming_if_absent(
        &tx,
        conv,
        id,
        "did:key:zPeer",
        1_000,
        "text/plain",
        body,
        &[0u8; 64],
        1_000,
        100,
    )
    .unwrap();
    tx.commit().unwrap();
}

#[test]
fn readmit_returns_held_rows_to_the_app_and_does_not_accept_them() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    insert_undecided(&s, &conv, "m:held", b"held body");
    s.apply_admission("m:held", &Admission::Hold("unknown-sender".into()), 1_000).unwrap();
    insert_undecided(&s, &conv, "m:other", b"other body");
    s.apply_admission("m:other", &Admission::Hold("contact-limit".into()), 1_000).unwrap();

    let count = s.readmit(&conv, &["unknown-sender".to_string()]).unwrap();

    assert_eq!(count, 1, "only the matching reason is re-asked");
    let row = s.get_message("m:held").unwrap().unwrap();
    assert_eq!(row.admission, "undecided", "the app must decide again, e.g. a new block");
    assert!(s.changes(&conv, 0, 10).unwrap().messages.is_empty());
    let due: Vec<_> = s.undecided_messages(i64::MAX).unwrap().into_iter().map(|m| m.id).collect();
    assert_eq!(due, vec!["m:held".to_string()]);
    assert_eq!(s.get_message("m:other").unwrap().unwrap().admission, "held");
}

#[test]
fn a_late_accept_is_numbered_once() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    insert_undecided(&s, &conv, "m:late", b"late body");
    s.apply_admission("m:late", &Admission::Hold("unknown-sender".into()), 1_000).unwrap();
    s.readmit(&conv, &[]).unwrap();

    s.apply_admission("m:late", &Admission::Accept, 1_020).unwrap();
    let first = s.changes(&conv, 0, 10).unwrap();
    assert_eq!(first.messages.len(), 1);
    let number = first.messages[0].visible_seq;
    assert!(number > 0);

    // A racing second answer must neither renumber the row nor hide it.
    s.apply_admission("m:late", &Admission::Accept, 1_030).unwrap();
    s.apply_admission("m:late", &Admission::Hold("late-hold".into()), 1_040).unwrap();
    let second = s.changes(&conv, 0, 10).unwrap();
    assert_eq!(second.messages.len(), 1);
    assert_eq!(second.messages[0].visible_seq, number);
}

#[test]
fn rows_the_host_wrote_itself_are_not_returned_for_a_re_ask() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    insert_undecided(&s, &conv, "m:sys", b"{}");
    s.conn().lock().unwrap().execute("UPDATE messages SET system = 1", []).unwrap();

    assert!(s.undecided_messages(i64::MAX).unwrap().is_empty());
}

#[test]
fn expiry_marks_held_messages_dropped() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    let now = 1_000;

    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            &conv,
            "m:old_held",
            "did:key:zPeer",
            1_000,
            "text/plain",
            b"old held",
            &[0u8; 64],
            now,
            100,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    s.apply_admission("m:old_held", &Admission::Hold("contact-limit".into()), now).unwrap();

    // expire_held_messages(max_age_ms, now_ms) returns Vec<(conv_id, msg_id)>.
    let expired = s.expire_held_messages(30_000, now + 40_000).unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].1, "m:old_held");

    for (conv_id, msg_id) in expired {
        s.apply_admission(
            &msg_id,
            &Admission::Drop(DropAnswer { reason: "expired".to_string(), report: false }),
            now + 40_000,
        )
        .unwrap();
        // The conversation id is required by delete_message but not by apply_admission.
        let _ = conv_id;
    }

    let msg = s.get_message("m:old_held").unwrap().unwrap();
    assert_eq!(msg.admission, "dropped");
    assert_eq!(msg.admission_reason.as_deref(), Some("expired"));
}

#[test]
fn dropped_messages_do_not_count_toward_quota() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    let now = 1_000;

    // Insert message 1 and drop it
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            &conv,
            "m:drop1",
            "did:key:zPeer",
            1_000,
            "text/plain",
            b"dropped body",
            &[0u8; 64],
            now,
            2,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    s.apply_admission(
        "m:drop1",
        &Admission::Drop(DropAnswer { reason: "spam".into(), report: false }),
        now,
    )
    .unwrap();

    // Now insert 2 more messages with quota limit of 2: both should succeed
    for i in 2..=3 {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let ok = s
            .insert_incoming_if_absent(
                &tx,
                &conv,
                &format!("m:ok{i}"),
                "did:key:zPeer",
                1_000 + i,
                "text/plain",
                b"ok body",
                &[0u8; 64],
                now + i,
                2,
            )
            .unwrap();
        assert!(ok);
        tx.commit().unwrap();
    }
}

#[test]
fn system_messages_skip_touch_conversation() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();

    // Verify initial last_activity via get_conversation (list_conversations
    // hides not-yet-opened direct conversations).
    let row = s.get_conversation(&conv).unwrap().unwrap();
    assert_eq!(row.last_activity_ms, 1_000);

    // Send a system message at 5_000 — must not advance last_activity.
    s.insert_outgoing_and_enqueue(
        &conv,
        "m:sys",
        "did:key:zMe",
        5_000,
        DELETION_REQUEST_CONTENT_TYPE,
        b"{}",
        &[0u8; 64],
        "did:key:zPeer",
        5_000,
        true,
    )
    .unwrap();

    let row = s.get_conversation(&conv).unwrap().unwrap();
    assert_eq!(row.last_activity_ms, 1_000, "system message must not advance last_activity");
}

fn drop_one(s: &ConversationStore, conv: &str, id: &str, at: i64) {
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            conv,
            id,
            "did:key:zPeer",
            at,
            "text/plain",
            b"unwanted",
            &[0u8; 64],
            at,
            100_000,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    s.apply_admission(
        id,
        &Admission::Drop(DropAnswer { reason: "blocked".into(), report: false }),
        at,
    )
    .unwrap();
}

#[test]
fn only_the_newest_dropped_rows_of_a_direct_chat_are_kept() {
    let (s, _dir) = store_with_config(ConversationConfig {
        max_dropped_per_conversation: 3,
        ..Default::default()
    });
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    for i in 0..6 {
        drop_one(&s, &conv, &format!("m:{i}"), 1_000 + i);
    }
    assert!(s.take_drop_prune_flag(), "a drop asks for a prune");

    let removed = s.prune_dropped().unwrap();

    assert_eq!(removed, 3);
    for gone in ["m:0", "m:1", "m:2"] {
        assert!(s.get_message(gone).unwrap().is_none(), "{gone} is the oldest");
    }
    for kept in ["m:3", "m:4", "m:5"] {
        assert!(s.get_message(kept).unwrap().is_some(), "{kept} is recent");
    }
}

#[test]
fn pruning_never_touches_accepted_rows_or_group_conversations() {
    let (s, _dir) = store_with_config(ConversationConfig {
        max_dropped_per_conversation: 1,
        ..Default::default()
    });
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    s.insert_outgoing_without_enqueue(
        &conv,
        "m:mine",
        "did:key:zMe",
        1,
        "text/plain",
        b"hi",
        1,
        "delivered",
    )
    .unwrap();
    {
        let conn = s.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO conversations (id, kind, owner_address, created_at, last_activity) \
             VALUES ('grp', 'group', 'did:key:zOwner', 1, 1)",
            [],
        )
        .unwrap();
    }
    for i in 0..4 {
        drop_one(&s, "grp", &format!("g:{i}"), 1_000 + i);
    }

    assert_eq!(s.prune_dropped().unwrap(), 0);

    assert!(s.get_message("m:mine").unwrap().is_some());
    for i in 0..4 {
        assert!(
            s.get_message(&format!("g:{i}")).unwrap().is_some(),
            "every member keeps the same rows"
        );
    }
}

#[test]
fn the_prune_cuts_only_the_chat_over_its_cap() {
    let (s, _dir) = store_with_config(ConversationConfig {
        max_dropped_per_conversation: 2,
        ..Default::default()
    });
    let busy = s.get_or_create_direct("did:key:zPeer", "conv:busy", 1_000).unwrap();
    let quiet = s.get_or_create_direct("did:key:zOther", "conv:quiet", 1_000).unwrap();
    for i in 0..5 {
        drop_one(&s, &busy, &format!("b:{i}"), 1_000 + i);
    }
    for i in 0..2 {
        drop_one(&s, &quiet, &format!("q:{i}"), 1_000 + i);
    }

    assert_eq!(s.prune_dropped().unwrap(), 3);

    assert!(s.get_message("b:2").unwrap().is_none());
    assert!(s.get_message("b:3").unwrap().is_some());
    for i in 0..2 {
        assert!(s.get_message(&format!("q:{i}")).unwrap().is_some(), "a chat at the cap keeps all");
    }
}

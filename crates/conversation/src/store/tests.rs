use syneroym_core::config::RetryPolicy;

use super::*;

fn store() -> ConversationStore {
    let dir = tempfile::tempdir().unwrap();
    // Leak the tempdir so the file lives for the test's duration; each
    // test gets its own directory so this is bounded.
    let path = Box::leak(Box::new(dir)).path();
    ConversationStore::open_encrypted(
        path,
        None,
        QueueConfig {
            retry: RetryPolicy {
                max_attempts: 5,
                initial_backoff_ms: 10,
                backoff_multiplier: 2.0,
                max_backoff_ms: 1000,
            },
            visibility_timeout_ms: 5000,
            dlq_max_rows: 100,
            max_pending_rows: 1000,
        },
        ConversationConfig::default(),
    )
    .unwrap()
}

#[test]
fn get_or_create_direct_is_idempotent() {
    let s = store();
    let id1 = s.get_or_create_direct("did:key:zPeer", "conv:precomputed", 1_000).unwrap();
    let id2 = s.get_or_create_direct("did:key:zPeer", "conv:precomputed-again", 2_000).unwrap();
    assert_eq!(id1, id2, "a second open-direct for the same peer must return the same id");
}

/// The send transaction is atomic: an injected failure between the two
/// writes leaves neither behind. Simulated here by a closure that
/// writes the message row then deliberately errors before enqueueing.
#[test]
fn the_send_transaction_is_atomic_under_injected_failure() {
    let s = store();
    let conv_id = s.get_or_create_direct("did:key:zPeer", "conv:1", 1_000).unwrap();
    let result = s.queue.transaction(|tx, _txq| {
        tx.execute(
            "INSERT INTO messages (id, conversation_id, author, sender_timestamp, received_at, \
             content_type, body, signature, outgoing, verified, state, last_error) VALUES \
             ('msg:1', ?1, 'a', 0, 0, 'text', X'00', X'00', 1, 1, 'pending', NULL)",
            params![conv_id],
        )?;
        Err::<(), _>(anyhow!("simulated failure before enqueue"))
    });
    assert!(result.is_err());
    assert!(s.get_message("msg:1").unwrap().is_none(), "the message row must have rolled back");
    assert!(s.queue.all().unwrap().is_empty(), "no enqueue must have landed either");
}

#[test]
fn the_author_id_index_rejects_a_repeat() {
    let s = store();
    let author = "did:key:zPeer";
    let conv_id = crate::ids::derive_conversation_id("did:key:zMe", author);
    // First delivery: a genuine insert.
    let first = {
        let conn = s.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let inserted = s
            .insert_incoming_if_absent(
                &tx,
                &conv_id,
                "msg:1",
                author,
                1_000,
                "text/plain",
                b"hi",
                &[0u8; 64],
                1_000,
                100,
            )
            .unwrap();
        tx.commit().unwrap();
        inserted
    };
    assert!(first, "the first delivery of (author, id) must insert");

    // A second insert for the exact same (author, id) must not create
    // a second row -- proven directly against the underlying
    // constraint via the incoming-insert path's INSERT OR IGNORE.
    let second = {
        let conn = s.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let inserted = s
            .insert_incoming_if_absent(
                &tx,
                &conv_id,
                "msg:1",
                author,
                1_000,
                "text/plain",
                b"hi",
                &[0u8; 64],
                2_000,
                100,
            )
            .unwrap();
        tx.commit().unwrap();
        inserted
    };
    assert!(!second, "a repeat (author, id) must be ignored, not error");
}

#[test]
fn exhausting_one_conversation_quota_leaves_another_conversation_unaffected() {
    let s = store();
    let conv_1 = "conv:1";
    let conv_2 = "conv:2";
    let author_1 = "did:key:zA";
    let author_2 = "did:key:zB";

    // Insert 2 messages into conv_1 with a cap of 2.
    {
        let conn = s.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            conv_1,
            "msg:1",
            author_1,
            1_000,
            "text/plain",
            b"1",
            &[0u8; 64],
            1_000,
            2,
        )
        .unwrap();
        s.insert_incoming_if_absent(
            &tx,
            conv_1,
            "msg:2",
            author_1,
            1_001,
            "text/plain",
            b"2",
            &[0u8; 64],
            1_001,
            2,
        )
        .unwrap();
        tx.commit().unwrap();
    }

    // 3rd message into conv_1 fails with quota exceeded.
    {
        let conn = s.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let res = s.insert_incoming_if_absent(
            &tx,
            conv_1,
            "msg:3",
            author_1,
            1_002,
            "text/plain",
            b"3",
            &[0u8; 64],
            1_002,
            2,
        );
        assert!(res.is_err(), "exceeding max_messages_per_conversation must error");
    }

    // conv_2 is unaffected and accepts messages.
    {
        let conn = s.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let res = s.insert_incoming_if_absent(
            &tx,
            conv_2,
            "msg:c2_1",
            author_2,
            1_000,
            "text/plain",
            b"hello",
            &[0u8; 64],
            1_000,
            2,
        );
        assert!(res.is_ok(), "other conversation must not be affected by conv_1's exhaustion");
        tx.commit().unwrap();
    }
}

#[test]
fn history_returns_the_documented_order_under_a_skewed_clock() {
    let s = store();
    let conv_id = s.get_or_create_direct("did:key:zPeer", "conv:1", 1_000).unwrap();
    // Out-of-order timestamps, as a skewed-clock sender would produce.
    for (id, ts, author) in
        [("msg:b", 500, "did:key:zA"), ("msg:a", 500, "did:key:zA"), ("msg:c", 100, "did:key:zA")]
    {
        s.insert_outgoing_and_enqueue(
            &conv_id,
            id,
            author,
            ts,
            "text/plain",
            b"x",
            &[0u8; 64],
            "did:key:zPeer",
            ts,
            false,
        )
        .unwrap();
    }
    let page = s.history(&conv_id, 10, None).unwrap();
    let ids: Vec<&str> = page.messages.iter().map(|m| m.id.as_str()).collect();
    // (sender_timestamp, author, id): msg:c (100) first, then msg:a
    // before msg:b at the same timestamp (id tiebreak).
    assert_eq!(ids, vec!["msg:c", "msg:a", "msg:b"]);
}

#[test]
fn history_pages_and_reports_a_next_cursor() {
    let s = store();
    let conv_id = s.get_or_create_direct("did:key:zPeer", "conv:1", 1_000).unwrap();
    for i in 0..5 {
        s.insert_outgoing_and_enqueue(
            &conv_id,
            &format!("msg:{i}"),
            "did:key:zA",
            i,
            "text/plain",
            b"x",
            &[0u8; 64],
            "did:key:zPeer",
            i,
            false,
        )
        .unwrap();
    }
    let page1 = s.history(&conv_id, 2, None).unwrap();
    assert_eq!(page1.messages.len(), 2);
    assert_eq!(page1.messages[0].id, "msg:0");
    assert_eq!(page1.messages[1].id, "msg:1");
    assert!(page1.next_cursor.is_some());

    let page2 = s.history(&conv_id, 2, page1.next_cursor.as_deref()).unwrap();
    assert_eq!(page2.messages[0].id, "msg:2");
    assert_eq!(page2.messages[1].id, "msg:3");
}

#[test]
fn local_identity_is_generated_once_and_persists() {
    let s = store();
    let first = s.local_identity_or_generate(|| (vec![1, 2, 3], vec![4, 5, 6])).unwrap();
    let second = s.local_identity_or_generate(|| (vec![9, 9, 9], vec![9, 9, 9])).unwrap();
    assert_eq!(
        &*first.account_state, &*second.account_state,
        "must not regenerate on a second call"
    );
    assert_eq!(&*first.sig_secret, &*second.sig_secret);
}

#[test]
fn prekey_rate_limit_refuses_past_the_configured_ceiling() {
    let cfg = ConversationConfig { prekey_requests_per_peer_per_hour: 2, ..Default::default() };
    let dir = tempfile::tempdir().unwrap();
    let path = Box::leak(Box::new(dir)).path();
    let s = ConversationStore::open_encrypted(
        path,
        None,
        QueueConfig {
            retry: RetryPolicy {
                max_attempts: 5,
                initial_backoff_ms: 10,
                backoff_multiplier: 2.0,
                max_backoff_ms: 1000,
            },
            visibility_timeout_ms: 5000,
            dlq_max_rows: 100,
            max_pending_rows: 1000,
        },
        cfg,
    )
    .unwrap();
    let now = 1_000_000;
    assert!(s.record_prekey_request("did:key:zPeer", now).unwrap());
    assert!(s.record_prekey_request("did:key:zPeer", now + 1).unwrap());
    assert!(
        !s.record_prekey_request("did:key:zPeer", now + 2).unwrap(),
        "the third request in the same hour must be refused"
    );
}

#[test]
fn heads_are_the_entries_with_no_child() {
    let s = store();
    let conv_id = "conv:g1";
    // Create conversation
    {
        let conn = s.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 1, 1000).unwrap();
        let entry1 = WireEntry {
            entry_id: "ent:1".to_string(),
            conversation_id: conv_id.to_string(),
            author: "svc:owner".to_string(),
            sender_timestamp_ms: 1000,
            epoch: 1,
            kind: EntryKind::Message,
            parents: vec![],
            ciphertext: Some(vec![1]),
            nonce: Some([0u8; 12]),
            payload: None,
            signature: [0u8; 64],
        };
        ConversationStore::insert_entry_if_absent(&tx, conv_id, &entry1, true, false).unwrap();

        let entry2 = WireEntry {
            entry_id: "ent:2".to_string(),
            conversation_id: conv_id.to_string(),
            author: "svc:owner".to_string(),
            sender_timestamp_ms: 1001,
            epoch: 1,
            kind: EntryKind::Message,
            parents: vec!["ent:1".to_string()],
            ciphertext: Some(vec![2]),
            nonce: Some([0u8; 12]),
            payload: None,
            signature: [0u8; 64],
        };
        ConversationStore::insert_entry_if_absent(&tx, conv_id, &entry2, true, false).unwrap();
        tx.commit().unwrap();
    }

    let heads = s.heads(conv_id).unwrap();
    assert_eq!(heads, vec!["ent:2"]);
}

#[test]
fn the_sync_cursor_never_skips_an_entry_inserted_out_of_timestamp_order() {
    let s = store();
    let conv_id = "conv:g1";
    {
        let conn = s.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 1, 1000).unwrap();
        for (id, ts) in [("ent:3", 3000), ("ent:2", 2000), ("ent:1", 1000)] {
            let entry = WireEntry {
                entry_id: id.to_string(),
                conversation_id: conv_id.to_string(),
                author: "svc:owner".to_string(),
                sender_timestamp_ms: ts,
                epoch: 1,
                kind: EntryKind::Message,
                parents: vec![],
                ciphertext: Some(vec![1]),
                nonce: Some([0u8; 12]),
                payload: None,
                signature: [0u8; 64],
            };
            ConversationStore::insert_entry_if_absent(&tx, conv_id, &entry, true, false).unwrap();
        }
        tx.commit().unwrap();
    }

    let entries = s.entries_after_seq(conv_id, 0, 10).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].entry_id, "ent:3");
    assert_eq!(entries[1].entry_id, "ent:2");
    assert_eq!(entries[2].entry_id, "ent:1");
}

#[test]
fn dag_entry_quota_is_per_conversation() {
    let s = store();
    let conv1 = "conv:g1";
    let conv2 = "conv:g2";
    {
        let conn = s.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        ConversationStore::get_or_create_group_shell(&tx, conv1, "svc:owner", 1, 1000).unwrap();
        ConversationStore::get_or_create_group_shell(&tx, conv2, "svc:owner", 1, 1000).unwrap();

        let entry1 = WireEntry {
            entry_id: "ent:1".to_string(),
            conversation_id: conv1.to_string(),
            author: "svc:owner".to_string(),
            sender_timestamp_ms: 1000,
            epoch: 1,
            kind: EntryKind::Message,
            parents: vec![],
            ciphertext: Some(vec![1]),
            nonce: Some([0u8; 12]),
            payload: None,
            signature: [0u8; 64],
        };
        ConversationStore::insert_entry_if_absent(&tx, conv1, &entry1, true, false).unwrap();
        tx.commit().unwrap();
    }

    assert_eq!(s.dag_entry_count(conv1).unwrap(), 1);
    assert_eq!(s.dag_entry_count(conv2).unwrap(), 0);
}

#[test]
fn recipients_remaining_reaches_zero_only_when_every_member_settles() {
    let s = store();
    let conv_id = s.get_or_create_direct("did:key:zPeer", "conv:1", 1000).unwrap();
    let msg_id = "msg:1";
    {
        let conn = s.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (id, conversation_id, author, sender_timestamp, received_at, \
             content_type, body, signature, outgoing, verified, state, last_error, system, \
             entry_id)
             VALUES (?1, ?2, 'svc:me', 1000, 1000, 'text/plain', X'00', X'00', 1, 1, 'pending', \
             NULL, 0, ?1)",
            params![msg_id, conv_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_recipients (message_id, member_address, state, last_error) \
             VALUES (?1, 'peer1', 'pending', NULL)",
            params![msg_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_recipients (message_id, member_address, state, last_error) \
             VALUES (?1, 'peer2', 'pending', NULL)",
            params![msg_id],
        )
        .unwrap();
    }

    assert_eq!(s.recipients_remaining(msg_id).unwrap(), 2);
    assert!(!s.any_recipient_failed(msg_id).unwrap());

    s.set_recipient_state(msg_id, "peer1", ConversationDeliveryState::Delivered, None).unwrap();
    assert_eq!(s.recipients_remaining(msg_id).unwrap(), 1);

    s.set_recipient_state(msg_id, "peer2", ConversationDeliveryState::Delivered, None).unwrap();
    assert_eq!(s.recipients_remaining(msg_id).unwrap(), 0);
}

#[test]
fn history_and_outbox_exclude_system_messages() {
    let s = store();
    let conv_id = s.get_or_create_direct("did:key:zPeer", "conv:1", 1000).unwrap();
    // Insert one regular message and one system message
    s.insert_outgoing_and_enqueue(
        &conv_id,
        "msg:regular",
        "svc:me",
        1000,
        "text/plain",
        b"hello",
        &[0u8; 64],
        "did:key:zPeer",
        1000,
        false,
    )
    .unwrap();
    s.insert_outgoing_and_enqueue(
        &conv_id,
        "msg:system",
        "svc:me",
        1001,
        "application/vnd.syneroym.group-key+json",
        b"{}",
        &[0u8; 64],
        "did:key:zPeer",
        1001,
        true,
    )
    .unwrap();

    let hist = s.history(&conv_id, 10, None).unwrap();
    assert_eq!(hist.messages.len(), 1);
    assert_eq!(hist.messages[0].id, "msg:regular");

    let outbox = s.outbox_messages().unwrap();
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].id, "msg:regular");
}

#[test]
fn list_conversations_excludes_system_conversations() {
    let s = store();
    let conv_id = "conv:sys";
    {
        let conn = s.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO conversations (id, kind, peer_address, owner_address, current_epoch, \
             system, created_at, last_activity)
             VALUES (?1, 'direct', 'peer_sys', NULL, 0, 1, 1000, 1000)",
            params![conv_id],
        )
        .unwrap();
    }
    let list = s.list_conversations().unwrap();
    assert!(list.is_empty(), "system conversation must be excluded from list_conversations");
}

#[test]
fn get_or_create_direct_clears_the_system_flag_on_an_existing_row() {
    let s = store();
    let peer = "peer_sys";
    let conv_id = "conv:sys";
    {
        let conn = s.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO conversations (id, kind, peer_address, owner_address, current_epoch, \
             system, created_at, last_activity)
             VALUES (?1, 'direct', ?2, NULL, 0, 1, 1000, 1000)",
            params![conv_id, peer],
        )
        .unwrap();
    }

    // Verify initially excluded
    assert!(s.list_conversations().unwrap().is_empty());

    // Now open_direct on the same peer
    let returned_id = s.get_or_create_direct(peer, "conv:ignored", 2000).unwrap();
    assert_eq!(returned_id, conv_id);

    let list = s.list_conversations().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, conv_id);
}

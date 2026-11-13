use std::{thread, time::Duration};

use syneroym_rpc::{
    Admission, ConversationDeliveryState, ConversationError, ConversationHistoryItem,
    ConversationHost,
};

use crate::{
    dag::{DELETION_REQUEST_CONTENT_TYPE, REFUSAL_NOTICE_CONTENT_TYPE},
    store::{SessionRow, now_ms},
    transport::tests::test_service,
};

mod events;

const ME: &str = "svc:me";
const PEER: &str = "svc:peer";

#[tokio::test]
async fn an_app_cannot_send_the_hosts_reserved_content_types() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();

    for reserved in [DELETION_REQUEST_CONTENT_TYPE, REFUSAL_NOTICE_CONTENT_TYPE] {
        let err = service.send(ME, &conv, reserved, vec![], b"{}".to_vec()).await.unwrap_err();
        assert!(matches!(err, ConversationError::InvalidArgument(_)), "{reserved}: {err:?}");
    }
}

#[tokio::test]
async fn a_held_incoming_message_cannot_be_deleted_by_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let store = service.store_for(ME).await.unwrap();
    {
        let conn = store.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        store
            .insert_incoming_if_absent(
                &tx,
                &conv,
                "m:held",
                PEER,
                1_000,
                "text/plain",
                &[],
                b"held text",
                &[0u8; 64],
                1_000,
                100,
            )
            .unwrap();
        tx.commit().unwrap();
    }
    store.apply_admission("m:held", &Admission::Hold("group-hidden".into()), 1_000).unwrap();

    let err = service.delete_message(ME, "m:held", true).await.unwrap_err();

    assert_eq!(err, ConversationError::NotFound, "the app never saw this message");
    assert!(!store.get_message("m:held").unwrap().unwrap().body.is_empty());
}

#[tokio::test]
async fn deleting_an_unsent_message_stops_it_and_still_asks_the_peer() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let id =
        service.send(ME, &conv, "text/plain", vec![], b"not yet delivered".to_vec()).await.unwrap();
    let store = service.store_for(ME).await.unwrap();
    assert_eq!(store.queue().all().unwrap().len(), 1);

    service.delete_message(ME, &id, true).await.unwrap();

    let row = store.get_message(&id).unwrap().unwrap();
    assert!(row.body.is_empty());
    assert_eq!(row.state, ConversationDeliveryState::Failed);
    // The worker skips the message's own queued item, since it is no longer
    // pending. A deletion request is queued too: the send may already have
    // reached the peer, with only the receipt lost.
    assert_eq!(store.queue().all().unwrap().len(), 2);
    let requests: i64 = store
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1 AND system = 1 AND \
             content_type = ?2",
            rusqlite::params![conv, DELETION_REQUEST_CONTENT_TYPE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(requests, 1);
}

#[tokio::test]
async fn a_system_message_does_not_move_a_chat_up_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let id = service.send(ME, &conv, "text/plain", vec![], b"hello".to_vec()).await.unwrap();
    let store = service.store_for(ME).await.unwrap();
    store.set_state(&id, ConversationDeliveryState::Delivered, None).unwrap();
    let before = store.get_conversation(&conv).unwrap().unwrap().last_activity_ms;

    thread::sleep(Duration::from_millis(5));
    service.delete_message(ME, &id, true).await.unwrap();

    let after = store.get_conversation(&conv).unwrap().unwrap().last_activity_ms;
    assert_eq!(after, before, "the deletion request is bookkeeping, not activity");
}

#[tokio::test]
async fn a_cursor_the_host_did_not_issue_is_an_invalid_argument() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;

    let err = service.export_history(ME, Some("not-a-cursor".to_string())).await.unwrap_err();

    assert!(matches!(err, ConversationError::InvalidArgument(_)), "{err:?}");
}

#[tokio::test]
async fn the_transcript_row_count_covers_what_the_digest_covers() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();
    service.send(ME, &conv, "text/plain", vec![], b"mine".to_vec()).await.unwrap();
    let store = service.store_for(ME).await.unwrap();
    {
        let conn = store.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        store
            .insert_incoming_if_absent(
                &tx,
                &conv,
                "m:held",
                PEER,
                1_000,
                "text/plain",
                &[],
                b"held text",
                &[0u8; 64],
                1_000,
                100,
            )
            .unwrap();
        tx.commit().unwrap();
    }
    store.apply_admission("m:held", &Admission::Hold("group-hidden".into()), 1_000).unwrap();

    let transcript = service.transcript_digest(ME, &conv).await.unwrap();

    // History shows one message; the digest, and so the count, also covers
    // the held one.
    assert_eq!(transcript.rows, 2);
    assert!(transcript.digest.starts_with("roym-transcript:"));
}

#[tokio::test]
async fn an_imported_message_that_was_pending_cannot_be_retried() {
    let old_dir = tempfile::tempdir().unwrap();
    let old = test_service(old_dir.path()).await;
    let conv = old.open_direct(ME, PEER).await.unwrap();
    let pending =
        old.send(ME, &conv, "text/plain", vec![], b"never left this machine".to_vec()).await;
    let pending = pending.unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let fresh = test_service(new_dir.path()).await;
    let mut cursor = None;
    loop {
        let chunk = old.export_history(ME, cursor).await.unwrap();
        fresh.import_history(ME, chunk.data).await.unwrap();
        cursor = chunk.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    let err = fresh.retry(ME, &pending).await.unwrap_err();

    assert!(matches!(err, ConversationError::InvalidArgument(_)), "{err:?}");
    let row = fresh.store_for(ME).await.unwrap().get_message(&pending).unwrap().unwrap();
    assert_eq!(row.state, ConversationDeliveryState::Failed);
    assert!(row.restored);
}

#[tokio::test]
async fn deleting_a_group_message_still_on_its_way_sends_the_request_behind_it() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let group = service.create_group(ME).await.unwrap();
    let store = service.store_for(ME).await.unwrap();
    store
        .upsert_session(
            &SessionRow {
                peer_address: "svc:bob".to_string(),
                pinned_sig_key: [42u8; 32],
                state: vec![1, 2, 3],
            },
            now_ms(),
        )
        .unwrap();
    service.add_member(ME, &group, "svc:bob").await.unwrap();
    store
        .insert_outgoing_without_enqueue(
            &group,
            "m:on-its-way",
            ME,
            1_000,
            "text/plain",
            &[],
            b"not everyone has it yet",
            1_000,
            "pending",
        )
        .unwrap();

    service.delete_message(ME, "m:on-its-way", true).await.unwrap();

    assert!(store.get_message("m:on-its-way").unwrap().unwrap().body.is_empty());
    let requests: i64 = store
        .conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id = ?1 AND system = 1 AND \
             content_type = ?2",
            rusqlite::params![group, DELETION_REQUEST_CONTENT_TYPE],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(requests, 1, "the group is asked to delete it too");
}

#[tokio::test]
async fn a_message_deleted_before_delivery_cannot_be_retried() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let id =
        service.send(ME, &conv, "text/plain", vec![], b"changed my mind".to_vec()).await.unwrap();
    service.delete_message(ME, &id, false).await.unwrap();

    let err = service.retry(ME, &id).await.unwrap_err();

    assert!(matches!(err, ConversationError::InvalidArgument(_)), "{err:?}");
    let row = service.store_for(ME).await.unwrap().get_message(&id).unwrap().unwrap();
    assert_eq!(row.state, ConversationDeliveryState::Failed, "it stays settled");
}

#[tokio::test]
async fn send_with_valid_languages_is_stored_and_returned_by_get_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let langs = vec!["hi".to_string(), "en".to_string()];
    let body = "नमस्ते world".as_bytes().to_vec();
    let id = service.send(ME, &conv, "text/plain", langs.clone(), body.clone()).await.unwrap();

    let store = service.store_for(ME).await.unwrap();
    let stored = store.get_message(&id).unwrap().unwrap();
    assert_eq!(stored.languages, langs);
    assert_eq!(stored.body, body);

    let hist = service.history(ME, &conv, 10, None).await.unwrap();
    assert_eq!(hist.items.len(), 1);
    match &hist.items[0] {
        ConversationHistoryItem::Message(m) => {
            assert_eq!(m.id, id);
            assert_eq!(m.languages, langs);
            assert_eq!(m.body, body);
        }
        _ => panic!("expected message item"),
    }
}

#[tokio::test]
async fn send_with_invalid_languages_is_refused_with_invalid_argument_and_stores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();

    let cases = vec![
        vec!["HI".to_string()],
        vec!["en".to_string(), "hi".to_string(), "ta".to_string(), "te".to_string()],
        vec!["hi".to_string(), "hi".to_string()],
    ];

    for bad_langs in cases {
        let err =
            service.send(ME, &conv, "text/plain", bad_langs, b"hello".to_vec()).await.unwrap_err();
        match err {
            ConversationError::InvalidArgument(msg) => {
                assert!(
                    msg.starts_with("message languages:"),
                    "expected error starting with 'message languages:', got: {msg}"
                );
            }
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    let hist = service.history(ME, &conv, 10, None).await.unwrap();
    assert_eq!(hist.items.len(), 0, "nothing should be stored");
}

#[tokio::test]
async fn search_validates_limits_before_searching() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;

    let long_query = "a".repeat(1025);
    let err = service.search(ME, &long_query, None, 10).await.unwrap_err();
    assert_eq!(
        err,
        ConversationError::InvalidArgument("search text is longer than 1024 bytes".to_string())
    );

    let huge_query = "a".repeat(10 * 1024 * 1024);
    let err = service.search(ME, &huge_query, None, 10).await.unwrap_err();
    assert_eq!(
        err,
        ConversationError::InvalidArgument("search text is longer than 1024 bytes".to_string())
    );

    let err = service.search(ME, "!!!", None, 10).await.unwrap_err();
    assert_eq!(
        err,
        ConversationError::InvalidArgument("search needs at least one word".to_string())
    );

    let query_17 = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike \
                    november oscar papa quebec";
    let err = service.search(ME, query_17, None, 10).await.unwrap_err();
    assert_eq!(
        err,
        ConversationError::InvalidArgument("search has more than 16 words".to_string())
    );
}

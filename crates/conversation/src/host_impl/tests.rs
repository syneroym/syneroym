use syneroym_rpc::{Admission, ConversationDeliveryState, ConversationError, ConversationHost};

use crate::{
    dag::{DELETION_REQUEST_CONTENT_TYPE, REFUSAL_NOTICE_CONTENT_TYPE},
    store::{SessionRow, now_ms},
    transport::tests::test_service,
};

const ME: &str = "svc:me";
const PEER: &str = "svc:peer";

#[tokio::test]
async fn an_app_cannot_send_the_hosts_reserved_content_types() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();

    for reserved in [DELETION_REQUEST_CONTENT_TYPE, REFUSAL_NOTICE_CONTENT_TYPE] {
        let err = service.send(ME, &conv, reserved, b"{}".to_vec()).await.unwrap_err();
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
async fn deleting_an_unsent_message_stops_its_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let id = service.send(ME, &conv, "text/plain", b"not yet delivered".to_vec()).await.unwrap();
    let store = service.store_for(ME).await.unwrap();
    assert_eq!(store.queue().all().unwrap().len(), 1);

    service.delete_message(ME, &id, true).await.unwrap();

    let row = store.get_message(&id).unwrap().unwrap();
    assert!(row.body.is_empty());
    assert_eq!(row.state, ConversationDeliveryState::Failed);
    // The worker skips a queued item whose message is no longer pending,
    // and no deletion request joined it.
    assert_eq!(store.queue().all().unwrap().len(), 1);
}

#[tokio::test]
async fn a_system_message_does_not_move_a_chat_up_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let conv = service.open_direct(ME, PEER).await.unwrap();
    let id = service.send(ME, &conv, "text/plain", b"hello".to_vec()).await.unwrap();
    let store = service.store_for(ME).await.unwrap();
    store.set_state(&id, ConversationDeliveryState::Delivered, None).unwrap();
    let before = store.get_conversation(&conv).unwrap().unwrap().last_activity_ms;

    std::thread::sleep(std::time::Duration::from_millis(5));
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
    service.send(ME, &conv, "text/plain", b"mine".to_vec()).await.unwrap();
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
    let pending = old.send(ME, &conv, "text/plain", b"never left this machine".to_vec()).await;
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
    let id = service.send(ME, &conv, "text/plain", b"changed my mind".to_vec()).await.unwrap();
    service.delete_message(ME, &id, false).await.unwrap();

    let err = service.retry(ME, &id).await.unwrap_err();

    assert!(matches!(err, ConversationError::InvalidArgument(_)), "{err:?}");
    let row = service.store_for(ME).await.unwrap().get_message(&id).unwrap().unwrap();
    assert_eq!(row.state, ConversationDeliveryState::Failed, "it stays settled");
}

use super::store;
use crate::{
    ids::derive_conversation_id,
    store::backup::{BACKUP_VERSION, BackupBundle},
};

#[test]
fn export_validates_cursor_strictly() {
    let s = store();
    let res = s.export_history(Some("bad_cursor_offset".to_string()));
    assert!(res.is_err(), "malformed cursor must return error");
}

#[test]
fn import_checks_backup_version() {
    let s = store();
    let bundle = BackupBundle {
        version: BACKUP_VERSION + 99,
        conversations: vec![],
        messages: vec![],
        dag_entries: vec![],
        group_members: vec![],
    };
    let data = serde_json::to_vec(&bundle).unwrap();
    let res = s.import_history("did:key:zMe", &data);
    assert!(res.is_err(), "unsupported backup version must be rejected");
}

#[test]
fn import_rejects_invalid_admission_and_state() {
    let s = store();
    let bad_admission_bundle = BackupBundle {
        version: BACKUP_VERSION,
        conversations: vec![],
        messages: vec![crate::store::backup::BackupMessage {
            id: "m:bad".to_string(),
            conversation_id: "conv:1".to_string(),
            author: "a".to_string(),
            sender_timestamp: 1000,
            received_at: 1000,
            content_type: "text/plain".to_string(),
            body: vec![],
            signature: [0u8; 64],
            outgoing: false,
            state: "delivered".to_string(),
            last_error: None,
            system: false,
            entry_id: None,
            admission: "invalid_admission_string".to_string(),
            admission_reason: None,
            deleted_at: None,
        }],
        dag_entries: vec![],
        group_members: vec![],
    };
    let data = serde_json::to_vec(&bad_admission_bundle).unwrap();
    let res = s.import_history("did:key:zMe", &data);
    assert!(res.is_err(), "invalid admission string must be rejected");
}

#[test]
fn transcript_digest_changes_when_messages_are_added() {
    // transcript_digest covers all non-system messages regardless of admission,
    // providing a stable fingerprint of the conversation's content footprint.
    let s = store();
    let conv = s.get_or_create_direct("did:key:zMe", "did:key:zPeer", 1_000).unwrap();
    let now = 1_000;

    s.insert_outgoing_and_enqueue(
        &conv,
        "m:1",
        "did:key:zMe",
        now,
        "text/plain",
        b"hello transcript",
        &[0u8; 64],
        "did:key:zPeer",
        now,
        false,
    )
    .unwrap();

    let digest1 = s.transcript_digest(&conv).unwrap();
    assert!(digest1.starts_with("roym-transcript:"));

    // Add a second message — digest must change.
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        s.insert_incoming_if_absent(
            &tx,
            &conv,
            "m:2",
            "did:key:zPeer",
            now + 1,
            "text/plain",
            b"second message",
            &[0u8; 64],
            now + 1,
            100,
        )
        .unwrap();
        tx.commit().unwrap();
    }

    // Digest changes even before admission is applied — all non-system rows
    // contribute to the fingerprint.
    let digest2 = s.transcript_digest(&conv).unwrap();
    assert_ne!(digest1, digest2, "adding a message must change the digest");
}

const ME: &str = "svc:me";
const PEER: &str = "svc:peer";

/// Exports every chunk and returns them in order.
fn export_all(s: &crate::store::ConversationStore) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut cursor = None;
    loop {
        let chunk = s.export_history(cursor).unwrap();
        chunks.push(chunk.data);
        cursor = chunk.next_cursor;
        if cursor.is_none() {
            return chunks;
        }
    }
}

fn node_with_history(messages: usize) -> (crate::store::ConversationStore, String) {
    let s = store();
    let conv_id = derive_conversation_id(ME, PEER);
    let conv = s.get_or_create_direct(PEER, &conv_id, 1_000).unwrap();
    for i in 0..messages {
        s.insert_outgoing_without_enqueue(
            &conv,
            &format!("m:{i:04}"),
            ME,
            1_000 + i as i64,
            "text/plain",
            format!("message number {i:04} about topic{i:04}").as_bytes(),
            1_000 + i as i64,
            "delivered",
        )
        .unwrap();
    }
    (s, conv)
}

#[test]
fn a_restore_at_the_same_address_leaves_the_chat_live() {
    let (old, conv) = node_with_history(3);
    let fresh = store();

    for chunk in export_all(&old) {
        fresh.import_history(ME, &chunk).unwrap();
    }

    let row = fresh.get_conversation(&conv).unwrap().unwrap();
    assert!(!row.restored, "the same address means the same chat");
    let reopened = fresh.get_or_create_direct(PEER, &derive_conversation_id(ME, PEER), 5_000);
    assert_eq!(reopened.unwrap(), conv, "open-direct must find the restored chat, not fail");
    let first = fresh.get_message("m:0000").unwrap().unwrap();
    assert!(!first.restored, "a message in a live chat is not read-only history");
}

#[test]
fn a_restore_at_another_address_is_read_only_history() {
    let (old, conv) = node_with_history(3);
    let fresh = store();

    for chunk in export_all(&old) {
        fresh.import_history("svc:somewhere-else", &chunk).unwrap();
    }

    assert!(fresh.get_conversation(&conv).unwrap().unwrap().restored);
    let live = fresh
        .get_or_create_direct(PEER, &derive_conversation_id("svc:somewhere-else", PEER), 5_000)
        .unwrap();
    assert_ne!(live, conv, "new messages go to a new chat, not into the restored one");
}

#[test]
fn a_large_history_comes_in_pages_and_each_message_arrives_once() {
    let (old, _) = node_with_history(450);
    let chunks = export_all(&old);
    assert!(chunks.len() > 2, "450 messages cannot fit one page");

    let fresh = store();
    let imported: u32 = chunks.iter().map(|c| fresh.import_history(ME, c).unwrap()).sum();
    assert_eq!(imported, 450);

    // Running the same import again is the way to resume after a failure.
    let again: u32 = chunks.iter().map(|c| fresh.import_history(ME, c).unwrap()).sum();
    assert_eq!(again, 0);
}

#[test]
fn a_reimported_message_is_searchable_under_its_own_row() {
    let (old, _) = node_with_history(3);
    let fresh = store();
    let chunks = export_all(&old);
    for chunk in &chunks {
        fresh.import_history(ME, chunk).unwrap();
    }
    for chunk in &chunks {
        fresh.import_history(ME, chunk).unwrap();
    }

    let found = fresh.search("topic0001", None, 10).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "m:0001");
}

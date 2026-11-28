use super::store;
use crate::store::backup::{BACKUP_VERSION, BackupBundle, BackupConversation, BackupMessage};

const CHAT: &str = "application/vnd.roym.message+json";

fn chat_body(text: &str, origin: &str) -> Vec<u8> {
    serde_json::json!({
        "chat_version": 1,
        "content_type": "text/plain",
        "text": text,
        "forward_mode": "signed",
        "origin": origin,
    })
    .to_string()
    .into_bytes()
}

#[test]
fn a_chat_body_is_found_by_a_word_of_its_text() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zPeer", "conv:ix-1", 1_000).unwrap();
    s.insert_outgoing_and_enqueue(
        &conv,
        "m:chat",
        "did:key:zMe",
        1_000,
        CHAT,
        &["en".to_string()],
        &chat_body("Market opens at nine", r#"{"envelope":"rec_abc"}"#),
        &[0u8; 64],
        "did:key:zPeer",
        1_000,
        false,
    )
    .unwrap();

    let hits = s.search("Market", Some(&conv), 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m:chat");
}

#[test]
fn a_chat_body_is_not_found_by_signed_or_forward_mode_or_an_origin_fragment() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zPeer", "conv:ix-2", 1_000).unwrap();
    s.insert_outgoing_and_enqueue(
        &conv,
        "m:chat",
        "did:key:zMe",
        1_000,
        CHAT,
        &[],
        &chat_body("Market opens at nine", r#"{"envelope":"rec_abc"}"#),
        &[0u8; 64],
        "did:key:zPeer",
        1_000,
        false,
    )
    .unwrap();

    for query in ["signed", "forward_mode", "envelope", "chat_version"] {
        let hits = s.search(query, Some(&conv), 10).unwrap();
        assert!(hits.is_empty(), "{query} must not match a chat body");
    }
}

#[test]
fn a_widget_body_is_still_found_by_its_whole_text() {
    let s = store();
    let conv = s.get_or_create_direct("did:key:zPeer", "conv:ix-3", 1_000).unwrap();
    let body = serde_json::json!({
        "widget_version": 1,
        "widget": { "prompt": "Favorite color" },
    })
    .to_string()
    .into_bytes();
    s.insert_outgoing_and_enqueue(
        &conv,
        "m:widget",
        "did:key:zMe",
        1_000,
        "application/vnd.roym.widget+json",
        &[],
        &body,
        &[0u8; 64],
        "did:key:zPeer",
        1_000,
        false,
    )
    .unwrap();

    let hits = s.search("Favorite", Some(&conv), 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "m:widget");
}

#[test]
fn a_restored_chat_body_is_indexed_by_the_same_rule() {
    let fresh = store();
    let bundle = BackupBundle {
        version: BACKUP_VERSION,
        conversations: vec![BackupConversation {
            id: "conv:ix-4".to_string(),
            kind: "direct".to_string(),
            peer_address: Some("did:key:zPeer".to_string()),
            owner_address: None,
            current_epoch: 0,
            name: None,
            created_at: 1_000,
            last_activity: 1_000,
        }],
        messages: vec![BackupMessage {
            id: "m:restored".to_string(),
            conversation_id: "conv:ix-4".to_string(),
            author: "did:key:zMe".to_string(),
            sender_timestamp: 1_000,
            received_at: 1_000,
            content_type: CHAT.to_string(),
            languages: vec![],
            body: chat_body("Market opens at nine", r#"{"envelope":"rec_abc"}"#),
            signature: [0u8; 64],
            outgoing: true,
            state: "delivered".to_string(),
            last_error: None,
            system: false,
            entry_id: None,
            admission: "accepted".to_string(),
            admission_reason: None,
            deleted_at: None,
        }],
        dag_entries: vec![],
        group_members: vec![],
    };
    let data = serde_json::to_vec(&bundle).unwrap();
    fresh.import_history("did:key:zMe", &data).unwrap();

    assert_eq!(fresh.search("Market", Some("conv:ix-4"), 10).unwrap().len(), 1);
    assert!(fresh.search("signed", Some("conv:ix-4"), 10).unwrap().is_empty());
}

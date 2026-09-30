#![allow(clippy::cognitive_complexity)]

use ed25519_dalek::SigningKey;
use rand::RngCore;
use syneroym_async_queue::QueueConfig;
use syneroym_core::config::RetryPolicy;
use syneroym_rpc::ConversationHost;

use super::*;
use crate::{
    crypto::{SessionCrypto, X3dhDoubleRatchetCrypto},
    dag::{self, EntryKind, GroupSyncRequest, PeerAssertion},
    envelope, group,
    store::{self, ConversationConfig, ConversationStore, SessionRow},
};

fn test_store() -> ConversationStore {
    let dir = tempfile::tempdir().unwrap();
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

async fn test_service(dir: &std::path::Path) -> std::sync::Arc<ConversationService> {
    let storage_provider: std::sync::Arc<dyn syneroym_data_db::traits::StorageProvider> =
        std::sync::Arc::new(
            syneroym_data_db::SqliteStorageProvider::new(dir.join("data"), false).unwrap(),
        );
    let key_store = std::sync::Arc::new(syneroym_data_keystore::KeyStore::new());
    let registry = syneroym_core::local_registry::EndpointRegistry::new(std::sync::Arc::new(
        syneroym_core::storage::MockStorage::new(),
    ))
    .await
    .unwrap();
    ConversationService::new(
        storage_provider,
        key_store,
        registry,
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
        crate::ConversationConfig::default(),
    )
    .unwrap()
}

/// A peer whose `payload.author` does not match the address the inbound
/// session is keyed on must be refused — this is what prevents peer C
/// from attributing a message to peer A.
#[tokio::test]
async fn a_mismatched_author_in_the_payload_is_refused() {
    use syneroym_rpc::ConversationHost;

    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let crypto = X3dhDoubleRatchetCrypto::new();

    let bundle_bytes = service.prekey_bundle("b-address", "a-address").await.unwrap();
    let bundle: PrekeyBundle = serde_json::from_slice(&bundle_bytes).unwrap();

    let store_a = test_store();
    let mut session_a =
        crypto.begin_session(&store_a, "a-address", "b-address", &bundle).await.unwrap();

    // Forge an envelope: session is from "a-address" but author claims "c-address".
    // Sign with store_a's legitimate signing key so that envelope::verify
    // passes under the pinned key, isolating the author == session.peer_address
    // guard.
    let identity =
        store_a.local_identity_or_generate(crate::crypto::generate_identity_bytes).unwrap();
    let sig_bytes: [u8; 32] = identity.sig_secret.as_slice().try_into().unwrap();
    let signing_key = SigningKey::from_bytes(&sig_bytes);
    let forged_sig = envelope::sign(
        &signing_key,
        "msg:forged",
        &crate::ids::derive_conversation_id("b-address", "c-address"),
        "c-address",
        1_000,
        "text/plain",
        b"hi",
    );
    let forged_payload = DeliveryPayload {
        message_id: "msg:forged".to_string(),
        conversation_id: crate::ids::derive_conversation_id("b-address", "c-address"),
        author: "c-address".to_string(),
        sender_timestamp_ms: 1_000,
        content_type: "text/plain".to_string(),
        body: b"hi".to_vec(),
        signature: forged_sig,
    };
    let env = crypto.encrypt(&mut session_a, &forged_payload).unwrap();
    let env_bytes = serde_json::to_vec(&env).unwrap();

    let result = service.peer_deliver("b-address", "a-address", env_bytes).await;
    assert!(
        matches!(result, Err(ConversationError::PermissionDenied)),
        "mismatched author must be rejected with PermissionDenied, got {result:?}"
    );
}

/// A sender that claims `author == svc` (the same-service exemption
/// path) must be refused.
#[tokio::test]
async fn self_injection_via_same_service_is_refused() {
    use syneroym_rpc::ConversationHost;

    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let crypto = X3dhDoubleRatchetCrypto::new();

    let bundle_bytes = service.prekey_bundle("b-address", "b-address").await.unwrap();
    let bundle: PrekeyBundle = serde_json::from_slice(&bundle_bytes).unwrap();

    let store_a = test_store();
    let mut session_a =
        crypto.begin_session(&store_a, "b-address", "b-address", &bundle).await.unwrap();

    // Forge: sign a payload where author == the receiver's own address.
    // Sign with store_a's legitimate signing key so envelope::verify
    // passes under the pinned key, isolating the author == svc guard.
    let identity =
        store_a.local_identity_or_generate(crate::crypto::generate_identity_bytes).unwrap();
    let sig_bytes: [u8; 32] = identity.sig_secret.as_slice().try_into().unwrap();
    let signing_key = SigningKey::from_bytes(&sig_bytes);
    let self_sig = envelope::sign(
        &signing_key,
        "msg:self",
        &crate::ids::derive_conversation_id("b-address", "b-address"),
        "b-address", // author == receiver
        1_000,
        "text/plain",
        b"hi",
    );
    let self_payload = DeliveryPayload {
        message_id: "msg:self".to_string(),
        conversation_id: crate::ids::derive_conversation_id("b-address", "b-address"),
        author: "b-address".to_string(),
        sender_timestamp_ms: 1_000,
        content_type: "text/plain".to_string(),
        body: b"hi".to_vec(),
        signature: self_sig,
    };
    let env = crypto.encrypt(&mut session_a, &self_payload).unwrap();
    let env_bytes = serde_json::to_vec(&env).unwrap();

    let result = service.peer_deliver("b-address", "b-address", env_bytes).await;
    assert!(
        matches!(result, Err(ConversationError::PermissionDenied)),
        "self-injection must be rejected with PermissionDenied, got {result:?}"
    );
}

/// Row 8: a `GroupKeyPayload` whose `owner` field does not match the
/// verified sender of the envelope carrying it must be refused. Drives
/// the real path (`peer_deliver_impl`), not just the store call it
/// happens to use — a forged `owner` field is exactly what an
/// unprivileged peer would send to try to seed a group it does not
/// control.
#[tokio::test]
async fn group_key_payload_claiming_a_different_owner_than_the_signer_is_rejected() {
    use syneroym_rpc::ConversationHost;

    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let crypto = X3dhDoubleRatchetCrypto::new();

    let bundle_bytes = service.prekey_bundle("victim", "attacker").await.unwrap();
    let bundle: PrekeyBundle = serde_json::from_slice(&bundle_bytes).unwrap();

    let store_attacker = test_store();
    let mut session_attacker =
        crypto.begin_session(&store_attacker, "attacker", "victim", &bundle).await.unwrap();

    let identity =
        store_attacker.local_identity_or_generate(crate::crypto::generate_identity_bytes).unwrap();
    let sig_bytes: [u8; 32] = identity.sig_secret.as_slice().try_into().unwrap();
    let signing_key = SigningKey::from_bytes(&sig_bytes);

    // The payload's author is "attacker" (matches the session, so the
    // envelope itself is legitimately signed), but the embedded
    // GroupKeyPayload claims "some-other-owner" owns the group.
    let key_payload = crate::dag::GroupKeyPayload {
        group_id: "conv:forged".to_string(),
        epoch: 1,
        key: [5u8; 32],
        members: vec!["attacker".to_string(), "victim".to_string()],
        owner: "some-other-owner".to_string(),
    };
    let body = serde_json::to_vec(&key_payload).unwrap();
    let sig = envelope::sign(
        &signing_key,
        "msg:forged-key",
        &crate::ids::derive_conversation_id("victim", "attacker"),
        "attacker",
        1_000,
        crate::dag::GROUP_KEY_CONTENT_TYPE,
        &body,
    );
    let payload = DeliveryPayload {
        message_id: "msg:forged-key".to_string(),
        conversation_id: crate::ids::derive_conversation_id("victim", "attacker"),
        author: "attacker".to_string(),
        sender_timestamp_ms: 1_000,
        content_type: crate::dag::GROUP_KEY_CONTENT_TYPE.to_string(),
        body,
        signature: sig,
    };
    let env = crypto.encrypt(&mut session_attacker, &payload).unwrap();
    let env_bytes = serde_json::to_vec(&env).unwrap();

    let result = service.peer_deliver("victim", "attacker", env_bytes).await;
    assert!(
        matches!(result, Err(ConversationError::PermissionDenied)),
        "a group-key payload whose owner does not match the signer must be rejected, got \
         {result:?}"
    );
}

#[tokio::test]
async fn group_push_with_unregistered_assertion_sender_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let store = service.store_for("svc:receiver").await.unwrap();

    // Create group on receiver with only svc:owner as member
    {
        let conn = store.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        ConversationStore::get_or_create_group_shell(&tx, "conv:g1", "svc:owner", 1, 1000).unwrap();
        let payload = crate::dag::MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:owner".to_string(),
            subject_sig_key: [1u8; 32],
            new_epoch: 1,
            member_list_hash: "hash".to_string(),
        };
        ConversationStore::apply_membership(&tx, "conv:g1", &payload).unwrap();
        tx.commit().unwrap();
    }

    // Stranger attempts group-push
    let sk = SigningKey::generate(&mut rand_core::OsRng);
    let entry = crate::dag::WireEntry {
        entry_id: "ent:1".to_string(),
        conversation_id: "conv:g1".to_string(),
        author: "svc:stranger".to_string(),
        sender_timestamp_ms: 1000,
        epoch: 1,
        kind: crate::dag::EntryKind::Message,
        parents: vec![],
        ciphertext: Some(vec![1]),
        nonce: Some([0u8; 12]),
        payload: None,
        signature: [0u8; 64],
    };
    let assertion =
        crate::dag::sign_peer_assertion(&sk, "svc:stranger", "conv:g1", 1000, &[0u8; 16]);
    let req = crate::dag::GroupPushRequest {
        from: assertion,
        group: "conv:g1".to_string(),
        entries: vec![entry],
    };

    let res = service.group_push_impl("svc:receiver", "svc:stranger", req).await;
    assert!(matches!(res, Err(ConversationError::PermissionDenied)));
}

fn pending_message(id: &str, body: &[u8]) -> StoredMessage {
    StoredMessage {
        id: id.to_string(),
        conversation_id: "conv:1".to_string(),
        author: "svc-a".to_string(),
        sender_timestamp_ms: 1_000,
        received_at_ms: 1_000,
        content_type: "text/plain".to_string(),
        body: body.to_vec(),
        signature: [0u8; 64],
        outgoing: true,
        verified: true,
        state: crate::ConversationDeliveryState::Pending,
        last_error: None,
        system: false,
        entry_id: None,
    }
}

/// A peer that is away gets many delivery attempts for one message. vodozemac
/// refuses a message more than 2000 steps ahead of what the receiver has
/// seen, so an attempt must never move the ratchet: the message is encrypted
/// once, and the same bytes are sent every time.
#[tokio::test]
async fn attempts_at_an_unreachable_peer_do_not_move_the_ratchet() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let store_a = service.store_for("svc-a").await.unwrap();
    let crypto_b = X3dhDoubleRatchetCrypto::new();
    let store_b = test_store();
    let bundle = crypto_b.prekey_bundle(&store_b).await.unwrap();
    let session = service.crypto.begin_session(&store_a, "svc-a", "svc-b", &bundle).await.unwrap();
    service.crypto.commit(&store_a, &session).await.unwrap();

    let first = pending_message("msg:1", b"first");
    let sealed = service.sealed_envelope("svc-a", &store_a, "svc-b", &first).await.unwrap();
    for _ in 0..2_500 {
        let again = service.sealed_envelope("svc-a", &store_a, "svc-b", &first).await.unwrap();
        assert_eq!(again, sealed, "a retry resends the stored bytes");
    }

    let second = pending_message("msg:2", b"second");
    let next = service.sealed_envelope("svc-a", &store_a, "svc-b", &second).await.unwrap();
    for (label, value) in [("first", sealed), ("second", next)] {
        let env: Envelope = serde_json::from_value(value).unwrap();
        let mut at_b = crypto_b.session_for_envelope(&store_b, &env).await.unwrap();
        let payload = crypto_b
            .decrypt(&mut at_b, &env)
            .unwrap_or_else(|e| panic!("the {label} message must decrypt: {e}"));
        assert_eq!(payload.message_id, if label == "first" { "msg:1" } else { "msg:2" });
        crypto_b.commit(&store_b, &at_b).await.unwrap();
    }

    store_a.delete_outbound_envelope("msg:1").unwrap();
    assert!(store_a.outbound_envelope("msg:1").unwrap().is_none());
}

#[tokio::test]
async fn removed_member_can_sync_its_own_removal() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let owner = "svc:owner";
    let bob = "svc:bob";

    let group_id = service.create_group_impl(owner).await.unwrap();
    let store_owner = service.store_for(owner).await.unwrap();

    let bob_sk = SigningKey::from_bytes(&[88u8; 32]);
    let bob_vk = bob_sk.verifying_key().to_bytes();

    store_owner
        .upsert_session(
            &crate::store::SessionRow {
                peer_address: bob.to_string(),
                pinned_sig_key: bob_vk,
                state: vec![1],
            },
            crate::store::now_ms(),
        )
        .unwrap();

    // Owner adds Bob, then removes Bob
    service.add_member(owner, &group_id, bob).await.unwrap();
    service.remove_member(owner, &group_id, bob).await.unwrap();

    // Bob sends a group-sync request to owner
    let now = crate::store::now_ms();
    let mut nonce = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::rng(), &mut nonce);
    let assertion = crate::dag::sign_peer_assertion(&bob_sk, bob, &group_id, now, &nonce);

    let req = crate::dag::GroupSyncRequest {
        from: assertion,
        group: group_id.clone(),
        after_seq: 0,
        limit: 10,
    };

    let resp = service.group_sync_impl(owner, bob, req).await.unwrap();
    assert!(!resp.entries.is_empty(), "removed member must receive removal history");
    let has_removal = resp.entries.iter().any(|e| {
        e.kind == crate::dag::EntryKind::Membership
            && e.payload.as_ref().map(|p| p.action == "remove").unwrap_or(false)
    });
    assert!(has_removal, "sync response must contain removal entry");
}

fn sign_assertion(sk: &SigningKey, addr: &str, group: &str) -> PeerAssertion {
    let now = store::now_ms();
    let mut nonce = [0u8; 16];
    rand::rng().fill_bytes(&mut nonce);
    dag::sign_peer_assertion(sk, addr, group, now, &nonce)
}

#[tokio::test]
async fn a_removed_member_catches_up_on_messages_from_before_its_removal() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let owner = "svc:owner";
    let bob = "svc:bob";

    let group_id = service.create_group_impl(owner).await.unwrap();
    let store_owner = service.store_for(owner).await.unwrap();

    let bob_sk = SigningKey::from_bytes(&[88u8; 32]);
    let bob_vk = bob_sk.verifying_key().to_bytes();
    store_owner
        .upsert_session(
            &SessionRow { peer_address: bob.to_string(), pinned_sig_key: bob_vk, state: vec![1] },
            store::now_ms(),
        )
        .unwrap();

    service.add_member(owner, &group_id, bob).await.unwrap();
    let msg_id =
        service.send(owner, &group_id, "text/plain", b"pre-removal".to_vec()).await.unwrap();
    service.remove_member(owner, &group_id, bob).await.unwrap();

    let req = GroupSyncRequest {
        from: sign_assertion(&bob_sk, bob, &group_id),
        group: group_id.clone(),
        after_seq: 0,
        limit: 10,
    };
    let resp = service.group_sync_impl(owner, bob, req).await.unwrap();

    let has_msg = resp.entries.iter().any(|e| e.kind == EntryKind::Message && e.entry_id == msg_id);
    let has_removal = resp.entries.iter().any(|e| {
        e.kind == EntryKind::Membership
            && e.payload.as_ref().map(|p| p.action == "remove").unwrap_or(false)
    });
    assert!(has_msg, "sync response must contain pre-removal message");
    assert!(has_removal, "sync response must contain removal entry");
}

#[tokio::test]
async fn a_removed_member_gets_no_old_epoch_message_signed_after_its_removal() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let owner = "svc:owner";
    let bob = "svc:bob";
    let alice = "svc:alice";

    let group_id = service.create_group_impl(owner).await.unwrap();
    let store_owner = service.store_for(owner).await.unwrap();

    let bob_sk = SigningKey::from_bytes(&[88u8; 32]);
    let bob_vk = bob_sk.verifying_key().to_bytes();
    store_owner
        .upsert_session(
            &SessionRow { peer_address: bob.to_string(), pinned_sig_key: bob_vk, state: vec![1] },
            store::now_ms(),
        )
        .unwrap();

    let alice_sk = SigningKey::from_bytes(&[77u8; 32]);
    let alice_vk = alice_sk.verifying_key().to_bytes();
    store_owner
        .upsert_session(
            &SessionRow {
                peer_address: alice.to_string(),
                pinned_sig_key: alice_vk,
                state: vec![1],
            },
            store::now_ms(),
        )
        .unwrap();

    service.add_member(owner, &group_id, bob).await.unwrap();
    service.add_member(owner, &group_id, alice).await.unwrap();
    service.remove_member(owner, &group_id, bob).await.unwrap();

    let removed_epoch = 4;
    let removed_at_ms = store_owner
        .removal_entry_timestamp(&group_id, bob, removed_epoch)
        .unwrap()
        .expect("removal timestamp must exist");

    let epoch_key = store_owner.epoch_key(&group_id, 3).unwrap().unwrap();
    let e_before = group::build_message_entry(
        &alice_sk,
        &group_id,
        alice,
        removed_at_ms - 1_000,
        3,
        vec![],
        &epoch_key,
        b"signed before removal",
    )
    .unwrap();
    let e_after = group::build_message_entry(
        &alice_sk,
        &group_id,
        alice,
        removed_at_ms + 1_000,
        3,
        vec![],
        &epoch_key,
        b"signed after removal",
    )
    .unwrap();

    {
        let conn = store_owner.conn.lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        ConversationStore::insert_entry_if_absent(&tx, &group_id, &e_before, true, false).unwrap();
        ConversationStore::insert_entry_if_absent(&tx, &group_id, &e_after, true, false).unwrap();
        tx.commit().unwrap();
    }

    let req = GroupSyncRequest {
        from: sign_assertion(&bob_sk, bob, &group_id),
        group: group_id.clone(),
        after_seq: 0,
        limit: 10,
    };
    let resp = service.group_sync_impl(owner, bob, req).await.unwrap();

    let has_before = resp.entries.iter().any(|e| e.entry_id == e_before.entry_id);
    let has_after = resp.entries.iter().any(|e| e.entry_id == e_after.entry_id);
    let has_removal = resp.entries.iter().any(|e| {
        e.kind == EntryKind::Membership
            && e.payload.as_ref().map(|p| p.action == "remove").unwrap_or(false)
    });

    assert!(has_before, "message signed before removal must be served to removed member");
    assert!(!has_after, "message signed after removal must NOT be served to removed member");
    assert!(has_removal, "removal entry must be served");
}

#[tokio::test]
async fn a_removed_member_is_served_nothing_before_the_removal_entry_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let owner = "svc:owner";
    let bob = "svc:bob";

    let group_id = service.create_group_impl(owner).await.unwrap();
    let store_owner = service.store_for(owner).await.unwrap();

    let bob_sk = SigningKey::from_bytes(&[88u8; 32]);
    let bob_vk = bob_sk.verifying_key().to_bytes();
    store_owner
        .upsert_session(
            &SessionRow { peer_address: bob.to_string(), pinned_sig_key: bob_vk, state: vec![1] },
            store::now_ms(),
        )
        .unwrap();

    service.add_member(owner, &group_id, bob).await.unwrap();

    {
        let conn = store_owner.conn.lock().unwrap();
        conn.execute(
            "UPDATE group_members SET removed_epoch = 99 WHERE conversation_id = ?1 AND \
             member_address = ?2",
            rusqlite::params![group_id, bob],
        )
        .unwrap();
    }

    let req = GroupSyncRequest {
        from: sign_assertion(&bob_sk, bob, &group_id),
        group: group_id.clone(),
        after_seq: 5,
        limit: 10,
    };
    let resp = service.group_sync_impl(owner, bob, req).await.unwrap();
    assert!(resp.entries.is_empty());
    assert_eq!(resp.next_seq, 5);
}

#[tokio::test]
async fn a_removed_member_gets_no_membership_entry_after_its_removal() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let owner = "svc:owner";
    let bob = "svc:bob";
    let charlie = "svc:charlie";

    let group_id = service.create_group_impl(owner).await.unwrap();
    let store_owner = service.store_for(owner).await.unwrap();

    let bob_sk = SigningKey::from_bytes(&[88u8; 32]);
    let bob_vk = bob_sk.verifying_key().to_bytes();
    store_owner
        .upsert_session(
            &SessionRow { peer_address: bob.to_string(), pinned_sig_key: bob_vk, state: vec![1] },
            store::now_ms(),
        )
        .unwrap();

    let charlie_sk = SigningKey::from_bytes(&[66u8; 32]);
    let charlie_vk = charlie_sk.verifying_key().to_bytes();
    store_owner
        .upsert_session(
            &SessionRow {
                peer_address: charlie.to_string(),
                pinned_sig_key: charlie_vk,
                state: vec![1],
            },
            store::now_ms(),
        )
        .unwrap();

    service.add_member(owner, &group_id, bob).await.unwrap();
    service.remove_member(owner, &group_id, bob).await.unwrap();
    service.add_member(owner, &group_id, charlie).await.unwrap();

    let req = GroupSyncRequest {
        from: sign_assertion(&bob_sk, bob, &group_id),
        group: group_id.clone(),
        after_seq: 0,
        limit: 10,
    };
    let resp = service.group_sync_impl(owner, bob, req).await.unwrap();

    let has_bob_removal = resp.entries.iter().any(|e| {
        e.kind == EntryKind::Membership
            && e.payload
                .as_ref()
                .map(|p| p.action == "remove" && p.subject_address == bob)
                .unwrap_or(false)
    });
    let has_charlie_add = resp.entries.iter().any(|e| {
        e.kind == EntryKind::Membership
            && e.payload.as_ref().map(|p| p.subject_address == charlie).unwrap_or(false)
    });

    assert!(has_bob_removal, "removal entry must be present");
    assert!(!has_charlie_add, "membership changes after removal must not be served");
}

#[tokio::test]
async fn a_stranger_is_still_refused() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path()).await;
    let owner = "svc:owner";
    let stranger = "svc:stranger";

    let group_id = service.create_group_impl(owner).await.unwrap();

    let stranger_sk = SigningKey::from_bytes(&[99u8; 32]);
    let req = GroupSyncRequest {
        from: sign_assertion(&stranger_sk, stranger, &group_id),
        group: group_id.clone(),
        after_seq: 0,
        limit: 10,
    };

    let err = service.group_sync_impl(owner, stranger, req).await.unwrap_err();
    assert_eq!(err, ConversationError::PermissionDenied);
}

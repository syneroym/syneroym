use ed25519_dalek::SigningKey;
use syneroym_async_queue::QueueConfig;
use syneroym_core::config::RetryPolicy;

use super::*;
use crate::{
    crypto::{SessionCrypto, X3dhDoubleRatchetCrypto},
    envelope,
    store::{ConversationConfig, ConversationStore},
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

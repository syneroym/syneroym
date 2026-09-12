use syneroym_async_queue::QueueConfig;
use syneroym_core::config::RetryPolicy;

use super::*;

fn store() -> ConversationStore {
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

#[test]
fn an_entry_authored_before_the_author_joined_is_refused() {
    let s = store();
    let conv_id = "conv:g1";
    let now = 1000;
    let conv = {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let c = ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 2, now)
            .unwrap();
        // Author joined at epoch 2
        let payload = MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:bob".to_string(),
            subject_sig_key: [1u8; 32],
            new_epoch: 2,
            member_list_hash: "hash".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &payload).unwrap();
        tx.commit().unwrap();
        c
    };

    // Entry at epoch 1 from svc:bob should be refused
    let sk = SigningKey::from_bytes(&[1u8; 32]);
    let entry = build_membership_entry(
        &sk,
        conv_id,
        "svc:bob",
        now,
        1,
        vec![],
        MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:bob".to_string(),
            subject_sig_key: [1u8; 32],
            new_epoch: 1,
            member_list_hash: "hash".to_string(),
        },
    );

    let res = validate_and_insert(&s, "svc:me", &conv, &entry);
    assert!(matches!(res, Err(ConversationError::PermissionDenied)));
}

#[test]
fn an_entry_authored_after_the_author_was_removed_is_refused() {
    let s = store();
    let conv_id = "conv:g1";
    let now = 1000;
    let conv = {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let c = ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 3, now)
            .unwrap();
        // Author joined at epoch 1, removed at epoch 2
        let add_payload = MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:bob".to_string(),
            subject_sig_key: [1u8; 32],
            new_epoch: 1,
            member_list_hash: "hash1".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &add_payload).unwrap();
        let remove_payload = MembershipPayload {
            action: "remove".to_string(),
            subject_address: "svc:bob".to_string(),
            subject_sig_key: [1u8; 32],
            new_epoch: 2,
            member_list_hash: "hash2".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &remove_payload).unwrap();
        tx.commit().unwrap();
        c
    };

    // Entry at epoch 2 from svc:bob should be refused
    let sk = SigningKey::from_bytes(&[1u8; 32]);
    let epoch_key = [9u8; 32];
    let entry =
        build_message_entry(&sk, conv_id, "svc:bob", now, 2, vec![], &epoch_key, b"hello").unwrap();

    let res = validate_and_insert(&s, "svc:me", &conv, &entry);
    assert!(matches!(res, Err(ConversationError::PermissionDenied)));
}

#[test]
fn a_membership_entry_not_signed_by_the_owner_is_refused() {
    let s = store();
    let conv_id = "conv:g1";
    let now = 1000;
    let conv = {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let c = ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 1, now)
            .unwrap();
        let payload = MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:bob".to_string(),
            subject_sig_key: [1u8; 32],
            new_epoch: 1,
            member_list_hash: "hash".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &payload).unwrap();
        tx.commit().unwrap();
        c
    };

    let sk = SigningKey::from_bytes(&[1u8; 32]);
    let entry = build_membership_entry(
        &sk,
        conv_id,
        "svc:bob",
        now,
        1,
        vec![],
        MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:charlie".to_string(),
            subject_sig_key: [2u8; 32],
            new_epoch: 2,
            member_list_hash: "hash2".to_string(),
        },
    );

    let res = validate_and_insert(&s, "svc:me", &conv, &entry);
    assert!(matches!(res, Err(ConversationError::PermissionDenied)));
}

#[test]
fn an_entry_whose_epoch_key_is_absent_stays_unapplied_and_applies_when_the_key_arrives() {
    let s = store();
    let conv_id = "conv:g1";
    let now = 1000;
    let sk = SigningKey::generate(&mut rand_core::OsRng);
    let vk_bytes = sk.verifying_key().to_bytes();
    let conv = {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let c = ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 1, now)
            .unwrap();
        let payload = MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:alice".to_string(),
            subject_sig_key: vk_bytes,
            new_epoch: 1,
            member_list_hash: "hash".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &payload).unwrap();
        tx.commit().unwrap();
        c
    };

    let epoch_key = [7u8; 32];
    let entry = build_message_entry(
        &sk,
        conv_id,
        "svc:alice",
        now,
        1,
        vec![],
        &epoch_key,
        &encode_body("text/plain", b"msg"),
    )
    .unwrap();

    // Validate and insert when key is not in group_epochs
    let (ins, msg_opt) = validate_and_insert(&s, "svc:me", &conv, &entry).unwrap();
    assert!(ins);
    assert!(msg_opt.is_none());

    // Unapplied entries should contain it
    let unapplied = s.unapplied_dag_entries(conv_id).unwrap();
    assert_eq!(unapplied.len(), 1);
    assert_eq!(unapplied[0].entry_id, entry.entry_id);

    // Now key arrives
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) VALUES (?1, 1, \
             ?2, ?3)",
            rusqlite::params![conv_id, epoch_key.as_slice(), now],
        )
        .unwrap();
        let (_, msg_opt) =
            apply_entry(&tx, "svc:me", conv_id, &unapplied[0], s.config(), now).unwrap();
        assert!(msg_opt.is_some());
        tx.commit().unwrap();
    }

    // Unapplied should now be empty
    assert!(s.unapplied_dag_entries(conv_id).unwrap().is_empty());
    let hist = s.history(conv_id, 10, None).unwrap();
    assert_eq!(hist.messages.len(), 1);
    assert_eq!(hist.messages[0].body, b"msg");
}

#[test]
fn members_excludes_a_removed_member_but_the_row_survives_for_signature_checks() {
    let s = store();
    let conv_id = "conv:g1";
    let now = 1000;
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 1, now).unwrap();
        let add_payload = MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:bob".to_string(),
            subject_sig_key: [3u8; 32],
            new_epoch: 1,
            member_list_hash: "h1".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &add_payload).unwrap();
        let rem_payload = MembershipPayload {
            action: "remove".to_string(),
            subject_address: "svc:bob".to_string(),
            subject_sig_key: [3u8; 32],
            new_epoch: 2,
            member_list_hash: "h2".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &rem_payload).unwrap();
        tx.commit().unwrap();
    }

    let members = s.current_members(conv_id).unwrap();
    assert!(!members.contains(&"svc:bob".to_string()));

    // But member_sig_key_at at epoch 1 still resolves
    let key_at_1 = s.member_sig_key_at(conv_id, "svc:bob", 1).unwrap();
    assert_eq!(key_at_1, Some([3u8; 32]));

    // At epoch 2 or later, it does not resolve
    let key_at_2 = s.member_sig_key_at(conv_id, "svc:bob", 2).unwrap();
    assert_eq!(key_at_2, None);
}

#[test]
fn membership_history_orders_on_the_same_three_part_key_as_messages() {
    let s = store();
    let conv_id = "conv:g1";
    let sk = SigningKey::generate(&mut rand_core::OsRng);
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 1, 1000).unwrap();
        let e1 = build_membership_entry(
            &sk,
            conv_id,
            "svc:owner",
            1000,
            1,
            vec![],
            MembershipPayload {
                action: "add".to_string(),
                subject_address: "svc:a".to_string(),
                subject_sig_key: [1u8; 32],
                new_epoch: 1,
                member_list_hash: "h1".to_string(),
            },
        );
        let e2 = build_membership_entry(
            &sk,
            conv_id,
            "svc:owner",
            1000,
            2,
            vec![],
            MembershipPayload {
                action: "add".to_string(),
                subject_address: "svc:b".to_string(),
                subject_sig_key: [2u8; 32],
                new_epoch: 2,
                member_list_hash: "h2".to_string(),
            },
        );
        ConversationStore::insert_entry_if_absent(&tx, conv_id, &e1, true, false).unwrap();
        ConversationStore::insert_entry_if_absent(&tx, conv_id, &e2, true, false).unwrap();
        tx.commit().unwrap();
    }

    let hist = s.membership_history(conv_id).unwrap();
    assert_eq!(hist.len(), 2);
    assert_eq!(hist[0].sender_timestamp, 1000);
    assert_eq!(hist[1].sender_timestamp, 1000);
    assert!(hist[0].entry < hist[1].entry);
}

fn stored_dag_from(entry: &WireEntry) -> StoredDagEntry {
    StoredDagEntry {
        seq: 0,
        entry_id: entry.entry_id.clone(),
        conversation_id: entry.conversation_id.clone(),
        author: entry.author.clone(),
        sender_timestamp_ms: entry.sender_timestamp_ms,
        epoch: entry.epoch,
        kind: entry.kind,
        header: canonical_entry_bytes(entry),
        ciphertext: entry.ciphertext.clone(),
        nonce: entry.nonce,
        payload: entry.payload.clone(),
        signature: entry.signature,
        applied: false,
        relay_pending: false,
        parents: vec![],
    }
}

#[test]
fn row_7_joiner_cannot_decrypt_pre_join_and_removed_cannot_decrypt_post_removal() {
    let s = store();
    let conv_id = "conv:g1";
    let now = 1000;
    let owner_sk = SigningKey::generate(&mut rand_core::OsRng);
    let alice_sk = SigningKey::generate(&mut rand_core::OsRng);

    // Epoch 1 key is created by owner
    let epoch1_key = [1u8; 32];
    let epoch2_key = [2u8; 32];

    // Store has group shell at epoch 2
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        ConversationStore::get_or_create_group_shell(&tx, conv_id, "svc:owner", 2, now).unwrap();
        // Alice joined at epoch 2 (did not join at epoch 1)
        let add_alice = MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:alice".to_string(),
            subject_sig_key: alice_sk.verifying_key().to_bytes(),
            new_epoch: 2,
            member_list_hash: hash_members(&["svc:alice".to_string(), "svc:owner".to_string()]),
        };
        ConversationStore::apply_membership(&tx, conv_id, &add_alice).unwrap();
        // Only epoch 2 key is known to Alice's store — a joiner never receives
        // the key distribution message for an epoch that predates it.
        tx.execute(
            "INSERT INTO group_epochs (conversation_id, epoch, key, created_at) VALUES (?1, 2, \
             ?2, ?3)",
            rusqlite::params![conv_id, epoch2_key.as_slice(), now],
        )
        .unwrap();
        tx.commit().unwrap();
    }

    // An entry from epoch 1 arrives (pre-join for Alice) — the epoch 1 key was
    // never distributed to her, so the store cannot decrypt it into a message.
    let entry_epoch1 = build_message_entry(
        &owner_sk,
        conv_id,
        "svc:owner",
        now,
        1,
        vec![],
        &epoch1_key,
        &encode_body("text/plain", b"epoch 1 secret"),
    )
    .unwrap();
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let stored_dag = stored_dag_from(&entry_epoch1);
        let (_, msg_opt) =
            apply_entry(&tx, "svc:alice", conv_id, &stored_dag, s.config(), now).unwrap();
        assert!(msg_opt.is_none(), "pre-join message must not decrypt without epoch key");
    }

    // Now Bob, who is removed at epoch 3, cannot decrypt a message sent at epoch
    // 3 either — the group-key distribution for epoch 3 is addressed to the
    // remaining members only, so Bob's store never receives that key.
    let bob_sk = SigningKey::generate(&mut rand_core::OsRng);
    {
        let conn = s.conn().lock().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let add_bob = MembershipPayload {
            action: "add".to_string(),
            subject_address: "svc:bob".to_string(),
            subject_sig_key: bob_sk.verifying_key().to_bytes(),
            new_epoch: 1,
            member_list_hash: "irrelevant-for-this-test".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &add_bob).unwrap();
        let remove_bob = MembershipPayload {
            action: "remove".to_string(),
            subject_address: "svc:bob".to_string(),
            subject_sig_key: bob_sk.verifying_key().to_bytes(),
            new_epoch: 3,
            member_list_hash: "irrelevant-for-this-test".to_string(),
        };
        ConversationStore::apply_membership(&tx, conv_id, &remove_bob).unwrap();
        tx.commit().unwrap();
    }
    let epoch3_key = [3u8; 32];
    let entry_epoch3 = build_message_entry(
        &owner_sk,
        conv_id,
        "svc:owner",
        now,
        3,
        vec![],
        &epoch3_key,
        &encode_body("text/plain", b"epoch 3 secret"),
    )
    .unwrap();
    let conn = s.conn().lock().unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    let stored_dag = stored_dag_from(&entry_epoch3);
    let (_, msg_opt) = apply_entry(&tx, "svc:bob", conv_id, &stored_dag, s.config(), now).unwrap();
    assert!(msg_opt.is_none(), "post-removal message must not decrypt without epoch key");
}

// Row 8 ("a group-key payload from a non-owner is rejected") is covered by
// `group_key_payload_claiming_a_different_owner_than_the_signer_is_rejected`
// in transport.rs — that test drives the real rejection path
// (`peer_deliver_impl`), not just the store call it happens to use.

async fn service_for_rekey_test(
    dir: &std::path::Path,
    rekey_secs: u64,
) -> std::sync::Arc<ConversationService> {
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
        crate::ConversationConfig {
            store: ConversationConfig {
                conversation_group_rekey_secs: rekey_secs,
                ..Default::default()
            },
        },
    )
    .unwrap()
}

/// Row 10: a scheduled rekey with stable membership changes the epoch
/// key, and it is the owner who distributes the new key.
#[tokio::test]
async fn row_10_scheduled_rekey_with_stable_membership_changes_the_key() {
    let dir = tempfile::tempdir().unwrap();
    // Rekey interval of zero: the very first tick is already overdue.
    let service = service_for_rekey_test(dir.path(), 0).await;
    let group_id = service.create_group_impl("svc:owner").await.unwrap();

    let store = service.store_for("svc:owner").await.unwrap();
    let (epoch_before, key_before) = {
        let (epoch, _) = store.current_epoch_row(&group_id).unwrap().unwrap();
        let key = store.epoch_key(&group_id, epoch).unwrap().unwrap();
        (epoch, key)
    };

    service.scheduled_rekey_once().await;

    let (epoch_after, _) = store.current_epoch_row(&group_id).unwrap().unwrap();
    assert!(epoch_after > epoch_before, "scheduled rekey must advance the epoch");
    let key_after = store.epoch_key(&group_id, epoch_after).unwrap().unwrap();
    assert_ne!(key_before, key_after, "scheduled rekey must generate a new key");
}

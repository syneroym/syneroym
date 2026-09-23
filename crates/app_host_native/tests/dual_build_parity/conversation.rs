use super::helpers::*;

/// Messaging round trip, with a settle step per build (publish is
/// fire-and-forget; delivery happens on a background task on both builds).
async fn poll_inbox_nonempty<D: Driver>(d: &D) -> Value {
    for _ in 0..50 {
        let result = d.run(r#"{"op":"read-inbox"}"#).await.unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        if v["ok"]["entries"].as_array().is_some_and(|a| !a.is_empty()) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("inbox never became non-empty");
}

#[tokio::test]
async fn both_builds_deliver_a_published_message_to_their_own_inbox() {
    let h = harness().await;

    h.wasm.run(r#"{"op":"subscribe-topic","topic":"chat"}"#).await.unwrap();
    h.native.run(r#"{"op":"subscribe-topic","topic":"chat"}"#).await.unwrap();

    h.wasm.run(r#"{"op":"publish-topic","topic":"chat","payload":"hi from wasm"}"#).await.unwrap();
    h.native
        .run(r#"{"op":"publish-topic","topic":"chat","payload":"hi from native"}"#)
        .await
        .unwrap();

    let wasm_inbox = poll_inbox_nonempty(&h.wasm).await;
    let native_inbox = poll_inbox_nonempty(&h.native).await;

    let wasm_topic = wasm_inbox["ok"]["entries"][0]["topic"].as_str().unwrap();
    let native_topic = native_inbox["ok"]["entries"][0]["topic"].as_str().unwrap();
    // Both namespace to `svc/<SERVICE_ID>/chat` -- byte-identical since both
    // stacks share one service id.
    assert_eq!(wasm_topic, native_topic);
    assert_eq!(wasm_topic, format!("svc/{SERVICE_ID}/chat"));
}

// -- Conversation scenarios not covered by the
// byte-comparison SCENARIOS table (a message id includes a random nonce,
// so `send-message`'s exact output cannot be compared verbatim across
// builds -- these assert on structure instead). --

#[tokio::test]
async fn open_direct_is_idempotent_on_both_builds() {
    let h = harness().await;
    let wasm_id_1 = open_conversation(&h.wasm, "peer-idempotent").await;
    let wasm_id_2 = open_conversation(&h.wasm, "peer-idempotent").await;
    assert_eq!(wasm_id_1, wasm_id_2, "wasm build: a second open-direct must return the same id");

    let native_id_1 = open_conversation(&h.native, "peer-idempotent").await;
    let native_id_2 = open_conversation(&h.native, "peer-idempotent").await;
    assert_eq!(
        native_id_1, native_id_2,
        "native build: a second open-direct must return the same id"
    );

    assert_eq!(wasm_id_1, native_id_1, "both builds must derive the same id for the same peer");
}

async fn open_conversation<D: Driver>(d: &D, peer_address: &str) -> String {
    let result = d
        .run(&format!(r#"{{"op":"open-conversation","peer_address":"{peer_address}"}}"#))
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    v["ok"]["conversation"].as_str().unwrap().to_string()
}

async fn assert_send_writes_pending_and_appears_in_the_outbox<D: Driver>(name: &str, driver: &D) {
    let conv = open_conversation(driver, "peer-send-pending").await;
    let send_result = driver
        .run(&format!(r#"{{"op":"send-message","conversation":"{conv}","body":"hello"}}"#))
        .await
        .unwrap();
    let send_v: Value = serde_json::from_str(&send_result).unwrap();
    let message_id = send_v["ok"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}: send-message did not return a message id: {send_v}"));

    let status_result = driver
        .run(&format!(r#"{{"op":"delivery-status","message":"{message_id}"}}"#))
        .await
        .unwrap();
    let status_v: Value = serde_json::from_str(&status_result).unwrap();
    assert_eq!(
        status_v["ok"]["state"], "pending",
        "{name}: a freshly sent message must be pending"
    );

    let outbox_result = driver.run(r#"{"op":"read-outbox"}"#).await.unwrap();
    let outbox_v: Value = serde_json::from_str(&outbox_result).unwrap();
    let entries = outbox_v["ok"]["outbox"].as_array().unwrap();
    assert!(
        entries.iter().any(|e| e["id"] == message_id),
        "{name}: the outbox must list the just-sent message"
    );
}

#[tokio::test]
async fn send_writes_pending_and_appears_in_the_outbox_on_both_builds() {
    let h = harness().await;
    assert_send_writes_pending_and_appears_in_the_outbox("wasm", &h.wasm).await;
    assert_send_writes_pending_and_appears_in_the_outbox("native", &h.native).await;
}

async fn assert_oversized_body_is_refused<D: Driver>(name: &str, driver: &D, oversized: &str) {
    let conv = open_conversation(driver, "peer-quota").await;
    let result = driver
        .run(&format!(r#"{{"op":"send-message","conversation":"{conv}","body":"{oversized}"}}"#))
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert!(v["err"].is_string(), "{name}: an oversized body must be refused, got {v}");
}

#[tokio::test]
async fn a_body_over_the_configured_limit_is_refused_on_both_builds() {
    let h = harness().await;
    let oversized = "x".repeat(300_000); // > conversation_max_body_bytes (262_144)
    assert_oversized_body_is_refused("wasm", &h.wasm, &oversized).await;
    assert_oversized_body_is_refused("native", &h.native, &oversized).await;
}

async fn assert_retry_on_pending_is_refused<D: Driver>(name: &str, driver: &D) {
    let conv = open_conversation(driver, "peer-retry-pending").await;
    let send_result = driver
        .run(&format!(r#"{{"op":"send-message","conversation":"{conv}","body":"hi"}}"#))
        .await
        .unwrap();
    let send_v: Value = serde_json::from_str(&send_result).unwrap();
    let message_id = send_v["ok"]["message"].as_str().unwrap();

    let retry_result =
        driver.run(&format!(r#"{{"op":"retry-message","message":"{message_id}"}}"#)).await.unwrap();
    let retry_v: Value = serde_json::from_str(&retry_result).unwrap();
    assert!(
        retry_v["err"].is_string(),
        "{name}: retrying a pending (not failed) message must be refused, got {retry_v}"
    );
}

#[tokio::test]
async fn retry_on_a_pending_message_is_invalid_argument_on_both_builds() {
    let h = harness().await;
    assert_retry_on_pending_is_refused("wasm", &h.wasm).await;
    assert_retry_on_pending_is_refused("native", &h.native).await;
}

async fn assert_create_group<D: Driver>(name: &str, driver: &D) {
    let create_res = driver.run(r#"{"op":"create-group"}"#).await.unwrap();
    let create_v: Value = serde_json::from_str(&create_res).unwrap();
    let conv_id = create_v["ok"]["conversation"].as_str().unwrap();
    assert!(conv_id.starts_with("conv:"), "{name}: group id must start with conv:");

    let members_res =
        driver.run(&format!(r#"{{"op":"members","conversation":"{conv_id}"}}"#)).await.unwrap();
    let members_v: Value = serde_json::from_str(&members_res).unwrap();
    let members = members_v["ok"]["members"].as_array().unwrap();
    assert_eq!(members, &vec![json!(SERVICE_ID)], "{name}: owner must be the first member");

    let history_res = driver
        .run(&format!(r#"{{"op":"membership-history","conversation":"{conv_id}"}}"#))
        .await
        .unwrap();
    let history_v: Value = serde_json::from_str(&history_res).unwrap();
    let events = history_v["ok"]["history"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{name}: genesis membership entry must exist");
    assert_eq!(events[0]["action"], "add");
    assert_eq!(events[0]["subject"], SERVICE_ID);
    assert_eq!(events[0]["epoch"], 1);

    // Test add_member on self is invalid argument
    let add_self = driver
        .run(&format!(
            r#"{{"op":"add-member","conversation":"{conv_id}","member_address":"{SERVICE_ID}"}}"#
        ))
        .await
        .unwrap();
    let add_self_v: Value = serde_json::from_str(&add_self).unwrap();
    assert!(
        add_self_v["err"].is_string(),
        "{name}: add_member for owner must return err response: {add_self}"
    );

    // Test remove_member for non-existent member is a no-op / success
    let rem_nonmember = driver
        .run(&format!(
            r#"{{"op":"remove-member","conversation":"{conv_id}","member_address":"peer-none"}}"#
        ))
        .await
        .unwrap();
    let rem_v: Value = serde_json::from_str(&rem_nonmember).unwrap();
    assert_eq!(rem_v["ok"]["removed"], true, "{name}: remove non-member succeeds as no-op");

    // Test sync_now on existing group succeeds
    let sync_res =
        driver.run(&format!(r#"{{"op":"sync-now","conversation":"{conv_id}"}}"#)).await.unwrap();
    let sync_v: Value = serde_json::from_str(&sync_res).unwrap();
    assert_eq!(sync_v["ok"]["synced"], true, "{name}: sync_now succeeds");
}

#[tokio::test]
async fn create_group_initializes_membership_and_epoch_on_both_builds() {
    let h = harness().await;
    assert_create_group("wasm", &h.wasm).await;
    assert_create_group("native", &h.native).await;
}

/// Drives `add-member` to a real second party (routed through `PeerProxy`
/// straight into the *other* stack's own `ConversationService`, so this is
/// a genuine `prekey-bundle` round trip -- an X3DH handshake, not a stub
/// response), then a group `send`, then `membership-history` -- exercising
/// exactly the group paths `assert_create_group` above could not reach
/// (its own `add-member` case is deliberately the *refused* one, adding the
/// owner to itself). `add-member`'s own network step (`fetch_prekey_bundle`)
/// is awaited inline by `change_membership_impl`, so no background worker
/// or settle delay is needed for these three assertions -- unlike group-key
/// distribution and DAG-entry relay, which are enqueued for the (unstarted,
/// in this harness) delivery worker and are not observed here.
async fn assert_group_add_member_send_and_history_on_a_populated_group<D: Driver>(
    name: &str,
    driver: &D,
) {
    let create_res = driver.run(r#"{"op":"create-group"}"#).await.unwrap();
    let create_v: Value = serde_json::from_str(&create_res).unwrap();
    let conv_id = create_v["ok"]["conversation"].as_str().unwrap().to_string();

    let add_res = driver
        .run(&format!(
            r#"{{"op":"add-member","conversation":"{conv_id}","member_address":"{PEER_SERVICE_ID}"}}"#
        ))
        .await
        .unwrap();
    let add_v: Value = serde_json::from_str(&add_res).unwrap();
    assert_eq!(
        add_v["ok"]["added"], true,
        "{name}: add-member on a real peer must succeed: {add_res}"
    );

    let members_res =
        driver.run(&format!(r#"{{"op":"members","conversation":"{conv_id}"}}"#)).await.unwrap();
    let members_v: Value = serde_json::from_str(&members_res).unwrap();
    let mut members: Vec<String> = members_v["ok"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap().to_string())
        .collect();
    members.sort();
    let mut expected = vec![SERVICE_ID.to_string(), PEER_SERVICE_ID.to_string()];
    expected.sort();
    assert_eq!(members, expected, "{name}: group must have both members after add-member");

    let send_res = driver
        .run(&format!(r#"{{"op":"send-message","conversation":"{conv_id}","body":"hello group"}}"#))
        .await
        .unwrap();
    let send_v: Value = serde_json::from_str(&send_res).unwrap();
    assert!(
        send_v["ok"]["message"].is_string(),
        "{name}: send-message on a populated group must succeed: {send_res}"
    );

    let history_res = driver
        .run(&format!(r#"{{"op":"membership-history","conversation":"{conv_id}"}}"#))
        .await
        .unwrap();
    let history_v: Value = serde_json::from_str(&history_res).unwrap();
    let events = history_v["ok"]["history"].as_array().unwrap();
    assert_eq!(
        events.len(),
        2,
        "{name}: membership history must hold the genesis and add-member events: {history_v:?}"
    );
    assert_eq!(events[0]["action"], "add");
    assert_eq!(events[0]["subject"], SERVICE_ID);
    assert_eq!(events[1]["action"], "add");
    assert_eq!(events[1]["subject"], PEER_SERVICE_ID);
}

#[tokio::test]
async fn group_add_member_send_and_history_are_identical_on_both_builds() {
    let h = harness().await;
    assert_group_add_member_send_and_history_on_a_populated_group("wasm", &h.wasm).await;
    assert_group_add_member_send_and_history_on_a_populated_group("native", &h.native).await;
}

/// Drives a full prekey-bundle -> X3DH session -> sign -> encrypt
/// -> `peer_deliver` exchange from an independent third `ConversationService`
/// (standing in for a real peer substrate) into each build's own
/// `ConversationService`, and confirms the delivered message lands in that
/// build's own store, `verified: true`, and reaches the guest/native app's
/// `on-message` export (read back through `read-conversation-inbox`) --
/// the host -> app direction neither the SCENARIOS table nor `run()` alone
/// can exercise, since `peer_deliver` is reachable only through the
/// peer-facing native-capability dispatch arm, not the guest surface.
const SENDER_ADDRESS: &str = "external-peer-address";

async fn assert_signed_delivery_is_verified_and_notifies_the_app<D: Driver>(
    name: &str,
    target_conversation: &ConversationService,
    driver: &D,
) {
    use syneroym_conversation::{
        crypto::{PrekeyBundle, SessionCrypto, X3dhDoubleRatchetCrypto, generate_identity_bytes},
        envelope::{self, DeliveryPayload},
        ids, store,
    };
    use syneroym_rpc::ConversationHost;

    {
        // The sender's own store -- an independent `ConversationStore`
        // standing in for a real peer substrate. Built directly (not
        // through a second `ConversationService`) since only session
        // establishment (`begin_session`/`encrypt`/`commit`) is needed.
        let sender_dir = tempfile::tempdir().unwrap();
        let sender_store = store::ConversationStore::open_encrypted(
            sender_dir.path(),
            None,
            QueueConfig {
                retry: RetryPolicy::default(),
                visibility_timeout_ms: 120_000,
                dlq_max_rows: 100,
                max_pending_rows: 1000,
            },
            store::ConversationConfig::default(),
        )
        .unwrap();
        let crypto = X3dhDoubleRatchetCrypto::new();

        let bundle_bytes =
            target_conversation.prekey_bundle(SERVICE_ID, SENDER_ADDRESS).await.unwrap();
        let bundle: PrekeyBundle = serde_json::from_slice(&bundle_bytes).unwrap();
        let mut session =
            crypto.begin_session(&sender_store, SENDER_ADDRESS, SERVICE_ID, &bundle).await.unwrap();

        let conversation_id = ids::derive_conversation_id(SENDER_ADDRESS, SERVICE_ID);
        let message_id = ids::derive_message_id(
            SENDER_ADDRESS,
            &conversation_id,
            1_000,
            "text/plain",
            b"hello from a peer",
            &[7u8; 16],
        );
        let identity = sender_store.local_identity_or_generate(generate_identity_bytes).unwrap();
        let sig_bytes: [u8; 32] = identity.sig_secret.as_slice().try_into().unwrap();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&sig_bytes);
        let signature = envelope::sign(
            &signing_key,
            &message_id,
            &conversation_id,
            SENDER_ADDRESS,
            1_000,
            "text/plain",
            b"hello from a peer",
        );
        let payload = DeliveryPayload {
            message_id: message_id.clone(),
            conversation_id: conversation_id.clone(),
            author: SENDER_ADDRESS.to_string(),
            sender_timestamp_ms: 1_000,
            content_type: "text/plain".to_string(),
            body: b"hello from a peer".to_vec(),
            signature,
        };
        let env = crypto.encrypt(&mut session, &payload).unwrap();
        crypto.commit(&sender_store, &session).await.unwrap();

        let env_bytes = serde_json::to_vec(&env).unwrap();
        let _ack_bytes = target_conversation
            .peer_deliver(SERVICE_ID, SENDER_ADDRESS, env_bytes)
            .await
            .unwrap_or_else(|e| panic!("{name}: peer_deliver failed: {e:?}"));
        // Both sides must derive the same conversation id — the receiver's
        // own value, computed independently, must match the sender's.
        let receiver_conv_id = ids::derive_conversation_id(SERVICE_ID, SENDER_ADDRESS);
        assert_eq!(receiver_conv_id, conversation_id);

        let history_result = driver
            .run(&format!(
                r#"{{"op":"read-history","conversation":"{receiver_conv_id}","limit":10}}"#
            ))
            .await
            .unwrap();
        let history_v: Value = serde_json::from_str(&history_result).unwrap();
        let messages = history_v["ok"]["messages"].as_array().unwrap();
        let delivered = messages.iter().find(|m| m["id"] == message_id).unwrap_or_else(|| {
            panic!("{name}: delivered message not found in history: {history_v}")
        });
        assert_eq!(
            delivered["verified"], true,
            "{name}: a validly signed delivery must be verified"
        );
        assert_eq!(
            delivered["state"], "delivered",
            "{name}: an inbound message is delivered on arrival"
        );

        // The app's own `on-message` export was called: the fixture
        // persists it through `data-layer`, read back here.
        let inbox_result = driver.run(r#"{"op":"read-conversation-inbox"}"#).await.unwrap();
        let inbox_v: Value = serde_json::from_str(&inbox_result).unwrap();
        let inbox_entries = inbox_v["ok"]["entries"].as_array().unwrap_or_else(|| {
            panic!("{name}: unexpected read-conversation-inbox response: {inbox_v}")
        });
        assert!(
            inbox_entries.iter().any(|e| e["id"] == message_id),
            "{name}: on-message must have notified the app, got {inbox_v}"
        );
    }
}

#[tokio::test]
async fn a_signed_delivery_from_an_external_peer_is_verified_and_notifies_the_app_on_both_builds() {
    let h = harness().await;
    assert_signed_delivery_is_verified_and_notifies_the_app("wasm", &h.wasm_conversation, &h.wasm)
        .await;
    assert_signed_delivery_is_verified_and_notifies_the_app(
        "native",
        &h.native_conversation,
        &h.native,
    )
    .await;
}

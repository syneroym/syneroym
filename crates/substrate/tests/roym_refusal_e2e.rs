#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! A message the receiving person's inbox turns away for a rate limit is
//! reported back: the sender's own row carries the reason next to a
//! `delivered` state, and the receiver shows nothing. Two independent
//! `syneroym-substrate` instances, each running the full Roym SynApp.
//!
//! Skips when the Roym wasm artifacts or the UI bundle are absent
//! (`mise run build:roym` / `mise run build:roym-ui`).

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::json;
use syneroym_identity::{Identity, substrate};

mod common;

use common::roym::{
    RoymNode as Node, fast_conversation_role, history_messages, roym_artifacts_present, wait_until,
};

#[tokio::test]
async fn a_rate_limited_sender_sees_the_reason_on_its_own_message() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let owner_a = Identity::generate().unwrap();
    let owner_b = Identity::generate().unwrap();
    let mut node_a = Node::boot(
        "node-a",
        dir_a.path().to_path_buf(),
        None,
        Identity::from_bytes(&owner_a.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let registry = node_a.registry_url.clone();
    node_a.full_bring_up().await;
    let mut node_b = Node::boot(
        "node-b",
        dir_b.path().to_path_buf(),
        Some(registry),
        Identity::from_bytes(&owner_b.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_b.full_bring_up().await;

    let owner_b_did = substrate::derive_did_key(&owner_b.public_key());
    let a_conv = node_a.dids["conversation"].clone();
    let b_conv = node_b.dids["conversation"].clone();
    node_a
        .rpc_ok("profile.set", json!({ "display_name": "Ann", "conversation_address": a_conv }))
        .await;
    let b_profile = node_b
        .rpc_ok("profile.set", json!({ "display_name": "Bo", "conversation_address": b_conv }))
        .await;
    node_a
        .rpc_ok(
            "contacts.upsert",
            json!({
                "person_did": owner_b_did,
                "profile_envelope": b_profile["envelope"].as_str().unwrap(),
            }),
        )
        .await;

    // B accepts no first contact at all, so A's first message is turned away.
    node_b.rpc_ok("contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 0 })).await;

    let opened = node_a.rpc_ok("conversation.open", json!({ "person_did": owner_b_did })).await;
    let conversation = opened["conversation_id"].as_str().unwrap().to_string();
    let sent = node_a
        .rpc_ok("conversation.send", json!({ "conversation": conversation, "body": "hello Bo" }))
        .await;
    let message_id = sent["message_id"].as_str().unwrap().to_string();

    let told = wait_until(Duration::from_secs(120), || {
        let (node_a, conversation, message_id) =
            (&node_a, conversation.clone(), message_id.clone());
        async move {
            let _ = node_a.rpc("conversation.retry", json!({ "message_id": message_id })).await;
            let hist = node_a
                .rpc_ok("conversation.history", json!({ "conversation": conversation }))
                .await;
            history_messages(&hist)
                .into_iter()
                .find(|m| m["id"] == message_id)
                .is_some_and(|m| m["state"] == "delivered" && m["refused"] == "rate-limited")
        }
    })
    .await;
    assert!(told, "the sender's row must carry the reason once the host delivered it");

    let b_list = node_b.rpc_ok("conversation.list", json!({})).await;
    let b_rows = b_list["conversations"].as_array().cloned().unwrap_or_default();
    assert!(
        b_rows.iter().all(|c| c["message_count"] == 0),
        "the receiver shows no message from a turned-away sender: {b_list}"
    );

    node_b.teardown().await;
    node_a.teardown().await;
}

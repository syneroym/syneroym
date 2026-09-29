#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Durable 1:1 messaging, end to end across two real `syneroym-substrate`
//! instances: A messages B while B is offline, the message stays `pending`
//! in A's own outbox; A restarts and the same item is still there, not
//! duplicated and not lost; B comes up and the message is delivered,
//! verified, and readable through the host interface; and no durable
//! content ever crosses `syneroym:messaging` (ADR-0013 §6).
//!
//! Both nodes come from `common::SubstrateNode`. Node A hosts the shared
//! registry and restarts mid-test, so its builder is captured and reused to
//! reboot on the same ports; Node B resolves through that registry.
//!
//! Skips when the dual-build-fixture wasm artifact is absent
//! (`mise run build:test-components`, or `cargo component build --release
//! --target wasm32-wasip2 -p syneroym-test-dual-build-fixture`).

use std::time::Duration;

use common::{
    SubstrateNode,
    conversation_fixture::{Deploy, deploy_fixture, fixture_run, fixture_wasm, publish_endpoint},
    roym::{fast_conversation_role, wait_until},
};
use rustls::crypto::ring;
use serde_json::json;
use syneroym_core::config::AppSandboxRole;
use syneroym_identity::{Identity, substrate};
use tokio::time;

mod common;

/// The reference scenario's steps 6-8, plus the never-reachable recipient
/// case: A sends to B while B does not exist yet (stronger than merely
/// offline) -- stays `pending`,
/// never `delivered`. A restarts; the same outbox item survives, not
/// duplicated. B then comes up; the message is delivered, verified on
/// arrival, and no durable content ever reached the pub/sub broker.
#[tokio::test]
#[expect(clippy::too_many_lines, reason = "linear conversation persistence and restart scenario")]
async fn a_message_survives_a_restart_and_delivers_once_the_peer_exists() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    let wasm = fixture_wasm().expect("dual-build-fixture wasm artifact not built");

    // Node A hosts the shared registry, so its own restart wipes it --
    // deliberately part of what this test proves survives. Its builder is
    // captured and reused for the reboot so the rebooted node keeps the
    // same ports, which node B still resolves the registry through.
    let a_dir = tempfile::tempdir().unwrap();
    let owner = Identity::generate().unwrap();
    let node_a_builder = SubstrateNode::builder()
        .owner(&owner)
        .base_path(a_dir.path())
        .inject_kek_bytes([0xcd; 32])
        .configure(|c| {
            c.roles.app_sandbox = Some(fast_conversation_role(
                AppSandboxRole::default().conversation_max_pending_age_secs,
            ))
        });
    let mut node_a = node_a_builder.clone().boot().await;
    let shared_registry = node_a.registry_url().to_string();

    let sender_master = Identity::generate().unwrap();
    let sender_did =
        deploy_fixture(&mut node_a, &sender_master, wasm.clone(), Deploy::Certified).await;

    // The peer's identity is deterministic from its master key, so it can
    // be named before node B ever boots -- this is the "recipient never
    // reachable" case, not merely "offline".
    let peer_master = Identity::generate().unwrap();
    let peer_did = substrate::derive_did_key(&peer_master.public_key());

    let conv_response = fixture_run(
        &node_a,
        &sender_did,
        &json!({"op": "open-conversation", "peer_address": peer_did}),
    )
    .await;
    let conversation_id = conv_response["ok"]["conversation"]
        .as_str()
        .expect("open-conversation must return an id")
        .to_string();

    let send_response = fixture_run(
        &node_a,
        &sender_did,
        &json!({"op": "send-message", "conversation": conversation_id, "body": "hello from A"}),
    )
    .await;
    let message_id = send_response["ok"]["message"]
        .as_str()
        .expect("send-message must return an id")
        .to_string();

    // The peer does not exist, so several consecutive polls must all see
    // `pending`, never `delivered`.
    for _ in 0..3 {
        let status = fixture_run(
            &node_a,
            &sender_did,
            &json!({"op": "delivery-status", "message": message_id}),
        )
        .await;
        assert_eq!(
            status["ok"]["state"], "pending",
            "a message to a peer that does not exist must never read as delivered"
        );
        time::sleep(Duration::from_millis(500)).await;
    }
    let outbox_before = fixture_run(&node_a, &sender_did, &json!({"op": "read-outbox"})).await;
    let entries_before = outbox_before["ok"]["outbox"].as_array().unwrap();
    assert_eq!(entries_before.len(), 1, "exactly one outbox row before the restart");
    assert_eq!(entries_before[0]["id"], message_id);

    // Row 4: restart the sending substrate with the message still pending.
    node_a.teardown().await;
    node_a = node_a_builder.boot().await;
    // A substrate does not bring its own deployed services back up by
    // itself (`proxy_outbox_e2e.rs`'s own precedent) -- redeploy under the
    // same identity, which also republishes the master anchor and the
    // endpoint record the in-memory registry (hosted by A itself) lost.
    // The conversation store itself lives in `app_local_data_dir`, which
    // *does* survive the restart (same `a_dir`), so this proves the
    // outbox's own persistence, not the deploy catalog's.
    let redeployed_sender_did =
        deploy_fixture(&mut node_a, &sender_master, wasm.clone(), Deploy::Certified).await;
    assert_eq!(redeployed_sender_did, sender_did, "redeploying must not change the service id");

    let outbox_after = fixture_run(&node_a, &sender_did, &json!({"op": "read-outbox"})).await;
    let entries_after = outbox_after["ok"]["outbox"].as_array().unwrap();
    assert_eq!(
        entries_after.len(),
        1,
        "the restart must leave exactly the same one item, not zero and not two"
    );
    assert_eq!(entries_after[0]["id"], message_id, "the same message id, not a new one");
    assert_eq!(
        entries_after[0]["state"], "pending",
        "still pending, not reset and not double-sent"
    );

    // The peer now comes up.
    let mut node_b = SubstrateNode::builder()
        .owner(&owner)
        .shared_registry(&shared_registry)
        .inject_kek_bytes([0xcd; 32])
        .boot()
        .await;
    let receiver_did = deploy_fixture(&mut node_b, &peer_master, wasm, Deploy::Certified).await;
    assert_eq!(
        receiver_did, peer_did,
        "the deployed service id must be the one A already addressed"
    );
    let node_b_mechanisms =
        node_b.substrate_client.lookup().await.expect("node B lookup failed").info.mechanisms;
    publish_endpoint(&peer_did, node_b.did(), node_b_mechanisms, &peer_master, &shared_registry)
        .await;

    // Delivery resumes on A's next tick.
    let delivered = wait_until(Duration::from_secs(30), || {
        let node_a = &node_a;
        let sender_did = sender_did.clone();
        let message_id = message_id.clone();
        async move {
            let status = fixture_run(
                node_a,
                &sender_did,
                &json!({"op": "delivery-status", "message": message_id}),
            )
            .await;
            status["ok"]["state"] == "delivered"
        }
    })
    .await;
    assert!(delivered, "the message must be delivered once the peer exists and resolves");

    // B's own history: verified on arrival, state delivered, through the
    // host interface -- not inferred from A's own bookkeeping.
    let history = fixture_run(
        &node_b,
        &receiver_did,
        &json!({"op": "read-history", "conversation": conversation_id, "limit": 10}),
    )
    .await;
    let messages = history["ok"]["messages"].as_array().unwrap();
    let received = messages
        .iter()
        .find(|m| m["id"] == message_id)
        .unwrap_or_else(|| panic!("delivered message not found in B's own history: {history}"));
    assert_eq!(received["verified"], true, "a validly signed cross-node delivery must be verified");
    assert_eq!(received["state"], "delivered");
    assert_eq!(received["body"], "hello from A");

    // The app's own on-message export was called (host -> app), read back
    // through data-layer, not in-process state.
    let inbox =
        fixture_run(&node_b, &receiver_did, &json!({"op": "read-conversation-inbox"})).await;
    let inbox_entries = inbox["ok"]["entries"].as_array().unwrap();
    assert!(
        inbox_entries.iter().any(|e| e["id"] == message_id),
        "on-message must have notified the app on B, got {inbox}"
    );

    // Row 6 (ADR-0013 §6): durable content never traverses
    // `syneroym:messaging` -- the pub/sub broker inbox stays empty on both
    // ends, asserted against the broker's own traffic, not by inspection.
    let a_broker_inbox = fixture_run(&node_a, &sender_did, &json!({"op": "read-inbox"})).await;
    assert_eq!(
        a_broker_inbox["ok"]["entries"].as_array().unwrap().len(),
        0,
        "no durable content may traverse the pub/sub broker on the sending side"
    );
    let b_broker_inbox = fixture_run(&node_b, &receiver_did, &json!({"op": "read-inbox"})).await;
    assert_eq!(
        b_broker_inbox["ok"]["entries"].as_array().unwrap().len(),
        0,
        "no durable content may traverse the pub/sub broker on the receiving side"
    );

    node_b.teardown().await;
    node_a.teardown().await;
}

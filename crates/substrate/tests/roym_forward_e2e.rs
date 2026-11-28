#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end test of forwarding a signed text message across three real
//! `syneroym-substrate` nodes: a direct chat, a group, the forward count,
//! the refusals, and the size and read-time measurements.

use std::{collections::HashMap, time::Duration};

use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_core::config::AppSandboxRole;
use syneroym_roym_core::{backup::bytes_from_hex, card::MAILBOX_CARD_BODY_BYTES};

mod common;

use common::{
    roym::{RoymNode, roym_artifacts_present, wait_delivered, wait_until},
    roym_group::{boot_trio, converge, form_group, group_role},
};

fn messages(history: &Value) -> Vec<Value> {
    history["messages"].as_array().cloned().unwrap_or_default()
}

async fn history(node: &RoymNode, conv: &str) -> Vec<Value> {
    messages(&node.rpc_ok("conversation.history", json!({ "conversation": conv })).await)
}

async fn wait_for_body(node: &RoymNode, conv: &str, text: &str) -> Vec<Value> {
    let ok = wait_until(Duration::from_secs(90), || async {
        history(node, conv).await.iter().any(|m| m["body"] == text)
    })
    .await;
    assert!(ok, "{} did not receive '{text}' in time", node.label);
    history(node, conv).await
}

async fn send_signed(node: &RoymNode, conv: &str, text: &str, languages: &[&str]) -> String {
    let sent = node
        .rpc_ok(
            "conversation.send",
            json!({
                "conversation": conv,
                "body": text,
                "languages": languages,
                "forward_mode": "signed",
            }),
        )
        .await;
    let id = sent["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node, &id).await, "{}: a signed message must deliver", node.label);
    id
}

/// Every message body in the node's conversation backup, by message id.
/// The backup carries the stored bytes, which a history view does not.
async fn exported_bodies(node: &RoymNode) -> HashMap<String, Vec<u8>> {
    let bundle = node.rpc_ok("conversation.export", json!({})).await;
    let chunks = bundle["sections"]["conversation_history"].as_array().cloned().unwrap_or_default();
    let mut out = HashMap::new();
    for chunk in chunks {
        let hex = chunk["payload"]["data"].as_str().expect("chunk data");
        let bytes = bytes_from_hex(hex).expect("chunk hex");
        let page: Value = serde_json::from_slice(&bytes).expect("chunk json");
        for m in page["messages"].as_array().cloned().unwrap_or_default() {
            let id = m["id"].as_str().expect("message id").to_string();
            let body: Vec<u8> = m["body"]
                .as_array()
                .expect("body bytes")
                .iter()
                .map(|b| b.as_u64().expect("byte") as u8)
                .collect();
            out.insert(id, body);
        }
    }
    out
}

async fn measure_bodies(alice: &RoymNode, conv: &str, short_id: &str) {
    let big = alice
        .rpc_ok(
            "conversation.send",
            json!({
                "conversation": conv,
                "body": "x".repeat(11 * 1024),
                "forward_mode": "signed",
            }),
        )
        .await;
    let big_id = big["message_id"].as_str().unwrap().to_string();
    let bodies = exported_bodies(alice).await;
    let short = bodies.get(short_id).expect("the short chat body");
    let origin = serde_json::from_slice::<Value>(short).expect("chat body json")["origin"]
        .as_str()
        .expect("origin string")
        .to_string();
    println!("origin envelope bytes: {}", origin.len());
    println!("short chat body bytes: {}", short.len());
    assert!(origin.len() < 2 * 1024, "an origin must stay under 2 KiB: {}", origin.len());
    let big_bytes = bodies[&big_id].len();
    println!(
        "11 KiB signed chat body bytes: {big_bytes}, mailbox estimate {MAILBOX_CARD_BODY_BYTES}, \
         under: {}",
        big_bytes < MAILBOX_CARD_BODY_BYTES
    );
}

async fn carol_reads_forward(carol: &RoymNode, gid: &str, author_did: &str) -> String {
    let rows = wait_for_body(carol, gid, "Market opens at 9").await;
    let row =
        rows.iter().find(|m| m["body"] == "Market opens at 9").expect("Carol has the forward");
    assert_eq!(row["forward"]["forwarded"], true, "{row}");
    assert_eq!(row["forward"]["forward_count"], 1);
    assert_eq!(row["forward"]["origin"]["author_did"], author_did);
    row["id"].as_str().unwrap().to_string()
}

async fn assert_forward_refused(bob: &RoymNode, message_id: &str, gid: &str) {
    let refused = bob
        .rpc("conversation.forward", json!({ "message_id": message_id, "conversation": gid }))
        .await;
    assert_eq!(refused["error"]["data"]["code"], "not-forwardable", "{refused}");
}

async fn measure_history(alice: &RoymNode) {
    let conv = alice
        .rpc_ok("conversation.open", json!({ "address": "did:key:zNeverAnsweringC7" }))
        .await["conversation_id"]
        .as_str()
        .unwrap()
        .to_string();
    for i in 0..200 {
        alice
            .rpc_ok(
                "conversation.send",
                json!({
                    "conversation": &conv,
                    "body": format!("timed message {i}"),
                    "forward_mode": "signed",
                }),
            )
            .await;
    }
    let started = std::time::Instant::now();
    let page =
        alice.rpc_ok("conversation.history", json!({ "conversation": &conv, "limit": 200 })).await;
    let elapsed = started.elapsed();
    assert_eq!(messages(&page).len(), 200);
    println!("history of 200 signed messages: {} ms", elapsed.as_millis());
    assert!(
        elapsed < Duration::from_millis(500),
        "a page of 200 signed messages must stay under 500 ms: {elapsed:?}"
    );
}

async fn assert_own_forward_keeps_count(
    alice: &RoymNode,
    bob: &RoymNode,
    alice_conv: &str,
    bob_conv: &str,
    group_forward: &str,
) {
    let own = bob
        .rpc_ok(
            "conversation.forward",
            json!({ "message_id": group_forward, "conversation": bob_conv }),
        )
        .await;
    assert!(wait_delivered(bob, own["message_id"].as_str().unwrap()).await);
    let seen = wait_until(Duration::from_secs(90), || async {
        history(alice, alice_conv)
            .await
            .iter()
            .any(|m| m["body"] == "Market opens at 9" && m["forward"]["forward_count"] == 1)
    })
    .await;
    assert!(seen, "Alice sees Bob's forward of her message");
    let rows = history(alice, alice_conv).await;
    let row = rows
        .iter()
        .find(|m| m["body"] == "Market opens at 9" && m["forward"]["forward_count"] == 1)
        .expect("Alice's own message, forwarded by Bob");
    assert_eq!(row["forward"]["origin"]["is_you"], true, "{row}");
}

#[tokio::test]
async fn a_forward_keeps_the_author_and_the_count_across_three_nodes() {
    let _serial_guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let role = group_role(3600, AppSandboxRole::default().conversation_max_pending_age_secs);
    let trio = boot_trio(role).await;
    let (alice, bob, carol) = (&trio.z, &trio.x, &trio.y);
    for node in [alice, bob, carol] {
        node.enrol_signing().await;
    }

    // Alice and Bob open the same one-to-one chat from each side.
    let alice_conv = alice
        .rpc_ok("conversation.open", json!({ "address": bob.dids["conversation"] }))
        .await["conversation_id"]
        .as_str()
        .unwrap()
        .to_string();
    let bob_conv = bob
        .rpc_ok("conversation.open", json!({ "address": alice.dids["conversation"] }))
        .await["conversation_id"]
        .as_str()
        .unwrap()
        .to_string();

    let source_on_alice = send_signed(alice, &alice_conv, "Market opens at 9", &["en"]).await;

    // Bob's view: checked origin, author Alice, forwardable.
    let bob_rows = wait_for_body(bob, &bob_conv, "Market opens at 9").await;
    let row = bob_rows
        .iter()
        .find(|m| m["body"] == "Market opens at 9")
        .expect("Bob has Alice's message");
    assert_eq!(row["forward"]["forwardable"], true, "{row}");
    assert_eq!(row["forward"]["origin"]["author_did"], alice.owner_did());
    assert_eq!(row["forward"]["origin"]["is_you"], false);
    let source_on_bob = row["id"].as_str().unwrap().to_string();

    measure_bodies(alice, &alice_conv, &source_on_alice).await;

    // Bob forms a group with Carol and forwards Alice's message there.
    let gid = form_group(bob, "Market", &[alice, carol]).await;
    let forwarded = bob
        .rpc_ok(
            "conversation.forward",
            json!({ "message_id": &source_on_bob, "conversation": &gid }),
        )
        .await;
    let forward_on_bob = forwarded["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(bob, &forward_on_bob).await, "Bob's forward delivers into the group");
    converge(&[alice, bob, carol], &gid, 5).await;
    let forward_on_carol = carol_reads_forward(carol, &gid, &alice.owner_did()).await;

    // Carol forwards it into the same group: the count is 2.
    let again = carol
        .rpc_ok(
            "conversation.forward",
            json!({ "message_id": &forward_on_carol, "conversation": &gid }),
        )
        .await;
    assert!(wait_delivered(carol, again["message_id"].as_str().unwrap()).await);
    converge(&[alice, bob, carol], &gid, 6).await;
    let rows = history(carol, &gid).await;
    let mut counts: Vec<i64> = rows
        .iter()
        .filter(|m| m["body"] == "Market opens at 9")
        .map(|m| m["forward"]["forward_count"].as_i64().unwrap())
        .collect();
    counts.sort_unstable();
    assert_eq!(counts, vec![1, 2], "{rows:?}");

    // A not-forwardable message cannot be forwarded.
    let quiet = alice
        .rpc_ok(
            "conversation.send",
            json!({
                "conversation": &alice_conv,
                "body": "just between us",
                "forward_mode": "not-forwardable",
            }),
        )
        .await;
    assert!(wait_delivered(alice, quiet["message_id"].as_str().unwrap()).await);
    let bob_rows = wait_for_body(bob, &bob_conv, "just between us").await;
    let quiet_id = bob_rows
        .iter()
        .find(|m| m["body"] == "just between us")
        .and_then(|m| m["id"].as_str())
        .expect("Bob has the not-forwardable message");
    assert_forward_refused(bob, quiet_id, &gid).await;

    // Bob forwards his own group forward into the chat with Alice. The count
    // stays 1, and Alice reads the origin as her own.
    assert_own_forward_keeps_count(alice, bob, &alice_conv, &bob_conv, &forward_on_bob).await;

    measure_history(alice).await;
    trio.teardown().await;
}

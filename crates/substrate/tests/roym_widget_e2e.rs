#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end integration tests for Roym widgets across real substrates:
//! a 1:1 poll between two nodes and a group poll across three nodes.

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::json;
use syneroym_identity::Identity;
use syneroym_roym_core::widget::poll_definition;

mod common;

use common::{
    roym::{RoymNode as Node, fast_conversation_role, roym_artifacts_present, wait_until},
    roym_flow::deliver_one_message,
    roym_group::{boot_trio, converge, form_group},
};

async fn setup_two_nodes(
    dir_a: &std::path::Path,
    dir_b: &std::path::Path,
    owner_a: &Identity,
    owner_b: &Identity,
) -> (Node, Node, String, String) {
    let mut node_a = Node::boot(
        "node-a",
        dir_a.to_path_buf(),
        None,
        Identity::from_bytes(&owner_a.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let shared_registry = node_a.registry_url.clone();
    node_a.full_bring_up().await;

    let mut node_b = Node::boot(
        "node-b",
        dir_b.to_path_buf(),
        Some(shared_registry),
        Identity::from_bytes(&owner_b.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_b.full_bring_up().await;

    let b_conv_did = node_b.dids["conversation"].clone();
    let a_conv_did = node_a.dids["conversation"].clone();

    let opened_a = node_a.rpc_ok("conversation.open", json!({ "address": b_conv_did })).await;
    let conv_a = opened_a["conversation_id"].as_str().unwrap().to_string();

    let opened_b = node_b.rpc_ok("conversation.open", json!({ "address": a_conv_did })).await;
    let conv_b = opened_b["conversation_id"].as_str().unwrap().to_string();

    deliver_one_message(&node_a, "node-b", &b_conv_did, "hello bob").await;

    (node_a, node_b, conv_a, conv_b)
}

struct ChatPair<'a> {
    node_a: &'a Node,
    node_b: &'a Node,
    conv_a: &'a str,
    conv_b: &'a str,
    wid_a: &'a str,
    wid_b: &'a str,
}

async fn wait_for_vote_counts(
    pair: &ChatPair<'_>,
    ans_id: &str,
    o1_expected: u64,
    o2_expected: u64,
) {
    let ok = wait_until(Duration::from_secs(60), || async {
        let _ = pair.node_b.rpc("conversation.retry", json!({ "message_id": ans_id })).await;
        let l = pair
            .node_a
            .rpc_ok(
                "widget.list",
                json!({ "conversation": pair.conv_a, "widget_ids": [pair.wid_a] }),
            )
            .await;
        l["widgets"].as_array().is_some_and(|w| {
            !w.is_empty()
                && w[0]["counts"]
                    == json!([
                        { "field": "option", "choice": "o1", "count": o1_expected },
                        { "field": "option", "choice": "o2", "count": o2_expected },
                    ])
        })
    })
    .await;
    assert!(ok, "vote {ans_id} was not reflected on Alice's node in time");

    let lb = pair
        .node_b
        .rpc_ok("widget.list", json!({ "conversation": pair.conv_b, "widget_ids": [pair.wid_b] }))
        .await;
    assert_eq!(
        lb["widgets"][0]["counts"],
        json!([
            { "field": "option", "choice": "o1", "count": o1_expected },
            { "field": "option", "choice": "o2", "count": o2_expected },
        ]),
        "Bob's local counts must match"
    );
}

#[tokio::test]
async fn a_poll_in_a_direct_chat_counts_each_persons_newest_vote() {
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

    let (node_a, node_b, conv_a, conv_b) =
        setup_two_nodes(dir_a.path(), dir_b.path(), &owner_a, &owner_b).await;

    let poll = poll_definition("Which day?", &["Saturday".into(), "Sunday".into()]);
    let sent_poll =
        node_a.rpc_ok("widget.send", json!({ "conversation": conv_a, "widget": poll })).await;
    let wid_a = sent_poll["message_id"].as_str().unwrap().to_string();

    let poll_delivered = wait_until(Duration::from_secs(60), || async {
        let _ = node_a.rpc("conversation.retry", json!({ "message_id": &wid_a })).await;
        let l = node_b.rpc_ok("widget.list", json!({ "conversation": conv_b })).await;
        l["widgets"].as_array().is_some_and(|arr| !arr.is_empty())
    })
    .await;
    assert!(poll_delivered, "poll was not delivered to Bob in time");

    let list_b = node_b.rpc_ok("widget.list", json!({ "conversation": conv_b })).await;
    let wid_b = list_b["widgets"][0]["widget_id"].as_str().unwrap().to_string();

    let pair = ChatPair {
        node_a: &node_a,
        node_b: &node_b,
        conv_a: &conv_a,
        conv_b: &conv_b,
        wid_a: &wid_a,
        wid_b: &wid_b,
    };

    let sent_ans1 = node_b
        .rpc_ok(
            "widget.answer",
            json!({ "conversation": conv_b, "widget_id": wid_b, "data": { "option": "o2" } }),
        )
        .await;
    let ans1_id = sent_ans1["message_id"].as_str().unwrap().to_string();
    wait_for_vote_counts(&pair, &ans1_id, 0, 1).await;

    let sent_ans2 = node_b
        .rpc_ok(
            "widget.answer",
            json!({ "conversation": conv_b, "widget_id": wid_b, "data": { "option": "o1" } }),
        )
        .await;
    let ans2_id = sent_ans2["message_id"].as_str().unwrap().to_string();
    wait_for_vote_counts(&pair, &ans2_id, 1, 0).await;

    node_a.teardown().await;
    node_b.teardown().await;
}

#[tokio::test]
async fn a_poll_in_a_group_shows_the_same_counts_on_every_member() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let trio = boot_trio(fast_conversation_role(3600)).await;
    let gid = form_group(&trio.z, "Poll Group", &[&trio.x, &trio.y]).await;

    let poll = poll_definition("Lunch?", &["Pizza".into(), "Tacos".into()]);
    let sent = trio.z.rpc_ok("widget.send", json!({ "conversation": gid, "widget": poll })).await;
    let wid = sent["message_id"].as_str().unwrap().to_string();

    converge(&[&trio.z, &trio.x, &trio.y], &gid, 5).await;

    trio.x
        .rpc_ok(
            "widget.answer",
            json!({ "conversation": gid, "widget_id": wid, "data": { "option": "o1" } }),
        )
        .await;

    trio.y
        .rpc_ok(
            "widget.answer",
            json!({ "conversation": gid, "widget_id": wid, "data": { "option": "o2" } }),
        )
        .await;

    converge(&[&trio.z, &trio.x, &trio.y], &gid, 7).await;

    for node in [&trio.z, &trio.x, &trio.y] {
        let l =
            node.rpc_ok("widget.list", json!({ "conversation": gid, "widget_ids": [&wid] })).await;
        assert_eq!(
            l["widgets"][0]["counts"],
            json!([
                { "field": "option", "choice": "o1", "count": 1 },
                { "field": "option", "choice": "o2", "count": 1 },
            ]),
            "counts on {}",
            node.label
        );
    }

    trio.teardown().await;
}

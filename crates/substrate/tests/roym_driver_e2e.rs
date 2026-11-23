#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! Background driver and wake tests across two independent installations.
//!
//! Verifies that without calling `transaction.sync` on either node,
//! background wakes drive agreements to `agreed`, both during live
//! message delivery and across a node restart (the reload rescan path).

use std::{path::Path, time::Duration};

use rustls::crypto::ring;
use serde_json::json;
use syneroym_identity::Identity;

mod common;

use common::{
    roym::{
        CoordinatorNode, RoymNode as Node, fast_conversation_role, roym_artifacts_present,
        wait_delivered, wait_until,
    },
    roym_flow::{open_request_conv, quote_terms},
};

async fn wait_for_card_filed(node: &Node, conv_id: Option<&str>, card_type: &str) -> bool {
    wait_until(Duration::from_secs(30), || async {
        let thread = if let Some(cid) = conv_id {
            node.rpc("transaction.thread", json!({ "conversation": cid })).await
        } else {
            let x_convs = node.rpc_ok("conversation.list", json!({})).await;
            let Some(convs) = x_convs["conversations"].as_array() else {
                return false;
            };
            let Some(conv) = convs.iter().find(|c| c["kind"] == "direct") else {
                return false;
            };
            let cid = conv["id"].as_str().unwrap();
            node.rpc("transaction.thread", json!({ "conversation": cid })).await
        };
        thread["result"]["cards"]
            .as_array()
            .is_some_and(|cards| cards.iter().any(|c| c["card_type"] == card_type))
    })
    .await
}

async fn wait_for_agreement_state(
    node: &Node,
    quote_record_id: &str,
    state: &str,
    timeout_secs: u64,
) -> bool {
    wait_until(Duration::from_secs(timeout_secs), || async {
        let agr = node.rpc("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
        agr["result"]["state"] == state
    })
    .await
}

async fn setup_driver_nodes(
    dir_x: &Path,
    dir_y: &Path,
    coord: Option<&CoordinatorNode>,
) -> (Node, Node, String, String) {
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();

    let (node_x, node_y) = if let Some(c) = coord {
        let nx = Node::boot_on(
            "node-x",
            dir_x.to_path_buf(),
            c,
            Identity::from_bytes(&owner_x.to_bytes()),
            fast_conversation_role(3600),
        )
        .await;
        let ny = Node::boot_on(
            "node-y",
            dir_y.to_path_buf(),
            c,
            Identity::from_bytes(&owner_y.to_bytes()),
            fast_conversation_role(3600),
        )
        .await;
        (nx, ny)
    } else {
        let mut nx = Node::boot(
            "node-x",
            dir_x.to_path_buf(),
            None,
            Identity::from_bytes(&owner_x.to_bytes()),
            fast_conversation_role(3600),
        )
        .await;
        let reg = nx.registry_url.clone();
        nx.full_bring_up().await;

        let mut ny = Node::boot(
            "node-y",
            dir_y.to_path_buf(),
            Some(reg),
            Identity::from_bytes(&owner_y.to_bytes()),
            fast_conversation_role(3600),
        )
        .await;
        ny.full_bring_up().await;
        (nx, ny)
    };

    let x_conv_did = node_x.dids["conversation"].clone();
    let y_conv_did = node_y.dids["conversation"].clone();

    node_x
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "X", "conversation_address": x_conv_did, "lang": "en" }),
        )
        .await;
    node_y
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "Y", "conversation_address": y_conv_did, "lang": "en" }),
        )
        .await;

    (node_x, node_y, x_conv_did, y_conv_did)
}

#[tokio::test]
async fn two_nodes_reach_agreement_through_driver_without_manual_sync() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let (node_x, node_y, x_conv_did, _y_conv_did) =
        setup_driver_nodes(dir_x.path(), dir_y.path(), None).await;

    node_x
        .rpc_ok(
            "listing.set",
            json!({
                "title": "Garden cleanup",
                "summary": "Full garden cleanup service",
                "lang": "en",
                "categories": ["gardening"],
                "payment": {
                    "currency": "EUR", "model": "fixed", "amount_minor": 4500,
                    "tax_included": true, "payee": "X Services"
                }
            }),
        )
        .await;

    let (y_conv_id, req_record_id) =
        open_request_conv(&node_y, &x_conv_did, "Prune hedges", &["gardening"]).await;

    assert!(wait_for_card_filed(&node_x, None, "enquiry").await, "enquiry card on X");

    let quote = node_x
        .rpc_ok(
            "quote.set",
            json!({
                "enquiry_record_id": req_record_id,
                "expires_in_secs": 3600,
                "terms": quote_terms("X Services", 4500),
            }),
        )
        .await;
    let quote_record_id = quote["record_id"].as_str().unwrap().to_string();
    let quote_msg_id = quote["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(&node_x, &quote_msg_id).await, "quote delivered to Y");

    assert!(wait_for_card_filed(&node_y, Some(&y_conv_id), "quote").await, "quote card on Y");

    let accept =
        node_y.rpc_ok("agreement.accept", json!({ "quote_record_id": quote_record_id })).await;
    let accept_msg = accept["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(&node_y, &accept_msg).await, "accept delivered to X");

    assert!(wait_for_agreement_state(&node_x, &quote_record_id, "agreed", 30).await, "X agreed");
    assert!(wait_for_agreement_state(&node_y, &quote_record_id, "agreed", 30).await, "Y agreed");
}

#[tokio::test]
async fn stopped_quote_author_restarts_and_countersigns_via_reload_path() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let coord = CoordinatorNode::boot().await;
    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let (mut node_x, node_y, x_conv_did, _y_conv_did) =
        setup_driver_nodes(dir_x.path(), dir_y.path(), Some(&coord)).await;

    node_x
        .rpc_ok(
            "listing.set",
            json!({
                "title": "Lawn mowing",
                "summary": "Regular lawn mowing service",
                "lang": "en",
                "categories": ["gardening"],
                "payment": {
                    "currency": "EUR", "model": "fixed", "amount_minor": 3000,
                    "tax_included": true, "payee": "X Services"
                }
            }),
        )
        .await;

    let (y_conv_id, req_record_id) =
        open_request_conv(&node_y, &x_conv_did, "Mow lawn", &["gardening"]).await;

    assert!(wait_for_card_filed(&node_x, None, "enquiry").await, "enquiry card on X");

    let quote = node_x
        .rpc_ok(
            "quote.set",
            json!({
                "enquiry_record_id": req_record_id,
                "expires_in_secs": 3600,
                "terms": quote_terms("X Services", 3000),
            }),
        )
        .await;
    let quote_record_id = quote["record_id"].as_str().unwrap().to_string();
    let quote_msg_id = quote["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(&node_x, &quote_msg_id).await, "quote delivered to Y");

    assert!(wait_for_card_filed(&node_y, Some(&y_conv_id), "quote").await, "quote card on Y");

    // Stop X before acceptance is delivered / processed
    node_x.stop(None).await;

    // Y accepts while X is stopped
    let accept =
        node_y.rpc_ok("agreement.accept", json!({ "quote_record_id": quote_record_id })).await;
    let accept_msg = accept["message_id"].as_str().unwrap().to_string();

    // Start X again; on restart its wake sees Reload, rescans conversations, and
    // countersigns
    node_x.resume(None).await;
    node_x.wait_for_proxy().await;
    assert!(wait_delivered(&node_y, &accept_msg).await, "accept delivered to X");

    assert!(wait_for_agreement_state(&node_x, &quote_record_id, "agreed", 45).await, "X agreed");
    assert!(wait_for_agreement_state(&node_y, &quote_record_id, "agreed", 45).await, "Y agreed");
}

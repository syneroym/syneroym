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

use std::time::Duration;

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

#[tokio::test]
#[expect(clippy::too_many_lines, reason = "linear two-node transaction driver agreement test")]
async fn two_nodes_reach_agreement_through_driver_without_manual_sync() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();

    let mut node_x = Node::boot(
        "node-x",
        dir_x.path().to_path_buf(),
        None,
        Identity::from_bytes(&owner_x.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let shared_registry = node_x.registry_url.clone();
    node_x.full_bring_up().await;

    let mut node_y = Node::boot(
        "node-y",
        dir_y.path().to_path_buf(),
        Some(shared_registry),
        Identity::from_bytes(&owner_y.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_y.full_bring_up().await;

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

    // Y opens conversation to X and sends an enquiry
    let (y_conv_id, req_record_id) =
        open_request_conv(&node_y, &x_conv_did, "Prune hedges", &["gardening"]).await;

    // X waits for the driver wake to file the enquiry card
    let filed_on_x = wait_until(Duration::from_secs(30), || async {
        let x_convs = node_x.rpc_ok("conversation.list", json!({})).await;
        let Some(convs) = x_convs["conversations"].as_array() else {
            return false;
        };
        let Some(conv) = convs.iter().find(|c| c["kind"] == "direct") else {
            return false;
        };
        let conv_id = conv["id"].as_str().unwrap();
        let thread = node_x.rpc("transaction.thread", json!({ "conversation": conv_id })).await;
        thread["result"]["cards"]
            .as_array()
            .is_some_and(|cards| cards.iter().any(|c| c["card_type"] == "enquiry"))
    })
    .await;
    assert!(filed_on_x, "enquiry card must be filed on X by the driver");

    // X sets a quote
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

    // Y waits for the driver wake to file the quote card
    let filed_on_y = wait_until(Duration::from_secs(30), || async {
        let thread = node_y.rpc("transaction.thread", json!({ "conversation": y_conv_id })).await;
        thread["result"]["cards"]
            .as_array()
            .is_some_and(|cards| cards.iter().any(|c| c["card_type"] == "quote"))
    })
    .await;
    assert!(filed_on_y, "quote card must be filed on Y by the driver");

    // Y accepts
    let accept =
        node_y.rpc_ok("agreement.accept", json!({ "quote_record_id": quote_record_id })).await;
    let accept_msg = accept["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(&node_y, &accept_msg).await, "accept delivered to X");

    // Both reach `agreed` within 30 s without calling transaction.sync on either
    // node
    let x_agreed = wait_until(Duration::from_secs(30), || async {
        let agr = node_x.rpc("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
        agr["result"]["state"] == "agreed"
    })
    .await;
    assert!(x_agreed, "X must reach agreed via driver");

    let y_agreed = wait_until(Duration::from_secs(30), || async {
        let agr = node_y.rpc("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
        agr["result"]["state"] == "agreed"
    })
    .await;
    assert!(y_agreed, "Y must reach agreed via driver");
}

#[tokio::test]
#[expect(clippy::too_many_lines, reason = "linear two-node transaction driver restart reload test")]
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
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();

    let mut node_x = Node::boot_on(
        "node-x",
        dir_x.path().to_path_buf(),
        &coord,
        Identity::from_bytes(&owner_x.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;

    let node_y = Node::boot_on(
        "node-y",
        dir_y.path().to_path_buf(),
        &coord,
        Identity::from_bytes(&owner_y.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;

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

    let filed_on_x = wait_until(Duration::from_secs(30), || async {
        let x_convs = node_x.rpc_ok("conversation.list", json!({})).await;
        let Some(convs) = x_convs["conversations"].as_array() else {
            return false;
        };
        let Some(conv) = convs.iter().find(|c| c["kind"] == "direct") else {
            return false;
        };
        let conv_id = conv["id"].as_str().unwrap();
        let thread = node_x.rpc("transaction.thread", json!({ "conversation": conv_id })).await;
        thread["result"]["cards"]
            .as_array()
            .is_some_and(|cards| cards.iter().any(|c| c["card_type"] == "enquiry"))
    })
    .await;
    assert!(filed_on_x, "enquiry card must be filed on X by the driver");

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

    // Y waits until the quote card is filed
    let filed_on_y = wait_until(Duration::from_secs(30), || async {
        let thread = node_y.rpc("transaction.thread", json!({ "conversation": y_conv_id })).await;
        thread["result"]["cards"]
            .as_array()
            .is_some_and(|cards| cards.iter().any(|c| c["card_type"] == "quote"))
    })
    .await;
    assert!(filed_on_y, "quote card must be filed on Y by the driver");

    // Stop X before acceptance is delivered / processed
    node_x.stop(None).await;

    // Y accepts while X is stopped
    let accept =
        node_y.rpc_ok("agreement.accept", json!({ "quote_record_id": quote_record_id })).await;
    let _accept_msg = accept["message_id"].as_str().unwrap().to_string();

    // Start X again; on restart its wake sees Reload, rescans conversations, and
    // countersigns
    node_x.resume(None).await;
    node_x.wait_for_proxy().await;
    assert!(wait_delivered(&node_y, &_accept_msg).await, "accept delivered to X");

    // Both reach `agreed` with zero transaction.sync calls
    let x_agreed = wait_until(Duration::from_secs(45), || async {
        let agr = node_x.rpc("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
        agr["result"]["state"] == "agreed"
    })
    .await;
    assert!(x_agreed, "X must reach agreed on restart via reload path");

    let y_agreed = wait_until(Duration::from_secs(45), || async {
        let agr = node_y.rpc("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
        agr["result"]["state"] == "agreed"
    })
    .await;
    assert!(y_agreed, "Y must reach agreed via driver after X countersigns");
}

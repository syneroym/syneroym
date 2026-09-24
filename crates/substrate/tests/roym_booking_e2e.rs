#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! The Roym product's booking arbitration: two consumers race for the one
//! seat a provider's slot holds, across three independent
//! `syneroym-substrate` instances, each running the full Roym SynApp.
//!
//! Proves the single-writer fence actually arbitrates a
//! real concurrent booking: exactly one consumer schedules, the other gets
//! a named conflict, and the winner's booking runs to completion through
//! both tracks. No directory is deployed anywhere.

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::transaction::DEFAULT_DATA_USE_NOTICE;

mod common;

use common::roym::{
    RoymNode as Node, fast_conversation_role, roym_artifacts_present, wait_delivered, wait_until,
};

async fn open_request_conv(node: &Node, y_conv_did: &str, description: &str) -> (String, String) {
    let opened = node.rpc_ok("conversation.open", json!({ "address": y_conv_did })).await;
    let conv_id = opened["conversation_id"].as_str().unwrap().to_string();
    let req = node
        .rpc_ok(
            "request.set",
            json!({
                "conversation": conv_id,
                "description": description,
                "categories": ["gardening"],
                "data_use_notice": DEFAULT_DATA_USE_NOTICE,
            }),
        )
        .await;
    let msg_id = req["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node, &msg_id).await, "request delivered to provider");
    (conv_id, req["record_id"].as_str().unwrap().to_string())
}

async fn provider_conv_for(y: &Node, peer_address: &str) -> String {
    let found = wait_until(Duration::from_secs(20), || async {
        let list = y.rpc_ok("conversation.list", json!({})).await;
        list["conversations"]
            .as_array()
            .map(|cs| cs.iter().any(|c| c["peer_address"] == peer_address))
            .unwrap_or(false)
    })
    .await;
    assert!(found, "provider sees a conversation with {peer_address}");
    let list = y.rpc_ok("conversation.list", json!({})).await;
    list["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["peer_address"] == peer_address)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

struct BootedCluster {
    node_x: Node,
    node_w: Node,
    node_y: Node,
    listing_id: String,
    slot_id: String,
}

async fn boot_three_nodes(
    dir_x: &std::path::Path,
    dir_w: &std::path::Path,
    dir_y: &std::path::Path,
    owner_x: &Identity,
    owner_w: &Identity,
    owner_y: &Identity,
) -> BootedCluster {
    let mut node_x = Node::boot(
        "node-x",
        dir_x.to_path_buf(),
        None,
        Identity::from_bytes(&owner_x.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let shared_registry = node_x.registry_url.clone();
    node_x.full_bring_up().await;

    let mut node_w = Node::boot(
        "node-w",
        dir_w.to_path_buf(),
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_w.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_w.full_bring_up().await;

    let mut node_y = Node::boot(
        "node-y",
        dir_y.to_path_buf(),
        Some(shared_registry),
        Identity::from_bytes(&owner_y.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_y.full_bring_up().await;

    let x_conv_did = node_x.dids["conversation"].clone();
    let w_conv_did = node_w.dids["conversation"].clone();
    let y_conv_did = node_y.dids["conversation"].clone();

    node_x
        .rpc_ok("profile.set", json!({ "display_name": "X", "conversation_address": x_conv_did }))
        .await;
    node_w
        .rpc_ok("profile.set", json!({ "display_name": "W", "conversation_address": w_conv_did }))
        .await;
    node_y
        .rpc_ok("profile.set", json!({ "display_name": "Y", "conversation_address": y_conv_did }))
        .await;

    let listing = node_y
        .rpc_ok(
            "listing.set",
            json!({
                "title": "Garden clearance",
                "summary": "Same-day one-off garden clearance",
                "categories": ["gardening"],
                "payment": {
                    "currency": "EUR", "model": "fixed", "amount_minor": 6000,
                    "tax_included": true, "payee": "Y Gardens"
                }
            }),
        )
        .await;
    let listing_id = listing["listing_id"].as_str().unwrap().to_string();

    let slot_start = 1_800_000_000_u64;
    let slot_end = slot_start + 3600;
    let avail = node_y
        .rpc_ok(
            "availability.set",
            json!({
                "listing_id": listing_id,
                "slots": [{ "start_secs": slot_start, "end_secs": slot_end, "capacity": 1 }],
            }),
        )
        .await;
    let slot_id = avail["slot_ids"][0].as_str().unwrap().to_string();

    BootedCluster { node_x, node_w, node_y, listing_id, slot_id }
}

struct QuoteExchange {
    x_conv_id: String,
    w_conv_id: String,
    y_conv_for_x: String,
    y_conv_for_w: String,
    x_quote_record_id: String,
    w_quote_record_id: String,
}

async fn exchange_quotes_and_accept(
    node_x: &Node,
    node_w: &Node,
    node_y: &Node,
    listing_id: &str,
    slot_id: &str,
) -> QuoteExchange {
    let x_conv_did = node_x.dids["conversation"].clone();
    let w_conv_did = node_w.dids["conversation"].clone();
    let y_conv_did = node_y.dids["conversation"].clone();

    let (x_conv_id, _x_req) = open_request_conv(node_x, &y_conv_did, "Clear the back garden").await;
    let (w_conv_id, _w_req) =
        open_request_conv(node_w, &y_conv_did, "Clear the front garden").await;

    let y_conv_for_x = provider_conv_for(node_y, &x_conv_did).await;
    let y_conv_for_w = provider_conv_for(node_y, &w_conv_did).await;

    let x_sync = node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_for_x })).await;
    assert_eq!(x_sync["filed"], 1);
    let w_sync = node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_for_w })).await;
    assert_eq!(w_sync["filed"], 1);

    let x_thread =
        node_y.rpc_ok("transaction.thread", json!({ "conversation": y_conv_for_x })).await;
    let x_req_record_id =
        x_thread["cards"].as_array().unwrap().iter().find(|c| c["card_type"] == "request").unwrap()
            ["record_id"]
            .as_str()
            .unwrap()
            .to_string();
    let w_thread =
        node_y.rpc_ok("transaction.thread", json!({ "conversation": y_conv_for_w })).await;
    let w_req_record_id =
        w_thread["cards"].as_array().unwrap().iter().find(|c| c["card_type"] == "request").unwrap()
            ["record_id"]
            .as_str()
            .unwrap()
            .to_string();

    let quote_terms = |amount: u64| {
        json!({
            "scope": "Clear garden waste and green bin collection",
            "currency": "EUR",
            "amount_minor": amount,
            "tax_minor": 0,
            "fees_minor": 0,
            "payment_methods": ["cash"],
            "payee": "Y Gardens",
            "payment_timing": "after-work",
            "location": { "where": "at-customer", "address": "1 Garden Lane" },
            "cancellation_terms": "24 hours notice required",
            "refund_terms": "Full refund if work not completed",
            "dispute_path": "Informal mediation",
        })
    };

    let x_quote = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "request_record_id": x_req_record_id,
                "listing_id": listing_id,
                "slot_id": slot_id,
                "expires_in_secs": 3600,
                "terms": quote_terms(6000),
            }),
        )
        .await;
    let x_quote_msg = x_quote["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_y, &x_quote_msg).await, "X's quote delivered");
    let x_quote_record_id = x_quote["record_id"].as_str().unwrap().to_string();

    let w_quote = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "request_record_id": w_req_record_id,
                "listing_id": listing_id,
                "slot_id": slot_id,
                "expires_in_secs": 3600,
                "terms": quote_terms(6000),
            }),
        )
        .await;
    let w_quote_msg = w_quote["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_y, &w_quote_msg).await, "W's quote delivered");
    let w_quote_record_id = w_quote["record_id"].as_str().unwrap().to_string();

    node_x.rpc_ok("transaction.sync", json!({ "conversation": x_conv_id })).await;
    node_w.rpc_ok("transaction.sync", json!({ "conversation": w_conv_id })).await;

    let x_accept =
        node_x.rpc_ok("agreement.accept", json!({ "quote_record_id": x_quote_record_id })).await;
    assert_eq!(x_accept["pair"]["state"], "half");
    let x_accept_msg = x_accept["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_x, &x_accept_msg).await, "X's accept delivered");

    let w_accept =
        node_w.rpc_ok("agreement.accept", json!({ "quote_record_id": w_quote_record_id })).await;
    assert_eq!(w_accept["pair"]["state"], "half");
    let w_accept_msg = w_accept["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_w, &w_accept_msg).await, "W's accept delivered");

    QuoteExchange {
        x_conv_id,
        w_conv_id,
        y_conv_for_x,
        y_conv_for_w,
        x_quote_record_id,
        w_quote_record_id,
    }
}

async fn complete_winner_lifecycle(
    winner_node: &Node,
    node_y: &Node,
    winner_quote_record_id: &str,
    winner_conv_id: &str,
    winner_conv_on_y: &str,
) {
    let pay_req =
        node_y.rpc_ok("payment.request", json!({ "agreement": winner_quote_record_id })).await;
    let pay_req_msg = pay_req["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_y, &pay_req_msg).await, "payment request delivered");

    winner_node.rpc_ok("transaction.sync", json!({ "conversation": winner_conv_id })).await;
    let winner_thread =
        winner_node.rpc_ok("transaction.thread", json!({ "conversation": winner_conv_id })).await;
    let pay_req_card = winner_thread["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["card_type"] == "payment-request")
        .expect("payment-request card in winner's thread");
    assert_eq!(pay_req_card["agreement_payee"], "Y Gardens");

    let x_ack = winner_node
        .rpc_ok("payment.acknowledge", json!({ "agreement": winner_quote_record_id }))
        .await;
    let x_ack_msg = x_ack["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(winner_node, &x_ack_msg).await, "consumer's payment ack delivered");

    node_y.rpc_ok("transaction.sync", json!({ "conversation": winner_conv_on_y })).await;
    let y_payment_after_consumer =
        node_y.rpc_ok("payment.get", json!({ "agreement": winner_quote_record_id })).await;
    assert_eq!(y_payment_after_consumer["track"], "claimed");

    let y_ack =
        node_y.rpc_ok("payment.acknowledge", json!({ "agreement": winner_quote_record_id })).await;
    assert_ne!(y_ack["state"], "already-recorded");
    let y_payment_after_provider =
        node_y.rpc_ok("payment.get", json!({ "agreement": winner_quote_record_id })).await;
    assert_eq!(y_payment_after_provider["track"], "acknowledged");

    let y_fulfil =
        node_y.rpc_ok("fulfilment.sign", json!({ "agreement": winner_quote_record_id })).await;
    let y_fulfil_msg = y_fulfil["message_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(node_y, &y_fulfil_msg).await, "provider's fulfilment sign delivered");

    winner_node.rpc_ok("transaction.sync", json!({ "conversation": winner_conv_id })).await;
    let x_fulfil =
        winner_node.rpc_ok("fulfilment.sign", json!({ "agreement": winner_quote_record_id })).await;
    let x_fulfil_msg = x_fulfil["message_id"].as_str().unwrap().to_string();
    assert!(
        wait_delivered(winner_node, &x_fulfil_msg).await,
        "consumer's fulfilment sign delivered"
    );

    let synced = wait_until(Duration::from_secs(30), || async {
        node_y.rpc_ok("transaction.sync", json!({ "conversation": winner_conv_on_y })).await;
        let b = node_y.rpc_ok("booking.get", json!({ "agreement": winner_quote_record_id })).await;
        b["state"] == "completed"
    })
    .await;
    assert!(synced, "provider's booking reaches completed");

    let synced_consumer = wait_until(Duration::from_secs(30), || async {
        winner_node.rpc_ok("transaction.sync", json!({ "conversation": winner_conv_id })).await;
        let b =
            winner_node.rpc_ok("booking.get", json!({ "agreement": winner_quote_record_id })).await;
        b["state"] == "completed"
    })
    .await;
    assert!(synced_consumer, "winner's own progress reaches completed");
}

async fn wait_and_verify_winner_scheduled(
    node_y: &Node,
    owner_y_did: &str,
    winner_node: &Node,
    winner_quote_record_id: &str,
    winner_conv_id: &str,
    winner_conv_on_y: &str,
) {
    let y_thread_winner =
        node_y.rpc_ok("transaction.thread", json!({ "conversation": winner_conv_on_y })).await;
    let y_cards_winner = y_thread_winner["cards"].as_array().unwrap();
    let y_prov_card = y_cards_winner
        .iter()
        .find(|c| c["card_type"] == "agreement-receipt" && c["issuer"] == owner_y_did)
        .expect("provider card in Y thread for winner");
    let prov_msg_id = y_prov_card["message_id"].as_str().unwrap();
    assert!(wait_delivered(node_y, prov_msg_id).await, "countersigned receipt delivered to winner");

    winner_node.rpc_ok("transaction.sync", json!({ "conversation": winner_conv_id })).await;
    let winner_booking =
        winner_node.rpc_ok("booking.get", json!({ "agreement": winner_quote_record_id })).await;
    assert_eq!(winner_booking["state"], "scheduled");
}

async fn assert_loser_retry(node_y: &Node, loser_node: &Node, loser_quote_record_id: &str) {
    let loser_retry = loser_node
        .rpc_ok("agreement.accept", json!({ "quote_record_id": loser_quote_record_id }))
        .await;
    assert_eq!(loser_retry["state"], "already-accepted");
    let loser_booking_after =
        node_y.rpc_ok("booking.get", json!({ "agreement": loser_quote_record_id })).await;
    assert_eq!(loser_booking_after["state"], "conflict");
}

async fn assert_no_directory(nodes: &[&Node]) {
    for node in nodes {
        let sources = node.rpc_ok("directory.sources", json!({})).await;
        let empty = sources["sources"].as_array().map(Vec::is_empty).unwrap_or(true);
        assert!(empty, "{} has no directory sources", node.label);
    }
}

#[tokio::test]
async fn a_losing_concurrent_booking_is_arbitrated_and_the_winner_completes() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let dir_w = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let owner_x = Identity::generate().unwrap();
    let owner_w = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();
    let owner_y_did = substrate::derive_did_key(&owner_y.public_key());

    let cluster =
        boot_three_nodes(dir_x.path(), dir_w.path(), dir_y.path(), &owner_x, &owner_w, &owner_y)
            .await;
    let (node_x, node_w, mut node_y) = (cluster.node_x, cluster.node_w, cluster.node_y);

    let quotes = exchange_quotes_and_accept(
        &node_x,
        &node_w,
        &node_y,
        &cluster.listing_id,
        &cluster.slot_id,
    )
    .await;

    // Restart Y between the accepts landing and the booking decision
    // running, so the decision itself must survive a fresh process.
    node_y.restart(None, None).await;

    // Y runs transaction.sync on both conversations concurrently -- both
    // consumer accept halves complete their pair and trigger the booking
    // decision on the same provider-held slot at the same time.
    let (sync_x_res, sync_w_res) = tokio::join!(
        node_y.rpc_ok("transaction.sync", json!({ "conversation": quotes.y_conv_for_x })),
        node_y.rpc_ok("transaction.sync", json!({ "conversation": quotes.y_conv_for_w })),
    );
    assert_eq!(sync_x_res["filed"], 1);
    assert_eq!(sync_w_res["filed"], 1);

    // Exactly one scheduled and one conflict; the loser's pair stays
    // half (the provider does not countersign a conflicting booking).
    let x_booking =
        node_y.rpc_ok("booking.get", json!({ "agreement": quotes.x_quote_record_id })).await;
    let w_booking =
        node_y.rpc_ok("booking.get", json!({ "agreement": quotes.w_quote_record_id })).await;
    let states: Vec<Value> = vec![x_booking["state"].clone(), w_booking["state"].clone()];
    let scheduled_count = states.iter().filter(|s| **s == json!("scheduled")).count();
    let conflict_count = states.iter().filter(|s| **s == json!("conflict")).count();
    assert_eq!(scheduled_count, 1, "exactly one booking is scheduled: {states:?}");
    assert_eq!(conflict_count, 1, "exactly one booking is a named conflict: {states:?}");

    let (
        winner_quote_record_id,
        winner_node,
        winner_conv_id,
        winner_conv_on_y,
        loser_quote_record_id,
        loser_node,
    ) = if x_booking["state"] == json!("scheduled") {
        (
            quotes.x_quote_record_id.clone(),
            &node_x,
            quotes.x_conv_id.clone(),
            quotes.y_conv_for_x.clone(),
            quotes.w_quote_record_id.clone(),
            &node_w,
        )
    } else {
        (
            quotes.w_quote_record_id.clone(),
            &node_w,
            quotes.w_conv_id.clone(),
            quotes.y_conv_for_w.clone(),
            quotes.x_quote_record_id.clone(),
            &node_x,
        )
    };

    let loser_agr =
        node_y.rpc_ok("agreement.get", json!({ "quote_record_id": loser_quote_record_id })).await;
    assert_eq!(loser_agr["pair"]["state"], "half", "the loser's pair is never completed");

    wait_and_verify_winner_scheduled(
        &node_y,
        &owner_y_did,
        winner_node,
        &winner_quote_record_id,
        &winner_conv_id,
        &winner_conv_on_y,
    )
    .await;

    // The winner's full payment and fulfilment flow to completion.
    complete_winner_lifecycle(
        winner_node,
        &node_y,
        &winner_quote_record_id,
        &winner_conv_id,
        &winner_conv_on_y,
    )
    .await;

    // The loser retries its accept -- already-accepted, no state change.
    assert_loser_retry(&node_y, loser_node, &loser_quote_record_id).await;

    assert_no_directory(&[&node_x, &node_w, &node_y]).await;

    node_x.teardown().await;
    node_w.teardown().await;
    node_y.teardown().await;
}

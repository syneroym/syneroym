#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! A work step of an extension, across two independent `syneroym-substrate`
//! instances: the person who gives a slot booking records `started`, the
//! card travels to the other node, and both nodes list the step.

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate};

mod common;

use common::{
    roym::{
        RoymNode as Node, fast_conversation_role, roym_artifacts_present, wait_delivered,
        wait_until,
    },
    roym_flow::{
        accept_quote, open_request_conv, quote_author_conv_for, quote_terms,
        request_record_on_quote_author, send_quote, wait_and_verify_winner_scheduled,
    },
};

struct Booking {
    node_x: Node,
    node_y: Node,
    owner_y_did: String,
    agreement: String,
    x_conv: String,
    y_conv: String,
    _dirs: [tempfile::TempDir; 2],
}

/// X books the one slot that Y offers, and both nodes hold the agreement.
async fn booked_slot() -> Booking {
    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let owner_y = Identity::generate().unwrap();
    let owner_y_did = substrate::derive_did_key(&owner_y.public_key());

    let mut node_x = Node::boot(
        "node-x",
        dir_x.path().to_path_buf(),
        None,
        Identity::generate().unwrap(),
        fast_conversation_role(3600),
    )
    .await;
    let registry = node_x.registry_url.clone();
    node_x.full_bring_up().await;
    let mut node_y = Node::boot(
        "node-y",
        dir_y.path().to_path_buf(),
        Some(registry),
        owner_y,
        fast_conversation_role(3600),
    )
    .await;
    node_y.full_bring_up().await;

    let x_address = node_x.dids["conversation"].clone();
    let y_address = node_y.dids["conversation"].clone();
    for (node, name, address) in [(&node_x, "X", &x_address), (&node_y, "Y", &y_address)] {
        node.rpc_ok(
            "profile.set",
            json!({ "display_name": name, "conversation_address": address, "lang": "en" }),
        )
        .await;
    }

    let listing = node_y
        .rpc_ok(
            "listing.set",
            json!({
                "title": "Garden clearance",
                "summary": "Same-day one-off garden clearance",
                "lang": "en",
                "categories": ["gardening"],
                "extension": { "id": "scheduling", "version": 1 },
                "booking": {
                    "lead_time_secs": 0,
                    "cancellation_window_secs": 0,
                    "max_per_booking": 1
                },
                "payment": {
                    "currency": "EUR", "model": "fixed", "amount_minor": 6000,
                    "tax_included": true, "payee": "Y Gardens"
                }
            }),
        )
        .await;
    let listing_id = listing["listing_id"].as_str().unwrap().to_string();
    let slot_start = 1_800_000_000_u64;
    let avail = node_y
        .rpc_ok(
            "availability.set",
            json!({
                "listing_id": listing_id,
                "slots": [{ "start_secs": slot_start, "end_secs": slot_start + 3600, "capacity": 1 }],
            }),
        )
        .await;
    let slot_id = avail["slot_ids"][0].as_str().unwrap().to_string();

    let (x_conv, _) =
        open_request_conv(&node_x, &y_address, "Clear the back garden", &["gardening"]).await;
    let y_conv = quote_author_conv_for(&node_y, &x_address).await;
    let request = request_record_on_quote_author(&node_y, &y_conv).await;
    let agreement =
        send_quote(&node_y, &request, &listing_id, &slot_id, quote_terms("Y Gardens", 6000)).await;
    accept_quote(&node_x, &x_conv, &agreement).await;
    node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv })).await;
    wait_and_verify_winner_scheduled(
        &node_y,
        &owner_y_did.clone(),
        &node_x,
        &agreement,
        &x_conv,
        &y_conv,
    )
    .await;

    Booking { node_x, node_y, owner_y_did, agreement, x_conv, y_conv, _dirs: [dir_x, dir_y] }
}

async fn steps(node: &Node, agreement: &str) -> Vec<Value> {
    let listed = node.rpc_ok("step.list", json!({ "agreement": agreement })).await;
    listed["steps"].as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn the_giver_records_a_step_and_the_other_node_lists_it() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }
    let b = booked_slot().await;

    // The giver of the slot is Y, the quote author.
    let added = b
        .node_y
        .rpc_ok(
            "step.add",
            json!({
                "agreement": b.agreement,
                "commitment_id": "c1",
                "step_type": "scheduling/started",
            }),
        )
        .await;
    let record_id = added["record_id"].as_str().unwrap().to_string();
    assert_eq!(added["role"], "quote-author");
    assert!(wait_delivered(&b.node_y, added["message_id"].as_str().unwrap()).await);

    // X files the card when it syncs.
    let filed = wait_until(Duration::from_secs(30), || async {
        b.node_x.rpc_ok("transaction.sync", json!({ "conversation": b.x_conv })).await;
        !steps(&b.node_x, &b.agreement).await.is_empty()
    })
    .await;
    assert!(filed, "node X lists the step after it syncs");

    let on_x = steps(&b.node_x, &b.agreement).await;
    assert_eq!(on_x.len(), 1);
    let step = &on_x[0];
    assert_eq!(step["record_id"], record_id);
    assert_eq!(step["step_type"], "scheduling/started");
    assert_eq!(step["role"], "giver");
    assert_eq!(step["issuer"], b.owner_y_did);
    assert_eq!(step["data"], json!({}));
    assert_eq!(step["verified"], true);
    assert_eq!(step["checked"], true);

    // The thread on X holds one step card with the plain data.
    let thread = b.node_x.rpc_ok("transaction.thread", json!({ "conversation": b.x_conv })).await;
    let cards: Vec<&Value> =
        thread["cards"].as_array().unwrap().iter().filter(|c| c["card_type"] == "step").collect();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["verified"], true);
    assert_eq!(cards[0]["data"]["data"], json!({}));

    // The sender's conversation message carries the disclosures.
    let y_hist = b.node_y.rpc_ok("conversation.history", json!({ "conversation": b.y_conv })).await;
    let step_msg = y_hist["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == added["message_id"].as_str().unwrap())
        .expect("step card message on sender");
    let card_body: Value = serde_json::from_str(step_msg["body"].as_str().unwrap()).unwrap();
    let disclosures = card_body["disclosures"].clone();

    // Either node can verify the signed record; with disclosures, data is revealed.
    let envelope = step["envelope"].as_str().unwrap();
    let verdict = b
        .node_x
        .rpc_ok("step.verify", json!({ "envelope": envelope, "disclosures": disclosures }))
        .await;
    assert_eq!(verdict["verified"], true);
    assert_eq!(verdict["issuer"], b.owner_y_did);
    assert_eq!(verdict["shown"]["data"], json!({}));
    assert!(verdict["hidden"].as_array().is_some_and(|h| h.is_empty()));

    // Without disclosures, data stays hidden.
    let hidden = b.node_x.rpc_ok("step.verify", json!({ "envelope": envelope })).await;
    assert_eq!(hidden["verified"], true);
    assert_eq!(hidden["hidden"], json!(["data"]));

    // The giver's own node lists the same record. A repeat makes no second one.
    let on_y = steps(&b.node_y, &b.agreement).await;
    assert_eq!(on_y.len(), 1);
    assert_eq!(on_y[0]["record_id"], record_id);
    let again = b
        .node_y
        .rpc_ok(
            "step.add",
            json!({
                "agreement": b.agreement,
                "commitment_id": "c1",
                "step_type": "scheduling/started",
            }),
        )
        .await;
    assert_eq!(again["state"], "already-recorded");
    assert_eq!(again["record_id"], record_id);

    // X receives the slot, so `started` is not its step.
    let refused = b
        .node_x
        .rpc_err(
            "step.add",
            json!({
                "agreement": b.agreement,
                "commitment_id": "c1",
                "step_type": "scheduling/started",
            }),
        )
        .await;
    assert_eq!(refused["data"]["code"], "not-your-step");

    b.node_x.teardown().await;
    b.node_y.teardown().await;
}

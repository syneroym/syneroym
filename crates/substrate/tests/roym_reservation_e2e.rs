#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! A reservation that the counterparty cannot keep, across two independent
//! `syneroym-substrate` instances.
//!
//! X is the quote author and writer. Y gives a lesson in one of its slots, and
//! X pays. Y reserves its seat when it accepts and confirms it when it files
//! the writer's countersigned half. A node cannot move its clock by 15
//! minutes, so the failure itself is a card that the test signs with Y's
//! person key: the writer's side of it, and the release on both nodes, are
//! real.

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    commitment_failure::{COMMITMENT_FAILURE_VERSION, CommitmentFailurePayload, FailureReason},
    record::{Envelope, RECORD_COMMITMENT_FAILURE, RecordDraft},
};

mod common;

use common::{
    roym::{
        RoymNode as Node, fast_conversation_role, roym_artifacts_present, wait_delivered,
        wait_until,
    },
    roym_flow::{
        accept_quote, open_request_conv, quote_author_conv_for, quote_terms,
        request_record_on_quote_author, seat_is_taken, send_quote_with,
    },
};

struct Pair {
    node_x: Node,
    node_y: Node,
    owner_y_did: String,
    x_conv: String,
    y_conv: String,
    request: String,
    listing_id: String,
    slot_id: String,
    slot_start: u64,
    _dirs: [tempfile::TempDir; 2],
}

/// X and Y with a conversation, one enquiry filed on X, and one slot on Y.
async fn pair_with_a_slot() -> Pair {
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
    let (listing_id, slot_id, slot_start) = offer_one_slot(&node_y).await;

    let (y_conv, _) = open_request_conv(&node_y, &x_address, "A lesson", &["gardening"]).await;
    let x_conv = quote_author_conv_for(&node_x, &y_address).await;
    let request = request_record_on_quote_author(&node_x, &x_conv).await;
    Pair {
        node_x,
        node_y,
        owner_y_did,
        x_conv,
        y_conv,
        request,
        listing_id,
        slot_id,
        slot_start,
        _dirs: [dir_x, dir_y],
    }
}

/// A listing on `node` with one slot of capacity 1.
async fn offer_one_slot(node: &Node) -> (String, String, u64) {
    let listing = node
        .rpc_ok(
            "listing.set",
            json!({
                "title": "Garden lessons",
                "summary": "One lesson in the garden",
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
    let start = 1_800_000_000_u64;
    let avail = node
        .rpc_ok(
            "availability.set",
            json!({
                "listing_id": listing_id,
                "slots": [{ "start_secs": start, "end_secs": start + 3600, "capacity": 1 }],
            }),
        )
        .await;
    (listing_id, avail["slot_ids"][0].as_str().unwrap().to_string(), start)
}

/// X's quote: Y (the counterparty) gives the lesson in its slot, and X pays.
fn lesson_for_payment(pair: &Pair) -> Value {
    let mut terms = quote_terms("Y Gardens", 6000);
    terms["listing_id"] = json!(pair.listing_id);
    let lesson = &mut terms["commitments"][0];
    lesson["giver"] = json!("counterparty");
    lesson["resource_kind"] = json!("service-in-slot");
    lesson["extension"] = json!({ "id": "scheduling", "version": 1 });
    lesson["window"] = json!({
        "earliest_secs": pair.slot_start,
        "latest_secs": pair.slot_start + 3600,
    });
    lesson["details"] = json!({
        "extension": "scheduling",
        "schema": "scheduling/slot@1",
        "data": { "listing_id": pair.listing_id, "slot_id": pair.slot_id },
        "extra": []
    });
    terms["commitments"][1]["giver"] = json!("quote-author");
    terms
}

/// X quotes, Y accepts, and both nodes hold an agreement in state `agreed`.
async fn agree(pair: &Pair) -> String {
    let agreement = send_quote_with(
        &pair.node_x,
        json!({
            "enquiry_record_id": pair.request,
            "expires_in_secs": 3600,
            "terms": lesson_for_payment(pair),
        }),
    )
    .await;
    accept_quote(&pair.node_y, &pair.y_conv, &agreement).await;
    pair.node_x.rpc_ok("transaction.sync", json!({ "conversation": pair.x_conv })).await;
    let agreed = wait_until(Duration::from_secs(30), || async {
        pair.node_y.rpc_ok("transaction.sync", json!({ "conversation": pair.y_conv })).await;
        let view =
            pair.node_y.rpc_ok("agreement.get", json!({ "quote_record_id": agreement })).await;
        view["state"] == "agreed"
    })
    .await;
    assert!(agreed, "Y files the countersigned half and the agreement is agreed");
    agreement
}

/// Whether the node's reservation for `agreement` is confirmed, and whether a
/// `free:` row ends it.
async fn reservation_of(node: &Node, agreement: &str) -> (bool, bool) {
    let bundle = node.rpc_ok("scheduling.export", json!({})).await;
    let rows = bundle["sections"]["reservations"].as_array().expect("reservations section");
    let reservation = rows
        .iter()
        .find(|r| {
            r["id"].as_str().is_some_and(|id| id.starts_with("rsv:"))
                && r["payload"]["agreement"] == agreement
        })
        .and_then(|r| r["id"].as_str()?.strip_prefix("rsv:"))
        .expect("a reservation row for the agreement");
    let confirmed = rows.iter().any(|r| {
        r["id"] == format!("decided:{reservation}") && r["payload"]["state"] == "confirmed"
    });
    let freed = rows.iter().any(|r| r["id"] == format!("free:{reservation}"));
    (confirmed, freed)
}

/// A failure record of `c1`, signed with Y's person key as Y's node would sign
/// it, as a card body.
fn failure_card(pair: &Pair, owner: &Identity, agreement: &str) -> String {
    let payload = CommitmentFailurePayload {
        agreement: agreement.to_string(),
        conversation: pair.y_conv.clone(),
        commitment_id: "c1".to_string(),
        reason: FailureReason::ReservationLost,
    };
    let draft = RecordDraft {
        version: COMMITMENT_FAILURE_VERSION,
        record_type: RECORD_COMMITMENT_FAILURE.to_string(),
        subject: agreement.to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, pair.owner_y_did.clone(), None, now_secs()).expect("envelope");
    env.attach_signature(z32::encode(&owner.sign(&bytes).to_bytes())).expect("signature");
    let envelope = env.to_json().expect("json");
    card::card_body(RECORD_COMMITMENT_FAILURE, COMMITMENT_FAILURE_VERSION, &envelope, &[])
        .expect("card body")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// The commitments of the newest snapshot that X holds: `(state, reason)` of
/// each commitment id.
async fn commitments_on(node: &Node, agreement: &str) -> Vec<(String, String, Option<String>)> {
    let out = node.rpc_ok("agreement.progress", json!({ "agreement": agreement })).await;
    out["snapshots"][0]["snapshot"]["commitments"]
        .as_array()
        .expect("a snapshot")
        .iter()
        .map(|c| {
            (
                c["commitment_id"].as_str().unwrap().to_string(),
                c["state"].as_str().unwrap().to_string(),
                c["reason"].as_str().map(str::to_string),
            )
        })
        .collect()
}

#[tokio::test]
async fn the_writer_marks_a_failed_slot_and_both_nodes_release() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }
    let pair = pair_with_a_slot().await;
    let y_key = Identity::from_bytes(&pair.node_y.owner.to_bytes());
    let agreement = agree(&pair).await;

    // Y reserved its seat when it accepted, and confirmed it when it filed the
    // countersigned half.
    assert!(seat_is_taken(&pair.node_y, &agreement).await);
    assert_eq!(reservation_of(&pair.node_y, &agreement).await, (true, false));

    // Y's node signs a commitment failure and sends it to X.
    let sent = pair
        .node_y
        .rpc_ok(
            "conversation.send",
            json!({
                "conversation": pair.y_conv,
                "body": failure_card(&pair, &y_key, &agreement),
                "content_type": CARD_CONTENT_TYPE,
            }),
        )
        .await;
    assert!(wait_delivered(&pair.node_y, sent["message_id"].as_str().unwrap()).await);

    // X's next snapshot shows c1 failed and c2 cancelled, and the deal closed.
    let failed = wait_until(Duration::from_secs(30), || async {
        pair.node_x.rpc_ok("transaction.sync", json!({ "conversation": pair.x_conv })).await;
        let view =
            pair.node_x.rpc_ok("agreement.get", json!({ "quote_record_id": agreement })).await;
        view["state"] == "closed"
    })
    .await;
    assert!(failed, "the writer files the failure and closes the agreement");
    assert_eq!(
        commitments_on(&pair.node_x, &agreement).await,
        vec![
            ("c1".to_string(), "failed".to_string(), Some("reservation-lost".to_string())),
            (
                "c2".to_string(),
                "cancelled".to_string(),
                Some("other-commitment-failed".to_string())
            ),
        ]
    );
    let thread =
        pair.node_x.rpc_ok("transaction.thread", json!({ "conversation": pair.x_conv })).await;
    let failure_cards = thread["cards"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["card_type"] == "commitment-failure")
        .count();
    assert_eq!(failure_cards, 1);

    // Y files X's snapshot and releases its seat.
    let released = wait_until(Duration::from_secs(30), || async {
        pair.node_y.rpc_ok("transaction.sync", json!({ "conversation": pair.y_conv })).await;
        !seat_is_taken(&pair.node_y, &agreement).await
    })
    .await;
    assert!(released, "Y frees the seat of the failed commitment");
    assert_eq!(reservation_of(&pair.node_y, &agreement).await, (true, true));

    // The seat can be booked again.
    let second = agree(&pair).await;
    assert_ne!(second, agreement);
    assert!(seat_is_taken(&pair.node_y, &second).await);

    pair.node_x.teardown().await;
    pair.node_y.teardown().await;
}

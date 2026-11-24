//! Dual-build parity tests for transaction driving and as-listed agreements.

use std::time::{Duration, Instant};

use serde_json::json;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_roym_core::transaction;

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_252_transaction_drive_with_two_changed_conversations() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    // Initial drive to register collections and capture the baseline event cursor.
    both_rpc(&h, "transaction.drive", json!({})).await;

    let peer1 = Identity::from_bytes(&[11; 32]);
    let peer2 = Identity::from_bytes(&[12; 32]);

    let peer1_did = derive_did_key(&peer1.public_key());
    let peer2_did = derive_did_key(&peer2.public_key());

    let conv1 = open_conv(&h, &peer1_did).await;
    let conv2 = open_conv(&h, &peer2_did).await;

    let (_, _, req1) = peer_signed_request_with_identity(&peer1, &conv1, 1, 1_000_000);
    let (_, _, req2) = peer_signed_request_with_identity(&peer2, &conv2, 1, 1_000_000);

    deliver_peer_card(
        &h,
        "m-req-1",
        &conv1,
        &peer1_did,
        transaction::RECORD_ENQUIRY,
        transaction::ENQUIRY_VERSION,
        &req1,
    )
    .await;

    deliver_peer_card(
        &h,
        "m-req-2",
        &conv2,
        &peer2_did,
        transaction::RECORD_ENQUIRY,
        transaction::ENQUIRY_VERSION,
        &req2,
    )
    .await;

    let (w, n) = both_rpc(&h, "transaction.drive", json!({})).await;
    assert_eq!(w["result"]["synced"], 2, "wasm synced: {w}");
    assert_eq!(n["result"]["synced"], 2, "native synced: {n}");
    assert_eq!(w["result"]["pending"], 0);
    assert_eq!(n["result"]["pending"], 0);
    assert_eq!(w["result"]["again"], false);
    assert_eq!(n["result"]["again"], false);

    let (tw1, tn1) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv1 })).await;
    let (tw2, tn2) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv2 })).await;
    assert_eq!(tw1["result"]["cards"].as_array().unwrap().len(), 1);
    assert_eq!(tn1["result"]["cards"].as_array().unwrap().len(), 1);
    assert_eq!(tw2["result"]["cards"].as_array().unwrap().len(), 1);
    assert_eq!(tn2["result"]["cards"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_stop_rule_measurement_200_agreements_8_conversations() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    // Initial drive ensures collections exist before seeding agreements
    both_rpc(&h, "transaction.drive", json!({})).await;

    for i in 0..200 {
        let agr_id = format!("rec_agr_{i}");
        let row = json!({
            "quote_record_id": agr_id,
            "quote_id": format!("q_{i}"),
            "conversation": format!("conv_unrelated_{i}"),
            "counterparty_did": peer_did(),
            "quote_author_did": h.owner_did,
            "terms": sample_quote_terms(),
            "state": "agreed",
            "seq": 1,
            "commitments": [],
            "updated_at_secs": 1_000_000,
        });
        seed_row(&h, true, "transaction", "agreements", &agr_id, row.clone()).await;
        seed_row(&h, false, "transaction", "agreements", &agr_id, row).await;
    }

    for i in 0..8 {
        let peer = Identity::from_bytes(&[20 + i as u8; 32]);
        let peer_did = derive_did_key(&peer.public_key());
        let conv = open_conv(&h, &peer_did).await;
        let (_, _, req) = peer_signed_request_with_identity(&peer, &conv, 1, 1_000_000);
        deliver_peer_card(
            &h,
            &format!("m-card-{i}"),
            &conv,
            &peer_did,
            transaction::RECORD_ENQUIRY,
            transaction::ENQUIRY_VERSION,
            &req,
        )
        .await;
    }

    let t0 = Instant::now();
    let (w, n) = both_rpc(&h, "transaction.drive", json!({})).await;
    let elapsed = t0.elapsed();

    eprintln!(
        "STOP_RULE_MEASUREMENT: transaction.drive took {} ms (elapsed: {:?})",
        elapsed.as_millis(),
        elapsed
    );

    assert_eq!(w["result"]["synced"], 8, "wasm synced: {w}");
    assert_eq!(n["result"]["synced"], 8, "native synced: {n}");
    assert!(elapsed < Duration::from_secs(2), "stop rule: must run under 2s, took {:?}", elapsed);
}

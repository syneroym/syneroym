use serde_json::{Value, json};
use syneroym_roym_core::transaction;
use syneroym_signed_record::Envelope;

use super::{fixtures::*, helpers::*};

async fn setup_active_agreement(h: &Harness, conv: &str, timing: &str) -> (String, Value) {
    let (req_rec_id, _req_env) = setup_peer_request(h, conv).await;
    let mut q_params = valid_quote_params(conv, &req_rec_id);
    q_params["terms"]["payment_timing"] = json!(timing);
    let (qw, _) = both_rpc(h, "quote.set", q_params).await;
    let quote_rec_id = qw["result"]["record_id"].as_str().unwrap().to_string();
    let quote_id = qw["result"]["quote_id"].as_str().unwrap().to_string();

    let (get_q, _) = both_rpc(h, "quote.get", json!({ "quote_id": quote_id })).await;
    let quote_env = Envelope::from_json(get_q["result"]["envelope"].as_str().unwrap()).unwrap();
    let terms = quote_env.payload["terms"].clone();

    let (agr_id, agr_env) = peer_signed_consumer_receipt(
        conv,
        &quote_rec_id,
        &peer_did(),
        &owner_did(),
        terms.clone(),
        1_001_000,
    );
    deliver_peer_card(
        h,
        &format!("m-agr-{agr_id}"),
        conv,
        &peer_did(),
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &agr_env,
    )
    .await;
    both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    (quote_rec_id, terms)
}

async fn setup_peer_request(h: &Harness, conv: &str) -> (String, String) {
    let (req_id, req_env) = peer_signed_request(conv, 1, 1_000_000);
    deliver_peer_card(
        h,
        &format!("m-req-{req_id}"),
        conv,
        &peer_did(),
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_env,
    )
    .await;
    both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    (req_id, req_env)
}

#[tokio::test]
async fn scenario_163_fulfilment_halves_and_completion_orders_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, terms) = setup_active_agreement(&h, &conv, "after-work").await;
    let amount_minor = terms["amount_minor"].as_i64().unwrap();
    let curr = terms["currency"].as_str().unwrap();

    // Provider starts booking
    both_rpc(&h, "booking.start", json!({ "agreement": agr_rec })).await;

    // Provider fulfilment half -> claimed
    let (fw, fn_) = both_rpc(&h, "fulfilment.sign", json!({ "agreement": agr_rec })).await;
    assert_eq!(fw["result"]["role"], "provider");
    assert_eq!(fn_["result"]["role"], "provider");

    let (fg_w, fg_n) = both_rpc(&h, "fulfilment.get", json!({ "agreement": agr_rec })).await;
    assert_eq!(fg_w["result"]["track"], "claimed");
    assert_eq!(fg_n["result"]["track"], "claimed");

    // Consumer fulfilment half -> acknowledged
    let (cf_id, cf_env) = peer_signed_fulfilment(
        &conv,
        &agr_rec,
        &peer_did(),
        &owner_did(),
        "consumer",
        terms.clone(),
        1_002_000,
    );
    deliver_peer_card(
        &h,
        &format!("m-ful-{cf_id}"),
        &conv,
        &peer_did(),
        syneroym_roym_core::record::RECORD_FULFILMENT_RECEIPT,
        syneroym_roym_core::fulfilment::FULFILMENT_RECEIPT_VERSION,
        &cf_env,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let (fg2_w, fg2_n) = both_rpc(&h, "fulfilment.get", json!({ "agreement": agr_rec })).await;
    assert_eq!(fg2_w["result"]["track"], "acknowledged");
    assert_eq!(fg2_n["result"]["track"], "acknowledged");

    // Both tracks acknowledged completes booking
    let (c_ack_id, c_ack_env) = peer_signed_payment_ack(
        &conv,
        &agr_rec,
        &peer_did(),
        &owner_did(),
        "consumer",
        curr,
        amount_minor,
        1_003_000,
        Some("cash"),
        None,
        None,
        1_003_000,
    );
    deliver_peer_card(
        &h,
        &format!("m-pay-{c_ack_id}"),
        &conv,
        &peer_did(),
        syneroym_roym_core::record::RECORD_PAYMENT_ACKNOWLEDGEMENT,
        syneroym_roym_core::payment::PAYMENT_ACKNOWLEDGEMENT_VERSION,
        &c_ack_env,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    both_rpc(&h, "payment.acknowledge", json!({ "agreement": agr_rec, "method": "cash" })).await;

    let (bw, bn) = both_rpc(&h, "booking.get", json!({ "agreement": agr_rec })).await;
    assert_eq!(bw["result"]["state"], "completed");
    assert_eq!(bn["result"]["state"], "completed");
    assert_eq!(bw["result"]["payment"], "acknowledged");
    assert_eq!(bn["result"]["payment"], "acknowledged");
    assert_eq!(bw["result"]["fulfilment"], "acknowledged");
    assert_eq!(bn["result"]["fulfilment"], "acknowledged");
}

#[tokio::test]
async fn scenario_164_fulfilment_halves_wrong_agreement_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, terms) = setup_active_agreement(&h, &conv, "after-work").await;

    // Fulfilment with non-existent agreement is refused
    let (bad_w, bad_n) =
        both_rpc(&h, "fulfilment.sign", json!({ "agreement": "rec_nonexistent" })).await;
    assert!(is_err(&bad_w, -32602));
    assert!(is_err(&bad_n, -32602));

    // Peer fulfilment with wrong consumer_did
    let (bad_cf_id, bad_cf_env) = peer_signed_fulfilment(
        &conv,
        &agr_rec,
        "did:key:zWrong",
        &owner_did(),
        "consumer",
        terms,
        1_002_000,
    );
    deliver_peer_card(
        &h,
        &format!("m-badful-{bad_cf_id}"),
        &conv,
        &peer_did(),
        syneroym_roym_core::record::RECORD_FULFILMENT_RECEIPT,
        syneroym_roym_core::fulfilment::FULFILMENT_RECEIPT_VERSION,
        &bad_cf_env,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let (mut tw, mut tn) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    strip_volatile(&mut tw);
    strip_volatile(&mut tn);
    normalize_message_ids(&mut tw);
    normalize_message_ids(&mut tn);
    normalize_progress_record_ids(&mut tw);
    normalize_progress_record_ids(&mut tn);
    assert_eq!(tw, tn);

    // A refused card carries no record id -- `refuse_card` persists
    // whatever `row` already held, and nothing sets one before that early
    // return -- so the refusal is found by its outcome, not its id.
    let cards = tw["result"]["cards"].as_array().unwrap();
    let unverified: Vec<_> = cards.iter().filter(|c| c["verified"] == false).collect();
    assert_eq!(unverified.len(), 1, "only the wrong-consumer card must be unverified");
}

#[tokio::test]
async fn scenario_165_track_window_expiration_and_late_half_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (req_rec_id, _req_env) = setup_peer_request(&h, &conv).await;
    let mut q_params = valid_quote_params(&conv, &req_rec_id);
    // Schedule ended in the distant past (more than 30 days ago)
    q_params["terms"]["schedule"] = json!({
        "earliest_secs": 1_000,
        "latest_secs": 2_000,
    });
    let (qw, _) = both_rpc(&h, "quote.set", q_params).await;
    let quote_rec_id = qw["result"]["record_id"].as_str().unwrap().to_string();
    let quote_id = qw["result"]["quote_id"].as_str().unwrap().to_string();

    let (get_q, _) = both_rpc(&h, "quote.get", json!({ "quote_id": quote_id })).await;
    let quote_env = Envelope::from_json(get_q["result"]["envelope"].as_str().unwrap()).unwrap();
    let q_payload = &quote_env.payload;

    let (agr_id, agr_env) = peer_signed_consumer_receipt(
        &conv,
        &quote_rec_id,
        &peer_did(),
        &owner_did(),
        q_payload["terms"].clone(),
        1_001_000,
    );
    deliver_peer_card(
        &h,
        &format!("m-agr-{agr_id}"),
        &conv,
        &peer_did(),
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &agr_env,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    // Booking.get detects track window expiration
    let (bw, bn) = both_rpc(&h, "booking.get", json!({ "agreement": quote_rec_id })).await;
    assert_eq!(bw["result"]["state"], "ended-unconfirmed");
    assert_eq!(bn["result"]["state"], "ended-unconfirmed");
    assert_eq!(bw["result"]["payment"], "unconfirmed");
    assert_eq!(bn["result"]["payment"], "unconfirmed");
    assert_eq!(bw["result"]["fulfilment"], "unconfirmed");
    assert_eq!(bn["result"]["fulfilment"], "unconfirmed");

    // Late half arriving after terminal state does not alter terminal state
    let (late_cf_id, late_cf_env) = peer_signed_fulfilment(
        &conv,
        &quote_rec_id,
        &peer_did(),
        &owner_did(),
        "consumer",
        q_payload["terms"].clone(),
        20_000_000,
    );
    deliver_peer_card(
        &h,
        &format!("m-lateful-{late_cf_id}"),
        &conv,
        &peer_did(),
        syneroym_roym_core::record::RECORD_FULFILMENT_RECEIPT,
        syneroym_roym_core::fulfilment::FULFILMENT_RECEIPT_VERSION,
        &late_cf_env,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let (b2w, b2n) = both_rpc(&h, "booking.get", json!({ "agreement": quote_rec_id })).await;
    assert_eq!(b2w["result"]["state"], "ended-unconfirmed");
    assert_eq!(b2n["result"]["state"], "ended-unconfirmed");
}

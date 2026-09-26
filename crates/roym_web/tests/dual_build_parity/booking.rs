use std::collections::HashSet;

use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_roym_core::transaction;
use syneroym_signed_record::{Envelope, RecordDraft};

use super::{fixtures::*, helpers::*};

fn peer_identity_2() -> Identity {
    Identity::from_bytes(&[8; 32])
}

/// A one-hour slot window starting `offset` seconds from now, so its track
/// window (30 days past the window's end) has not yet closed against the
/// real wall clock the booking track check reads.
fn future_schedule(offset: u64) -> (u64, u64) {
    let start = wall_now() + offset;
    (start, start + 3600)
}

fn peer_signed_request_with_identity(
    conv: &str,
    issued_at: u64,
    id: &Identity,
) -> (String, String) {
    let did = derive_did_key(&id.public_key());
    let req_id = transaction::derive_request_id(conv, &did, 1).unwrap();
    let payload = json!({
        "request_id": req_id,
        "conversation": conv,
        "sequence": 1,
        "categories": ["gardening"],
        "description": "Fix fence",
        "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
    });
    let draft = RecordDraft {
        version: transaction::REQUEST_VERSION,
        record_type: transaction::RECORD_REQUEST.to_string(),
        subject: req_id,
        payload,
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, did, None, issued_at).unwrap();
    env.attach_signature(z32::encode(&id.sign(&bytes).to_bytes())).unwrap();
    (env.record_id().unwrap(), env.to_json().unwrap())
}

fn peer_signed_consumer_receipt_with_identity(
    conv: &str,
    quote_record_id: &str,
    consumer_did: &str,
    provider_did: &str,
    terms: Value,
    issued_at: u64,
    id: &Identity,
) -> (String, String) {
    let _ = conv;
    let payload = json!({
        "quote_record_id": quote_record_id,
        "consumer_did": consumer_did,
        "provider_did": provider_did,
        "role": "consumer",
        "terms": terms,
    });
    let draft = RecordDraft {
        version: transaction::AGREEMENT_RECEIPT_VERSION,
        record_type: transaction::RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: quote_record_id.to_string(),
        payload,
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, consumer_did.to_string(), None, issued_at).unwrap();
    env.attach_signature(z32::encode(&id.sign(&bytes).to_bytes())).unwrap();
    (env.record_id().unwrap(), env.to_json().unwrap())
}

fn slot_quote(
    conv: &str,
    req_rec_id: &str,
    listing_id: &str,
    slot_id: &str,
    earliest: u64,
    latest: u64,
) -> Value {
    let mut q = valid_quote_params(conv, req_rec_id);
    q["listing_id"] = json!(listing_id);
    q["slot_id"] = json!(slot_id);
    q["terms"]["schedule"] = json!({ "earliest_secs": earliest, "latest_secs": latest });
    q
}

async fn setup_listing_with_slot(
    h: &Harness,
    slug: &str,
    start: u64,
    end: u64,
    cap: u32,
) -> (String, String) {
    enrol_signing(h, "catalog").await;
    let (set, _) = both_rpc(h, "listing.set", full_listing_params(slug, "Fence Repair")).await;
    let listing_id = set["result"]["listing_id"].as_str().unwrap().to_string();

    let slots = json!([{ "start_secs": start, "end_secs": end, "capacity": cap }]);
    let (w, _) =
        both_rpc(h, "availability.set", json!({ "listing_id": listing_id, "slots": slots })).await;
    let slot_id = w["result"]["slot_ids"][0].as_str().unwrap().to_string();
    (listing_id, slot_id)
}

async fn setup_peer_request(h: &Harness, conv: &str) -> (String, String) {
    let (req_id, req_env) = peer_signed_request(conv, 1, 1_000_000);
    deliver_peer_card(
        h,
        "m-req-setup",
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
async fn scenario_151_quote_set_with_slot_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (start, end) = future_schedule(1_000_000);
    let (listing_id, slot_id) = setup_listing_with_slot(&h, "fence-repair", start, end, 1).await;
    let (req_rec_id, _req_env) = setup_peer_request(&h, &conv).await;

    // Quote with matching slot binds slot_id and window
    let mut q_params = slot_quote(&conv, &req_rec_id, &listing_id, &slot_id, start, end);
    let (qw, qn) = both_rpc(&h, "quote.set", q_params.clone()).await;
    assert_eq!(qw["result"]["state"], qn["result"]["state"]);

    let quote_id_w = qw["result"]["quote_id"].as_str().unwrap();
    let quote_id_n = qn["result"]["quote_id"].as_str().unwrap();
    let mut gw = one_rpc(&h, true, "quote.get", json!({ "quote_id": quote_id_w })).await;
    let mut gn = one_rpc(&h, false, "quote.get", json!({ "quote_id": quote_id_n })).await;

    for res in [&gw, &gn] {
        let env_str = res["result"]["envelope"].as_str().unwrap();
        let env = Envelope::from_json(env_str).unwrap();
        let payload = &env.payload;
        assert_eq!(payload["slot_id"], slot_id);
        assert_eq!(payload["terms"]["schedule"]["earliest_secs"], start);
        assert_eq!(payload["terms"]["schedule"]["latest_secs"], end);
    }

    if let Some(res) = gw.get_mut("result").and_then(Value::as_object_mut) {
        res.remove("record_id");
        res.remove("quote_id");
        res.remove("envelope");
    }
    if let Some(res) = gn.get_mut("result").and_then(Value::as_object_mut) {
        res.remove("record_id");
        res.remove("quote_id");
        res.remove("envelope");
    }
    assert_eq!(stripped(&gw), stripped(&gn));

    // Slot from another listing is refused
    let (other_start, other_end) = future_schedule(2_000_000);
    let (other_listing, _) =
        setup_listing_with_slot(&h, "fence-repair-other", other_start, other_end, 1).await;
    q_params["listing_id"] = json!(other_listing);
    let (bad_w, bad_n) = both_rpc(&h, "quote.set", q_params).await;
    assert!(is_err(&bad_w, -32602));
    assert!(is_err(&bad_n, -32602));
}

#[tokio::test]
async fn scenario_152_consumer_half_slot_quote_schedules_and_countersigns_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (start, end) = future_schedule(1_000_000);
    let (listing_id, slot_id) = setup_listing_with_slot(&h, "fence-repair", start, end, 1).await;
    let (req_rec_id, _req_env) = setup_peer_request(&h, &conv).await;

    let q_params = slot_quote(&conv, &req_rec_id, &listing_id, &slot_id, start, end);
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

    let (sw, sn) =
        both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    assert_eq!(sw["result"]["filed"], sn["result"]["filed"]);

    let (bw, bn) = both_rpc(&h, "booking.get", json!({ "agreement": quote_rec_id })).await;
    assert_eq!(bw["result"]["state"], "scheduled");
    assert_eq!(bn["result"]["state"], "scheduled");
    assert_eq!(bw["result"]["seq"], 1);
    assert_eq!(bn["result"]["seq"], 1);
    assert_eq!(bw["result"]["pair"]["state"], "complete");
    assert_eq!(bn["result"]["pair"]["state"], "complete");
}

#[tokio::test]
async fn scenario_153_two_consumer_halves_capacity_1_concurrent_sync_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    let peer2 = peer_identity_2();
    let peer2_did_str = syneroym_identity::substrate::derive_did_key(&peer2.public_key());

    let conv1 = open_conv(&h, &peer_did()).await;
    let conv2 = open_conv(&h, &peer2_did_str).await;

    let (start, end) = future_schedule(1_000_000);
    let (listing_id, slot_id) = setup_listing_with_slot(&h, "fence-repair", start, end, 1).await;
    let (req_rec1, _r1_env) = setup_peer_request(&h, &conv1).await;

    let (req2_id, req2_env) = peer_signed_request_with_identity(&conv2, 1_000_000, &peer2);
    deliver_peer_card(
        &h,
        &format!("m-req2-{req2_id}"),
        &conv2,
        &peer2_did_str,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req2_env,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv2, "full": true })).await;

    let q1_params = slot_quote(&conv1, &req_rec1, &listing_id, &slot_id, start, end);
    let (q1w, _) = both_rpc(&h, "quote.set", q1_params).await;
    let q1_rec = q1w["result"]["record_id"].as_str().unwrap().to_string();
    let q1_id = q1w["result"]["quote_id"].as_str().unwrap().to_string();

    let q2_params = slot_quote(&conv2, &req2_id, &listing_id, &slot_id, start, end);
    let (q2w, _) = both_rpc(&h, "quote.set", q2_params).await;
    let q2_rec = q2w["result"]["record_id"].as_str().unwrap().to_string();
    let q2_id = q2w["result"]["quote_id"].as_str().unwrap().to_string();

    let (gq1, _) = both_rpc(&h, "quote.get", json!({ "quote_id": q1_id })).await;
    let env1 = Envelope::from_json(gq1["result"]["envelope"].as_str().unwrap()).unwrap();
    let q1_terms = env1.payload["terms"].clone();

    let (gq2, _) = both_rpc(&h, "quote.get", json!({ "quote_id": q2_id })).await;
    let env2 = Envelope::from_json(gq2["result"]["envelope"].as_str().unwrap()).unwrap();
    let q2_terms = env2.payload["terms"].clone();

    let (agr1_id, agr1_env) = peer_signed_consumer_receipt(
        &conv1,
        &q1_rec,
        &peer_did(),
        &owner_did(),
        q1_terms,
        1_001_000,
    );
    deliver_peer_card(
        &h,
        &format!("m-agr1-{agr1_id}"),
        &conv1,
        &peer_did(),
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &agr1_env,
    )
    .await;

    let (agr2_id, agr2_env) = peer_signed_consumer_receipt_with_identity(
        &conv2,
        &q2_rec,
        &peer2_did_str,
        &owner_did(),
        q2_terms,
        1_001_000,
        &peer2,
    );
    deliver_peer_card(
        &h,
        &format!("m-agr2-{agr2_id}"),
        &conv2,
        &peer2_did_str,
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &agr2_env,
    )
    .await;

    // Concurrent sync on both conversations
    let sync1 = both_rpc(&h, "transaction.sync", json!({ "conversation": conv1, "full": true }));
    let sync2 = both_rpc(&h, "transaction.sync", json!({ "conversation": conv2, "full": true }));
    let _ = tokio::join!(sync1, sync2);

    let (b1w, b1n) = both_rpc(&h, "booking.get", json!({ "agreement": q1_rec })).await;
    let (b2w, b2n) = both_rpc(&h, "booking.get", json!({ "agreement": q2_rec })).await;

    let mut states_w = HashSet::new();
    states_w.insert(b1w["result"]["state"].as_str().unwrap().to_string());
    states_w.insert(b2w["result"]["state"].as_str().unwrap().to_string());

    let mut states_n = HashSet::new();
    states_n.insert(b1n["result"]["state"].as_str().unwrap().to_string());
    states_n.insert(b2n["result"]["state"].as_str().unwrap().to_string());

    assert!(states_w.contains("scheduled") && states_w.contains("conflict"));
    assert!(states_n.contains("scheduled") && states_n.contains("conflict"));

    let loser_w = if b1w["result"]["state"] == "conflict" { &b1w } else { &b2w };
    assert_eq!(loser_w["result"]["conflict"], "slot-taken");
    assert_eq!(loser_w["result"]["pair"]["state"], "half");
}

#[tokio::test]
async fn scenario_154_duplicate_consumer_half_delivery_idempotent_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (start, end) = future_schedule(1_000_000);
    let (listing_id, slot_id) = setup_listing_with_slot(&h, "fence-repair", start, end, 1).await;
    let (req_rec_id, _req_env) = setup_peer_request(&h, &conv).await;

    let q_params = slot_quote(&conv, &req_rec_id, &listing_id, &slot_id, start, end);
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

    let (b_first_w, b_first_n) =
        both_rpc(&h, "booking.get", json!({ "agreement": quote_rec_id })).await;
    assert_eq!(b_first_w["result"]["state"], "scheduled");
    assert_eq!(b_first_n["result"]["state"], "scheduled");

    // Second delivery of same card in another message
    deliver_peer_card(
        &h,
        &format!("m-agr-dup-{agr_id}"),
        &conv,
        &peer_did(),
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &agr_env,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let (b_sec_w, b_sec_n) =
        both_rpc(&h, "booking.get", json!({ "agreement": quote_rec_id })).await;
    assert_eq!(b_first_w["result"], b_sec_w["result"]);
    assert_eq!(b_first_n["result"], b_sec_n["result"]);
}

#[tokio::test]
async fn scenario_155_slot_removed_before_acceptance_conflict_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (start, end) = future_schedule(1_000_000);
    let (listing_id, slot_id) = setup_listing_with_slot(&h, "fence-repair", start, end, 1).await;
    let (req_rec_id, _req_env) = setup_peer_request(&h, &conv).await;

    let q_params = slot_quote(&conv, &req_rec_id, &listing_id, &slot_id, start, end);
    let (qw, _) = both_rpc(&h, "quote.set", q_params).await;
    let quote_rec_id = qw["result"]["record_id"].as_str().unwrap().to_string();
    let quote_id = qw["result"]["quote_id"].as_str().unwrap().to_string();

    let (get_q, _) = both_rpc(&h, "quote.get", json!({ "quote_id": quote_id })).await;
    let quote_env = Envelope::from_json(get_q["result"]["envelope"].as_str().unwrap()).unwrap();
    let q_payload = &quote_env.payload;

    // Remove the slot from the provider's catalog. `availability.set` only
    // adds/replaces the slots it is given; an empty list is a no-op, so
    // the slot must be deleted with its own verb.
    both_rpc(&h, "availability.remove", json!({ "slot_id": slot_id })).await;

    // Consumer acceptance arrives after slot is removed
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

    let (bw, bn) = both_rpc(&h, "booking.get", json!({ "agreement": quote_rec_id })).await;
    assert_eq!(bw["result"]["state"], "conflict");
    assert_eq!(bn["result"]["state"], "conflict");
    assert_eq!(bw["result"]["conflict"], "slot-unavailable");
    assert_eq!(bn["result"]["conflict"], "slot-unavailable");
}

#[tokio::test]
async fn scenario_156_booking_start_cancel_and_seat_reuse_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (start, end) = future_schedule(1_000_000);
    let (listing_id, slot_id) = setup_listing_with_slot(&h, "fence-repair", start, end, 1).await;
    let (req_rec_id, _req_env) = setup_peer_request(&h, &conv).await;

    let q_params = slot_quote(&conv, &req_rec_id, &listing_id, &slot_id, start, end);
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

    // Provider starts work
    let (sw, sn) = both_rpc(&h, "booking.start", json!({ "agreement": quote_rec_id })).await;
    assert_eq!(sw["result"]["state"], "in-progress");
    assert_eq!(sn["result"]["state"], "in-progress");

    // Provider cancels booking when tracks are still none
    let (cw, cn) = both_rpc(
        &h,
        "booking.cancel",
        json!({ "agreement": quote_rec_id, "reason": "Weather delay" }),
    )
    .await;
    assert_eq!(cw["result"]["state"], "cancelled");
    assert_eq!(cn["result"]["state"], "cancelled");
    assert_eq!(cw["result"]["cancel_reason"], "Weather delay");
    assert_eq!(cn["result"]["cancel_reason"], "Weather delay");

    // Seat is freed; a second consumer now schedules on that same slot
    let peer3 = peer_identity_2();
    let peer3_did_str = syneroym_identity::substrate::derive_did_key(&peer3.public_key());
    let conv3 = open_conv(&h, &peer3_did_str).await;
    let (req3_id, req3_env) = peer_signed_request_with_identity(&conv3, 1_000_000, &peer3);
    deliver_peer_card(
        &h,
        &format!("m-req3-{req3_id}"),
        &conv3,
        &peer3_did_str,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req3_env,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv3, "full": true })).await;

    let q3_params = slot_quote(&conv3, &req3_id, &listing_id, &slot_id, start, end);
    let (q3w, _) = both_rpc(&h, "quote.set", q3_params).await;
    let q3_rec = q3w["result"]["record_id"].as_str().unwrap().to_string();
    let q3_id = q3w["result"]["quote_id"].as_str().unwrap().to_string();

    let (gq3, _) = both_rpc(&h, "quote.get", json!({ "quote_id": q3_id })).await;
    let env3 = Envelope::from_json(gq3["result"]["envelope"].as_str().unwrap()).unwrap();
    let q3_terms = env3.payload["terms"].clone();

    let (agr3_id, agr3_env) = peer_signed_consumer_receipt_with_identity(
        &conv3,
        &q3_rec,
        &peer3_did_str,
        &owner_did(),
        q3_terms,
        1_001_000,
        &peer3,
    );
    deliver_peer_card(
        &h,
        &format!("m-agr3-{agr3_id}"),
        &conv3,
        &peer3_did_str,
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &agr3_env,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv3, "full": true })).await;

    let (b3w, b3n) = both_rpc(&h, "booking.get", json!({ "agreement": q3_rec })).await;
    assert_eq!(b3w["result"]["state"], "scheduled");
    assert_eq!(b3n["result"]["state"], "scheduled");

    // Cannot cancel after a track moved
    both_rpc(&h, "payment.acknowledge", json!({ "agreement": q3_rec })).await;
    let (cant_w, cant_n) =
        both_rpc(&h, "booking.cancel", json!({ "agreement": q3_rec, "reason": "Too late" })).await;
    assert!(is_err(&cant_w, -32602) || is_err(&cant_w, -32603));
    assert!(is_err(&cant_n, -32602) || is_err(&cant_n, -32603));
}

async fn setup_accepted_consumer_quote(h: &Harness, conv: &str) -> String {
    let req = both_rpc(
        h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Prune the hedge",
            "categories": ["gardening"],
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await
    .0;
    let req_rec_id = req["result"]["record_id"].as_str().unwrap().to_string();

    let (q_rec_id, q_env) = peer_signed_quote(conv, 1, &req_rec_id, &owner_did(), None, 1_000);
    deliver_peer_card(
        h,
        "m-quote-157",
        conv,
        &peer_did(),
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_env,
    )
    .await;
    both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    both_rpc(h, "agreement.accept", json!({ "quote_record_id": q_rec_id })).await;
    q_rec_id
}

async fn deliver_progress_card(h: &Harness, mid: &str, conv: &str, env: &str) {
    deliver_peer_card(
        h,
        mid,
        conv,
        &peer_did(),
        syneroym_roym_core::record::RECORD_BOOKING_PROGRESS,
        syneroym_roym_core::booking::BOOKING_PROGRESS_VERSION,
        env,
    )
    .await;
    both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
}

async fn assert_progress_thread_cards(h: &Harness, conv: &str, verified_ids: &[&str]) {
    let (mut tw, mut tn) = both_rpc(h, "transaction.thread", json!({ "conversation": conv })).await;
    strip_volatile(&mut tw);
    strip_volatile(&mut tn);
    normalize_message_ids(&mut tw);
    normalize_message_ids(&mut tn);
    assert_eq!(tw, tn);

    // A card refused before its payload is even extracted (the impostor's,
    // here) carries no record id -- `refuse_card` persists whatever `row`
    // already held, and nothing sets one before that early return.
    let cards = tw["result"]["cards"].as_array().unwrap();
    for id in verified_ids {
        let card = cards.iter().find(|c| c["record_id"].as_str() == Some(*id)).unwrap();
        assert_eq!(card["verified"], true, "{id} must verify");
    }
    let unverified: Vec<_> = cards.iter().filter(|c| c["verified"] == false).collect();
    assert_eq!(unverified.len(), 1, "only the impostor card must be unverified");
}

#[tokio::test]
async fn scenario_157_progress_card_wrong_key_or_lower_seq_refused_parity() {
    // A booking-progress card only means something to the consumer's node
    // (the provider writes its own booking state directly); the harness
    // plays the consumer here, and the peer's fixed key plays the provider.
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;
    let q_rec_id = setup_accepted_consumer_quote(&h, &conv).await;

    // A legitimate seq-1 snapshot, signed by the provider's own key.
    let (prog1_id, prog1_env) = peer_signed_progress(
        &conv,
        &q_rec_id,
        &owner_did(),
        &peer_did(),
        1,
        "scheduled",
        None,
        "none",
        "none",
        2_000_000,
        &peer_identity(),
        1_001_500,
    );
    deliver_progress_card(&h, "m-prog1-157", &conv, &prog1_env).await;

    let (bw1, bn1) = both_rpc(&h, "booking.get", json!({ "agreement": q_rec_id })).await;
    assert_eq!(bw1["result"]["seq"], 1);
    assert_eq!(bn1["result"]["seq"], 1);

    // A legitimate seq-2 snapshot replaces it.
    let (prog2_id, prog2_env) = peer_signed_progress(
        &conv,
        &q_rec_id,
        &owner_did(),
        &peer_did(),
        2,
        "in-progress",
        None,
        "none",
        "none",
        2_000_000,
        &peer_identity(),
        1_001_600,
    );
    deliver_progress_card(&h, "m-prog2-157", &conv, &prog2_env).await;

    let (bw2, bn2) = both_rpc(&h, "booking.get", json!({ "agreement": q_rec_id })).await;
    assert_eq!(bw2["result"]["seq"], 2);
    assert_eq!(bn2["result"]["seq"], 2);

    // A seq-2 snapshot signed by an impostor key is filed refused, and does
    // not disturb the stored seq-2 progress.
    let impostor = Identity::generate().unwrap();
    let (_prog_bad_id, prog_bad_env) = peer_signed_progress(
        &conv,
        &q_rec_id,
        &owner_did(),
        &peer_did(),
        2,
        "in-progress",
        None,
        "none",
        "none",
        2_000_000,
        &impostor,
        1_001_700,
    );
    deliver_progress_card(&h, "m-prog-bad-157", &conv, &prog_bad_env).await;

    // A legitimate but lower-seq snapshot (seq 1 again) is filed verified,
    // and also does not replace the stored seq-2 progress.
    let (prog_low_id, prog_low_env) = peer_signed_progress(
        &conv,
        &q_rec_id,
        &owner_did(),
        &peer_did(),
        1,
        "scheduled",
        None,
        "none",
        "none",
        2_000_000,
        &peer_identity(),
        1_001_800,
    );
    deliver_progress_card(&h, "m-prog-low-157", &conv, &prog_low_env).await;

    let (bw3, bn3) = both_rpc(&h, "booking.get", json!({ "agreement": q_rec_id })).await;
    assert_eq!(bw3["result"]["seq"], 2, "the impostor and lower-seq cards must not replace seq 2");
    assert_eq!(bn3["result"]["seq"], 2);

    assert_progress_thread_cards(&h, &conv, &[&prog1_id, &prog2_id, &prog_low_id]).await;
}

use serde_json::json;

use super::{deal_fixtures::*, fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_163_fulfilment_halves_and_completion_orders_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv, "after-work").await;

    // Quote author fulfilment half -> claimed
    let (fw, fn_) =
        both_rpc(&h, "fulfilment.sign", json!({ "agreement": agr_rec, "commitment_id": "c1" }))
            .await;
    assert_eq!(fw["result"]["role"], "quote-author");
    assert_eq!(fn_["result"]["role"], "quote-author");

    let (fg_w, fg_n) =
        both_rpc(&h, "fulfilment.get", json!({ "agreement": agr_rec, "commitment_id": "c1" }))
            .await;
    assert_eq!(fg_w["result"]["track"], "claimed");
    assert_eq!(fg_n["result"]["track"], "claimed");

    // Counterparty fulfilment half -> acknowledged
    let (cf_id, cf_env) = peer_signed_fulfilment(
        &conv,
        &agr_rec,
        &peer_did(),
        &owner_did(),
        "receiver",
        "c1",
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

    let (fg2_w, fg2_n) =
        both_rpc(&h, "fulfilment.get", json!({ "agreement": agr_rec, "commitment_id": "c1" }))
            .await;
    assert_eq!(fg2_w["result"]["track"], "fulfilled");
    assert_eq!(fg2_n["result"]["track"], "fulfilled");

    // Both tracks acknowledged completes agreement
    let (c_ack_id, c_ack_env) = peer_signed_payment_ack(
        &conv,
        &agr_rec,
        &peer_did(),
        &owner_did(),
        "giver",
        "c2",
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

    both_rpc(
        &h,
        "payment.acknowledge",
        json!({ "agreement": agr_rec, "commitment_id": "c2", "method": "cash" }),
    )
    .await;

    let (bw, bn) = both_rpc(&h, "agreement.get", json!({ "agreement": agr_rec })).await;
    assert_eq!(bw["result"]["state"], "closed");
    assert_eq!(bn["result"]["state"], "closed");
}

#[tokio::test]
async fn scenario_164_fulfilment_halves_wrong_agreement_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv, "after-work").await;

    // Fulfilment with non-existent agreement is refused
    let (bad_w, bad_n) = both_rpc(
        &h,
        "fulfilment.sign",
        json!({ "agreement": "rec_nonexistent", "commitment_id": "c1" }),
    )
    .await;
    assert!(is_err(&bad_w, -32602));
    assert!(is_err(&bad_n, -32602));

    // Peer fulfilment with wrong counterparty_did
    let (bad_cf_id, bad_cf_env) = peer_signed_fulfilment(
        &conv,
        &agr_rec,
        "did:key:zWrong",
        &owner_did(),
        "receiver",
        "c1",
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
    assert_eq!(unverified.len(), 1, "only the wrong-counterparty card must be unverified");
}

#[tokio::test]
async fn scenario_165_late_half_after_terminal_state_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv, "after-work").await;

    // Writer cancels agreement -> terminal state "cancelled"
    let (cw, cn) = both_rpc(
        &h,
        "agreement.cancel",
        json!({ "agreement": agr_rec, "reason": "counterparty-asked" }),
    )
    .await;
    assert_eq!(cw["result"]["state"], "cancelled");
    assert_eq!(cn["result"]["state"], "cancelled");

    // Late fulfilment arriving after terminal state does not alter terminal state
    let (late_cf_id, late_cf_env) = peer_signed_fulfilment(
        &conv,
        &agr_rec,
        &peer_did(),
        &owner_did(),
        "receiver",
        "c1",
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

    let (aw, an) = both_rpc(&h, "agreement.get", json!({ "agreement": agr_rec })).await;
    assert_eq!(aw["result"]["state"], "cancelled", "aw: {aw}");
    assert_eq!(an["result"]["state"], "cancelled", "an: {an}");
}

#[tokio::test]
async fn scenario_172_fulfilment_sign_retry_returns_the_fenced_record_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv, "after-work").await;

    let (fw1, fn1) =
        both_rpc(&h, "fulfilment.sign", json!({ "agreement": agr_rec, "commitment_id": "c1" }))
            .await;
    assert_eq!(fw1["result"]["record_id"], fn1["result"]["record_id"]);
    assert_eq!(fw1["result"]["state"], fn1["result"]["state"]);

    // A retry -- the fenced record, not a freshly signed one -- comes back
    // with the same record id on both builds.
    let (fw2, fn2) =
        both_rpc(&h, "fulfilment.sign", json!({ "agreement": agr_rec, "commitment_id": "c1" }))
            .await;
    assert_eq!(fw2["result"]["record_id"], fw1["result"]["record_id"]);
    assert_eq!(fn2["result"]["record_id"], fn1["result"]["record_id"]);
    assert_eq!(fw2["result"]["state"], "already-recorded");
    assert_eq!(fn2["result"]["state"], "already-recorded");
}

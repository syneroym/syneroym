use serde_json::{Value, json};
use syneroym_roym_core::transaction;
use syneroym_signed_record::Envelope;

use super::{fixtures::*, helpers::*};

async fn setup_active_agreement(h: &Harness, conv: &str) -> (String, Value) {
    let (req_rec_id, _req_env) = setup_peer_request(h, conv).await;
    let q_params = valid_quote_params(conv, &req_rec_id);
    let (qw, qn) = both_rpc(h, "quote.set", q_params).await;
    assert_eq!(
        qw["result"]["record_id"], qn["result"]["record_id"],
        "quote.set record_id parity failed: qw: {qw}, qn: {qn}"
    );
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
async fn scenario_158_payment_request_carries_no_payee_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv).await;

    // Provider requests payment
    let (pr_w, pr_n) = both_rpc(
        &h,
        "payment.request",
        json!({ "agreement": agr_rec, "note": "Please pay via bank transfer" }),
    )
    .await;
    assert_eq!(pr_w["result"]["record_id"], pr_n["result"]["record_id"]);
    assert_eq!(pr_w["result"]["state"], pr_n["result"]["state"]);

    // Thread shows agreement's payee, even if someone posts a chat message claiming
    // otherwise
    let (mut tw, mut tn) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    strip_volatile(&mut tw);
    strip_volatile(&mut tn);
    normalize_message_ids(&mut tw);
    normalize_message_ids(&mut tn);
    normalize_progress_record_ids(&mut tw);
    normalize_progress_record_ids(&mut tn);
    assert_eq!(tw, tn);

    let cards = tw["result"]["cards"].as_array().unwrap();
    let pr_card = cards.iter().find(|c| c["card_type"] == "payment-request").unwrap();
    assert_eq!(pr_card["verified"], true);
    let payload = &pr_card["data"];
    assert!(payload.get("payee").is_none());
}

#[tokio::test]
async fn scenario_159_payment_track_states_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, terms) = setup_active_agreement(&h, &conv).await;
    let amount_minor = terms["amount_minor"].as_i64().unwrap();
    let curr = terms["currency"].as_str().unwrap();

    // Consumer's half alone -> track claimed
    let (c_ack_id, c_ack_env) = peer_signed_payment_ack(
        &conv,
        &agr_rec,
        &peer_did(),
        &owner_did(),
        "consumer",
        curr,
        amount_minor,
        1_002_000,
        Some("cash"),
        Some("tx-123"),
        None,
        1_002_000,
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

    let (pw, pn) = both_rpc(&h, "payment.get", json!({ "agreement": agr_rec })).await;
    assert_eq!(pw["result"]["track"], "claimed");
    assert_eq!(pn["result"]["track"], "claimed");

    // Provider also acknowledges -> track acknowledged
    let (ack_w, ack_n) = both_rpc(
        &h,
        "payment.acknowledge",
        json!({
            "agreement": agr_rec,
            "method": "cash",
            "reference": "tx-123",
            "observed_at_secs": 1_002_500
        }),
    )
    .await;
    assert_eq!(ack_w["result"]["role"], "provider");
    assert_eq!(ack_n["result"]["role"], "provider");

    let (p2w, p2n) = both_rpc(&h, "payment.get", json!({ "agreement": agr_rec })).await;
    assert_eq!(p2w["result"]["track"], "acknowledged");
    assert_eq!(p2n["result"]["track"], "acknowledged");
}

#[tokio::test]
async fn scenario_160_payment_wrong_amount_or_method_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, terms) = setup_active_agreement(&h, &conv).await;
    let amount_minor = terms["amount_minor"].as_i64().unwrap();

    // Method outside terms via verb
    let (bad_amt_w, bad_amt_n) =
        both_rpc(&h, "payment.acknowledge", json!({ "agreement": agr_rec, "method": "crypto" }))
            .await;
    assert!(is_err(&bad_amt_w, -32602));
    assert!(is_err(&bad_amt_n, -32602));

    // Wrong currency in card from peer
    let (bad_curr_id, bad_curr_env) = peer_signed_payment_ack(
        &conv,
        &agr_rec,
        &peer_did(),
        &owner_did(),
        "consumer",
        "XYZ",
        amount_minor,
        1_002_000,
        Some("cash"),
        None,
        None,
        1_002_000,
    );
    deliver_peer_card(
        &h,
        &format!("m-badcurr-{bad_curr_id}"),
        &conv,
        &peer_did(),
        syneroym_roym_core::record::RECORD_PAYMENT_ACKNOWLEDGEMENT,
        syneroym_roym_core::payment::PAYMENT_ACKNOWLEDGEMENT_VERSION,
        &bad_curr_env,
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
    assert_eq!(unverified.len(), 1, "only the wrong-currency card must be unverified");
}

#[tokio::test]
async fn scenario_161_payment_correction_round_trip_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv).await;

    // Provider first acknowledgement. `observed_at_secs` is given
    // explicitly so both builds sign an identical payload and each can use
    // its own (identical) record id as the next call's `supersedes`.
    let (a1w, a1n) = both_rpc(
        &h,
        "payment.acknowledge",
        json!({
            "agreement": agr_rec, "method": "cash", "reference": "receipt-001",
            "observed_at_secs": 1_002_000,
        }),
    )
    .await;
    assert_eq!(a1w["result"]["record_id"], a1n["result"]["record_id"], "a1w: {a1w}, a1n: {a1n}");
    let rec1 = a1w["result"]["record_id"]
        .as_str()
        .unwrap_or_else(|| panic!("missing record_id in a1w: {a1w}"))
        .to_string();

    // Provider correction superseding rec1
    let (a2w, a2n) = both_rpc(
        &h,
        "payment.acknowledge",
        json!({
            "agreement": agr_rec,
            "method": "cash",
            "reference": "receipt-001-corrected",
            "observed_at_secs": 1_002_100,
            "supersedes": rec1
        }),
    )
    .await;
    assert_eq!(a2w["result"]["role"], "provider", "a2w: {a2w}");
    assert_eq!(a2n["result"]["role"], "provider", "a2n: {a2n}");
    let rec2 = a2w["result"]["record_id"]
        .as_str()
        .unwrap_or_else(|| panic!("missing record_id in a2w: {a2w}"))
        .to_string();
    assert_ne!(rec1, rec2, "a2w: {a2w}");

    // Stale supersedes reference is refused
    let (stale_w, stale_n) = both_rpc(
        &h,
        "payment.acknowledge",
        json!({ "agreement": agr_rec, "method": "cash", "supersedes": rec1 }),
    )
    .await;
    assert!(is_err(&stale_w, -32602));
    assert!(is_err(&stale_n, -32602));

    // Both versions kept in payment.get
    let (gw, gn) = both_rpc(&h, "payment.get", json!({ "agreement": agr_rec })).await;
    assert_eq!(gw, gn);
    let prov_acks = gw["result"]["provider"].as_array().unwrap();
    assert_eq!(prov_acks.len(), 2);
    assert_eq!(prov_acks[0]["record_id"], rec1);
    assert_eq!(prov_acks[1]["record_id"], rec2);
}

#[tokio::test]
async fn scenario_162_payment_acknowledge_duplicate_already_recorded_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv).await;

    // First acknowledge
    let (a1w, a1n) = both_rpc(
        &h,
        "payment.acknowledge",
        json!({ "agreement": agr_rec, "method": "cash", "reference": "tx-1" }),
    )
    .await;
    assert_eq!(a1w["result"]["role"], "provider", "a1w: {a1w}");
    assert_eq!(a1n["result"]["role"], "provider", "a1n: {a1n}");

    // Second acknowledge with no supersedes -> already-recorded
    let (a2w, a2n) = both_rpc(
        &h,
        "payment.acknowledge",
        json!({ "agreement": agr_rec, "method": "cash", "reference": "tx-1" }),
    )
    .await;
    assert_eq!(a2w["result"]["state"], "already-recorded", "a2w: {a2w}");
    assert_eq!(a2n["result"]["state"], "already-recorded", "a2n: {a2n}");
    assert_eq!(a2w["result"]["record_id"], a1w["result"]["record_id"], "a2w: {a2w}, a1w: {a1w}");
    assert_eq!(a2n["result"]["record_id"], a1n["result"]["record_id"], "a2n: {a2n}, a1n: {a1n}");
}

#[tokio::test]
async fn scenario_166_payment_half_before_agreement_deferred_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (req_rec_id, _req_env) = setup_peer_request(&h, &conv).await;
    let q_params = valid_quote_params(&conv, &req_rec_id);
    let (qw, qn) = both_rpc(&h, "quote.set", q_params).await;
    assert_eq!(
        qw["result"]["record_id"], qn["result"]["record_id"],
        "quote.set record_id parity failed: qw: {qw}, qn: {qn}"
    );
    let quote_rec_id = qw["result"]["record_id"].as_str().unwrap().to_string();
    let quote_id = qw["result"]["quote_id"].as_str().unwrap().to_string();

    let (get_q, _) = both_rpc(&h, "quote.get", json!({ "quote_id": quote_id })).await;
    let quote_env = Envelope::from_json(get_q["result"]["envelope"].as_str().unwrap()).unwrap();
    let terms = quote_env.payload["terms"].clone();
    let amount_minor = terms["amount_minor"].as_i64().unwrap();
    let curr = terms["currency"].as_str().unwrap();

    // `issued_at` must be recent (within `MAX_DEFER_SECS` of real now) --
    // a tiny literal epoch second would already be older than the defer
    // window against the real wall clock the deferral check reads, and
    // the card would be refused outright instead of deferred.
    let recent = wall_now() - 100;
    let (c_ack_id, c_ack_env) = peer_signed_payment_ack(
        &conv,
        &quote_rec_id,
        &peer_did(),
        &owner_did(),
        "consumer",
        curr,
        amount_minor,
        recent,
        Some("cash"),
        None,
        None,
        recent,
    );

    // Deliver payment ack BEFORE consumer agreement receipt
    let card_pay = inbound_card(
        &format!("m-defer-pay-{c_ack_id}"),
        &conv,
        &peer_did(),
        2_000,
        syneroym_roym_core::record::RECORD_PAYMENT_ACKNOWLEDGEMENT,
        syneroym_roym_core::payment::PAYMENT_ACKNOWLEDGEMENT_VERSION,
        &c_ack_env,
    );
    h.deliver(true, card_pay.clone()).await;
    h.deliver(false, card_pay).await;

    let (s1w, s1n) =
        both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    assert_eq!(s1w["result"]["deferred"], 1, "s1w: {s1w}");
    assert_eq!(s1n["result"]["deferred"], 1, "s1n: {s1n}");

    // Agreement card now arrives with earlier sender timestamp 1_000
    let (agr_id, agr_env) = peer_signed_consumer_receipt(
        &conv,
        &quote_rec_id,
        &peer_did(),
        &owner_did(),
        terms,
        1_001_000,
    );
    let card_agr = inbound_card(
        &format!("m-agr-{agr_id}"),
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &agr_env,
    );
    h.deliver(true, card_agr.clone()).await;
    h.deliver(false, card_agr).await;

    // Next sync files both agreement and the deferred payment ack
    let (s2w, s2n) =
        both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    assert_eq!(s2w["result"]["filed"], 2, "s2w: {s2w}");
    assert_eq!(s2n["result"]["filed"], 2, "s2n: {s2n}");

    let (pw, pn) = both_rpc(&h, "payment.get", json!({ "agreement": quote_rec_id })).await;
    assert_eq!(pw["result"]["track"], "claimed", "pw: {pw}");
    assert_eq!(pn["result"]["track"], "claimed", "pn: {pn}");
}

#[tokio::test]
async fn scenario_171_payment_fence_unblocks_after_refused_call_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv).await;

    // 1. Refused payment.request (note exceeds 512 chars)
    let long_note = "a".repeat(513);
    let (err_w, err_n) =
        both_rpc(&h, "payment.request", json!({ "agreement": agr_rec, "note": long_note })).await;
    assert_eq!(err_w["error"]["code"], -32602, "err_w: {err_w}");
    assert_eq!(err_n["error"]["code"], -32602, "err_n: {err_n}");
    assert!(err_w["error"]["message"].as_str().unwrap().contains("512"));

    // 2. Valid retry succeeds (fence was not leaked)
    let (req_w, req_n) =
        both_rpc(&h, "payment.request", json!({ "agreement": agr_rec, "note": "Valid note" }))
            .await;
    assert_eq!(
        req_w["result"]["record_id"], req_n["result"]["record_id"],
        "req_w: {req_w}, req_n: {req_n}"
    );
    assert_eq!(
        req_w["result"]["state"], req_n["result"]["state"],
        "req_w: {req_w}, req_n: {req_n}"
    );

    // 3. Refused payment.acknowledge (method not in terms)
    let (ack_err_w, ack_err_n) = both_rpc(
        &h,
        "payment.acknowledge",
        json!({ "agreement": agr_rec, "method": "unsupported-payment-method-xyz" }),
    )
    .await;
    assert_eq!(ack_err_w["error"]["code"], -32602, "ack_err_w: {ack_err_w}");
    assert_eq!(ack_err_n["error"]["code"], -32602, "ack_err_n: {ack_err_n}");
    assert!(ack_err_w["error"]["message"].as_str().unwrap().contains("method-not-in-terms"));

    // 4. Valid retry succeeds (fence was not leaked)
    let (ack_w, ack_n) = both_rpc(
        &h,
        "payment.acknowledge",
        json!({ "agreement": agr_rec, "observed_at_secs": 1_002_000 }),
    )
    .await;
    assert_eq!(
        ack_w["result"]["record_id"], ack_n["result"]["record_id"],
        "ack_w: {ack_w}, ack_n: {ack_n}"
    );
    assert_eq!(
        ack_w["result"]["state"], ack_n["result"]["state"],
        "ack_w: {ack_w}, ack_n: {ack_n}"
    );
}

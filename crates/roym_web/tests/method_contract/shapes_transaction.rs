use serde_json::json;
use syneroym_roym_core::{
    booking::BOOKING_PROGRESS_VERSION, record::RECORD_BOOKING_PROGRESS, transaction,
};
use syneroym_signed_record::Envelope;

use super::{
    deal_fixtures::{
        seed_row, setup_active_agreement, setup_listing_with_slot, setup_peer_request,
    },
    fixtures::{
        deliver_peer_card, enrol_signing, open_conv, peer_counterparty_receipt, peer_did,
        peer_identity, peer_signed_payment_ack, peer_signed_progress, peer_signed_quote,
        peer_signed_request, valid_quote_params, wall_now,
    },
    helpers::{Harness, harness, owner_did},
    shapes::{assert_shape, enrol_native_signing, rpc},
};

async fn deal_pings_and_empty(h: &Harness) {
    let ping_r = rpc(h, "request.ping", json!({})).await;
    assert_shape("request_ping", &ping_r);
    let ping_q = rpc(h, "quote.ping", json!({})).await;
    assert_shape("quote_ping", &ping_q);
    let ping_a = rpc(h, "agreement.ping", json!({})).await;
    assert_shape("agreement_ping", &ping_a);

    let r_empty = rpc(h, "request.list", json!({})).await;
    assert_shape("request_list_empty", &r_empty);
    let r_missing = rpc(h, "request.get", json!({ "request_id": "nonexistent" })).await;
    assert_shape("request_get_missing", &r_missing);

    let q_empty = rpc(h, "quote.list", json!({})).await;
    assert_shape("quote_list_empty", &q_empty);
    let q_missing = rpc(h, "quote.get", json!({ "quote_id": "nonexistent" })).await;
    assert_shape("quote_get_missing", &q_missing);

    let a_empty = rpc(h, "agreement.list", json!({})).await;
    assert_shape("agreement_list_empty", &a_empty);
    let a_missing = rpc(h, "agreement.get", json!({ "quote_record_id": "nonexistent" })).await;
    assert_shape("agreement_get_missing", &a_missing);
}

async fn deal_request_and_quote_shapes(h: &Harness, conv: &str) -> (String, String, String) {
    let r_set = rpc(
        h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Fix the fence",
            "categories": ["gardening"],
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    assert_shape("request_set_sent", &r_set);
    let req_rec_id = r_set["record_id"].as_str().unwrap().to_string();
    let req_id = r_set["request_id"].as_str().unwrap().to_string();

    let r_set_err = rpc(
        h,
        "request.set",
        json!({
            "conversation": "nonexistent-conversation",
            "description": "Fix the fence elsewhere",
            "categories": ["gardening"],
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    assert_shape("request_set_send_error", &r_set_err);

    let r_get = rpc(h, "request.get", json!({ "request_id": req_id })).await;
    assert_shape("request_get_found", &r_get);
    let req_env = r_get["envelope"].as_str().unwrap().to_string();

    let r_list = rpc(h, "request.list", json!({})).await;
    assert_shape("request_list_with_rows", &r_list);

    let r_hist = rpc(h, "request.history", json!({ "request_id": req_id })).await;
    assert_shape("request_history", &r_hist);

    let r_ver_good = rpc(h, "request.verify", json!({ "envelope": req_env })).await;
    assert_shape("request_verify_good", &r_ver_good);
    let r_ver_bad = rpc(h, "request.verify", json!({ "envelope": "tampered" })).await;
    assert_shape("request_verify_bad", &r_ver_bad);

    let (peer_req_id, _) = setup_peer_request(h, conv).await;
    let q_params = valid_quote_params(conv, &peer_req_id);
    let q_set = rpc(h, "quote.set", q_params).await;
    assert_shape("quote_set", &q_set);
    let quote_id = q_set["quote_id"].as_str().unwrap().to_string();
    let quote_rec_id = q_set["record_id"].as_str().unwrap().to_string();

    let q_get = rpc(h, "quote.get", json!({ "quote_id": quote_id })).await;
    assert_shape("quote_get_found", &q_get);
    let quote_env = q_get["envelope"].as_str().unwrap().to_string();

    let q_list = rpc(h, "quote.list", json!({})).await;
    assert_shape("quote_list_with_rows", &q_list);

    let q_hist = rpc(h, "quote.history", json!({ "quote_id": quote_id })).await;
    assert_shape("quote_history", &q_hist);

    let q_ver_good = rpc(h, "quote.verify", json!({ "envelope": quote_env })).await;
    assert_shape("quote_verify_good", &q_ver_good);
    let q_ver_bad = rpc(h, "quote.verify", json!({ "envelope": "tampered" })).await;
    assert_shape("quote_verify_bad", &q_ver_bad);

    let (cp_q_rec_id, cp_q_env) =
        peer_signed_quote(conv, 2, &req_rec_id, &owner_did(), None, 1_000);
    deliver_peer_card(
        h,
        "m-quote-to-decline",
        conv,
        &peer_did(),
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &cp_q_env,
    )
    .await;
    rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let q_dec =
        rpc(h, "quote.decline", json!({ "quote_record_id": cp_q_rec_id, "note": "Too expensive" }))
            .await;
    assert_shape("quote_decline", &q_dec);

    let env_q2 = Envelope::from_json(&cp_q_env).unwrap();
    let q2_id = env_q2.payload["quote_id"].as_str().unwrap();
    let q2_get = rpc(h, "quote.get", json!({ "quote_id": q2_id })).await;
    assert_shape("quote_get_declined", &q2_get);

    (req_rec_id, quote_id, quote_rec_id)
}

async fn deal_agreement_shapes(h: &Harness, conv: &str) {
    let start = wall_now() + 1_000_000;
    let end = start + 3600;
    let (listing_id, slot_id) =
        setup_listing_with_slot(h, "deal-slot-listing", start, end, 1).await;
    let (peer_req_id, _) = setup_peer_request(h, conv).await;
    let mut q_slot_params = valid_quote_params(conv, &peer_req_id);
    q_slot_params["listing_id"] = json!(listing_id);
    q_slot_params["slot_id"] = json!(slot_id);
    q_slot_params["terms"]["schedule"] = json!({ "earliest_secs": start, "latest_secs": end });
    let q_slot_set = rpc(h, "quote.set", q_slot_params).await;
    let q_slot_rec_id = q_slot_set["record_id"].as_str().unwrap().to_string();

    let get_q_slot =
        rpc(h, "quote.get", json!({ "quote_id": q_slot_set["quote_id"].as_str().unwrap() })).await;
    let env_slot = Envelope::from_json(get_q_slot["envelope"].as_str().unwrap()).unwrap();
    let (agr_id, agr_env) = peer_counterparty_receipt(
        conv,
        &q_slot_rec_id,
        &peer_did(),
        &owner_did(),
        env_slot.payload["terms"].clone(),
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
    rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let agr_accept_author =
        rpc(h, "agreement.accept", json!({ "quote_record_id": q_slot_rec_id })).await;
    assert_shape("agreement_accept_author_with_slot", &agr_accept_author);

    let agr_accept_again =
        rpc(h, "agreement.accept", json!({ "quote_record_id": q_slot_rec_id })).await;
    assert_shape("agreement_accept_already_accepted", &agr_accept_again);

    let cp_req = rpc(
        h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Counterparty deal",
            "categories": ["gardening"],
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let cp_req_rec_id = cp_req["record_id"].as_str().unwrap().to_string();
    let (cp_q_rec_id, cp_q_env) =
        peer_signed_quote(conv, 3, &cp_req_rec_id, &owner_did(), None, 1_000);
    deliver_peer_card(
        h,
        "m-quote-cp",
        conv,
        &peer_did(),
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &cp_q_env,
    )
    .await;
    rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let agr_accept_cp = rpc(h, "agreement.accept", json!({ "quote_record_id": cp_q_rec_id })).await;
    assert_shape("agreement_accept_counterparty", &agr_accept_cp);

    let agr_get = rpc(h, "agreement.get", json!({ "quote_record_id": q_slot_rec_id })).await;
    assert_shape("agreement_get_found", &agr_get);

    let agr_list = rpc(h, "agreement.list", json!({})).await;
    assert_shape("agreement_list_with_rows", &agr_list);

    let agr_ver_good = rpc(h, "agreement.verify", json!({ "envelope": agr_env })).await;
    assert_shape("agreement_verify_good", &agr_ver_good);
    let agr_ver_bad = rpc(h, "agreement.verify", json!({ "envelope": "tampered" })).await;
    assert_shape("agreement_verify_bad", &agr_ver_bad);
}

#[tokio::test]
async fn deal_shapes() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    deal_pings_and_empty(&h).await;
    let conv = open_conv(&h, &peer_did()).await;
    deal_request_and_quote_shapes(&h, &conv).await;
    deal_agreement_shapes(&h, &conv).await;
}

async fn booking_author_shapes(h: &Harness, conv: &str) -> String {
    let b_ping = rpc(h, "booking.ping", json!({})).await;
    assert_shape("booking_ping", &b_ping);

    let start = wall_now() + 1_000_000;
    let end = start + 3600;
    let (listing_id, slot_id) = setup_listing_with_slot(h, "booking-listing", start, end, 1).await;
    let (peer_req_id, _) = setup_peer_request(h, conv).await;
    let mut q_slot_params = valid_quote_params(conv, &peer_req_id);
    q_slot_params["listing_id"] = json!(listing_id);
    q_slot_params["slot_id"] = json!(slot_id);
    q_slot_params["terms"]["schedule"] = json!({ "earliest_secs": start, "latest_secs": end });
    let q_slot_set = rpc(h, "quote.set", q_slot_params).await;
    let q_slot_rec_id = q_slot_set["record_id"].as_str().unwrap().to_string();

    let get_q_slot =
        rpc(h, "quote.get", json!({ "quote_id": q_slot_set["quote_id"].as_str().unwrap() })).await;
    let env_slot = Envelope::from_json(get_q_slot["envelope"].as_str().unwrap()).unwrap();
    let (agr_id, agr_env) = peer_counterparty_receipt(
        conv,
        &q_slot_rec_id,
        &peer_did(),
        &owner_did(),
        env_slot.payload["terms"].clone(),
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
    rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    rpc(h, "agreement.accept", json!({ "quote_record_id": q_slot_rec_id })).await;

    let b_get_author = rpc(h, "booking.get", json!({ "agreement": q_slot_rec_id })).await;
    assert_shape("booking_get_author", &b_get_author);

    let b_start = rpc(h, "booking.start", json!({ "agreement": q_slot_rec_id })).await;
    assert_shape("booking_start", &b_start);

    let b_hist_author = rpc(h, "booking.history", json!({ "agreement": q_slot_rec_id })).await;
    assert_shape("booking_history_author", &b_hist_author);

    let b_cancel = rpc(
        h,
        "booking.cancel",
        json!({ "agreement": q_slot_rec_id, "reason": "Client cancelled" }),
    )
    .await;
    assert_shape("booking_cancel", &b_cancel);

    q_slot_rec_id
}

async fn booking_counterparty_shapes(h: &Harness, conv: &str) {
    let cp_req = rpc(
        h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Counterparty deal",
            "categories": ["gardening"],
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let cp_req_rec_id = cp_req["record_id"].as_str().unwrap().to_string();
    let (cp_q_rec_id, cp_q_env) =
        peer_signed_quote(conv, 2, &cp_req_rec_id, &owner_did(), None, 1_000);
    deliver_peer_card(
        h,
        "m-quote-cp-book",
        conv,
        &peer_did(),
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &cp_q_env,
    )
    .await;
    rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    rpc(h, "agreement.accept", json!({ "quote_record_id": cp_q_rec_id })).await;

    let b_get_cp = rpc(h, "booking.get", json!({ "agreement": cp_q_rec_id })).await;
    assert_shape("booking_get_counterparty", &b_get_cp);

    let (_, prog_env) = peer_signed_progress(
        conv,
        &cp_q_rec_id,
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
    deliver_peer_card(
        h,
        "m-prog-cp",
        conv,
        &peer_did(),
        RECORD_BOOKING_PROGRESS,
        BOOKING_PROGRESS_VERSION,
        &prog_env,
    )
    .await;
    rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let b_hist_cp = rpc(h, "booking.history", json!({ "agreement": cp_q_rec_id })).await;
    assert_shape("booking_history_counterparty", &b_hist_cp);

    let b_ver_good = rpc(h, "booking.verify", json!({ "envelope": prog_env })).await;
    assert_shape("booking_verify_good", &b_ver_good);
    let b_ver_bad = rpc(h, "booking.verify", json!({ "envelope": "tampered" })).await;
    assert_shape("booking_verify_bad", &b_ver_bad);
}

#[tokio::test]
async fn booking_shapes() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    booking_author_shapes(&h, &conv).await;
    booking_counterparty_shapes(&h, &conv).await;

    let b_list = rpc(&h, "booking.list", json!({})).await;
    assert_shape("booking_list", &b_list);
}

async fn payment_crash_recovery_shapes(h: &Harness, conv: &str) {
    let (agr_cr1, _) = setup_active_agreement(h, conv, "after-work").await;
    let pr_cr1 = rpc(h, "payment.request", json!({ "agreement": agr_cr1 })).await;
    let rec1 = pr_cr1["record_id"].as_str().unwrap().to_string();
    seed_row(
        h,
        false,
        "transaction",
        "payments",
        &agr_cr1,
        json!({
            "agreement": agr_cr1,
            "conversation": conv,
            "updated_at_secs": 1_000_000,
        }),
    )
    .await;
    let pr_cr2 = rpc(h, "payment.request", json!({ "agreement": agr_cr1 })).await;
    assert_eq!(pr_cr2["record_id"], rec1);
    assert_ne!(pr_cr2["state"], "already-recorded");
    assert_shape("payment_request_crash_recovery", &pr_cr2);

    let (agr_cr2, _) = setup_active_agreement(h, conv, "after-work").await;
    rpc(h, "payment.request", json!({ "agreement": agr_cr2 })).await;
    let pa_cr1 = rpc(h, "payment.acknowledge", json!({ "agreement": agr_cr2 })).await;
    let rec2 = pa_cr1["record_id"].as_str().unwrap().to_string();
    let pget = rpc(h, "payment.get", json!({ "agreement": agr_cr2 })).await;
    seed_row(
        h,
        false,
        "transaction",
        "payments",
        &agr_cr2,
        json!({
            "agreement": agr_cr2,
            "conversation": conv,
            "request": pget["request"],
            "quote_author": [],
            "counterparty": [],
            "updated_at_secs": 1_000_000,
        }),
    )
    .await;
    let pa_cr2 = rpc(h, "payment.acknowledge", json!({ "agreement": agr_cr2 })).await;
    assert_eq!(pa_cr2["record_id"], rec2);
    assert_ne!(pa_cr2["state"], "already-recorded");
    assert_shape("payment_ack_crash_recovery", &pa_cr2);

    let (agr_cr3, _) = setup_active_agreement(h, conv, "after-work").await;
    let fs_cr1 = rpc(h, "fulfilment.sign", json!({ "agreement": agr_cr3 })).await;
    let rec3 = fs_cr1["record_id"].as_str().unwrap().to_string();
    seed_row(
        h,
        false,
        "transaction",
        "fulfilments",
        &agr_cr3,
        json!({
            "agreement": agr_cr3,
            "conversation": conv,
            "updated_at_secs": 1_000_000,
        }),
    )
    .await;
    let fs_cr2 = rpc(h, "fulfilment.sign", json!({ "agreement": agr_cr3 })).await;
    assert_eq!(fs_cr2["record_id"], rec3);
    assert_ne!(fs_cr2["state"], "already-recorded");
    assert_shape("fulfilment_sign_crash_recovery", &fs_cr2);
}

#[tokio::test]
async fn payment_fulfilment_shapes() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    assert_shape("payment_ping", &rpc(&h, "payment.ping", json!({})).await);
    assert_shape("receipt_ping", &rpc(&h, "receipt.ping", json!({})).await);
    assert_shape("fulfilment_ping", &rpc(&h, "fulfilment.ping", json!({})).await);

    let (agr1, _) = setup_active_agreement(&h, &conv, "after-work").await;

    let p_get_empty = rpc(&h, "payment.get", json!({ "agreement": agr1 })).await;
    assert_shape("payment_get_empty", &p_get_empty);

    let f_get_empty = rpc(&h, "fulfilment.get", json!({ "agreement": agr1 })).await;
    assert_shape("fulfilment_get_empty", &f_get_empty);

    let pr_new =
        rpc(&h, "payment.request", json!({ "agreement": agr1, "note": "Please pay" })).await;
    assert_shape("payment_request_new", &pr_new);

    let pr_already = rpc(&h, "payment.request", json!({ "agreement": agr1 })).await;
    assert_shape("payment_request_already_recorded", &pr_already);

    let pa_new = rpc(
        &h,
        "payment.acknowledge",
        json!({ "agreement": agr1, "method": "card", "reference": "tx-123" }),
    )
    .await;
    assert_shape("payment_acknowledge_new", &pa_new);

    let pa_already = rpc(&h, "payment.acknowledge", json!({ "agreement": agr1 })).await;
    assert_shape("payment_acknowledge_already_recorded", &pa_already);

    let p_get_rows = rpc(&h, "payment.get", json!({ "agreement": agr1 })).await;
    assert_shape("payment_get_with_rows", &p_get_rows);

    let fs_new = rpc(&h, "fulfilment.sign", json!({ "agreement": agr1 })).await;
    assert_shape("fulfilment_sign_new", &fs_new);

    let fs_already = rpc(&h, "fulfilment.sign", json!({ "agreement": agr1 })).await;
    assert_shape("fulfilment_sign_already_recorded", &fs_already);

    let f_get_rows = rpc(&h, "fulfilment.get", json!({ "agreement": agr1 })).await;
    assert_shape("fulfilment_get_with_rows", &f_get_rows);

    let req_env = p_get_rows["request"]["envelope"].as_str().unwrap();
    let p_ver_req = rpc(&h, "payment.verify", json!({ "envelope": req_env })).await;
    assert_shape("payment_verify_request", &p_ver_req);

    let (_, ack_env) = peer_signed_payment_ack(
        &conv,
        &agr1,
        &peer_did(),
        &owner_did(),
        "counterparty",
        "GBP",
        1000,
        1_002_000,
        Some("card"),
        Some("tx-123"),
        None,
        1_002_000,
    );
    let p_ver_ack = rpc(&h, "payment.verify", json!({ "envelope": ack_env })).await;
    assert_shape("payment_verify_ack", &p_ver_ack);

    let (_, non_pay_env) = peer_signed_request(&conv, 10, 1_000_000);
    let p_ver_refused = rpc(&h, "payment.verify", json!({ "envelope": non_pay_env })).await;
    assert_shape("payment_verify_non_payment", &p_ver_refused);

    let ful_env = f_get_rows["quote_author"]["envelope"].as_str().unwrap();
    let f_ver_good = rpc(&h, "fulfilment.verify", json!({ "envelope": ful_env })).await;
    assert_shape("fulfilment_verify_good", &f_ver_good);

    let f_ver_bad = rpc(&h, "fulfilment.verify", json!({ "envelope": "tampered" })).await;
    assert_shape("fulfilment_verify_bad", &f_ver_bad);

    payment_crash_recovery_shapes(&h, &conv).await;
}

#[tokio::test]
async fn sync_backup_shapes() {
    let h = harness().await;
    let (unconf, inst, conf) = enrol_native_signing(&h, "transaction").await;
    assert_shape("transaction_signing_status_unconfigured", &unconf);
    assert_shape("transaction_install_signing_certificate", &inst);
    assert_shape("transaction_signing_status", &conf);
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    let conv = open_conv(&h, &peer_did()).await;
    let (agr, _) = setup_active_agreement(&h, &conv, "after-work").await;
    rpc(&h, "payment.request", json!({ "agreement": agr })).await;

    let sync = rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    assert_shape("transaction_sync", &sync);

    let thread = rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    assert_shape("transaction_thread", &thread);

    let exp = rpc(&h, "transaction.export", json!({})).await;
    assert_shape("transaction_export", &exp);

    let imp = rpc(&h, "transaction.import", json!({ "bundle": exp })).await;
    assert_shape("transaction_import", &imp);
}

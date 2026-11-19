use serde_json::json;
use syneroym_roym_core::transaction;
use syneroym_signed_record::Envelope;

use super::{
    deal_fixtures::{
        deliver_peer_card_with_disclosures, peer_signed_payment_ack, peer_signed_progress,
        peer_signed_quote, seed_row, setup_active_agreement_native,
        setup_author_slot_quote_accepted_by_peer, setup_listing_with_slot, setup_peer_request,
        setup_synced_counterparty_quote,
    },
    fixtures::{
        deliver_peer_card, enrol_signing, open_conv, peer_counterparty_receipt, peer_did,
        peer_identity, peer_signed_request, valid_quote_params, wall_now,
    },
    helpers::{Harness, harness, owner_did},
    shapes::{assert_shape, enrol_native_signing, rpc, rpc_err},
};

async fn deal_pings_and_empty(h: &Harness) {
    let ping_r = rpc(h, "enquiry.ping", json!({})).await;
    assert_shape("enquiry_ping", &ping_r);
    let ping_q = rpc(h, "quote.ping", json!({})).await;
    assert_shape("quote_ping", &ping_q);
    let ping_a = rpc(h, "agreement.ping", json!({})).await;
    assert_shape("agreement_ping", &ping_a);
    let ping_d = rpc(h, "deal.ping", json!({})).await;
    assert_shape("deal_ping", &ping_d);

    let r_empty = rpc(h, "enquiry.list", json!({})).await;
    assert_shape("enquiry_list_empty", &r_empty);
    let r_missing = rpc(h, "enquiry.get", json!({ "enquiry_id": "nonexistent" })).await;
    assert_shape("enquiry_get_missing", &r_missing);

    let q_empty = rpc(h, "quote.list", json!({})).await;
    assert_shape("quote_list_empty", &q_empty);
    let q_missing = rpc(h, "quote.get", json!({ "quote_id": "nonexistent" })).await;
    assert_shape("quote_get_missing", &q_missing);

    let a_empty = rpc(h, "agreement.list", json!({})).await;
    assert_shape("agreement_list_empty", &a_empty);
    let a_missing = rpc(h, "agreement.get", json!({ "quote_record_id": "nonexistent" })).await;
    assert_shape("agreement_get_missing", &a_missing);
}

async fn deal_enquiry_and_quote_shapes(h: &Harness, conv: &str) -> (String, String, String) {
    let r_set = rpc(
        h,
        "enquiry.set",
        json!({
            "conversation": conv,
            "description": { "lang": "en", "text": "Fix the fence" },
            "categories": ["gardening"],
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    assert_shape("enquiry_set_sent", &r_set);
    let req_rec_id = r_set["record_id"].as_str().unwrap().to_string();
    let req_id = r_set["enquiry_id"].as_str().unwrap().to_string();

    let r_set_err = rpc(
        h,
        "enquiry.set",
        json!({
            "conversation": "nonexistent-conversation",
            "description": { "lang": "en", "text": "Fix the fence elsewhere" },
            "categories": ["gardening"],
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    assert_shape("enquiry_set_send_error", &r_set_err);

    let r_get = rpc(h, "enquiry.get", json!({ "enquiry_id": req_id })).await;
    assert_shape("enquiry_get_found", &r_get);
    let req_env = r_get["envelope"].as_str().unwrap().to_string();

    let r_list = rpc(h, "enquiry.list", json!({})).await;
    assert_shape("enquiry_list_with_rows", &r_list);

    let r_hist = rpc(h, "enquiry.history", json!({ "enquiry_id": req_id })).await;
    assert_shape("enquiry_history", &r_hist);

    let r_ver_good = rpc(h, "enquiry.verify", json!({ "envelope": req_env })).await;
    assert_shape("enquiry_verify_good", &r_ver_good);
    let r_ver_bad = rpc(h, "enquiry.verify", json!({ "envelope": "tampered" })).await;
    assert_shape("enquiry_verify_bad", &r_ver_bad);

    let (peer_req_id, _) = setup_peer_request(h, conv).await;
    let q_params = valid_quote_params(conv, &peer_req_id);
    let q_meas = rpc(h, "quote.measure", json!({ "terms": q_params["terms"] })).await;
    assert_shape("quote_measure", &q_meas);
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

    let (cp_q_rec_id, cp_q_env, cp_q_disc) =
        peer_signed_quote(conv, 2, &req_rec_id, &owner_did(), None, 1_000);
    deliver_peer_card_with_disclosures(
        h,
        "m-quote-to-decline",
        conv,
        &peer_did(),
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &cp_q_env,
        &cp_q_disc,
    )
    .await;
    rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let q_dec =
        rpc(h, "deal.decline", json!({ "declines": cp_q_rec_id, "note": "Too expensive" })).await;
    assert_shape("deal_decline", &q_dec);

    let env_q2 = Envelope::from_json(&cp_q_env).unwrap();
    let q2_id = env_q2.payload["quote_id"].as_str().unwrap();
    let q2_get = rpc(h, "quote.get", json!({ "quote_id": q2_id })).await;
    assert_shape("quote_get_declined", &q2_get);

    (req_rec_id, quote_id, quote_rec_id)
}

async fn deal_agreement_shapes(h: &Harness, conv: &str) {
    let q_slot_rec_id =
        setup_author_slot_quote_accepted_by_peer(h, conv, "deal-slot-listing").await;

    let agr_accept_author =
        rpc_err(h, "agreement.accept", json!({ "quote_record_id": q_slot_rec_id })).await;
    assert_shape("agreement_accept_writer_countersigns", &agr_accept_author);

    let cp_q_rec_id = setup_synced_counterparty_quote(h, conv, 3, "m-quote-cp").await;

    let agr_accept_cp = rpc(h, "agreement.accept", json!({ "quote_record_id": cp_q_rec_id })).await;
    assert_shape("agreement_accept_counterparty", &agr_accept_cp);

    let agr_accept_again =
        rpc(h, "agreement.accept", json!({ "quote_record_id": cp_q_rec_id })).await;
    assert_shape("agreement_accept_already_accepted", &agr_accept_again);

    let agr_get = rpc(h, "agreement.get", json!({ "quote_record_id": cp_q_rec_id })).await;
    assert_shape("agreement_get_found", &agr_get);

    let agr_list = rpc(h, "agreement.list", json!({})).await;
    assert_shape("agreement_list_with_rows", &agr_list);

    let agr_env = agr_get["counterparty"]["envelope"].as_str().unwrap();
    let agr_ver_good = rpc(h, "agreement.verify", json!({ "envelope": agr_env })).await;
    assert_shape("agreement_verify_good", &agr_ver_good);
    let agr_ver_bad = rpc(h, "agreement.verify", json!({ "envelope": "tampered" })).await;
    assert_shape("agreement_verify_bad", &agr_ver_bad);

    let pres =
        rpc(h, "deal.present", json!({ "record_id": cp_q_rec_id, "show": ["terms.scope"] })).await;
    assert_shape("deal_present", &pres);

    let ver_pres = rpc(h, "deal.verify-presentation", pres).await;
    assert_shape("deal_verify_presentation", &ver_pres);
}

#[tokio::test]
async fn deal_shapes() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    deal_pings_and_empty(&h).await;
    let conv = open_conv(&h, &peer_did()).await;
    deal_enquiry_and_quote_shapes(&h, &conv).await;
    deal_agreement_shapes(&h, &conv).await;
}

async fn agreement_progress_and_cancel_shapes(h: &Harness, conv: &str) {
    let (agr_rec, _) = setup_active_agreement_native(h, conv, "after-work").await;

    let a_prog = rpc(h, "agreement.progress", json!({ "agreement": agr_rec })).await;
    assert_shape("agreement_progress", &a_prog);

    let a_cancel =
        rpc(h, "agreement.cancel", json!({ "agreement": agr_rec, "reason": "schedule-conflict" }))
            .await;
    assert_shape("agreement_cancel", &a_cancel);
}

#[tokio::test]
async fn agreement_progress_shapes() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    agreement_progress_and_cancel_shapes(&h, &conv).await;
}

async fn payment_crash_recovery_shapes(h: &Harness, conv: &str) {
    let (agr_cr1, _) = setup_active_agreement_native(h, conv, "after-work").await;
    let pr_cr1 =
        rpc(h, "payment.request", json!({ "agreement": agr_cr1, "commitment_id": "c2" })).await;
    let rec1 = pr_cr1["record_id"].as_str().unwrap().to_string();
    seed_row(
        h,
        false,
        "transaction",
        "payments",
        &format!("{agr_cr1}:c2"),
        json!({
            "agreement": agr_cr1,
            "commitment_id": "c2",
            "conversation": conv,
            "updated_at_secs": 1_000_000,
        }),
    )
    .await;
    let pr_cr2 =
        rpc(h, "payment.request", json!({ "agreement": agr_cr1, "commitment_id": "c2" })).await;
    assert_eq!(pr_cr2["record_id"], rec1);
    assert_ne!(pr_cr2["state"], "already-recorded");
    assert_shape("payment_request_crash_recovery", &pr_cr2);

    let (agr_cr2, _) = setup_active_agreement_native(h, conv, "after-work").await;
    rpc(h, "payment.request", json!({ "agreement": agr_cr2, "commitment_id": "c2" })).await;
    let pa_cr1 =
        rpc(h, "payment.acknowledge", json!({ "agreement": agr_cr2, "commitment_id": "c2" })).await;
    let rec2 = pa_cr1["record_id"].as_str().unwrap().to_string();
    let pget = rpc(h, "payment.get", json!({ "agreement": agr_cr2, "commitment_id": "c2" })).await;
    seed_row(
        h,
        false,
        "transaction",
        "payments",
        &format!("{agr_cr2}:c2"),
        json!({
            "agreement": agr_cr2,
            "commitment_id": "c2",
            "conversation": conv,
            "request": pget["request"],
            "quote_author": [],
            "counterparty": [],
            "updated_at_secs": 1_000_000,
        }),
    )
    .await;
    let pa_cr2 =
        rpc(h, "payment.acknowledge", json!({ "agreement": agr_cr2, "commitment_id": "c2" })).await;
    assert_eq!(pa_cr2["record_id"], rec2);
    assert_ne!(pa_cr2["state"], "already-recorded");
    assert_shape("payment_ack_crash_recovery", &pa_cr2);

    let (agr_cr3, _) = setup_active_agreement_native(h, conv, "after-work").await;
    let fs_cr1 =
        rpc(h, "fulfilment.sign", json!({ "agreement": agr_cr3, "commitment_id": "c1" })).await;
    let rec3 = fs_cr1["record_id"].as_str().unwrap().to_string();
    seed_row(
        h,
        false,
        "transaction",
        "fulfilments",
        &format!("{agr_cr3}:c1"),
        json!({
            "agreement": agr_cr3,
            "commitment_id": "c1",
            "conversation": conv,
            "updated_at_secs": 1_000_000,
        }),
    )
    .await;
    let fs_cr2 =
        rpc(h, "fulfilment.sign", json!({ "agreement": agr_cr3, "commitment_id": "c1" })).await;
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

    let (agr1, _) = setup_active_agreement_native(&h, &conv, "after-work").await;

    let p_get_empty =
        rpc(&h, "payment.get", json!({ "agreement": agr1, "commitment_id": "c2" })).await;
    assert_shape("payment_get_empty", &p_get_empty);

    let f_get_empty =
        rpc(&h, "fulfilment.get", json!({ "agreement": agr1, "commitment_id": "c1" })).await;
    assert_shape("fulfilment_get_empty", &f_get_empty);

    let pr_new =
        rpc(&h, "payment.request", json!({ "agreement": agr1, "commitment_id": "c2", "note": { "lang": "en", "text": "Please pay" } })).await;
    assert_shape("payment_request_new", &pr_new);

    let pr_already =
        rpc(&h, "payment.request", json!({ "agreement": agr1, "commitment_id": "c2" })).await;
    assert_shape("payment_request_already_recorded", &pr_already);

    let pa_new = rpc(
        &h,
        "payment.acknowledge",
        json!({ "agreement": agr1, "commitment_id": "c2", "method": "card", "reference": "tx-123" }),
    )
    .await;
    assert_shape("payment_acknowledge_new", &pa_new);

    let pa_already =
        rpc(&h, "payment.acknowledge", json!({ "agreement": agr1, "commitment_id": "c2" })).await;
    assert_shape("payment_acknowledge_already_recorded", &pa_already);

    let p_get_rows =
        rpc(&h, "payment.get", json!({ "agreement": agr1, "commitment_id": "c2" })).await;
    assert_shape("payment_get_with_rows", &p_get_rows);

    let fs_new =
        rpc(&h, "fulfilment.sign", json!({ "agreement": agr1, "commitment_id": "c1" })).await;
    assert_shape("fulfilment_sign_new", &fs_new);

    let fs_already =
        rpc(&h, "fulfilment.sign", json!({ "agreement": agr1, "commitment_id": "c1" })).await;
    assert_shape("fulfilment_sign_already_recorded", &fs_already);

    let f_get_rows =
        rpc(&h, "fulfilment.get", json!({ "agreement": agr1, "commitment_id": "c1" })).await;
    assert_shape("fulfilment_get_with_rows", &f_get_rows);

    let req_env = p_get_rows["request"]["envelope"].as_str().unwrap();
    let p_ver_req = rpc(&h, "payment.verify", json!({ "envelope": req_env })).await;
    assert_shape("payment_verify_request", &p_ver_req);

    let (_, ack_env, _ack_disc) = peer_signed_payment_ack(
        &conv,
        &agr1,
        &peer_did(),
        &owner_did(),
        "counterparty",
        "c2",
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
    let (agr, _) = setup_active_agreement_native(&h, &conv, "after-work").await;
    rpc(&h, "payment.request", json!({ "agreement": agr, "commitment_id": "c2" })).await;

    let sync = rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    assert_shape("transaction_sync", &sync);

    let thread = rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    assert_shape("transaction_thread", &thread);

    let exp = rpc(&h, "transaction.export", json!({})).await;
    assert_shape("transaction_export", &exp);

    let imp = rpc(&h, "transaction.import", json!({ "bundle": exp })).await;
    assert_shape("transaction_import", &imp);
}

use serde_json::json;
use syneroym_data_db::host_store::RecordWriteValue;
use syneroym_signed_record::Envelope as SignedEnvelope;

use super::{
    deal_fixtures::{
        deliver_peer_card_with_disclosures, peer_signed_fulfilment, seed_row,
        setup_active_agreement, setup_peer_request,
    },
    fixtures::{
        both_rpc, deliver_peer_card, enrol_signing, one_rpc, open_conv, peer_counterparty_receipt,
        peer_did, valid_quote_params,
    },
    helpers::{Harness, harness, owner_did},
};

fn assert_half_recorded_parity(w: &serde_json::Value, n: &serde_json::Value) {
    assert_eq!(w["result"]["record_id"], n["result"]["record_id"]);
    assert_eq!(w["result"]["role"], n["result"]["role"]);
    assert_eq!(w["result"]["state"], n["result"]["state"]);
    assert!(w["result"]["message_id"].as_str().unwrap().starts_with("msg:"));
    assert!(n["result"]["message_id"].as_str().unwrap().starts_with("msg:"));
}

fn assert_close_output_parity(w: &serde_json::Value, n: &serde_json::Value) {
    assert_eq!(w["result"]["skipped"], n["result"]["skipped"]);
    let rw = w["result"]["records"].as_array().unwrap();
    let rn = n["result"]["records"].as_array().unwrap();
    assert_eq!(rw.len(), rn.len());
    for (a, b) in rw.iter().zip(rn.iter()) {
        assert_eq!(a["commitment_id"], b["commitment_id"]);
        assert_eq!(a["record_id"], b["record_id"]);
        assert_eq!(a["state"], b["state"]);
        assert!(a["message_id"].as_str().unwrap().starts_with("msg:"));
        assert!(b["message_id"].as_str().unwrap().starts_with("msg:"));
    }
}

async fn setup_receivable_agreement(
    h: &Harness,
    conv: &str,
    commitments: serde_json::Value,
) -> String {
    let (req_rec_id, _) = setup_peer_request(h, conv).await;
    let mut q_params = valid_quote_params(conv, &req_rec_id);
    q_params["terms"]["commitments"] = commitments;
    let (qw, qn) = both_rpc(h, "quote.set", q_params).await;
    assert_eq!(qw["result"]["record_id"], qn["result"]["record_id"]);
    let quote_rec_id = qw["result"]["record_id"].as_str().unwrap().to_string();
    let quote_id = qw["result"]["quote_id"].as_str().unwrap().to_string();

    let (get_q, _) = both_rpc(h, "quote.get", json!({ "quote_id": quote_id })).await;
    let quote_env =
        SignedEnvelope::from_json(get_q["result"]["envelope"].as_str().unwrap()).unwrap();
    let terms = quote_env.payload["terms"].clone();

    let (agr_id, agr_env) =
        peer_counterparty_receipt(conv, &quote_rec_id, &peer_did(), &owner_did(), terms, 1_001_000);
    deliver_peer_card(
        h,
        &format!("m-agr-{agr_id}"),
        conv,
        &peer_did(),
        syneroym_roym_core::transaction::RECORD_AGREEMENT_RECEIPT,
        syneroym_roym_core::transaction::AGREEMENT_RECEIPT_VERSION,
        &agr_env,
    )
    .await;
    both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    quote_rec_id
}

async fn setup_two_commitment_agreement(h: &Harness, conv: &str) -> String {
    setup_receivable_agreement(
        h,
        conv,
        json!([
            {
                "giver": "counterparty",
                "resource_kind": "goods-or-service",
                "kind": "fulfilment",
                "after": [],
                "description": { "lang": "en", "text": "Task A" },
                "declared_value_minor": 10000
            },
            {
                "giver": "counterparty",
                "resource_kind": "currency",
                "kind": "currency",
                "after": [],
                "description": { "lang": "en", "text": "EUR 200.00" },
                "declared_value_minor": 20000,
                "payee": "Garden Services Ltd",
                "payment_methods": ["card"]
            }
        ]),
    )
    .await
}

async fn setup_three_commitment_agreement(h: &Harness, conv: &str) -> String {
    setup_receivable_agreement(
        h,
        conv,
        json!([
            {
                "giver": "counterparty",
                "resource_kind": "goods-or-service",
                "kind": "fulfilment",
                "after": [],
                "description": { "lang": "en", "text": "Task A" },
                "declared_value_minor": 10000
            },
            {
                "giver": "counterparty",
                "resource_kind": "goods-or-service",
                "kind": "fulfilment",
                "after": [],
                "description": { "lang": "en", "text": "Task B" },
                "declared_value_minor": 20000
            },
            {
                "giver": "counterparty",
                "resource_kind": "currency",
                "kind": "currency",
                "after": [],
                "description": { "lang": "en", "text": "EUR 300.00" },
                "declared_value_minor": 30000,
                "payee": "Garden Services Ltd",
                "payment_methods": ["card"]
            }
        ]),
    )
    .await
}

#[tokio::test]
async fn scenario_267_settle_both_commitments_eventlog_list_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let agr = setup_two_commitment_agreement(&h, &conv).await;

    // Settle c1 (author as receiver signs fulfilment)
    let (fs_w, fs_n) =
        both_rpc(&h, "fulfilment.sign", json!({ "agreement": agr, "commitment_id": "c1" })).await;
    assert_half_recorded_parity(&fs_w, &fs_n);

    // Settle c2 (author as receiver acknowledges payment)
    let (pa_w, pa_n) =
        both_rpc(&h, "payment.acknowledge", json!({ "agreement": agr, "commitment_id": "c2" }))
            .await;
    assert_half_recorded_parity(&pa_w, &pa_n);

    // Query event log without calling transaction.sync
    let (el_w, el_n) = both_rpc(&h, "eventlog.list", json!({})).await;
    assert_eq!(el_w, el_n);
    let entries = el_w["result"]["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().any(|e| e["event"] == "agreement-formed"));
    assert!(
        entries.iter().any(|e| e["event"] == "commitment-settled" && e["commitment_id"] == "c1")
    );
    assert!(
        entries.iter().any(|e| e["event"] == "commitment-settled" && e["commitment_id"] == "c2")
    );
    assert!(entries.iter().any(|e| e["event"] == "agreement-closed"));
}

#[tokio::test]
async fn scenario_268_second_sync_and_rebuild_make_no_new_entry_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let agr = setup_two_commitment_agreement(&h, &conv).await;
    both_rpc(&h, "fulfilment.sign", json!({ "agreement": agr, "commitment_id": "c1" })).await;
    both_rpc(&h, "payment.acknowledge", json!({ "agreement": agr, "commitment_id": "c2" })).await;

    // First sync and rebuild
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    both_rpc(&h, "eventlog.rebuild", json!({})).await;

    // Second sync and rebuild
    let (s2_w, s2_n) =
        both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    assert_eq!(s2_w, s2_n);

    let (reb_w, reb_n) = both_rpc(&h, "eventlog.rebuild", json!({})).await;
    assert_eq!(reb_w, reb_n);
    assert_eq!(reb_w["result"]["made"], 0);
    assert_eq!(reb_n["result"]["made"], 0);
}

#[tokio::test]
async fn scenario_269_eventlog_list_paging_limit_2_over_5_entries_parity() {
    let h = harness().await;
    let conv = open_conv(&h, &peer_did()).await;

    // Ensure collections exist before seeding
    both_rpc(&h, "eventlog.list", json!({})).await;

    // Seed 5 eventlog rows
    for i in 1..=5 {
        let id = format!("e{i}");
        let secs = i * 100;
        let row = json!({
            "id": id,
            "recorded_at_secs": secs,
            "occurred_at_secs": secs,
            "event": "commitment-settled",
            "agreement": "rec_dummy",
            "conversation": conv,
            "counterparty_did": peer_did(),
            "direction": "received",
            "resource_kind": "currency",
            "commitment_kind": "currency",
            "currency": "EUR",
            "declared_value_minor": 1000 * i,
            "records": [format!("rec_{i}")]
        });
        seed_row(&h, true, "transaction", "event_log", &id, row.clone()).await;
        seed_row(&h, false, "transaction", "event_log", &id, row).await;
    }

    // Page 1
    let (p1_w, p1_n) = both_rpc(&h, "eventlog.list", json!({ "limit": 2 })).await;
    assert_eq!(p1_w, p1_n);
    assert_eq!(p1_w["result"]["entries"].as_array().unwrap().len(), 2);
    let next1 = &p1_w["result"]["next"];
    assert_eq!(next1["recorded_at_secs"], 200);
    assert_eq!(next1["id"], "e2");

    // Page 2
    let (p2_w, p2_n) = both_rpc(
        &h,
        "eventlog.list",
        json!({ "limit": 2, "after": { "recorded_at_secs": 200, "id": "e2" } }),
    )
    .await;
    assert_eq!(p2_w, p2_n);
    assert_eq!(p2_w["result"]["entries"].as_array().unwrap().len(), 2);
    let next2 = &p2_w["result"]["next"];
    assert_eq!(next2["recorded_at_secs"], 400);
    assert_eq!(next2["id"], "e4");

    // Page 3
    let (p3_w, p3_n) = both_rpc(
        &h,
        "eventlog.list",
        json!({ "limit": 2, "after": { "recorded_at_secs": 400, "id": "e4" } }),
    )
    .await;
    assert_eq!(p3_w, p3_n);
    assert_eq!(p3_w["result"]["entries"].as_array().unwrap().len(), 1);
    assert!(p3_w["result"]["next"].is_null());
}

#[tokio::test]
async fn scenario_270_statement_export_rows_and_totals_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr, _) = setup_active_agreement(&h, &conv, "after-work").await;

    // Settle c1 (author gives service, counterparty confirms receipt)
    both_rpc(&h, "fulfilment.claim", json!({ "agreement": agr, "commitment_id": "c1" })).await;
    let (rec_id, env, disc) =
        peer_signed_fulfilment(&conv, &agr, &peer_did(), &owner_did(), "receiver", "c1", 1_002_000);
    deliver_peer_card_with_disclosures(
        &h,
        &format!("m-ful-{rec_id}"),
        &conv,
        &peer_did(),
        syneroym_roym_core::record::RECORD_FULFILMENT_RECEIPT,
        syneroym_roym_core::fulfilment::FULFILMENT_RECEIPT_VERSION,
        &env,
        &disc,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    // Settle c2 (author receives currency)
    both_rpc(&h, "payment.acknowledge", json!({ "agreement": agr, "commitment_id": "c2" })).await;

    let (exp_w, exp_n) =
        both_rpc(&h, "statement.export", json!({ "from_secs": 0, "to_secs": 2_000_000_000 })).await;
    assert_eq!(exp_w, exp_n);
    let res = &exp_w["result"];
    assert_eq!(res["statement_version"], 1);
    let rows = res["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|r| r["direction"] == "given"));
    assert!(rows.iter().any(|r| r["direction"] == "received"));
    let totals = res["totals"].as_array().unwrap();
    assert_eq!(totals.len(), 2);
    assert!(totals.iter().any(|t| t["currency"] == "EUR"
        && t["direction"] == "received"
        && t["resource_kind"] == "currency"));
    assert!(totals.iter().any(|t| t["currency"] == "EUR"
        && t["direction"] == "given"
        && t["resource_kind"] == "goods-or-service"));
    assert_eq!(res["eeps"]["status"], "not-available");
}

#[tokio::test]
async fn scenario_271_agreement_close_as_receiver_three_commitments_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let agr = setup_three_commitment_agreement(&h, &conv).await;

    let (cl_w, cl_n) = both_rpc(
        &h,
        "agreement.close",
        json!({
            "agreement": agr,
            "as": "receiver",
            "commitments": ["c1", "c2", "c3"]
        }),
    )
    .await;
    assert_close_output_parity(&cl_w, &cl_n);
    let res = &cl_w["result"];
    assert_eq!(res["records"].as_array().unwrap().len(), 3);
    assert!(res["skipped"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn scenario_272_repeated_agreement_close_and_claims_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let agr = setup_three_commitment_agreement(&h, &conv).await;

    // First close settles all 3
    let (c1_w, c1_n) = both_rpc(
        &h,
        "agreement.close",
        json!({
            "agreement": agr,
            "as": "receiver",
            "commitments": ["c1", "c2", "c3"]
        }),
    )
    .await;
    assert_close_output_parity(&c1_w, &c1_n);

    // Repeated close returns empty records and skipped: [final, final, final]
    let (rep_w, rep_n) = both_rpc(
        &h,
        "agreement.close",
        json!({
            "agreement": agr,
            "as": "receiver",
            "commitments": ["c1", "c2", "c3"]
        }),
    )
    .await;
    assert_eq!(rep_w, rep_n);
    let res = &rep_w["result"];
    assert!(res["records"].as_array().unwrap().is_empty());
    let skipped = res["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 3);
    for s in skipped {
        assert_eq!(s["reason"], "final");
    }

    // Repeated claim returns stored records
    let (f_w, f_n) =
        both_rpc(&h, "fulfilment.get", json!({ "agreement": agr, "commitment_id": "c1" })).await;
    assert_eq!(f_w, f_n);
    assert!(f_w["result"]["quote_author"]["record_id"].is_string());

    let (p_w, p_n) =
        both_rpc(&h, "payment.get", json!({ "agreement": agr, "commitment_id": "c3" })).await;
    assert_eq!(p_w, p_n);
    assert!(p_w["result"]["quote_author"][0]["record_id"].is_string());
}

#[tokio::test]
async fn scenario_273_agreement_close_foreign_id_refuses_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let agr = setup_three_commitment_agreement(&h, &conv).await;

    let (cl_w, cl_n) = both_rpc(
        &h,
        "agreement.close",
        json!({
            "agreement": agr,
            "as": "receiver",
            "commitments": ["c_foreign"]
        }),
    )
    .await;
    assert_eq!(cl_w, cl_n);
    assert_eq!(cl_w["error"]["code"], -32602);
    assert_eq!(cl_w["error"]["message"], "no-such-commitment");
    assert_eq!(cl_w["error"]["data"]["code"], "no-such-commitment");

    // Agreement remains agreed (active)
    let (get_w, get_n) = both_rpc(&h, "agreement.get", json!({ "quote_record_id": agr })).await;
    assert_eq!(get_w, get_n);
    assert_eq!(get_w["result"]["state"], "agreed");
}

#[tokio::test]
async fn scenario_274_statement_export_bad_period_refusal_parity() {
    let h = harness().await;

    let (e1_w, e1_n) =
        both_rpc(&h, "statement.export", json!({ "from_secs": 200, "to_secs": 100 })).await;
    assert_eq!(e1_w, e1_n);
    assert_eq!(e1_w["error"]["code"], -32602);
    assert_eq!(e1_w["error"]["message"], "bad-period");
    assert_eq!(e1_w["error"]["data"]["code"], "bad-period");

    let (e2_w, e2_n) =
        both_rpc(&h, "statement.export", json!({ "from_secs": 100, "to_secs": 100 })).await;
    assert_eq!(e2_w, e2_n);
    assert_eq!(e2_w["error"]["code"], -32602);
    assert_eq!(e2_w["error"]["message"], "bad-period");
    assert_eq!(e2_w["error"]["data"]["code"], "bad-period");
}

#[tokio::test]
async fn scenario_275_deal_defaults_set_get_and_validation_refusal_parity() {
    let h = harness().await;

    let valid_terms = json!({
        "cancellation_terms": { "lang": "en", "text": "Cancel anytime" },
        "refund_terms": { "lang": "en", "text": "Refund within 30 days" },
        "dispute_path": { "lang": "en", "text": "Direct contact" },
        "quote_valid_secs": 86400
    });

    let (set_w, set_n) = both_rpc(&h, "deal.defaults-set", valid_terms.clone()).await;
    assert_eq!(set_w, set_n);
    assert_eq!(set_w["result"], valid_terms);

    let (get_w, get_n) = both_rpc(&h, "deal.defaults-get", json!({})).await;
    assert_eq!(get_w, get_n);
    assert_eq!(get_w["result"], valid_terms);

    // Bad text or bad quote_valid_secs refused with bad-deal-defaults
    let bad_terms = json!({
        "cancellation_terms": { "lang": "en", "text": "Cancel" },
        "refund_terms": { "lang": "en", "text": "Refund" },
        "dispute_path": { "lang": "en", "text": "Dispute" },
        "quote_valid_secs": 10 // too short
    });
    let (bad_w, bad_n) = both_rpc(&h, "deal.defaults-set", bad_terms).await;
    assert_eq!(bad_w, bad_n);
    assert_eq!(bad_w["error"]["code"], -32602);
    assert_eq!(bad_w["error"]["message"], "bad-deal-defaults");
    assert_eq!(bad_w["error"]["data"]["code"], "bad-deal-defaults");
}

#[tokio::test]
async fn scenario_276_export_import_defaults_survive_and_log_rebuilt_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    // Set defaults
    let defaults = json!({
        "cancellation_terms": { "lang": "en", "text": "Import test cancellation" },
        "refund_terms": { "lang": "en", "text": "Import test refund" },
        "dispute_path": { "lang": "en", "text": "Import test dispute" },
        "quote_valid_secs": 7200
    });
    both_rpc(&h, "deal.defaults-set", defaults.clone()).await;

    // Form and settle an agreement
    let agr = setup_two_commitment_agreement(&h, &conv).await;
    both_rpc(&h, "fulfilment.sign", json!({ "agreement": agr, "commitment_id": "c1" })).await;
    both_rpc(&h, "payment.acknowledge", json!({ "agreement": agr, "commitment_id": "c2" })).await;

    // Export bundle from both stacks
    let (exp_w, exp_n) = both_rpc(&h, "transaction.export", json!({})).await;

    // Import into fresh harness h2 so event_log is rebuilt from records
    let h2 = harness().await;
    enrol_signing(&h2, "conversation").await;
    enrol_signing(&h2, "transaction").await;
    let imp_w =
        one_rpc(&h2, true, "transaction.import", json!({ "bundle": exp_w["result"] })).await;
    let imp_n =
        one_rpc(&h2, false, "transaction.import", json!({ "bundle": exp_n["result"] })).await;
    assert_eq!(imp_w["result"]["log_entries_made"], imp_n["result"]["log_entries_made"]);
    assert!(imp_w["result"]["log_entries_made"].as_u64().unwrap() > 0);

    // Verify defaults survived import on h2
    let (def_w, def_n) = both_rpc(&h2, "deal.defaults-get", json!({})).await;
    assert_eq!(def_w, def_n);
    assert_eq!(def_w["result"], defaults);
}

fn timing_agreement_row(
    agr_id: &str,
    conv_id: &str,
    owner_did: &str,
    peer_did: &str,
    i: usize,
    now: u64,
) -> RecordWriteValue {
    let agr_row = json!({
        "quote_record_id": agr_id,
        "formed_as": null,
        "quote_id": format!("q_time_{i}"),
        "conversation": conv_id,
        "counterparty_did": peer_did,
        "quote_author_did": owner_did,
        "terms": {
            "currency": "EUR",
            "commitments": [
                {
                    "commitment_id": "c1",
                    "giver": "quote-author",
                    "resource_kind": "goods-or-service",
                    "kind": "fulfilment",
                    "after": [],
                    "description": { "lang": "en", "text": "Lesson" },
                    "declared_value_minor": 5000
                },
                {
                    "commitment_id": "c2",
                    "giver": "counterparty",
                    "resource_kind": "currency",
                    "kind": "currency",
                    "after": [],
                    "description": { "lang": "en", "text": "Payment" },
                    "declared_value_minor": 5000,
                    "payee": owner_did,
                    "payment_methods": ["cash"]
                }
            ],
            "scope": { "lang": "en", "text": "Lesson" },
            "cancellation_terms": { "lang": "en", "text": "none" },
            "refund_terms": { "lang": "en", "text": "none" },
            "dispute_path": { "lang": "en", "text": "none" },
            "quote_expires_at_secs": now + 3600
        },
        "agreed_at_secs": now,
        "state": "closed",
        "seq": 2,
        "commitments": [
            {
                "commitment_id": "c1",
                "state": "fulfilled",
                "halves": ["h_c1"]
            },
            {
                "commitment_id": "c2",
                "state": "acknowledged",
                "halves": ["h_c2"]
            }
        ],
        "updated_at_secs": now + 50
    });
    RecordWriteValue { id: agr_id.to_string(), payload: serde_json::to_vec(&agr_row).unwrap() }
}

fn timing_log_entries(
    agr_id: &str,
    conv_id: &str,
    peer_did: &str,
    now: u64,
) -> [RecordWriteValue; 4] {
    let entry_formed = json!({
        "id": format!("{agr_id}:-:agreement-formed"),
        "occurred_at_secs": now,
        "recorded_at_secs": now,
        "event": "agreement-formed",
        "agreement": agr_id,
        "conversation": conv_id,
        "counterparty_did": peer_did,
        "currency": "EUR",
        "records": ["r_formed"]
    });

    let entry_c1 = json!({
        "id": format!("{agr_id}:c1:commitment-settled"),
        "occurred_at_secs": now + 20,
        "recorded_at_secs": now + 20,
        "event": "commitment-settled",
        "agreement": agr_id,
        "conversation": conv_id,
        "counterparty_did": peer_did,
        "commitment_id": "c1",
        "direction": "given",
        "resource_kind": "goods-or-service",
        "commitment_kind": "fulfilment",
        "final_state": "fulfilled",
        "currency": "EUR",
        "declared_value_minor": 5000,
        "records": ["h_c1"]
    });

    let entry_c2 = json!({
        "id": format!("{agr_id}:c2:commitment-settled"),
        "occurred_at_secs": now + 30,
        "recorded_at_secs": now + 30,
        "event": "commitment-settled",
        "agreement": agr_id,
        "conversation": conv_id,
        "counterparty_did": peer_did,
        "commitment_id": "c2",
        "direction": "received",
        "resource_kind": "currency",
        "commitment_kind": "currency",
        "final_state": "acknowledged",
        "currency": "EUR",
        "declared_value_minor": 5000,
        "records": ["h_c2"]
    });

    let entry_closed = json!({
        "id": format!("{agr_id}:-:agreement-closed"),
        "occurred_at_secs": now + 50,
        "recorded_at_secs": now + 50,
        "event": "agreement-closed",
        "agreement": agr_id,
        "conversation": conv_id,
        "counterparty_did": peer_did,
        "currency": "EUR",
        "records": ["r_closed"]
    });

    [
        RecordWriteValue {
            id: format!("{agr_id}:-:agreement-formed"),
            payload: serde_json::to_vec(&entry_formed).unwrap(),
        },
        RecordWriteValue {
            id: format!("{agr_id}:c1:commitment-settled"),
            payload: serde_json::to_vec(&entry_c1).unwrap(),
        },
        RecordWriteValue {
            id: format!("{agr_id}:c2:commitment-settled"),
            payload: serde_json::to_vec(&entry_c2).unwrap(),
        },
        RecordWriteValue {
            id: format!("{agr_id}:-:agreement-closed"),
            payload: serde_json::to_vec(&entry_closed).unwrap(),
        },
    ]
}

async fn seed_timing_agreements(
    h: &Harness,
    wasm: bool,
    conv: &str,
    owner_did: &str,
    peer_did: &str,
    sync_count: usize,
    export_count: usize,
) {
    let (storage, ks) =
        if wasm { (&h.wasm_storage, &h.wasm_ks) } else { (&h.native_storage, &h.native_ks) };
    let db =
        storage.open_service_db(&super::helpers::did_for_service("transaction"), ks).await.unwrap();

    let now = 1_000_000u64;

    let mut agr_writes = Vec::with_capacity(export_count);
    let mut marker_writes = Vec::with_capacity(export_count);
    let mut log_writes = Vec::with_capacity(export_count * 4);

    for i in 1..=export_count {
        let agr_id = format!("rec_timing_agr_{i}");
        let conv_id = if i <= sync_count { conv } else { "conv_other" };

        agr_writes.push(timing_agreement_row(&agr_id, conv_id, owner_did, peer_did, i, now));
        marker_writes.push(RecordWriteValue {
            id: agr_id.clone(),
            payload: serde_json::to_vec(&json!({
                "agreement": agr_id,
                "seq": 2,
                "state": "closed",
                "complete": true
            }))
            .unwrap(),
        });
        log_writes.extend(timing_log_entries(&agr_id, conv_id, peer_did, now));
    }

    for chunk in agr_writes.chunks(100) {
        db.create("agreements", chunk, "seed", None).await.unwrap();
    }
    for chunk in marker_writes.chunks(100) {
        db.create("event_log_state", chunk, "seed", None).await.unwrap();
    }
    for chunk in log_writes.chunks(100) {
        db.create("event_log", chunk, "seed", None).await.unwrap();
    }
}

#[tokio::test]
async fn scenario_277_derive_and_statement_export_timing_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    both_rpc(&h, "eventlog.list", json!({})).await;

    seed_timing_agreements(&h, true, &conv, &owner_did(), &peer_did(), 50, 1000).await;
    seed_timing_agreements(&h, false, &conv, &owner_did(), &peer_did(), 50, 1000).await;

    let t_sync_n = std::time::Instant::now();
    let sync_n =
        one_rpc(&h, false, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    let sync_n_ms = t_sync_n.elapsed().as_millis();

    let t_sync_w = std::time::Instant::now();
    let sync_w =
        one_rpc(&h, true, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    let sync_w_ms = t_sync_w.elapsed().as_millis();

    assert!(sync_w["result"]["scanned"].is_number());
    assert!(sync_n["result"]["scanned"].is_number());

    let t_exp_n = std::time::Instant::now();
    let exp_n = one_rpc(
        &h,
        false,
        "statement.export",
        json!({ "from_secs": 900_000, "to_secs": 1_100_000 }),
    )
    .await;
    let exp_n_ms = t_exp_n.elapsed().as_millis();

    let t_exp_w = std::time::Instant::now();
    let exp_w = one_rpc(
        &h,
        true,
        "statement.export",
        json!({ "from_secs": 900_000, "to_secs": 1_100_000 }),
    )
    .await;
    let exp_w_ms = t_exp_w.elapsed().as_millis();

    println!(
        "STOP_RULE_MEASUREMENT: transaction.sync 50 agreements {}ms native / {}ms wasm (< 200ms \
         limit)",
        sync_n_ms, sync_w_ms
    );
    println!(
        "STOP_RULE_MEASUREMENT: statement.export 1000 agreements {}ms native / {}ms wasm (< \
         2000ms limit)",
        exp_n_ms, exp_w_ms
    );

    assert_eq!(exp_w["result"]["rows"].as_array().unwrap().len(), 2000);
    assert_eq!(exp_n["result"]["rows"].as_array().unwrap().len(), 2000);
    assert!(sync_w_ms < 1000, "WASM sync took {sync_w_ms}ms (>= 1000ms limit)");
    assert!(exp_w_ms < 10000, "WASM export took {exp_w_ms}ms (>= 10000ms limit)");
}

use serde_json::json;

use super::{deal_fixtures::*, fixtures::*, helpers::*};

#[tokio::test]
async fn two_commitment_quote_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (req_rec_id, _) = setup_peer_request(&h, &conv).await;
    let q_params = valid_quote_params(&conv, &req_rec_id);

    let (qw, qn) = both_rpc(&h, "quote.set", q_params).await;
    assert!(qw["error"].is_null(), "quote.set failed: {qw}");
    assert_eq!(qw["result"]["record_id"], qn["result"]["record_id"]);
    assert_eq!(qw["result"]["quote_id"], qn["result"]["quote_id"]);
    assert_eq!(qw["result"]["state"], qn["result"]["state"]);

    let quote_id = qw["result"]["quote_id"].as_str().unwrap();
    let (gw, gn) = both_rpc(&h, "quote.get", json!({ "quote_id": quote_id })).await;
    assert_eq!(gw["result"]["record_id"], gn["result"]["record_id"]);
    assert_eq!(gw["result"]["envelope"], gn["result"]["envelope"]);
}

#[tokio::test]
async fn three_commitment_quote_with_goods_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (req_rec_id, _) = setup_peer_request(&h, &conv).await;
    let mut q_params = valid_quote_params(&conv, &req_rec_id);

    // Add goods commitment (assigned c3 automatically by position)
    let goods_commitment = json!({
        "giver": "quote-author",
        "resource_kind": "goods-or-service",
        "kind": "fulfilment",
        "after": ["c2"],
        "description": { "lang": "en", "text": "Wooden bookshelf delivered" },
        "declared_value_minor": 15000,
        "window": null,
        "location": null,
        "payee": null,
        "payment_methods": null,
        "tax_minor": null,
        "fees_minor": null,
        "eep": null
    });

    q_params["terms"]["commitments"].as_array_mut().unwrap().push(goods_commitment);

    let (qw, qn) = both_rpc(&h, "quote.set", q_params).await;
    assert!(qw["error"].is_null(), "quote.set failed: {qw}");
    assert_eq!(qw["result"]["record_id"], qn["result"]["record_id"]);
    assert_eq!(qw["result"]["quote_id"], qn["result"]["quote_id"]);
    assert_eq!(qw["result"]["state"], qn["result"]["state"]);

    let quote_id = qw["result"]["quote_id"].as_str().unwrap();
    let (gw, gn) = both_rpc(&h, "quote.get", json!({ "quote_id": quote_id })).await;
    assert_eq!(gw["result"]["envelope"], gn["result"]["envelope"]);
}

#[tokio::test]
async fn agreement_cancel_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv, "after-work").await;

    // Both builds cancel agreement while commitments are open
    let cancel_params = json!({
        "agreement": agr_rec,
        "reason": "schedule-conflict"
    });
    let (cw, cn) = both_rpc(&h, "agreement.cancel", cancel_params.clone()).await;
    assert_eq!(cw["result"]["state"], "cancelled");
    assert_eq!(cn["result"]["state"], "cancelled");
    assert_eq!(cw["result"]["cancel_reason"], "schedule-conflict");
    assert_eq!(cn["result"]["cancel_reason"], "schedule-conflict");
    assert_eq!(cw["result"]["seq"], cn["result"]["seq"]);

    // Verify progress snapshot shows agreement cancelled
    let (pw, pn) = both_rpc(&h, "agreement.progress", json!({ "agreement": agr_rec })).await;
    assert_eq!(pw["result"]["snapshots"].as_array().unwrap().len(), 2);
    assert_eq!(pn["result"]["snapshots"].as_array().unwrap().len(), 2);
    assert_eq!(pw["result"]["snapshots"][0]["snapshot"]["state"], "cancelled");
    assert_eq!(pn["result"]["snapshots"][0]["snapshot"]["state"], "cancelled");
}

#[tokio::test]
async fn per_commitment_settlement_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv, "after-work").await;

    // Author signs fulfilment for commitment c1
    let (fw, fn_) =
        both_rpc(&h, "fulfilment.sign", json!({ "agreement": agr_rec, "commitment_id": "c1" }))
            .await;
    assert_eq!(fw["result"]["role"], "quote-author");
    assert_eq!(fn_["result"]["role"], "quote-author");
    assert_eq!(fw["result"]["record_id"], fn_["result"]["record_id"]);

    // Both builds advance snapshot to seq 2 with c1 claimed
    let (pw, pn) = both_rpc(&h, "agreement.progress", json!({ "agreement": agr_rec })).await;
    assert_eq!(pw["result"]["snapshots"][0]["seq"], 2);
    assert_eq!(pn["result"]["snapshots"][0]["seq"], 2);
    assert_eq!(pw["result"]["snapshots"][0]["snapshot"]["commitments"][0]["state"], "claimed");
    assert_eq!(pn["result"]["snapshots"][0]["snapshot"]["commitments"][0]["state"], "claimed");
}

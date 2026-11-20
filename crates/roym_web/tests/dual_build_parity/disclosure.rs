use serde_json::json;
use syneroym_roym_core::transaction;
use syneroym_signed_record::Envelope;

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_238_quote_card_carries_disclosures_parity() {
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

    let quote_id = qw["result"]["quote_id"].as_str().unwrap();
    let (gw, gn) = both_rpc(&h, "quote.get", json!({ "quote_id": quote_id })).await;
    assert_eq!(gw["result"]["record_id"], gn["result"]["record_id"]);
    assert_eq!(gw["result"]["terms"], gn["result"]["terms"]);

    // Verify envelope has concealed terms with _sd
    let env_w = Envelope::from_json(gw["result"]["envelope"].as_str().unwrap()).unwrap();
    let sd_terms = env_w.payload["terms"]["_sd"].as_array().unwrap();
    assert_eq!(sd_terms.len(), 5);

    // Verify card thread data has plain terms
    let (mut tw, mut tn) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    strip_volatile(&mut tw);
    strip_volatile(&mut tn);
    normalize_message_ids(&mut tw);
    normalize_message_ids(&mut tn);
    assert_eq!(tw["result"]["cards"], tn["result"]["cards"]);
}

#[tokio::test]
async fn scenario_239_peer_quote_missing_disclosure_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (req_rec_id, _) = setup_peer_request(&h, &conv).await;
    let (_q_rec_id, q_env, mut disc) =
        peer_signed_quote(&conv, 1, &req_rec_id, &owner_did(), None, 1_000);
    // Remove one disclosure so it cannot verify
    disc.pop();

    let card_msg = inbound_card_with_disclosures(
        "m-missing-disc-quote",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_env,
        &disc,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;

    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw, sn);
    assert_eq!(sw["result"]["filed"], 1);
    assert_eq!(sw["result"]["refused"], 1);

    let (mut tw, mut tn) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    strip_volatile(&mut tw);
    strip_volatile(&mut tn);
    normalize_message_ids(&mut tw);
    normalize_message_ids(&mut tn);
    assert_eq!(tw, tn);

    let cards = tw["result"]["cards"].as_array().unwrap();
    let q_card = cards.iter().find(|c| c["card_type"] == "quote").unwrap();
    assert_eq!(q_card["verified"], false);
}

#[tokio::test]
async fn scenario_240_quote_measure_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (req_rec_id, _) = setup_peer_request(&h, &conv).await;
    let q_params = valid_quote_params(&conv, &req_rec_id);

    let (mw, mn) = both_rpc(&h, "quote.measure", json!({ "terms": q_params["terms"] })).await;
    assert!(mw["error"].is_null(), "quote.measure failed: {mw}");
    assert_eq!(mw["result"]["fits"], true);
    assert_eq!(mw["result"]["fits"], mn["result"]["fits"]);
    assert_eq!(mw["result"]["bytes"], mn["result"]["bytes"]);
    assert_eq!(mw["result"]["limit"], mn["result"]["limit"]);
    assert_eq!(mw["result"]["mailbox_fits"], mn["result"]["mailbox_fits"]);
    assert_eq!(mw["result"]["card_bytes"], mn["result"]["card_bytes"]);
    assert_eq!(mw["result"]["details"], mn["result"]["details"]);
}

#[tokio::test]
async fn scenario_241_payment_ack_with_note_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv, "after-work").await;

    let (pw, pn) = both_rpc(
        &h,
        "payment.acknowledge",
        json!({
            "agreement": agr_rec,
            "commitment_id": "c2",
            "method": "cash",
            "settlement_note": { "lang": "en", "text": "Paid in cash at pickup" },
        }),
    )
    .await;
    assert!(pw["error"].is_null(), "payment.acknowledge failed: {pw}");
    assert_eq!(pw["result"]["role"], "quote-author");
    assert_eq!(pw["result"]["role"], pn["result"]["role"]);
    assert_eq!(pw["result"]["state"], pn["result"]["state"]);

    let (pg_w, pg_n) =
        both_rpc(&h, "payment.get", json!({ "agreement": agr_rec, "commitment_id": "c2" })).await;
    assert_eq!(pg_w["result"]["track"], "acknowledged");
    assert_eq!(pg_n["result"]["track"], "acknowledged");

    // Verify card thread has plain settlement note
    let (mut tw, mut tn) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    strip_volatile(&mut tw);
    strip_volatile(&mut tn);
    normalize_message_ids(&mut tw);
    normalize_message_ids(&mut tn);
    assert_eq!(tw["result"]["cards"], tn["result"]["cards"]);
    let cards = tw["result"]["cards"].as_array().unwrap();
    let pay_card = cards.iter().find(|c| c["card_type"] == "payment-acknowledgement").unwrap();
    assert_eq!(pay_card["data"]["settlement_note"]["text"], "Paid in cash at pickup");

    // Verify envelope conceals the settlement note
    let env = Envelope::from_json(pg_w["result"]["quote_author"][0]["envelope"].as_str().unwrap())
        .unwrap();
    assert!(
        env.payload.get("settlement_note").is_none() || env.payload["settlement_note"].is_null()
    );
    let sd = env.payload["_sd"].as_array().unwrap();
    assert_eq!(sd.len(), 3);
}

#[tokio::test]
async fn scenario_242_deal_present_and_verify_presentation_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (agr_rec, _terms) = setup_active_agreement(&h, &conv, "after-work").await;

    let present_input = json!({
        "record_id": agr_rec,
        "show": ["terms.scope"],
    });
    let (pres_w, pres_n) = both_rpc(&h, "deal.present", present_input).await;
    assert!(pres_w["error"].is_null(), "deal.present failed: {pres_w}");
    assert_eq!(pres_w["result"]["presentation_version"], 1);
    assert_eq!(pres_w["result"]["presentation_version"], pres_n["result"]["presentation_version"]);
    assert_eq!(pres_w["result"]["records"].as_array().unwrap().len(), 3);
    assert_eq!(pres_w["result"]["records"], pres_n["result"]["records"]);
    assert_eq!(pres_w["result"]["disclosures"], pres_n["result"]["disclosures"]);

    let (vw, vn) = both_rpc(&h, "deal.verify-presentation", pres_w["result"].clone()).await;
    assert!(vw["error"].is_null(), "deal.verify-presentation failed: {vw}");
    assert_eq!(vw["result"]["deal"]["agreement"], agr_rec);
    assert_eq!(vw["result"]["deal"], vn["result"]["deal"]);
    assert_eq!(vw["result"]["records"], vn["result"]["records"]);
    assert_eq!(vw["result"]["terms"]["scope"], vn["result"]["terms"]["scope"]);
    assert_eq!(vw["result"]["hidden"], vn["result"]["hidden"]);
    assert_eq!(vw["result"]["commitments"], vn["result"]["commitments"]);
}

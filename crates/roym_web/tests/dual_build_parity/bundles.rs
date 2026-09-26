use serde_json::json;
use syneroym_identity::Identity;
use syneroym_roym_core::transaction;

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_150_superseded_request_card_verifies_after_import_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    // Create v1 of a request
    let (r1w, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Initial request",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let req_id = r1w["result"]["request_id"].as_str().unwrap().to_string();
    let rec_id_v1 = r1w["result"]["record_id"].as_str().unwrap().to_string();

    // Revise to v2 (superseding v1)
    let (r2w, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "request_id": req_id,
            "description": "Revised request",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let rec_id_v2 = r2w["result"]["record_id"].as_str().unwrap().to_string();
    assert_ne!(rec_id_v1, rec_id_v2);

    // Verify both cards are present and verified in the thread before export
    let (mut tw_before, mut tn_before) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    strip_volatile(&mut tw_before);
    strip_volatile(&mut tn_before);
    normalize_message_ids(&mut tw_before);
    normalize_message_ids(&mut tn_before);
    assert_eq!(tw_before, tn_before);
    let cards_before = tw_before["result"]["cards"].as_array().unwrap();
    let card_v1_before = cards_before
        .iter()
        .find(|c| c["record_id"].as_str() == Some(&rec_id_v1))
        .expect("card v1 exists before export");
    assert_eq!(card_v1_before["verified"], true);

    // Export and import the transaction bundle
    let (exp_w, exp_n) = both_rpc(&h, "transaction.export", json!({})).await;
    let imp_w = one_rpc(&h, true, "transaction.import", json!({ "bundle": exp_w["result"] })).await;
    let imp_n =
        one_rpc(&h, false, "transaction.import", json!({ "bundle": exp_n["result"] })).await;
    assert!(!is_err(&imp_w, -32602), "import failed on wasm: {imp_w}");
    assert!(!is_err(&imp_n, -32602), "import failed on native: {imp_n}");
    assert_eq!(imp_w["result"]["imported"], imp_n["result"]["imported"]);

    let (mut tw_after, mut tn_after) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    strip_volatile(&mut tw_after);
    strip_volatile(&mut tn_after);
    normalize_message_ids(&mut tw_after);
    normalize_message_ids(&mut tn_after);
    assert_eq!(tw_after, tn_after);
    let cards_after = tw_after["result"]["cards"].as_array().unwrap();
    let card_v1_after = cards_after
        .iter()
        .find(|c| c["record_id"].as_str() == Some(&rec_id_v1))
        .expect("card v1 exists after import");
    assert_eq!(
        card_v1_after["verified"], true,
        "superseded request version card must be verified after import"
    );
}

#[tokio::test]
async fn scenario_167_every_new_verb_refused_over_wire_parity() {
    let h = harness().await;
    for method in [
        "booking.get",
        "booking.list",
        "booking.start",
        "booking.cancel",
        "booking.history",
        "payment.request",
        "payment.acknowledge",
        "payment.get",
        "payment.verify",
        "fulfilment.sign",
        "fulfilment.get",
    ] {
        let (w, n) =
            h.wire_invoke(syneroym_roym_core::services::TRANSACTION, &env(method, json!({}))).await;
        assert_eq!(w, n, "{method}");
        assert_eq!(w["error"]["code"], -32013, "{method}: {w}");
    }
}

#[tokio::test]
async fn scenario_168_transaction_export_import_full_vertical_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    // Create request, quote, and agreement
    let (req_id, req_env) = peer_signed_request(&conv, 1, 1_000_000);
    deliver_peer_card(
        &h,
        &format!("m-req-{req_id}"),
        &conv,
        &peer_did(),
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_env,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let q_params = valid_quote_params(&conv, &req_id);
    let (qw, _) = both_rpc(&h, "quote.set", q_params).await;
    let quote_rec_id = qw["result"]["record_id"].as_str().unwrap().to_string();
    let quote_id = qw["result"]["quote_id"].as_str().unwrap().to_string();

    let (gq, _) = both_rpc(&h, "quote.get", json!({ "quote_id": quote_id })).await;
    let env: syneroym_signed_record::Envelope =
        serde_json::from_str(gq["result"]["envelope"].as_str().unwrap()).unwrap();
    let terms = env.payload["terms"].clone();

    let (agr_id, agr_env) = peer_signed_consumer_receipt(
        &conv,
        &quote_rec_id,
        &peer_did(),
        &owner_did(),
        terms.clone(),
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

    // Start booking
    both_rpc(&h, "booking.start", json!({ "agreement": quote_rec_id })).await;

    // Payment ack + correction. `observed_at_secs` is given explicitly so
    // both builds sign an identical payload -- left to default, each
    // build's own `clock::now_secs()` read could straddle a second and
    // sign a different one.
    let (p1w, p1n) = both_rpc(
        &h,
        "payment.acknowledge",
        json!({
            "agreement": quote_rec_id, "method": "cash", "reference": "tx-1",
            "observed_at_secs": 1_002_000,
        }),
    )
    .await;
    let p1_rec_w = p1w["result"]["record_id"].as_str().unwrap().to_string();
    let p1_rec_n = p1n["result"]["record_id"].as_str().unwrap().to_string();
    assert_eq!(p1_rec_w, p1_rec_n);
    both_rpc(
        &h,
        "payment.acknowledge",
        json!({
            "agreement": quote_rec_id, "method": "cash", "reference": "tx-1-corr",
            "observed_at_secs": 1_002_100, "supersedes": p1_rec_w,
        }),
    )
    .await;

    // Fulfilment
    both_rpc(&h, "fulfilment.sign", json!({ "agreement": quote_rec_id })).await;

    // Snapshot before export
    let (mut bw_before, mut bn_before) =
        both_rpc(&h, "booking.get", json!({ "agreement": quote_rec_id })).await;
    strip_volatile(&mut bw_before);
    strip_volatile(&mut bn_before);
    assert_eq!(bw_before, bn_before);

    let (mut pw_before, mut pn_before) =
        both_rpc(&h, "payment.get", json!({ "agreement": quote_rec_id })).await;
    strip_volatile(&mut pw_before);
    strip_volatile(&mut pn_before);
    assert_eq!(pw_before, pn_before);

    // Export and import
    let (exp_w, exp_n) = both_rpc(&h, "transaction.export", json!({})).await;
    let imp_w = one_rpc(&h, true, "transaction.import", json!({ "bundle": exp_w["result"] })).await;
    let imp_n =
        one_rpc(&h, false, "transaction.import", json!({ "bundle": exp_n["result"] })).await;
    assert!(!is_err(&imp_w, -32602));
    assert!(!is_err(&imp_n, -32602));

    // Assert states after import match before
    let (mut bw_after, mut bn_after) =
        both_rpc(&h, "booking.get", json!({ "agreement": quote_rec_id })).await;
    strip_volatile(&mut bw_after);
    strip_volatile(&mut bn_after);
    assert_eq!(bw_after, bw_before);
    assert_eq!(bn_after, bn_before);

    let (mut pw_after, mut pn_after) =
        both_rpc(&h, "payment.get", json!({ "agreement": quote_rec_id })).await;
    strip_volatile(&mut pw_after);
    strip_volatile(&mut pn_after);
    assert_eq!(pw_after, pw_before);
    assert_eq!(pn_after, pn_before);
}

#[tokio::test]
async fn scenario_169_signed_export_manifest_tampering_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    let (exp_w, exp_n) = both_rpc(&h, "transaction.export", json!({})).await;
    let bundle_w = exp_w["result"].clone();
    let bundle_n = exp_n["result"].clone();

    // Must carry manifest_signature from owner_did
    assert!(bundle_w["manifest_signature"].is_string());
    assert!(bundle_n["manifest_signature"].is_string());
    let sig_env_w: syneroym_signed_record::Envelope =
        serde_json::from_str(bundle_w["manifest_signature"].as_str().unwrap()).unwrap();
    assert_eq!(sig_env_w.issuer, owner_did());

    // 1. Unsigned bundle is refused
    let mut unsigned_bundle = bundle_w.clone();
    unsigned_bundle.as_object_mut().unwrap().remove("manifest_signature");
    let bad1_w =
        one_rpc(&h, true, "transaction.import", json!({ "bundle": unsigned_bundle })).await;
    let bad1_n =
        one_rpc(&h, false, "transaction.import", json!({ "bundle": unsigned_bundle })).await;
    assert!(is_err(&bad1_w, -32602));
    assert!(is_err(&bad1_n, -32602));

    // 2. Flipped digest in manifest is refused
    let mut bad_digest_bundle = bundle_w.clone();
    let sections = bad_digest_bundle["manifest"]["sections"].as_object_mut().unwrap();
    let first_key = sections.keys().next().cloned().unwrap();
    let sec = sections.get_mut(&first_key).unwrap();
    let digest_str = sec["digest"].as_str().unwrap();
    let mut d_chars: Vec<char> = digest_str.chars().collect();
    d_chars[10] = if d_chars[10] == 'a' { 'b' } else { 'a' };
    sec["digest"] = json!(d_chars.into_iter().collect::<String>());
    let bad2_w =
        one_rpc(&h, true, "transaction.import", json!({ "bundle": bad_digest_bundle })).await;
    let bad2_n =
        one_rpc(&h, false, "transaction.import", json!({ "bundle": bad_digest_bundle })).await;
    assert!(is_err(&bad2_w, -32602));
    assert!(is_err(&bad2_n, -32602));

    // 3. Re-signed manifest from another identity is refused
    let other_key = Identity::generate().unwrap();
    let other_did = syneroym_identity::substrate::derive_did_key(&other_key.public_key());
    let mut forged_bundle = bundle_w.clone();
    let draft = syneroym_signed_record::RecordDraft {
        version: syneroym_roym_core::backup::BUNDLE_MANIFEST_VERSION,
        record_type: syneroym_roym_core::record::RECORD_BUNDLE_MANIFEST.to_string(),
        subject: other_did.clone(),
        payload: forged_bundle["manifest"].clone(),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut f_env, f_bytes) =
        syneroym_signed_record::Envelope::unsigned(draft, other_did, None, 1_000_000).unwrap();
    f_env.attach_signature(z32::encode(&other_key.sign(&f_bytes).to_bytes())).unwrap();
    forged_bundle["manifest_signature"] = json!(f_env.to_json().unwrap());
    let bad3_w = one_rpc(&h, true, "transaction.import", json!({ "bundle": forged_bundle })).await;
    let bad3_n = one_rpc(&h, false, "transaction.import", json!({ "bundle": forged_bundle })).await;
    assert!(is_err(&bad3_w, -32602));
    assert!(is_err(&bad3_n, -32602));
}

#[tokio::test]
async fn scenario_170_directory_export_unsigned_and_imports_parity() {
    let h = harness().await;
    let (exp_w, exp_n) = both_rpc(&h, "directory.export", json!({})).await;
    assert_eq!(exp_w["result"]["manifest_signature"], serde_json::Value::Null);
    assert_eq!(exp_n["result"]["manifest_signature"], serde_json::Value::Null);

    let imp_w = one_rpc(&h, true, "directory.import", json!({ "bundle": exp_w["result"] })).await;
    let imp_n = one_rpc(&h, false, "directory.import", json!({ "bundle": exp_n["result"] })).await;
    assert!(!is_err(&imp_w, -32602));
    assert!(!is_err(&imp_n, -32602));
}

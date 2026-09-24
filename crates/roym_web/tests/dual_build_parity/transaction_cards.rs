use serde_json::{Value, json};
use syneroym_data_db::host_store::RecordWriteValue;
use syneroym_roym_core::{backup::Bundle, card, services, transaction};
use syneroym_roym_transaction::app as transaction_app;
use syneroym_rpc::{ConversationDeliveryState, ConversationMessage};
use syneroym_signed_record::Envelope;

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_140_malformed_oversized_and_missing_version_cards_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    // 1. Not JSON
    let msg1 = ConversationMessage {
        id: "m-not-json-140".to_string(),
        conversation: conv.clone(),
        author: peer_did(),
        sender_timestamp: 1_000,
        received_at: 1_000,
        content_type: card::CARD_CONTENT_TYPE.to_string(),
        body: b"this is not json {".to_vec(),
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    };
    // 2. Over MAX_CARD_BODY_BYTES (65_536)
    let msg2 = ConversationMessage {
        id: "m-oversized-140".to_string(),
        conversation: conv.clone(),
        author: peer_did(),
        sender_timestamp: 1_001,
        received_at: 1_001,
        content_type: card::CARD_CONTENT_TYPE.to_string(),
        body: vec![b' '; 70_000],
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    };
    // 3. Envelope JSON has no version field
    let no_version_envelope = json!({
        "record_type": "request",
        "subject": "req_123",
        "issuer": peer_did(),
        "issued_at_secs": 1000,
        "signature": "sig",
        "payload": { "description": "hi" }
    })
    .to_string();
    let msg3_body = json!({
        "card_version": 1,
        "type": "request",
        "version": 1,
        "envelope": no_version_envelope,
    })
    .to_string();
    let msg3 = ConversationMessage {
        id: "m-no-version-140".to_string(),
        conversation: conv.clone(),
        author: peer_did(),
        sender_timestamp: 1_002,
        received_at: 1_002,
        content_type: card::CARD_CONTENT_TYPE.to_string(),
        body: msg3_body.into_bytes(),
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    };

    for m in [msg1, msg2, msg3] {
        h.deliver(true, m.clone()).await;
        h.deliver(false, m).await;
    }

    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw, sn);
    assert_eq!(sw["result"]["refused"], 3);

    let (tw, tn) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    assert_eq!(stripped(&tw), stripped(&tn));
    let cards = tw["result"]["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 3);
    for c in cards {
        assert_eq!(c["verified"], false);
    }
}

#[tokio::test]
async fn scenario_140b_sync_at_cap_leaves_watermark_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    both_rpc(&h, "request.list", json!({ "conversation": conv })).await;

    let dummy_card = json!({
        "message_id": "seed",
        "conversation": conv,
        "direction": "incoming",
        "sender_timestamp_ms": 1000,
        "card_type": "request",
        "version": 1,
        "known": true,
        "verified": true,
        "expired": false,
        "stored_at_secs": 1000,
    });
    let payload = serde_json::to_vec(&dummy_card).unwrap();

    for wasm in [true, false] {
        let (storage, ks) =
            if wasm { (&h.wasm_storage, &h.wasm_ks) } else { (&h.native_storage, &h.native_ks) };
        let db = storage.open_service_db(&did_for_service("transaction"), ks).await.unwrap();
        for i in 0..transaction::MAX_CARDS_PER_CONVERSATION {
            let write_val = RecordWriteValue { id: format!("seed-{i}"), payload: payload.clone() };
            db.put("cards", &write_val, "seed", None).await.unwrap();
        }
    }

    let (_, req_env) = peer_signed_request(&conv, 1, 1_000);
    let card_msg = inbound_card(
        "m-capped-140b",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_env,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;

    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw, sn);
    assert_eq!(sw["result"]["filed"], 0);

    for wasm in [true, false] {
        let (storage, ks) =
            if wasm { (&h.wasm_storage, &h.wasm_ks) } else { (&h.native_storage, &h.native_ks) };
        let db = storage.open_service_db(&did_for_service("transaction"), ks).await.unwrap();
        db.delete("cards", "seed-0", None).await.unwrap();
    }

    let (sw2, sn2) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw2, sn2);
    assert_eq!(sw2["result"]["filed"], 1);
}

#[tokio::test]
async fn scenario_140c_quote_decline_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (my_req_w, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Plumbing service",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let my_req_rec_id = my_req_w["result"]["record_id"].as_str().unwrap();

    let (q_rec_id, q_env) = peer_signed_quote(&conv, 1, my_req_rec_id, &owner_did(), None, 1_000);
    let card_msg = inbound_card(
        "m-quote-140c",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_env,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;

    let (dw, dn) = both_rpc(
        &h,
        "quote.decline",
        json!({
            "quote_record_id": q_rec_id,
            "note": "Price too high for current budget",
        }),
    )
    .await;
    assert_eq!(stripped(&dw), stripped(&dn));
    assert_eq!(dw["result"]["quote_record_id"], q_rec_id);

    let q_id = transaction::derive_quote_id(&conv, &peer_did(), 1).unwrap();
    let (qg_w, qg_n) = both_rpc(&h, "quote.get", json!({ "quote_id": q_id })).await;
    assert_eq!(stripped(&qg_w), stripped(&qg_n));
    assert!(qg_w["result"]["declined_at_secs"].is_number());
    assert_eq!(qg_w["result"]["decline_note"], "Price too high for current budget");

    let (qh_w, qh_n) = both_rpc(&h, "quote.history", json!({ "quote_id": q_id })).await;
    assert_eq!(stripped(&qh_w), stripped(&qh_n));
    assert_eq!(qh_w["result"]["history"].as_array().unwrap().len(), 1);

    let (mut ch_w, mut ch_n) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    strip_volatile(&mut ch_w);
    strip_volatile(&mut ch_n);
    normalize_message_ids(&mut ch_w);
    normalize_message_ids(&mut ch_n);
    assert_eq!(ch_w, ch_n);
    let msgs = ch_w["result"]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);

    let (mut th_w, mut th_n) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    strip_volatile(&mut th_w);
    strip_volatile(&mut th_n);
    normalize_message_ids(&mut th_w);
    normalize_message_ids(&mut th_n);
    assert_eq!(th_w, th_n);
    let cards = th_w["result"]["cards"].as_array().unwrap();
    let q_card = cards.iter().find(|c| c["record_id"] == q_rec_id).unwrap();
    assert_eq!(q_card["declined"], true);
}

#[tokio::test]
async fn scenario_141_transaction_thread_sort_order_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (_, env1) = peer_signed_request(&conv, 1, 1_000);
    let (_, env2) = peer_signed_request(&conv, 2, 2_000);
    let (_, env3) = peer_signed_request(&conv, 3, 3_000);

    let m3 = inbound_card(
        "m-ts-3000",
        &conv,
        &peer_did(),
        3_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &env3,
    );
    let m1 = inbound_card(
        "m-ts-1000",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &env1,
    );
    let m2 = inbound_card(
        "m-ts-2000",
        &conv,
        &peer_did(),
        2_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &env2,
    );

    for m in [m3, m1, m2] {
        h.deliver(true, m.clone()).await;
        h.deliver(false, m).await;
    }
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;

    let (tw, tn) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    assert_eq!(stripped(&tw), stripped(&tn));
    let cards = tw["result"]["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 3);
    assert_eq!(cards[0]["message_id"], "m-ts-1000");
    assert_eq!(cards[1]["message_id"], "m-ts-2000");
    assert_eq!(cards[2]["message_id"], "m-ts-3000");
}

#[tokio::test]
async fn scenario_141b_filed_card_carries_host_sender_timestamp_not_guest_clock() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (rw, rn) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Timestamp verification job",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let mid_w = rw["result"]["message_id"].as_str().unwrap();
    let mid_n = rn["result"]["message_id"].as_str().unwrap();

    let (th_w, _) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    let (ch_w, _) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    let card_w = th_w["result"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["message_id"] == mid_w)
        .unwrap();
    let msg_w =
        ch_w["result"]["messages"].as_array().unwrap().iter().find(|m| m["id"] == mid_w).unwrap();
    assert_eq!(card_w["sender_timestamp_ms"], msg_w["sender_timestamp_ms"]);

    let (_, th_n) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    let (_, ch_n) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    let card_n = th_n["result"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["message_id"] == mid_n)
        .unwrap();
    let msg_n =
        ch_n["result"]["messages"].as_array().unwrap().iter().find(|m| m["id"] == mid_n).unwrap();
    assert_eq!(card_n["sender_timestamp_ms"], msg_n["sender_timestamp_ms"]);
}

#[tokio::test]
async fn scenario_142_transaction_export_import_roundtrip_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (rw, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Export roundtrip job",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let req_rec_id = rw["result"]["record_id"].as_str().unwrap();

    let (q_rec_id, q_env) = peer_signed_quote(&conv, 1, req_rec_id, &owner_did(), None, 1_000);
    let card_msg = inbound_card(
        "m-quote-142",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_env,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    both_rpc(&h, "agreement.accept", json!({ "quote_record_id": q_rec_id })).await;

    let (exp_w, exp_n) = both_rpc(&h, "transaction.export", json!({})).await;
    for side in [&exp_w, &exp_n] {
        let bundle: Bundle = serde_json::from_value(side["result"].clone()).unwrap();
        bundle.check_integrity().expect("exported bundle must pass integrity");
        let sections = &bundle.manifest.sections;
        assert_eq!(sections["requests"].schema_version, 3);
        assert_eq!(sections["quotes"].schema_version, 3);
        assert_eq!(sections["agreements"].schema_version, 3);
        assert_eq!(sections["cards"].schema_version, 3);
    }

    let bundle_val = exp_w["result"].clone();

    // Clean import succeeds
    let (iw, in_) = both_rpc(&h, "transaction.import", json!({ "bundle": bundle_val })).await;
    assert_eq!(iw, in_);
    assert_eq!(iw["result"]["imported"], true);

    // A tampered quotes section, with its own digest recomputed to match,
    // still fails the bundle's overall signed manifest (it was signed
    // over the untampered manifest) and refuses the whole import.
    let mut tampered_bundle: Bundle = serde_json::from_value(bundle_val.clone()).unwrap();
    let quote_rows = tampered_bundle.sections.get_mut("quotes").unwrap();
    let mut payload = quote_rows[0].get("payload").unwrap().clone();
    let env_str = payload["envelope"].as_str().unwrap();
    let mut env: Value = serde_json::from_str(env_str).unwrap();
    env["payload"]["terms"]["scope"] = json!("Tampered scope");
    payload["envelope"] = json!(env.to_string());
    quote_rows[0]["payload"] = payload;
    let new_digest = Bundle::digest(transaction_app::SCHEMA_VERSION, quote_rows).unwrap();
    tampered_bundle.manifest.sections.insert("quotes".to_string(), new_digest);

    let (tw, tn) = both_rpc(&h, "transaction.import", json!({ "bundle": tampered_bundle })).await;
    assert_eq!(tw, tn);
    assert_eq!(tw["error"]["code"], -32602);
    assert!(tw["error"]["message"].as_str().unwrap().contains("signed manifest"));

    // Same for a tampered agreement half.
    let mut tampered_bundle_agr: Bundle = serde_json::from_value(bundle_val).unwrap();
    let agr_rows = tampered_bundle_agr.sections.get_mut("agreements").unwrap();
    let mut agr_payload = agr_rows[0].get("payload").unwrap().clone();
    let consumer_env_str = agr_payload["consumer"]["envelope"].as_str().unwrap();
    let mut c_env: Value = serde_json::from_str(consumer_env_str).unwrap();
    c_env["payload"]["terms"]["scope"] = json!("Tampered agreement scope");
    agr_payload["consumer"]["envelope"] = json!(c_env.to_string());
    agr_rows[0]["payload"] = agr_payload;
    let new_agr_digest = Bundle::digest(transaction_app::SCHEMA_VERSION, agr_rows).unwrap();
    tampered_bundle_agr.manifest.sections.insert("agreements".to_string(), new_agr_digest);

    let (aw, an) =
        both_rpc(&h, "transaction.import", json!({ "bundle": tampered_bundle_agr })).await;
    assert_eq!(aw, an);
    assert_eq!(aw["error"]["code"], -32602);
    assert!(aw["error"]["message"].as_str().unwrap().contains("signed manifest"));
}

#[tokio::test]
async fn scenario_143_guard_transaction_verbs_local_admission() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (rw, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Guard request",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let req_id = rw["result"]["request_id"].as_str().unwrap().to_string();
    let req_rec_id = rw["result"]["record_id"].as_str().unwrap().to_string();

    let (rg, _) = both_rpc(&h, "request.get", json!({ "request_id": req_id })).await;
    let req_env = rg["result"]["envelope"].clone();

    let (peer_q_rec_id, peer_q_env) =
        peer_signed_quote(&conv, 1, &req_rec_id, &owner_did(), None, 1_000);
    let q_card = inbound_card(
        "m-guard-q",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &peer_q_env,
    );
    h.deliver(true, q_card.clone()).await;
    h.deliver(false, q_card).await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;

    let (exp, _) = both_rpc(&h, "transaction.export", json!({})).await;
    let bundle = exp["result"].clone();

    let calls: Vec<(&str, Value)> = vec![
        ("request.ping", json!({})),
        ("quote.ping", json!({})),
        ("agreement.ping", json!({})),
        ("receipt.ping", json!({})),
        (
            "request.set",
            json!({
                "conversation": conv,
                "description": "Another",
                "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
            }),
        ),
        ("request.get", json!({ "request_id": req_id })),
        ("request.list", json!({})),
        ("request.history", json!({ "request_id": req_id })),
        ("request.verify", json!({ "envelope": req_env })),
        (
            "quote.set",
            json!({
                "request_record_id": req_rec_id,
                "expires_in_secs": 3600,
                "terms": sample_quote_terms(),
            }),
        ),
        ("quote.get", json!({ "quote_id": "quo_dummy" })),
        ("quote.list", json!({})),
        ("quote.history", json!({ "quote_id": "quo_dummy" })),
        ("quote.verify", json!({ "envelope": peer_q_env })),
        ("quote.decline", json!({ "quote_record_id": peer_q_rec_id })),
        ("agreement.accept", json!({ "quote_record_id": peer_q_rec_id })),
        ("agreement.get", json!({ "quote_record_id": peer_q_rec_id })),
        ("agreement.list", json!({})),
        ("agreement.verify", json!({ "envelope": req_env })),
        ("transaction.sync", json!({ "conversation": conv })),
        ("transaction.thread", json!({ "conversation": conv })),
        ("transaction.export", json!({})),
        ("transaction.import", json!({ "bundle": bundle })),
        ("transaction.signing-status", json!({})),
        ("transaction.install-signing-certificate", json!({})),
    ];

    for (method, params) in calls {
        let (w, n) = both_rpc(&h, method, params).await;
        for (label, v) in [("wasm", &w), ("native", &n)] {
            let code = v["error"]["code"].as_i64();
            assert_ne!(code, Some(-32601), "{label} {method} answered method-not-found: {v}");
            assert_ne!(code, Some(-32013), "{label} {method} answered wire-refused: {v}");
        }
    }
}

#[tokio::test]
async fn scenario_144_every_transaction_verb_refused_over_wire_parity() {
    let h = harness().await;
    for method in [
        "request.ping",
        "quote.ping",
        "agreement.ping",
        "receipt.ping",
        "request.set",
        "request.get",
        "request.list",
        "request.history",
        "request.verify",
        "quote.set",
        "quote.get",
        "quote.list",
        "quote.history",
        "quote.verify",
        "quote.decline",
        "agreement.accept",
        "agreement.get",
        "agreement.list",
        "agreement.verify",
        "transaction.sync",
        "transaction.thread",
        "transaction.export",
        "transaction.import",
        "transaction.signing-status",
        "transaction.install-signing-certificate",
    ] {
        let (w, n) = h.wire_invoke(services::TRANSACTION, &env(method, json!({}))).await;
        assert_eq!(w, n, "{method}");
        assert_eq!(w["error"]["code"], -32013, "{method}: {w}");
    }
}

#[tokio::test]
async fn scenario_145_currency_unknown_refused_and_jpy_accepted_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    // 1. listing.set with "XYZ" is refused
    let mut listing_params = full_listing_params("unknown-cur-listing", "Unknown cur");
    listing_params["payment"]["currency"] = json!("XYZ");
    let (lw, ln) = both_rpc(&h, "listing.set", listing_params).await;
    assert_eq!(lw, ln);
    assert_eq!(lw["error"]["code"], -32602);
    assert!(lw["error"]["message"].as_str().unwrap().contains("XYZ"));

    // 2. quote.set with "XYZ" is refused
    let (req_rec_id, req_env) = peer_signed_request(&conv, 1, 1_000);
    let card_msg = inbound_card(
        "m-req-145",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_env,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;

    let mut xyz_terms = sample_quote_terms();
    xyz_terms["currency"] = json!("XYZ");
    let (qw_xyz, qn_xyz) = both_rpc(
        &h,
        "quote.set",
        json!({
            "request_record_id": req_rec_id,
            "expires_in_secs": 3600,
            "terms": xyz_terms,
        }),
    )
    .await;
    assert_eq!(qw_xyz, qn_xyz);
    assert_eq!(qw_xyz["error"]["code"], -32602);
    assert!(qw_xyz["error"]["message"].as_str().unwrap().contains("XYZ"));

    // 3. quote.set with "JPY" (minor exponent 0) is accepted and signed
    let mut jpy_terms = sample_quote_terms();
    jpy_terms["currency"] = json!("JPY");
    jpy_terms["amount_minor"] = json!(5000);
    jpy_terms["tax_minor"] = json!(0);
    jpy_terms["fees_minor"] = json!(0);
    let (qw_jpy, qn_jpy) = both_rpc(
        &h,
        "quote.set",
        json!({
            "request_record_id": req_rec_id,
            "expires_in_secs": 3600,
            "terms": jpy_terms,
        }),
    )
    .await;
    assert_eq!(qw_jpy["result"]["quote_id"], qn_jpy["result"]["quote_id"]);

    let q_id = qw_jpy["result"]["quote_id"].as_str().unwrap();
    let (gw, gn) = both_rpc(&h, "quote.get", json!({ "quote_id": q_id })).await;
    assert_eq!(gw["result"]["quote_id"], gn["result"]["quote_id"]);
    let q_data_w = Envelope::from_json(gw["result"]["envelope"].as_str().unwrap()).unwrap().payload;
    let q_data_n = Envelope::from_json(gn["result"]["envelope"].as_str().unwrap()).unwrap().payload;
    assert_eq!(q_data_w["terms"]["currency"], "JPY");
    assert_eq!(q_data_w["terms"]["amount_minor"], 5000);
    assert_eq!(q_data_n["terms"]["currency"], "JPY");
    assert_eq!(q_data_n["terms"]["amount_minor"], 5000);
}

#[tokio::test]
async fn scenario_146_peer_replaying_older_request_or_quote_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    // 1. Peer sends Request version 2 (newer timestamp 2_000)
    let (req_v2_rec_id, req_v2_env) = peer_signed_request_with(&conv, 1, 2_000, None, None);
    let card_msg_v2 = inbound_card(
        "m-req-146-v2",
        &conv,
        &peer_did(),
        2_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_v2_env,
    );
    h.deliver(true, card_msg_v2.clone()).await;
    h.deliver(false, card_msg_v2).await;
    let (sw1, sn1) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw1, sn1);
    assert_eq!(sw1["result"]["filed"], 1);
    assert_eq!(sw1["result"]["refused"], 0);

    // Peer replaying older Request version 1 (older timestamp 1_000, not
    // superseding) is refused
    let (req_v1_rec_id, req_v1_env) = peer_signed_request_with(&conv, 1, 1_000, None, None);
    assert_ne!(req_v1_rec_id, req_v2_rec_id);
    let card_msg_v1 = inbound_card(
        "m-req-146-v1",
        &conv,
        &peer_did(),
        2_100,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_v1_env,
    );
    h.deliver(true, card_msg_v1.clone()).await;
    h.deliver(false, card_msg_v1).await;
    let (sw2, sn2) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw2, sn2);
    assert_eq!(sw2["result"]["filed"], 1);
    assert_eq!(sw2["result"]["refused"], 1);

    let (tw, tn) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    assert_eq!(stripped(&tw), stripped(&tn));
    let cards = tw["result"]["cards"].as_array().unwrap();
    let refused_card = cards.iter().find(|c| c["message_id"] == "m-req-146-v1").unwrap();
    assert_eq!(refused_card["verified"], false);
    assert!(refused_card["reason"].as_str().unwrap().contains("newer or equal version"));

    // 2. Quote: node creates a request, then peer sends quotes answering it
    let (rw, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Replay quote test",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let own_req_rec_id = rw["result"]["record_id"].as_str().unwrap();

    let (q_v2_rec_id, q_v2_env) =
        peer_signed_quote_with(&conv, 1, own_req_rec_id, &owner_did(), Some(10_000), 2_000, None);
    let card_q_v2 = inbound_card(
        "m-quote-146-v2",
        &conv,
        &peer_did(),
        2_000,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_v2_env,
    );
    h.deliver(true, card_q_v2.clone()).await;
    h.deliver(false, card_q_v2).await;
    let (sw3, sn3) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw3, sn3);
    assert_eq!(sw3["result"]["filed"], 1);
    assert_eq!(sw3["result"]["refused"], 0);

    // Peer replaying older Quote version 1 (issued_at 1_000) is refused
    let (q_v1_rec_id, q_v1_env) =
        peer_signed_quote_with(&conv, 1, own_req_rec_id, &owner_did(), Some(10_000), 1_000, None);
    assert_ne!(q_v1_rec_id, q_v2_rec_id);
    let card_q_v1 = inbound_card(
        "m-quote-146-v1",
        &conv,
        &peer_did(),
        2_200,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_v1_env,
    );
    h.deliver(true, card_q_v1.clone()).await;
    h.deliver(false, card_q_v1).await;
    let (sw4, sn4) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw4, sn4);
    assert_eq!(sw4["result"]["filed"], 1);
    assert_eq!(sw4["result"]["refused"], 1);

    let (mut tw2, mut tn2) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    let cards2 = tw2["result"]["cards"].as_array().unwrap();
    let refused_quote_card = cards2.iter().find(|c| c["message_id"] == "m-quote-146-v1").unwrap();
    assert_eq!(refused_quote_card["verified"], false);
    assert!(refused_quote_card["reason"].as_str().unwrap().contains("newer or equal version"));

    strip_volatile(&mut tw2);
    strip_volatile(&mut tn2);
    normalize_message_ids(&mut tw2);
    normalize_message_ids(&mut tn2);
    assert_eq!(tw2, tn2);
}

#[tokio::test]
async fn scenario_147_replaying_declined_quote_preserves_decline_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (rw, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Decline replay job",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let req_rec_id = rw["result"]["record_id"].as_str().unwrap();

    let (q_rec_id, q_env) = peer_signed_quote(&conv, 1, req_rec_id, &owner_did(), None, 1_000);
    let card_msg = inbound_card(
        "m-quote-147",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_env,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;

    let (gw, gn) = both_rpc(&h, "quote.list", json!({ "conversation": conv })).await;
    assert_eq!(stripped(&gw), stripped(&gn));
    let quote_id = gw["result"]["quotes"][0]["quote_id"].as_str().unwrap();

    // Node declines the quote using quote_record_id
    let (dw, dn) = both_rpc(
        &h,
        "quote.decline",
        json!({
            "quote_record_id": q_rec_id,
            "note": "Price is too high",
        }),
    )
    .await;
    assert_eq!(dw, dn);
    assert_eq!(dw["result"]["declined"], true);

    // In thread, quote shows declined: Some(true)
    let (mut tw1, mut tn1) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    let cards1 = tw1["result"]["cards"].as_array().unwrap();
    let q_card1 = cards1.iter().find(|c| c["card_type"] == "quote").unwrap();
    assert_eq!(q_card1["declined"], true);

    strip_volatile(&mut tw1);
    strip_volatile(&mut tn1);
    normalize_message_ids(&mut tw1);
    normalize_message_ids(&mut tn1);
    assert_eq!(tw1, tn1);

    // Peer replays the exact same quote card in a new message
    let card_msg_replay = inbound_card(
        "m-quote-147-replay",
        &conv,
        &peer_did(),
        1_100,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_env,
    );
    h.deliver(true, card_msg_replay.clone()).await;
    h.deliver(false, card_msg_replay).await;
    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw, sn);

    // Quote pointer row still has declined_at_secs and note. quote.get and
    // quote.list both carry the pointer row's own updated_at_secs (see
    // strip_volatile), so this is a stripped comparison, not a raw one.
    let (qg_w, qg_n) = both_rpc(&h, "quote.get", json!({ "quote_id": quote_id })).await;
    assert_eq!(stripped(&qg_w), stripped(&qg_n));
    assert!(qg_w["result"]["declined_at_secs"].is_number());
    assert_eq!(qg_w["result"]["decline_note"], "Price is too high");

    // Thread still shows declined: Some(true) on the quote
    let (mut tw2, mut tn2) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    let cards2 = tw2["result"]["cards"].as_array().unwrap();
    let q_cards: Vec<_> = cards2.iter().filter(|c| c["card_type"] == "quote").collect();
    for qc in q_cards {
        assert_eq!(qc["declined"], true);
    }

    strip_volatile(&mut tw2);
    strip_volatile(&mut tn2);
    normalize_message_ids(&mut tw2);
    normalize_message_ids(&mut tn2);
    assert_eq!(tw2, tn2);
}

#[expect(clippy::too_many_lines, reason = "linear transaction cards expiry scenario")]
#[tokio::test]
async fn scenario_148_request_or_receipt_with_expiry_is_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    // 1. Peer sends Request with expires_at_secs: Some(...)
    let (_, req_env) = peer_signed_request_with(&conv, 1, 1_000, Some(5_000), None);
    let card_req = inbound_card(
        "m-req-148",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_env,
    );
    h.deliver(true, card_req.clone()).await;
    h.deliver(false, card_req).await;
    let (sw1, sn1) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw1, sn1);
    assert_eq!(sw1["result"]["filed"], 1);
    assert_eq!(sw1["result"]["refused"], 1);

    let (mut tw1, mut tn1) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    let cards1 = tw1["result"]["cards"].as_array().unwrap();
    let req_card = cards1.iter().find(|c| c["message_id"] == "m-req-148").unwrap();
    assert_eq!(req_card["verified"], false);
    assert!(req_card["reason"].as_str().unwrap().contains("may not declare an expiry"));

    strip_volatile(&mut tw1);
    strip_volatile(&mut tn1);
    normalize_message_ids(&mut tw1);
    normalize_message_ids(&mut tn1);
    assert_eq!(tw1, tn1);

    // 2. Setup a valid request and quote so we can test peer sending receipt with
    //    expiry
    let (rw, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Receipt expiry test",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let valid_req_id = rw["result"]["record_id"].as_str().unwrap();

    let (q_rec_id, q_env) = peer_signed_quote(&conv, 2, valid_req_id, &owner_did(), None, 1_000);
    let card_q = inbound_card(
        "m-quote-148",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_env,
    );
    h.deliver(true, card_q.clone()).await;
    h.deliver(false, card_q).await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;

    // Peer sends agreement-receipt with expires_at_secs set
    let qg = both_rpc(&h, "quote.list", json!({ "conversation": conv })).await.0;
    let env_json = qg["result"]["quotes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|q| q["record_id"] == q_rec_id)
        .unwrap()["envelope"]
        .as_str()
        .unwrap();
    let quote_envelope = Envelope::from_json(env_json).unwrap();
    let terms = quote_envelope.payload["terms"].clone();

    let (_, peer_receipt_env) = peer_signed_consumer_receipt_with_expiry(
        &conv,
        &q_rec_id,
        &peer_did(),
        &owner_did(),
        terms,
        1_050,
        Some(10_000),
    );
    let card_receipt = inbound_card(
        "m-receipt-148",
        &conv,
        &peer_did(),
        1_050,
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &peer_receipt_env,
    );
    h.deliver(true, card_receipt.clone()).await;
    h.deliver(false, card_receipt).await;
    let (sw2, sn2) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw2, sn2);
    assert_eq!(sw2["result"]["filed"], 1);
    assert_eq!(sw2["result"]["refused"], 1);

    let (mut tw2, mut tn2) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    let cards2 = tw2["result"]["cards"].as_array().unwrap();
    let rec_card = cards2.iter().find(|c| c["message_id"] == "m-receipt-148").unwrap();
    assert_eq!(rec_card["verified"], false);
    assert!(rec_card["reason"].as_str().unwrap().contains("may not declare an expiry"));

    strip_volatile(&mut tw2);
    strip_volatile(&mut tn2);
    normalize_message_ids(&mut tw2);
    normalize_message_ids(&mut tn2);
    assert_eq!(tw2, tn2);

    let (gw, gn) = both_rpc(&h, "agreement.get", json!({ "quote_record_id": q_rec_id })).await;
    assert_eq!(gw, gn);
    assert!(gw["result"].is_null());
}

#[tokio::test]
async fn scenario_149_quote_naming_wrong_consumer_did_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (rw, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Wrong consumer DID test",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let req_rec_id = rw["result"]["record_id"].as_str().unwrap();

    // Peer creates quote for req_rec_id, but sets consumer_did to a stranger DID
    let stranger_did = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
    let (q_rec_id, q_env) = peer_signed_quote(&conv, 1, req_rec_id, stranger_did, None, 1_000);
    let card_msg = inbound_card(
        "m-quote-149",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &q_env,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;
    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw, sn);
    assert_eq!(sw["result"]["filed"], 1);
    assert_eq!(sw["result"]["refused"], 1);

    let (mut tw, mut tn) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    let cards = tw["result"]["cards"].as_array().unwrap();
    let q_card = cards.iter().find(|c| c["message_id"] == "m-quote-149").unwrap();
    assert_eq!(q_card["verified"], false);
    assert!(
        q_card["reason"].as_str().unwrap().contains("consumer_did does not match request issuer")
    );

    strip_volatile(&mut tw);
    strip_volatile(&mut tn);
    normalize_message_ids(&mut tw);
    normalize_message_ids(&mut tn);
    assert_eq!(tw, tn);

    // Attempting to accept the quote fails with -32602
    let (aw, an) = both_rpc(&h, "agreement.accept", json!({ "quote_record_id": q_rec_id })).await;
    assert_eq!(aw, an);
    assert_eq!(aw["error"]["code"], -32602);
}

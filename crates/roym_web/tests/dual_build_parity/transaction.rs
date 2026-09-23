use serde_json::{Value, json};
use syneroym_roym_core::{card, transaction};
use syneroym_rpc::{ConversationDeliveryState, ConversationMessage};
use syneroym_signed_record::Envelope;

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_122_transaction_certificate_verbs_parity() {
    let h = harness().await;
    let (w, n) = both_rpc(&h, "transaction.signing-status", json!({})).await;
    assert_eq!(w["result"]["certificate"]["state"], "missing");
    assert_eq!(n["result"]["certificate"]["state"], "missing");
    enrol_signing(&h, "transaction").await;
    let (w2, n2) = both_rpc(&h, "transaction.signing-status", json!({})).await;
    assert_eq!(w2["result"]["certificate"]["state"], "installed");
    assert_eq!(n2["result"]["certificate"]["state"], "installed");
}

#[tokio::test]
async fn scenario_123_request_set_signs_stores_and_sends_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let params = json!({
        "conversation": conv,
        "description": "Install new fence",
        "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
    });
    let (w, n) = both_rpc(&h, "request.set", params).await;
    assert_eq!(w["result"]["request_id"], n["result"]["request_id"]);
    assert_eq!(w["result"]["record_id"], n["result"]["record_id"]);
    assert_eq!(w["result"]["state"], n["result"]["state"]);
    let req_id = w["result"]["request_id"].as_str().unwrap().to_string();

    let expected_req_id = transaction::derive_request_id(&conv, &owner_did(), 1).unwrap();
    assert_eq!(req_id, expected_req_id);

    let (gw, gn) = both_rpc(&h, "request.get", json!({ "request_id": req_id })).await;
    assert_eq!(gw["result"]["envelope"], gn["result"]["envelope"]);
    let env_str = gw["result"]["envelope"].as_str().unwrap();

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    normalize_message_ids(&mut hw);
    normalize_message_ids(&mut hn);
    assert_eq!(hw, hn);
    let msgs = hw["result"]["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["content_type"], card::CARD_CONTENT_TYPE);
    let card: card::Card =
        serde_json::from_slice(msgs[0]["body"].as_str().unwrap().as_bytes()).unwrap();
    assert_eq!(card.card_type, transaction::RECORD_REQUEST);
    assert_eq!(card.envelope, env_str);
}

#[tokio::test]
async fn scenario_124_request_set_without_enrolment_answers_not_enrolled_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    let conv = open_conv(&h, &peer_did()).await;

    let params = json!({
        "conversation": conv,
        "description": "Unenrolled request",
        "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
    });
    let (w, n) = both_rpc(&h, "request.set", params).await;
    assert_eq!(w, n);
    assert_eq!(w["error"]["code"], -32602);
    assert!(w["error"]["message"].as_str().unwrap().contains("signing-not-enrolled"));
}

#[tokio::test]
async fn scenario_125_request_set_with_id_produces_superseding_version_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let params1 = json!({
        "conversation": conv,
        "description": "Initial fence proposal",
        "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
    });
    let (w1, n1) = both_rpc(&h, "request.set", params1).await;
    assert_eq!(w1["result"]["request_id"], n1["result"]["request_id"]);
    let req_id = w1["result"]["request_id"].as_str().unwrap().to_string();
    let rec_id_1 = w1["result"]["record_id"].as_str().unwrap().to_string();
    assert_eq!(w1["result"]["version_count"], 1);

    let params2 = json!({
        "conversation": conv,
        "request_id": req_id,
        "description": "Revised fence proposal with painted finish",
        "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
    });
    let (w2, n2) = both_rpc(&h, "request.set", params2).await;
    assert_eq!(w2["result"]["request_id"], n2["result"]["request_id"]);
    assert_eq!(w2["result"]["request_id"], req_id);
    assert_eq!(w2["result"]["version_count"], 2);
    let rec_id_2 = w2["result"]["record_id"].as_str().unwrap().to_string();
    assert_ne!(rec_id_1, rec_id_2);

    let (gw, gn) = both_rpc(&h, "request.get", json!({ "request_id": req_id })).await;
    assert_eq!(gw["result"]["envelope"], gn["result"]["envelope"]);
    let env2 = Envelope::from_json(gw["result"]["envelope"].as_str().unwrap()).unwrap();
    assert_eq!(env2.supersedes, Some(rec_id_1.clone()));

    let (hw, hn) = both_rpc(&h, "request.history", json!({ "request_id": req_id })).await;
    assert_eq!(stripped(&hw), stripped(&hn));
    let hist = hw["result"]["history"].as_array().unwrap();
    assert_eq!(hist.len(), 2);
}

#[tokio::test]
async fn scenario_126_quote_set_refuses_own_request_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let params = json!({
        "conversation": conv,
        "description": "My own job request",
        "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
    });
    let (w, _) = both_rpc(&h, "request.set", params).await;
    let rec_id = w["result"]["record_id"].as_str().unwrap().to_string();

    let quote_params = json!({
        "request_record_id": rec_id,
        "expires_in_secs": 3600,
        "terms": sample_quote_terms(),
    });
    let (qw, qn) = both_rpc(&h, "quote.set", quote_params).await;
    assert_eq!(qw, qn);
    assert_eq!(qw["error"]["code"], -32602);
    assert!(
        qw["error"]["message"]
            .as_str()
            .unwrap()
            .contains("a request cannot be quoted by the person who made it")
    );
}

#[tokio::test]
async fn scenario_127_peer_request_card_is_filed_by_sync_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (rec_id, env_json) = peer_signed_request(&conv, 1, 1_000);
    let card_msg = inbound_card(
        "m-peer-req-127",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &env_json,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;

    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw, sn);
    assert_eq!(sw["result"]["filed"], 1);

    let (tw, tn) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    assert_eq!(stripped(&tw), stripped(&tn));
    let cards = tw["result"]["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["card_type"], transaction::RECORD_REQUEST);
    assert_eq!(cards[0]["verified"], true);
    assert_eq!(cards[0]["issuer"], peer_did());
    assert_eq!(cards[0]["record_id"], rec_id);
}

#[tokio::test]
async fn scenario_128_quote_set_against_filed_request_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (req_rec_id, env_json) = peer_signed_request(&conv, 1, 1_000);
    let card_msg = inbound_card(
        "m-peer-req-128",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &env_json,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;

    let quote_params = json!({
        "request_record_id": req_rec_id,
        "expires_in_secs": 3600,
        "terms": sample_quote_terms(),
    });
    let (qw, qn) = both_rpc(&h, "quote.set", quote_params).await;
    assert_eq!(qw["result"]["quote_id"], qn["result"]["quote_id"]);
    let q_id = qw["result"]["quote_id"].as_str().unwrap().to_string();

    let (gw, gn) = both_rpc(&h, "quote.get", json!({ "quote_id": q_id })).await;
    assert_eq!(gw["result"]["quote_id"], gn["result"]["quote_id"]);
    assert_eq!(gw["result"]["consumer_did"], peer_did());
    assert_eq!(gn["result"]["consumer_did"], peer_did());
    let env_w = Envelope::from_json(gw["result"]["envelope"].as_str().unwrap()).unwrap();
    let env_n = Envelope::from_json(gn["result"]["envelope"].as_str().unwrap()).unwrap();
    assert!(env_w.expires_at_secs.is_some());
    assert!(env_n.expires_at_secs.is_some());
    assert!(env_w.expires_at_secs.unwrap() > env_w.issued_at_secs);
    assert!(env_n.expires_at_secs.unwrap() > env_n.issued_at_secs);

    let (hw, hn) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    let msgs_w = hw["result"]["messages"].as_array().unwrap();
    let msgs_n = hn["result"]["messages"].as_array().unwrap();
    assert!(
        msgs_w
            .iter()
            .any(|m| m["content_type"] == card::CARD_CONTENT_TYPE && m["direction"] == "outgoing")
    );
    assert!(
        msgs_n
            .iter()
            .any(|m| m["content_type"] == card::CARD_CONTENT_TYPE && m["direction"] == "outgoing")
    );
}

#[tokio::test]
async fn scenario_129_agreement_accept_provider_and_consumer_halves_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    // Part A: Provider accepts its own quote
    let (req_rec_id, req_env) = peer_signed_request(&conv, 1, 1_000);
    let card_msg = inbound_card(
        "m-peer-req-129",
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

    let quote_params = json!({
        "request_record_id": req_rec_id,
        "expires_in_secs": 3600,
        "terms": sample_quote_terms(),
    });
    let (qw, qn) = both_rpc(&h, "quote.set", quote_params).await;
    let q_rec_id_w = qw["result"]["record_id"].as_str().unwrap().to_string();
    let q_rec_id_n = qn["result"]["record_id"].as_str().unwrap().to_string();

    let mut aw =
        one_rpc(&h, true, "agreement.accept", json!({ "quote_record_id": q_rec_id_w })).await;
    let mut an =
        one_rpc(&h, false, "agreement.accept", json!({ "quote_record_id": q_rec_id_n })).await;
    // agreement.accept mints its own receipt envelope with
    // clock::now_secs() (unlike a listing's pinned signing clock), so
    // record_id/quote_record_id -- content hashes over that envelope --
    // and agreement_record_id/message_id are each build's own values,
    // never equal to the other build's.
    if let Some(res) = aw.get_mut("result").and_then(Value::as_object_mut) {
        res.remove("message_id");
        res.remove("agreement_record_id");
        res.remove("record_id");
        res.remove("quote_record_id");
    }
    if let Some(res) = an.get_mut("result").and_then(Value::as_object_mut) {
        res.remove("message_id");
        res.remove("agreement_record_id");
        res.remove("record_id");
        res.remove("quote_record_id");
    }
    assert_eq!(stripped(&aw), stripped(&an));
    assert_eq!(aw["result"]["role"], "provider");
    assert_eq!(aw["result"]["pair"]["state"], "half");

    let gw = one_rpc(&h, true, "agreement.get", json!({ "quote_record_id": q_rec_id_w })).await;
    let gn = one_rpc(&h, false, "agreement.get", json!({ "quote_record_id": q_rec_id_n })).await;
    assert_eq!(gw["result"]["pair"]["state"], "half");
    assert_eq!(gn["result"]["pair"]["state"], "half");
    assert!(gw["result"]["provider"].is_object());
    assert!(gn["result"]["provider"].is_object());
    assert!(gw["result"]["consumer"].is_null());
    assert!(gn["result"]["consumer"].is_null());

    // Part B: Consumer accepts peer-issued quote
    let (my_req_w, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Another request",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let my_req_rec_id = my_req_w["result"]["record_id"].as_str().unwrap();

    let (peer_q_rec_id, peer_q_env) =
        peer_signed_quote(&conv, 1, my_req_rec_id, &owner_did(), None, 1_000);
    let q_card_msg = inbound_card(
        "m-peer-quote-129",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &peer_q_env,
    );
    h.deliver(true, q_card_msg.clone()).await;
    h.deliver(false, q_card_msg).await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;

    let (mut cw, mut cn) =
        both_rpc(&h, "agreement.accept", json!({ "quote_record_id": peer_q_rec_id })).await;
    if let Some(res) = cw.get_mut("result").and_then(Value::as_object_mut) {
        res.remove("message_id");
        res.remove("record_id");
    }
    if let Some(res) = cn.get_mut("result").and_then(Value::as_object_mut) {
        res.remove("message_id");
        res.remove("record_id");
    }
    assert_eq!(stripped(&cw), stripped(&cn));
    assert_eq!(cw["result"]["role"], "consumer");
    assert_eq!(cw["result"]["pair"]["state"], "half");

    // Not a full-struct comparison: pair.consumer is a ReceiptHalf whose
    // own record_id/envelope/issued_at_secs are each build's own
    // clock::now_secs()-derived values, same as above.
    let (cgw, cgn) =
        both_rpc(&h, "agreement.get", json!({ "quote_record_id": peer_q_rec_id })).await;
    assert_eq!(cgw["result"]["pair"]["state"], "half");
    assert_eq!(cgn["result"]["pair"]["state"], "half");
    assert!(cgw["result"]["consumer"].is_object());
    assert!(cgn["result"]["consumer"].is_object());
    assert!(cgw["result"]["provider"].is_null());
    assert!(cgn["result"]["provider"].is_null());
}

#[tokio::test]
async fn scenario_130_full_agreement_pair_countersigns_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    for wasm in [true, false] {
        let conv = open_conv(&h, &peer_did()).await;

        // Node issues quote to peer
        let (req_rec_id, req_env) = peer_signed_request(&conv, 1, 1_000);
        let card_msg = inbound_card(
            &format!("m-req-130-{wasm}"),
            &conv,
            &peer_did(),
            1_000,
            transaction::RECORD_REQUEST,
            transaction::REQUEST_VERSION,
            &req_env,
        );
        h.deliver(wasm, card_msg).await;
        one_rpc(&h, wasm, "transaction.sync", json!({ "conversation": conv })).await;

        let quote_params = json!({
            "request_record_id": req_rec_id,
            "expires_in_secs": 3600,
            "terms": sample_quote_terms(),
        });
        let q = one_rpc(&h, wasm, "quote.set", quote_params).await;
        let q_rec_id = q["result"]["record_id"].as_str().unwrap().to_string();

        // Read terms from quote
        let qg =
            one_rpc(&h, wasm, "quote.get", json!({ "quote_id": q["result"]["quote_id"] })).await;
        let env = Envelope::from_json(qg["result"]["envelope"].as_str().unwrap()).unwrap();
        let terms = env.payload["terms"].clone();

        // Peer signs consumer receipt half
        let (_, peer_receipt_env) =
            peer_signed_consumer_receipt(&conv, &q_rec_id, &peer_did(), &owner_did(), terms, 1_100);
        let receipt_card = inbound_card(
            &format!("m-receipt-130-{wasm}"),
            &conv,
            &peer_did(),
            1_100,
            transaction::RECORD_AGREEMENT_RECEIPT,
            transaction::AGREEMENT_RECEIPT_VERSION,
            &peer_receipt_env,
        );
        h.deliver(wasm, receipt_card).await;

        // Sync files it and countersigns!
        let s = one_rpc(&h, wasm, "transaction.sync", json!({ "conversation": conv })).await;
        assert_eq!(s["result"]["filed"], 1);
        assert_eq!(s["result"]["countersigned"], 1);

        // Agreement is complete!
        let g = one_rpc(&h, wasm, "agreement.get", json!({ "quote_record_id": q_rec_id })).await;
        assert_eq!(g["result"]["pair"]["state"], "complete");
        let c_half = &g["result"]["consumer"];
        let p_half = &g["result"]["provider"];
        assert!(c_half.is_object());
        assert!(p_half.is_object());

        // Both halves payloads differ only in role
        let c_env = Envelope::from_json(c_half["envelope"].as_str().unwrap()).unwrap();
        let p_env = Envelope::from_json(p_half["envelope"].as_str().unwrap()).unwrap();
        let mut c_payload = c_env.payload.clone();
        let mut p_payload = p_env.payload.clone();
        assert_eq!(c_payload["role"], "consumer");
        assert_eq!(p_payload["role"], "provider");
        c_payload["role"] = json!("same");
        p_payload["role"] = json!("same");
        assert_eq!(c_payload, p_payload);
    }
}

#[tokio::test]
async fn scenario_131_consumer_half_terms_differ_filed_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    for wasm in [true, false] {
        let conv = open_conv(&h, &peer_did()).await;

        let (req_rec_id, req_env) = peer_signed_request(&conv, 1, 1_000);
        let card_msg = inbound_card(
            &format!("m-req-131-{wasm}"),
            &conv,
            &peer_did(),
            1_000,
            transaction::RECORD_REQUEST,
            transaction::REQUEST_VERSION,
            &req_env,
        );
        h.deliver(wasm, card_msg).await;
        one_rpc(&h, wasm, "transaction.sync", json!({ "conversation": conv })).await;

        let quote_params = json!({
            "request_record_id": req_rec_id,
            "expires_in_secs": 3600,
            "terms": sample_quote_terms(),
        });
        let q = one_rpc(&h, wasm, "quote.set", quote_params).await;
        let q_rec_id = q["result"]["record_id"].as_str().unwrap().to_string();

        let qg =
            one_rpc(&h, wasm, "quote.get", json!({ "quote_id": q["result"]["quote_id"] })).await;
        let env = Envelope::from_json(qg["result"]["envelope"].as_str().unwrap()).unwrap();
        let mut terms = env.payload["terms"].clone();
        terms["amount_minor"] = json!(99999);

        let (_, peer_receipt_env) =
            peer_signed_consumer_receipt(&conv, &q_rec_id, &peer_did(), &owner_did(), terms, 1_100);
        let receipt_card = inbound_card(
            &format!("m-receipt-131-{wasm}"),
            &conv,
            &peer_did(),
            1_100,
            transaction::RECORD_AGREEMENT_RECEIPT,
            transaction::AGREEMENT_RECEIPT_VERSION,
            &peer_receipt_env,
        );
        h.deliver(wasm, receipt_card).await;

        let s = one_rpc(&h, wasm, "transaction.sync", json!({ "conversation": conv })).await;
        assert_eq!(s["result"]["refused"], 1);
        assert_eq!(s["result"]["countersigned"], 0);

        let t = one_rpc(&h, wasm, "transaction.thread", json!({ "conversation": conv })).await;
        let cards = t["result"]["cards"].as_array().unwrap();
        let refused =
            cards.iter().find(|c| c["message_id"] == format!("m-receipt-131-{wasm}")).unwrap();
        assert_eq!(refused["verified"], false);

        let g = one_rpc(&h, wasm, "agreement.get", json!({ "quote_record_id": q_rec_id })).await;
        assert!(g["result"].is_null());
    }
}

#[tokio::test]
async fn scenario_132_consumer_half_from_stranger_filed_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    for wasm in [true, false] {
        let conv = open_conv(&h, &peer_did()).await;

        let (req_rec_id, req_env) = peer_signed_request(&conv, 1, 1_000);
        let card_msg = inbound_card(
            &format!("m-req-132-{wasm}"),
            &conv,
            &peer_did(),
            1_000,
            transaction::RECORD_REQUEST,
            transaction::REQUEST_VERSION,
            &req_env,
        );
        h.deliver(wasm, card_msg).await;
        one_rpc(&h, wasm, "transaction.sync", json!({ "conversation": conv })).await;

        let quote_params = json!({
            "request_record_id": req_rec_id,
            "expires_in_secs": 3600,
            "terms": sample_quote_terms(),
        });
        let q = one_rpc(&h, wasm, "quote.set", quote_params).await;
        let q_rec_id = q["result"]["record_id"].as_str().unwrap().to_string();

        let qg =
            one_rpc(&h, wasm, "quote.get", json!({ "quote_id": q["result"]["quote_id"] })).await;
        let env = Envelope::from_json(qg["result"]["envelope"].as_str().unwrap()).unwrap();
        let terms = env.payload["terms"].clone();

        let stranger_did = "did:key:zStranger132";
        let (_, receipt_env) = peer_signed_consumer_receipt(
            &conv,
            &q_rec_id,
            stranger_did,
            &owner_did(),
            terms,
            1_100,
        );
        let receipt_card = inbound_card(
            &format!("m-receipt-132-{wasm}"),
            &conv,
            &peer_did(),
            1_100,
            transaction::RECORD_AGREEMENT_RECEIPT,
            transaction::AGREEMENT_RECEIPT_VERSION,
            &receipt_env,
        );
        h.deliver(wasm, receipt_card).await;

        let s = one_rpc(&h, wasm, "transaction.sync", json!({ "conversation": conv })).await;
        assert_eq!(s["result"]["refused"], 1);

        let t = one_rpc(&h, wasm, "transaction.thread", json!({ "conversation": conv })).await;
        let cards = t["result"]["cards"].as_array().unwrap();
        let refused =
            cards.iter().find(|c| c["message_id"] == format!("m-receipt-132-{wasm}")).unwrap();
        assert_eq!(refused["verified"], false);

        let g = one_rpc(&h, wasm, "agreement.get", json!({ "quote_record_id": q_rec_id })).await;
        assert!(g["result"].is_null());
    }
}

#[tokio::test]
async fn scenario_133_expired_quote_card_filed_verified_with_terms_accept_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (my_req_w, _) = both_rpc(
        &h,
        "request.set",
        json!({
            "conversation": conv,
            "description": "Window cleaning",
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await;
    let my_req_rec_id = my_req_w["result"]["record_id"].as_str().unwrap();

    let (q_rec_id, q_env) =
        peer_signed_quote(&conv, 1, my_req_rec_id, &owner_did(), Some(500), 100);
    let card_msg = inbound_card(
        "m-expired-quote-133",
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

    let (mut tw, mut tn) =
        both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    strip_volatile(&mut tw);
    strip_volatile(&mut tn);
    normalize_message_ids(&mut tw);
    normalize_message_ids(&mut tn);
    assert_eq!(tw, tn);
    let cards = tw["result"]["cards"].as_array().unwrap();
    let q_card = cards.iter().find(|c| c["card_type"] == "quote").unwrap();
    assert_eq!(q_card["verified"], true);
    assert_eq!(q_card["expired"], true);
    assert!(q_card["data"]["terms"].is_object());

    let (aw, an) = both_rpc(&h, "agreement.accept", json!({ "quote_record_id": q_rec_id })).await;
    assert_eq!(aw, an);
    assert_eq!(aw["error"]["code"], -32602);
    assert!(aw["error"]["message"].as_str().unwrap().contains("quote-expired"));

    let (agw, agn) = both_rpc(&h, "agreement.get", json!({ "quote_record_id": q_rec_id })).await;
    assert_eq!(stripped(&agw), stripped(&agn));
    assert!(agw["result"].is_null());
    assert!(agn["result"].is_null());
}

#[tokio::test]
async fn scenario_133b_completed_pair_survives_quote_expiry_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    for wasm in [true, false] {
        let conv = open_conv(&h, &peer_did()).await;

        let (req_rec_id, req_env) = peer_signed_request(&conv, 1, 1_000);
        let card_msg = inbound_card(
            &format!("m-req-133b-{wasm}"),
            &conv,
            &peer_did(),
            1_000,
            transaction::RECORD_REQUEST,
            transaction::REQUEST_VERSION,
            &req_env,
        );
        h.deliver(wasm, card_msg).await;
        one_rpc(&h, wasm, "transaction.sync", json!({ "conversation": conv })).await;

        let quote_params = json!({
            "request_record_id": req_rec_id,
            "expires_in_secs": 3600,
            "terms": sample_quote_terms(),
        });
        let q = one_rpc(&h, wasm, "quote.set", quote_params).await;
        let q_rec_id = q["result"]["record_id"].as_str().unwrap().to_string();

        let qg =
            one_rpc(&h, wasm, "quote.get", json!({ "quote_id": q["result"]["quote_id"] })).await;
        let env = Envelope::from_json(qg["result"]["envelope"].as_str().unwrap()).unwrap();
        let terms = env.payload["terms"].clone();

        let (_, peer_receipt_env) =
            peer_signed_consumer_receipt(&conv, &q_rec_id, &peer_did(), &owner_did(), terms, 1_050);
        let receipt_card = inbound_card(
            &format!("m-receipt-133b-{wasm}"),
            &conv,
            &peer_did(),
            1_050,
            transaction::RECORD_AGREEMENT_RECEIPT,
            transaction::AGREEMENT_RECEIPT_VERSION,
            &peer_receipt_env,
        );
        h.deliver(wasm, receipt_card).await;
        one_rpc(&h, wasm, "transaction.sync", json!({ "conversation": conv })).await;

        let g = one_rpc(&h, wasm, "agreement.get", json!({ "quote_record_id": q_rec_id })).await;
        assert_eq!(g["result"]["pair"]["state"], "complete");
        assert!(g["result"]["terms"].is_object());
    }
}

#[tokio::test]
async fn scenario_134_sync_is_idempotent_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (_, req_env) = peer_signed_request(&conv, 1, 1_000);
    let card_msg = inbound_card(
        "m-req-134",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_env,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;

    let (sw1, sn1) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw1, sn1);
    assert_eq!(sw1["result"]["filed"], 1);

    let (sw2, sn2) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw2, sn2);
    assert_eq!(sw2["result"]["filed"], 0);

    let (sw3, sn3) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw3, sn3);
    assert_eq!(sw3["result"]["filed"], 0);

    let (tw, tn) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    assert_eq!(stripped(&tw), stripped(&tn));
    assert_eq!(tw["result"]["cards"].as_array().unwrap().len(), 1);

    let (rw, rn) = both_rpc(&h, "request.list", json!({ "conversation": conv })).await;
    assert_eq!(stripped(&rw), stripped(&rn));
    assert_eq!(rw["result"]["requests"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_135_sync_full_rescan_files_nothing_new_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (_, req_env) = peer_signed_request(&conv, 1, 1_000);
    let card_msg = inbound_card(
        "m-req-135",
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

    let (sw, sn) =
        both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    assert_eq!(sw, sn);
    assert_eq!(sw["result"]["filed"], 0);
    assert!(sw["result"]["scanned"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn scenario_136_card_declared_quote_with_request_envelope_filed_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let (_, req_env) = peer_signed_request(&conv, 1, 1_000);
    let card_msg = inbound_card(
        "m-mismatch-136",
        &conv,
        &peer_did(),
        1_000,
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &req_env,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;

    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw, sn);
    assert_eq!(sw["result"]["refused"], 1);

    let (tw, tn) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    assert_eq!(stripped(&tw), stripped(&tn));
    let cards = tw["result"]["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["verified"], false);
}

#[tokio::test]
async fn scenario_137_unknown_card_type_filed_unknown_unverified_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let raw_body = json!({
        "card_version": 1,
        "type": "nonexistent-fancy-card",
        "version": 1,
        "envelope": "{}",
    })
    .to_string();
    let msg = ConversationMessage {
        id: "m-unknown-137".to_string(),
        conversation: conv.clone(),
        author: peer_did(),
        sender_timestamp: 1_000,
        received_at: 1_000,
        content_type: card::CARD_CONTENT_TYPE.to_string(),
        body: raw_body.into_bytes(),
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    };
    h.deliver(true, msg.clone()).await;
    h.deliver(false, msg).await;

    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw, sn);
    assert_eq!(sw["result"]["unknown"], 1);

    let (tw, tn) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    assert_eq!(stripped(&tw), stripped(&tn));
    let cards = tw["result"]["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["known"], false);
    assert_eq!(cards[0]["verified"], false);
    assert!(cards[0]["data"].is_null());
}

#[tokio::test]
async fn scenario_138_known_card_type_without_producer_filed_unverified_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;

    let raw_body = json!({
        "card_version": 1,
        "type": "payment-request",
        "version": 1,
        "envelope": "{}",
    })
    .to_string();
    let msg = ConversationMessage {
        id: "m-no-producer-138".to_string(),
        conversation: conv.clone(),
        author: peer_did(),
        sender_timestamp: 1_000,
        received_at: 1_000,
        content_type: card::CARD_CONTENT_TYPE.to_string(),
        body: raw_body.into_bytes(),
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    };
    h.deliver(true, msg.clone()).await;
    h.deliver(false, msg).await;

    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv })).await;
    assert_eq!(sw, sn);
    assert_eq!(sw["result"]["refused"], 1);

    let (tw, tn) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv })).await;
    assert_eq!(stripped(&tw), stripped(&tn));
    let cards = tw["result"]["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["known"], true);
    assert_eq!(cards[0]["verified"], false);
    assert!(cards[0]["reason"].as_str().unwrap().contains("no producer"));
}

#[tokio::test]
async fn scenario_139_card_payload_names_different_conversation_filed_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv_a = open_conv(&h, &peer_did()).await;
    let conv_b = "conv-other-139";

    let (_, req_env) = peer_signed_request(conv_b, 1, 1_000);
    let card_msg = inbound_card(
        "m-wrong-conv-139",
        &conv_a,
        &peer_did(),
        1_000,
        transaction::RECORD_REQUEST,
        transaction::REQUEST_VERSION,
        &req_env,
    );
    h.deliver(true, card_msg.clone()).await;
    h.deliver(false, card_msg).await;

    let (sw, sn) = both_rpc(&h, "transaction.sync", json!({ "conversation": conv_a })).await;
    assert_eq!(sw, sn);
    assert_eq!(sw["result"]["refused"], 1);

    let (tw, tn) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv_a })).await;
    assert_eq!(stripped(&tw), stripped(&tn));
    let cards = tw["result"]["cards"].as_array().unwrap();
    assert_eq!(cards[0]["verified"], false);
    assert!(cards[0]["reason"].as_str().unwrap().contains("conversation"));
}

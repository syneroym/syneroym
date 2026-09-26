use serde_json::{Value, json};
use syneroym_data_db::host_store::RecordWriteValue;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_roym_core::{
    backup::{BUNDLE_MANIFEST_VERSION, Bundle},
    card,
    record::RECORD_BUNDLE_MANIFEST,
    transaction,
};
use syneroym_roym_transaction::app as transaction_app;
use syneroym_rpc::{ConversationDeliveryState, ConversationMessage};
use syneroym_signed_record::{Envelope, RecordDraft};

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
    let mut tampered_bundle_agr: Bundle = serde_json::from_value(bundle_val.clone()).unwrap();
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

    assert_per_record_tamper_refused(&h, &bundle_val).await;
}

fn resign_bundle(bundle: &mut Bundle, owner: &Identity, now_secs: u64) {
    let payload = serde_json::to_value(&bundle.manifest).unwrap();
    let draft = RecordDraft {
        version: BUNDLE_MANIFEST_VERSION,
        record_type: RECORD_BUNDLE_MANIFEST.to_string(),
        subject: bundle.manifest.subject_did.clone(),
        payload,
        expires_at_secs: None,
        supersedes: None,
    };
    let owner_did = derive_did_key(&owner.public_key());
    let (mut env, bytes) = Envelope::unsigned(draft, owner_did, None, now_secs).unwrap();
    let sig = z32::encode(&owner.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    bundle.manifest_signature = Some(env.to_json().unwrap());
}

async fn assert_tampered_core_records(h: &Harness, bundle_val: &Value, now: u64) {
    // 1. Tampered quote with re-signed manifest: manifest signature passes,
    // but inner quote verification fails and names quote.
    let mut b_quote: Bundle = serde_json::from_value(bundle_val.clone()).unwrap();
    let q_rows = b_quote.sections.get_mut("quotes").unwrap();
    let mut payload = q_rows[0].get("payload").unwrap().clone();
    let env_str = payload["envelope"].as_str().unwrap();
    let mut env: Value = serde_json::from_str(env_str).unwrap();
    env["payload"]["terms"]["scope"] = json!("Tampered scope");
    payload["envelope"] = json!(env.to_string());
    q_rows[0]["payload"] = payload;
    let new_digest = Bundle::digest(transaction_app::SCHEMA_VERSION, q_rows).unwrap();
    b_quote.manifest.sections.insert("quotes".to_string(), new_digest);
    resign_bundle(&mut b_quote, &h.owner, now);

    let (qw, qn) = both_rpc(h, "transaction.import", json!({ "bundle": b_quote })).await;
    assert_eq!(qw, qn);
    assert_eq!(qw["error"]["code"], -32602);
    assert!(qw["error"]["message"].as_str().unwrap().contains("envelope does not verify"));

    // 2. Tampered agreement with re-signed manifest: inner consumer half fails.
    let mut b_agr: Bundle = serde_json::from_value(bundle_val.clone()).unwrap();
    let agr_rows = b_agr.sections.get_mut("agreements").unwrap();
    let mut agr_payload = agr_rows[0].get("payload").unwrap().clone();
    let c_str = agr_payload["consumer"]["envelope"].as_str().unwrap();
    let mut c_env: Value = serde_json::from_str(c_str).unwrap();
    c_env["payload"]["terms"]["scope"] = json!("Tampered agreement scope");
    agr_payload["consumer"]["envelope"] = json!(c_env.to_string());
    agr_rows[0]["payload"] = agr_payload;
    let new_agr_digest = Bundle::digest(transaction_app::SCHEMA_VERSION, agr_rows).unwrap();
    b_agr.manifest.sections.insert("agreements".to_string(), new_agr_digest);
    resign_bundle(&mut b_agr, &h.owner, now);

    let (aw, an) = both_rpc(h, "transaction.import", json!({ "bundle": b_agr })).await;
    assert_eq!(aw, an);
    assert_eq!(aw["error"]["code"], -32602);
    assert!(aw["error"]["message"].as_str().unwrap().contains("consumer receipt does not verify"));
}

async fn assert_tampered_ledger_record(h: &Harness, bundle_val: &Value, now: u64) {
    // 3. Tampered ledger step with re-signed manifest.
    let mut b_ledger: Bundle = serde_json::from_value(bundle_val.clone()).unwrap();
    let bad_step_row = json!({
        "id": "step:tampered:1",
        "payload": {
            "kind": "step",
            "agreement": "rec-quote-142",
            "slot_id": null,
            "seat": null,
            "created_at_secs": now,
            "step": {
                "seq": 1,
                "event": "scheduled",
                "snapshot": {
                    "agreement": "rec-quote-142",
                    "conversation": "conv-142",
                    "consumer_did": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
                    "provider_did": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
                    "seq": 1,
                    "state": "scheduled",
                    "payment": "none",
                    "fulfilment": "none",
                    "conflict": null,
                    "track_window_ends_at_secs": now + 86400,
                    "cancelled_by": null,
                    "cancel_reason": null
                },
                "envelope": "{\"tampered\": true}",
                "record_id": "rec_tampered_step_1"
            }
        }
    });
    b_ledger.sections.insert("ledger".to_string(), vec![bad_step_row]);
    let l_digest =
        Bundle::digest(transaction_app::SCHEMA_VERSION, b_ledger.sections.get("ledger").unwrap())
            .unwrap();
    b_ledger.manifest.sections.insert("ledger".to_string(), l_digest);
    resign_bundle(&mut b_ledger, &h.owner, now);

    let (lw, ln) = both_rpc(h, "transaction.import", json!({ "bundle": b_ledger })).await;
    assert_eq!(lw, ln);
    assert_eq!(lw["error"]["code"], -32602);
    assert!(lw["error"]["message"].as_str().unwrap().contains("does not verify"));
}

async fn assert_tampered_progress_record(h: &Harness, bundle_val: &Value, now: u64) {
    // 4. Tampered progress with re-signed manifest.
    let mut b_progress: Bundle = serde_json::from_value(bundle_val.clone()).unwrap();
    let bad_prog_row = json!({
        "id": "rec-quote-142",
        "payload": {
            "agreement": "rec-quote-142",
            "conversation": "conv-142",
            "seq": 1,
            "writer": "writer",
            "received_at_secs": now,
            "record_id": "rec_tampered_prog_1",
            "envelope": "{\"tampered\": true}",
            "snapshot": {
                "agreement": "rec-quote-142",
                "conversation": "conv-142",
                "consumer_did": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
                "provider_did": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
                "seq": 1,
                "state": "scheduled",
                "payment": "none",
                "fulfilment": "none",
                "conflict": null,
                "track_window_ends_at_secs": now + 86400,
                "cancelled_by": null,
                "cancel_reason": null
            }
        }
    });
    b_progress.sections.insert("progress".to_string(), vec![bad_prog_row]);
    let p_digest = Bundle::digest(
        transaction_app::SCHEMA_VERSION,
        b_progress.sections.get("progress").unwrap(),
    )
    .unwrap();
    b_progress.manifest.sections.insert("progress".to_string(), p_digest);
    resign_bundle(&mut b_progress, &h.owner, now);

    let (pw, pn) = both_rpc(h, "transaction.import", json!({ "bundle": b_progress })).await;
    assert_eq!(pw, pn);
    assert_eq!(pw["error"]["code"], -32602);
    assert!(pw["error"]["message"].as_str().unwrap().contains("does not verify"));
}

async fn assert_tampered_payment_record(h: &Harness, bundle_val: &Value, now: u64) {
    // 5. Tampered payment with re-signed manifest.
    let mut b_pay: Bundle = serde_json::from_value(bundle_val.clone()).unwrap();
    let agr_id = b_pay
        .sections
        .get("agreements")
        .and_then(|rows| rows.first())
        .and_then(|r| r.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let bad_pay_row = json!({
        "id": agr_id,
        "payload": {
            "agreement": agr_id,
            "conversation": "conv-142",
            "updated_at_secs": now,
            "request": null,
            "consumer": [
                {
                    "record_id": "rec_bad_1",
                    "issuer": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
                    "issued_at_secs": now,
                    "envelope": "{\"tampered\": true}"
                }
            ],
            "provider": []
        }
    });
    b_pay.sections.insert("payments".to_string(), vec![bad_pay_row]);
    let pay_digest =
        Bundle::digest(transaction_app::SCHEMA_VERSION, b_pay.sections.get("payments").unwrap())
            .unwrap();
    b_pay.manifest.sections.insert("payments".to_string(), pay_digest);
    resign_bundle(&mut b_pay, &h.owner, now);

    let (pay_w, pay_n) = both_rpc(h, "transaction.import", json!({ "bundle": b_pay })).await;
    assert_eq!(pay_w, pay_n);
    assert_eq!(pay_w["error"]["code"], -32602);
    assert!(
        pay_w["error"]["message"].as_str().unwrap().contains("acknowledgement does not verify")
    );
}

async fn assert_tampered_vertical_records(h: &Harness, bundle_val: &Value, now: u64) {
    assert_tampered_ledger_record(h, bundle_val, now).await;
    assert_tampered_progress_record(h, bundle_val, now).await;
    assert_tampered_payment_record(h, bundle_val, now).await;
}

async fn assert_per_record_tamper_refused(h: &Harness, bundle_val: &Value) {
    let now = wall_now();
    assert_tampered_core_records(h, bundle_val, now).await;
    assert_tampered_vertical_records(h, bundle_val, now).await;
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

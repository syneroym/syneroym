#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! The Roym product's transaction vertical: requests, quotes, and agreement
//! receipts across two independent `syneroym-substrate` instances, each
//! running the full Roym SynApp (the `wasm32-wasip2` build) under its own owner
//! identity.
//!
//! Proves an offer is agreed across two installations:
//! consumer finds provider by signed listing without a directory, sends a
//! signed request, receives a signed quote, and accepts it. Provider
//! countersigns and both parties reach a completed agreement pair with
//! identical terms. Also proves tampered cards are refused and never verified.

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card::{CARD_CONTENT_TYPE, card_body},
    record::RECORD_REQUEST,
    transaction::{DEFAULT_DATA_USE_NOTICE, REQUEST_VERSION},
};
use syneroym_signed_record::Envelope;

mod common;

use common::roym::{
    RoymNode as Node, fast_conversation_role, roym_artifacts_present, wait_delivered, wait_until,
};

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "linear roym transaction agreement across two installations"
)]
async fn an_offer_is_agreed_across_two_installations() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();
    let owner_x_did = substrate::derive_did_key(&owner_x.public_key());
    let owner_y_did = substrate::derive_did_key(&owner_y.public_key());

    // Step 1: Boot X and Y; deploy Roym on both; enrol signing on all 4 services on
    // each.
    let mut node_x = Node::boot(
        "node-x",
        dir_x.path().to_path_buf(),
        None,
        Identity::from_bytes(&owner_x.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let shared_registry = node_x.registry_url.clone();
    node_x.full_bring_up().await;

    let mut node_y = Node::boot(
        "node-y",
        dir_y.path().to_path_buf(),
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_y.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_y.full_bring_up().await;

    let x_conv_did = node_x.dids["conversation"].clone();
    let y_conv_did = node_y.dids["conversation"].clone();

    // Step 2: Y sets a profile and an active listing; X sets a profile.
    node_x
        .rpc_ok("profile.set", json!({ "display_name": "X", "conversation_address": x_conv_did }))
        .await;
    node_y
        .rpc_ok("profile.set", json!({ "display_name": "Y", "conversation_address": y_conv_did }))
        .await;

    let y_listing = node_y
        .rpc_ok(
            "listing.set",
            json!({
                "title": "Bike repair",
                "summary": "Same-day service at your door",
                "categories": ["cycling"],
                "payment": {
                    "currency": "EUR", "model": "fixed", "amount_minor": 4500,
                    "tax_included": true, "payee": "Y Repairs"
                }
            }),
        )
        .await;
    let y_listing_id = y_listing["listing_id"].as_str().unwrap().to_string();
    let y_listing_row = node_y.rpc_ok("listing.get", json!({ "listing_id": y_listing_id })).await;
    let y_listing_envelope = y_listing_row["envelope"].as_str().unwrap().to_string();

    let x_verify = node_x.rpc_ok("listing.verify", json!({ "envelope": y_listing_envelope })).await;
    assert_eq!(x_verify["verified"], true);
    let y_conv_address = x_verify["conversation_address"].as_str().unwrap().to_string();
    assert_eq!(y_conv_address, y_conv_did);

    // Step 3: X conversation.open to Y's conversation address from the signed
    // listing.
    let opened = node_x.rpc_ok("conversation.open", json!({ "address": y_conv_address })).await;
    let x_conv_id = opened["conversation_id"].as_str().unwrap().to_string();

    // Step 4: X request.set -> a card is sent. Wait for delivered.
    let req_res = node_x
        .rpc_ok(
            "request.set",
            json!({
                "conversation": x_conv_id,
                "description": "Fix my front brake cable",
                "categories": ["cycling"],
                "data_use_notice": DEFAULT_DATA_USE_NOTICE,
            }),
        )
        .await;
    let req_record_id = req_res["record_id"].as_str().unwrap().to_string();
    let req_msg_id = req_res["message_id"].as_str().unwrap().to_string();

    let delivered = wait_delivered(&node_x, &req_msg_id).await;
    assert!(delivered, "request card was delivered to Y");

    // Step 5: Y transaction.sync { conversation } -> filed: 1; transaction.thread
    // shows verified request.
    let y_has_conv = wait_until(Duration::from_secs(20), || async {
        let list = node_y.rpc_ok("conversation.list", json!({})).await;
        list["conversations"].as_array().map(|c| !c.is_empty()).unwrap_or(false)
    })
    .await;
    assert!(y_has_conv, "Y has received the conversation");
    let y_convs = node_y.rpc_ok("conversation.list", json!({})).await;
    let y_conv_id = y_convs["conversations"][0]["id"].as_str().unwrap().to_string();

    let y_sync = node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_id })).await;
    assert_eq!(y_sync["filed"], 1);

    let y_thread = node_y.rpc_ok("transaction.thread", json!({ "conversation": y_conv_id })).await;
    let y_cards = y_thread["cards"].as_array().unwrap();
    assert_eq!(y_cards.len(), 1);
    assert_eq!(y_cards[0]["verified"], true);
    assert_eq!(y_cards[0]["card_type"], "request");
    assert_eq!(y_cards[0]["issuer"], owner_x_did);

    // Step 6: Y quote.set with every AgreedTerms field filled and expires_in_secs =
    // 3600 -> card sent.
    let quote_res = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "request_record_id": req_record_id,
                "expires_in_secs": 3600,
                "terms": {
                    "scope": "Replace brake cable and adjust pads",
                    "currency": "EUR",
                    "amount_minor": 4500,
                    "tax_minor": 500,
                    "fees_minor": 0,
                    "payment_methods": ["cash", "sepa"],
                    "payee": "Y Repairs",
                    "payment_timing": "after-work",
                    "schedule": {
                        "earliest_secs": 1800000000,
                        "latest_secs": 1800003600
                    },
                    "location": {
                        "where": "at-customer",
                        "address": "123 High Street"
                    },
                    "cancellation_terms": "24 hours notice required for full refund",
                    "refund_terms": "Full refund if work not completed",
                    "dispute_path": "Small claims court or informal mediation"
                }
            }),
        )
        .await;
    let quote_record_id = quote_res["record_id"].as_str().unwrap().to_string();
    let quote_msg_id = quote_res["message_id"].as_str().unwrap().to_string();

    let quote_delivered = wait_delivered(&node_y, &quote_msg_id).await;
    assert!(quote_delivered, "quote card was delivered to X");

    // Step 7: X transaction.sync -> files the quote; thread shows it verified with
    // X as consumer_did.
    let x_sync = node_x.rpc_ok("transaction.sync", json!({ "conversation": x_conv_id })).await;
    assert_eq!(x_sync["filed"], 1);

    let x_thread = node_x.rpc_ok("transaction.thread", json!({ "conversation": x_conv_id })).await;
    let x_cards = x_thread["cards"].as_array().unwrap();
    let quote_card =
        x_cards.iter().find(|c| c["card_type"] == "quote").expect("quote card present");
    assert_eq!(quote_card["verified"], true);
    assert_eq!(quote_card["data"]["consumer_did"], owner_x_did);

    // Step 8: X agreement.accept { quote_record_id } -> role: consumer, pair: half.
    let accept_res =
        node_x.rpc_ok("agreement.accept", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(accept_res["role"], "consumer");
    assert_eq!(accept_res["pair"]["state"], "half");
    assert_eq!(accept_res["pair"]["role"], "consumer");
    let accept_msg_id = accept_res["message_id"].as_str().unwrap().to_string();

    let accept_delivered = wait_delivered(&node_x, &accept_msg_id).await;
    assert!(accept_delivered, "accept card was delivered to Y");

    // Step 9: Y transaction.sync -> countersigned: 1; agreement.get reports pair:
    // complete.
    let y_sync2 = node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_id })).await;
    assert_eq!(y_sync2["countersigned"], 1);

    let y_agr = node_y.rpc_ok("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(y_agr["pair"]["state"], "complete");
    assert!(y_agr["consumer"].is_object());
    assert!(y_agr["provider"].is_object());

    // Wait until X receives the provider's countersigned card
    let y_thread2 = node_y.rpc_ok("transaction.thread", json!({ "conversation": y_conv_id })).await;
    let y_cards2 = y_thread2["cards"].as_array().unwrap();
    let y_prov_card = y_cards2
        .iter()
        .find(|c| c["card_type"] == "agreement-receipt" && c["issuer"] == owner_y_did)
        .expect("provider card in Y thread");
    let prov_msg_id = y_prov_card["message_id"].as_str().unwrap();
    let prov_delivered = wait_delivered(&node_y, prov_msg_id).await;
    assert!(prov_delivered, "countersigned receipt delivered to X");

    // Step 10: X transaction.sync -> X agreement.get reports pair: complete,
    // payloads differ only in role.
    let _x_sync2 = node_x.rpc_ok("transaction.sync", json!({ "conversation": x_conv_id })).await;
    let x_agr = node_x.rpc_ok("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(x_agr["pair"]["state"], "complete");
    assert!(x_agr["consumer"].is_object());
    assert!(x_agr["provider"].is_object());

    let x_consumer_env =
        Envelope::from_json(x_agr["consumer"]["envelope"].as_str().unwrap()).unwrap();
    let x_provider_env =
        Envelope::from_json(x_agr["provider"]["envelope"].as_str().unwrap()).unwrap();
    let mut c_payload = x_consumer_env.payload.clone();
    let mut p_payload = x_provider_env.payload.clone();
    assert_eq!(c_payload["role"], "consumer");
    assert_eq!(p_payload["role"], "provider");
    c_payload["role"] = json!("same");
    p_payload["role"] = json!("same");
    assert_eq!(c_payload, p_payload);

    // Step 11: Every field the Records table names is present on both halves.
    for half_payload in [&x_consumer_env.payload, &x_provider_env.payload] {
        assert_eq!(half_payload["consumer_did"], owner_x_did);
        assert_eq!(half_payload["provider_did"], owner_y_did);
        assert_eq!(half_payload["quote_record_id"], quote_record_id);
        let terms = &half_payload["terms"];
        assert!(terms["payee"].is_string() && !terms["payee"].as_str().unwrap().is_empty());
        assert!(
            terms["quote_expires_at_secs"].is_u64()
                && terms["quote_expires_at_secs"].as_u64().unwrap() > 0
        );
        assert!(
            terms["cancellation_terms"].is_string()
                && !terms["cancellation_terms"].as_str().unwrap().is_empty()
        );
        assert!(
            terms["refund_terms"].is_string()
                && !terms["refund_terms"].as_str().unwrap().is_empty()
        );
        assert!(
            terms["dispute_path"].is_string()
                && !terms["dispute_path"].as_str().unwrap().is_empty()
        );
    }

    // Step 12: Restart X, redeploy on resume, and re-read agreement.get: still
    // complete.
    let step10_consumer_env = x_agr["consumer"]["envelope"].as_str().unwrap().to_string();
    let step10_provider_env = x_agr["provider"]["envelope"].as_str().unwrap().to_string();

    node_x.restart(None, None).await;

    let x_agr_restarted =
        node_x.rpc_ok("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(x_agr_restarted["pair"]["state"], "complete");
    assert_eq!(x_agr_restarted["consumer"]["envelope"], step10_consumer_env);
    assert_eq!(x_agr_restarted["provider"]["envelope"], step10_provider_env);

    // Step 13: X sends plain chat message claiming different payee; agreement payee
    // is unchanged.
    node_x
        .rpc_ok(
            "conversation.send",
            json!({
                "conversation": x_conv_id,
                "body": "Please send money to payee: Fake Scammer instead",
            }),
        )
        .await;

    let x_agr_check =
        node_x.rpc_ok("agreement.get", json!({ "quote_record_id": quote_record_id })).await;
    assert_eq!(x_agr_check["terms"]["payee"], "Y Repairs");

    // Step 14: No directory source configured on either installation; sources is
    // empty.
    let dir_sources = node_x.rpc_ok("directory.sources", json!({})).await;
    let empty_sources = dir_sources["sources"].as_array().map(Vec::is_empty).unwrap_or(true);
    assert!(empty_sources, "no directory sources on X");

    node_x.teardown().await;
    node_y.teardown().await;
}

#[tokio::test]
async fn a_tampered_card_is_filed_refused_and_never_verified() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();

    let mut node_x = Node::boot(
        "node-x",
        dir_x.path().to_path_buf(),
        None,
        Identity::from_bytes(&owner_x.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    let shared_registry = node_x.registry_url.clone();
    node_x.full_bring_up().await;

    let mut node_y = Node::boot(
        "node-y",
        dir_y.path().to_path_buf(),
        Some(shared_registry.clone()),
        Identity::from_bytes(&owner_y.to_bytes()),
        fast_conversation_role(3600),
    )
    .await;
    node_y.full_bring_up().await;

    let x_conv_did = node_x.dids["conversation"].clone();
    let y_conv_did = node_y.dids["conversation"].clone();

    node_x
        .rpc_ok("profile.set", json!({ "display_name": "X", "conversation_address": x_conv_did }))
        .await;
    node_y
        .rpc_ok("profile.set", json!({ "display_name": "Y", "conversation_address": y_conv_did }))
        .await;

    let opened = node_x.rpc_ok("conversation.open", json!({ "address": y_conv_did })).await;
    let x_conv_id = opened["conversation_id"].as_str().unwrap().to_string();

    let req_res = node_x
        .rpc_ok(
            "request.set",
            json!({
                "conversation": x_conv_id,
                "description": "Legitimate request description",
                "categories": ["cycling"],
                "data_use_notice": DEFAULT_DATA_USE_NOTICE,
            }),
        )
        .await;
    let req_id = req_res["request_id"].as_str().unwrap().to_string();
    let req_get = node_x.rpc_ok("request.get", json!({ "request_id": req_id })).await;
    let valid_envelope_str = req_get["envelope"].as_str().unwrap();

    // Tamper the payload of the envelope: change one field without updating
    // signature
    let mut env_val: Value = serde_json::from_str(valid_envelope_str).unwrap();
    if let Some(p) = env_val.get_mut("payload")
        && let Some(desc) = p.get_mut("description")
    {
        *desc = json!("Tampered description");
    }
    let tampered_env_str = serde_json::to_string(&env_val).unwrap();

    let tampered_card = card_body(RECORD_REQUEST, REQUEST_VERSION, &tampered_env_str).unwrap();

    let sent = node_x
        .rpc_ok(
            "conversation.send",
            json!({
                "conversation": x_conv_id,
                "body": tampered_card,
                "content_type": CARD_CONTENT_TYPE,
            }),
        )
        .await;
    let sent_msg_id = sent["message_id"].as_str().unwrap().to_string();

    let delivered = wait_delivered(&node_x, &sent_msg_id).await;
    assert!(delivered, "tampered card was delivered to Y");

    let y_has_conv = wait_until(Duration::from_secs(20), || async {
        let list = node_y.rpc_ok("conversation.list", json!({})).await;
        list["conversations"].as_array().map(|c| !c.is_empty()).unwrap_or(false)
    })
    .await;
    assert!(y_has_conv, "Y received conversation");
    let y_convs = node_y.rpc_ok("conversation.list", json!({})).await;
    let y_conv_id = y_convs["conversations"][0]["id"].as_str().unwrap().to_string();

    node_y.rpc_ok("transaction.sync", json!({ "conversation": y_conv_id })).await;

    let y_thread = node_y.rpc_ok("transaction.thread", json!({ "conversation": y_conv_id })).await;
    let cards = y_thread["cards"].as_array().unwrap();
    let tampered_card_row =
        cards.iter().find(|c| c["message_id"] == sent_msg_id).expect("tampered card in thread");
    assert_eq!(tampered_card_row["verified"], false);
    assert!(
        tampered_card_row["reason"].is_string()
            && !tampered_card_row["reason"].as_str().unwrap().is_empty()
    );
    assert!(tampered_card_row["data"].is_null());

    let y_req_list = node_y.rpc_ok("request.list", json!({ "conversation": y_conv_id })).await;
    let y_requests = y_req_list["requests"].as_array().unwrap();
    assert!(y_requests.iter().all(|r| r["envelope"] != tampered_env_str));

    node_x.teardown().await;
    node_y.teardown().await;
}

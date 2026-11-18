#![allow(
    clippy::cognitive_complexity,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    dead_code
)]
//! Roym deal lifecycle e2e tests across two independent substrate nodes:
//! simultaneous cross-quotes and barter/goods deal completion.

use std::time::Duration;

use rustls::crypto::ring;
use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    deal::DealTerms,
    lang::LangTag,
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RecordDraft},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, DEFAULT_DATA_USE_NOTICE, DealRole,
        QuotePayload,
    },
};

mod common;

use common::{
    roym::{
        RoymNode as Node, fast_conversation_role, roym_artifacts_present, wait_delivered,
        wait_until,
    },
    roym_flow::{
        accept_quote, open_request_conv, quote_author_conv_for, request_record_on_quote_author,
        wait_and_verify_winner_scheduled,
    },
};

struct CrossQuoteFixture {
    _dir_x: tempfile::TempDir,
    _dir_y: tempfile::TempDir,
    node_x: Node,
    node_y: Node,
    conv_x: String,
    conv_y: String,
    req_x: String,
    req_y: String,
}

async fn setup_cross_quote_fixture() -> CrossQuoteFixture {
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
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "X", "conversation_address": x_conv_did, "lang": "en" }),
        )
        .await;
    node_y
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "Y", "conversation_address": y_conv_did, "lang": "en" }),
        )
        .await;

    let (conv_x, _) =
        open_request_conv(&node_x, &y_conv_did, "Fix my bike brake", &["cycling"]).await;
    let conv_y = quote_author_conv_for(&node_y, &x_conv_did).await;
    let req_x = request_record_on_quote_author(&node_y, &conv_y).await;

    let y_enq = node_y
        .rpc_ok(
            "enquiry.set",
            json!({
                "conversation": conv_y,
                "description": { "lang": "en", "text": "Need tutoring lesson" },
                "categories": ["tutoring"],
                "data_use_notice": DEFAULT_DATA_USE_NOTICE,
            }),
        )
        .await;
    let y_enq_msg = y_enq["message_id"].as_str().unwrap().to_string();
    let req_y = y_enq["record_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(&node_y, &y_enq_msg).await, "Y enquiry delivered to X");

    node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;

    CrossQuoteFixture { _dir_x: dir_x, _dir_y: dir_y, node_x, node_y, conv_x, conv_y, req_x, req_y }
}

#[tokio::test]
async fn each_party_quotes_the_other_at_once() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let CrossQuoteFixture { _dir_x, _dir_y, node_x, node_y, conv_x, conv_y, req_x, req_y } =
        setup_cross_quote_fixture().await;

    let quote_x_fut = node_x.rpc_ok(
        "quote.set",
        json!({
            "enquiry_record_id": req_y,
            "expires_in_secs": 3600,
            "terms": {
                "currency": "EUR",
                "commitments": [
                    {
                        "giver": "quote-author",
                        "resource_kind": "goods-or-service",
                        "kind": "fulfilment",
                        "after": [],
                        "description": { "lang": "en", "text": "1 hour math lesson" },
                        "declared_value_minor": 4000
                    },
                    {
                        "giver": "counterparty",
                        "resource_kind": "currency",
                        "kind": "currency",
                        "after": ["c1"],
                        "description": { "lang": "en", "text": "EUR 40.00" },
                        "declared_value_minor": 4000,
                        "payee": "X Tutoring"
                    }
                ],
                "scope": { "lang": "en", "text": "Math tutoring" },
                "cancellation_terms": { "lang": "en", "text": "24h notice" },
                "refund_terms": { "lang": "en", "text": "Full refund" },
                "dispute_path": { "lang": "en", "text": "Informal" }
            }
        }),
    );

    let quote_y_fut = node_y.rpc_ok(
        "quote.set",
        json!({
            "enquiry_record_id": req_x,
            "expires_in_secs": 3600,
            "terms": {
                "currency": "EUR",
                "commitments": [
                    {
                        "giver": "quote-author",
                        "resource_kind": "goods-or-service",
                        "kind": "fulfilment",
                        "after": [],
                        "description": { "lang": "en", "text": "Brake repair" },
                        "declared_value_minor": 4500
                    },
                    {
                        "giver": "counterparty",
                        "resource_kind": "currency",
                        "kind": "currency",
                        "after": ["c1"],
                        "description": { "lang": "en", "text": "EUR 45.00" },
                        "declared_value_minor": 4500,
                        "payee": "Y Repairs"
                    }
                ],
                "scope": { "lang": "en", "text": "Brake repair" },
                "cancellation_terms": { "lang": "en", "text": "24h notice" },
                "refund_terms": { "lang": "en", "text": "Full refund" },
                "dispute_path": { "lang": "en", "text": "Informal" }
            }
        }),
    );

    let (quote_x_res, quote_y_res) = tokio::join!(quote_x_fut, quote_y_fut);

    let msg_x = quote_x_res["message_id"].as_str().unwrap();
    let msg_y = quote_y_res["message_id"].as_str().unwrap();
    assert!(wait_delivered(&node_x, msg_x).await, "X quote delivered");
    assert!(wait_delivered(&node_y, msg_y).await, "Y quote delivered");

    node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;
    node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;

    let q_on_x = node_x
        .rpc_ok("quote.get", json!({ "quote_id": quote_x_res["quote_id"].as_str().unwrap() }))
        .await;
    let q_on_y = node_y
        .rpc_ok("quote.get", json!({ "quote_id": quote_y_res["quote_id"].as_str().unwrap() }))
        .await;
    assert_ne!(quote_x_res["record_id"], quote_y_res["record_id"]);
    assert_eq!(q_on_x["mine"], true);
    assert_eq!(q_on_y["mine"], true);
    assert!(q_on_x["declined_at_secs"].is_null());
    assert!(q_on_y["declined_at_secs"].is_null());
}

struct BarterFixture {
    _dir_x: tempfile::TempDir,
    _dir_y: tempfile::TempDir,
    node_x: Node,
    node_y: Node,
    conv_x: String,
    conv_y: String,
    req_x: String,
    owner_y_did: String,
}

async fn setup_barter_fixture() -> BarterFixture {
    let dir_x = tempfile::tempdir().unwrap();
    let dir_y = tempfile::tempdir().unwrap();
    let owner_x = Identity::generate().unwrap();
    let owner_y = Identity::generate().unwrap();
    let owner_y_did = substrate::derive_did_key(&owner_y.public_key());

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
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "X", "conversation_address": x_conv_did, "lang": "en" }),
        )
        .await;
    node_y
        .rpc_ok(
            "profile.set",
            json!({ "display_name": "Y", "conversation_address": y_conv_did, "lang": "en" }),
        )
        .await;

    let (conv_x, _) =
        open_request_conv(&node_x, &y_conv_did, "Exchange skills and materials", &["gardening"])
            .await;
    let conv_y = quote_author_conv_for(&node_y, &x_conv_did).await;
    let req_x = request_record_on_quote_author(&node_y, &conv_y).await;

    BarterFixture {
        _dir_x: dir_x,
        _dir_y: dir_y,
        node_x,
        node_y,
        conv_x,
        conv_y,
        req_x,
        owner_y_did,
    }
}

async fn exchange_barter_fulfilments(
    node_x: &Node,
    node_y: &Node,
    conv_x: &str,
    conv_y: &str,
    quote_rec_id: &str,
) {
    let y_f1 = node_y
        .rpc_ok("fulfilment.sign", json!({ "agreement": quote_rec_id, "commitment_id": "c1" }))
        .await;
    let y_f1_msg = y_f1["message_id"].as_str().unwrap();
    assert!(wait_delivered(node_y, y_f1_msg).await, "Y c1 fulfilment delivered");

    node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;
    let x_f1_rc = node_x
        .rpc_ok("fulfilment.sign", json!({ "agreement": quote_rec_id, "commitment_id": "c1" }))
        .await;
    let x_f1_msg = x_f1_rc["message_id"].as_str().unwrap();
    assert!(wait_delivered(node_x, x_f1_msg).await, "X c1 receiver half delivered");

    node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;

    let x_f2 = node_x
        .rpc_ok("fulfilment.sign", json!({ "agreement": quote_rec_id, "commitment_id": "c2" }))
        .await;
    let x_f2_msg = x_f2["message_id"].as_str().unwrap();
    assert!(wait_delivered(node_x, x_f2_msg).await, "X c2 fulfilment delivered");

    node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;
    let y_f2_rc = node_y
        .rpc_ok("fulfilment.sign", json!({ "agreement": quote_rec_id, "commitment_id": "c2" }))
        .await;
    let y_f2_msg = y_f2_rc["message_id"].as_str().unwrap();
    assert!(wait_delivered(node_y, y_f2_msg).await, "Y c2 receiver half delivered");
}

async fn wait_both_agreements_closed(
    node_x: &Node,
    node_y: &Node,
    conv_x: &str,
    conv_y: &str,
    quote_rec_id: &str,
) {
    let y_closed = wait_until(Duration::from_secs(30), || async {
        node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;
        let b = node_y.rpc_ok("agreement.get", json!({ "quote_record_id": quote_rec_id })).await;
        b["state"] == "closed"
    })
    .await;
    assert!(y_closed, "Y agreement reaches closed");

    let x_closed = wait_until(Duration::from_secs(30), || async {
        node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;
        let b = node_x.rpc_ok("agreement.get", json!({ "quote_record_id": quote_rec_id })).await;
        b["state"] == "closed"
    })
    .await;
    assert!(x_closed, "X agreement reaches closed");
}

#[tokio::test]
async fn service_for_service_and_goods_deals_close() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let BarterFixture { _dir_x, _dir_y, node_x, node_y, conv_x, conv_y, req_x, owner_y_did } =
        setup_barter_fixture().await;

    let quote_res = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "enquiry_record_id": req_x,
                "expires_in_secs": 3600,
                "terms": {
                    "currency": "EUR",
                    "commitments": [
                        {
                            "giver": "quote-author",
                            "resource_kind": "goods-or-service",
                            "kind": "fulfilment",
                            "after": [],
                            "description": { "lang": "en", "text": "Garden landscaping service" },
                            "declared_value_minor": 20000
                        },
                        {
                            "giver": "counterparty",
                            "resource_kind": "goods-or-service",
                            "kind": "fulfilment",
                            "after": ["c1"],
                            "description": { "lang": "en", "text": "Timber materials delivered" },
                            "declared_value_minor": 20000
                        }
                    ],
                    "scope": { "lang": "en", "text": "Landscaping in exchange for timber" },
                    "cancellation_terms": { "lang": "en", "text": "Mutual consent" },
                    "refund_terms": { "lang": "en", "text": "Return materials or equivalent service" },
                    "dispute_path": { "lang": "en", "text": "Mediation" }
                }
            }),
        )
        .await;
    let q_msg = quote_res["message_id"].as_str().unwrap();
    let quote_rec_id = quote_res["record_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(&node_y, q_msg).await, "quote delivered");

    accept_quote(&node_x, &conv_x, &quote_rec_id).await;

    node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;
    wait_and_verify_winner_scheduled(
        &node_y,
        &owner_y_did,
        &node_x,
        &quote_rec_id,
        &conv_x,
        &conv_y,
    )
    .await;

    exchange_barter_fulfilments(&node_x, &node_y, &conv_x, &conv_y, &quote_rec_id).await;
    wait_both_agreements_closed(&node_x, &node_y, &conv_x, &conv_y, &quote_rec_id).await;
}

fn simple_service_terms(desc: &str, price_minor: i64) -> serde_json::Value {
    json!({
        "currency": "EUR",
        "commitments": [
            {
                "giver": "quote-author",
                "resource_kind": "goods-or-service",
                "kind": "fulfilment",
                "after": [],
                "description": { "lang": "en", "text": desc },
                "declared_value_minor": price_minor,
            },
            {
                "giver": "counterparty",
                "resource_kind": "currency",
                "kind": "currency",
                "after": ["c1"],
                "description": { "lang": "en", "text": format!("EUR {}.00", price_minor / 100) },
                "declared_value_minor": price_minor,
                "payee": "Y Repairs",
            }
        ],
        "scope": { "lang": "en", "text": desc },
        "cancellation_terms": { "lang": "en", "text": "24h notice" },
        "refund_terms": { "lang": "en", "text": "Full refund" },
        "dispute_path": { "lang": "en", "text": "Informal" },
    })
}

fn make_hand_signed_acceptance(
    node: &Node,
    quote_record_id: &str,
    quote_author_did: &str,
    counterparty_did: &str,
    terms: DealTerms,
    now: u64,
) -> String {
    let payload = AgreementReceiptPayload {
        quote_record_id: quote_record_id.to_string(),
        counterparty_did: counterparty_did.to_string(),
        quote_author_did: quote_author_did.to_string(),
        role: DealRole::Counterparty,
        terms,
        viewed_lang: LangTag::parse_canonical("en").expect("lang"),
        viewed_via_mt: false,
    };
    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: quote_record_id.to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, counterparty_did.to_string(), None, now).expect("env");
    let sig = z32::encode(&node.owner.sign(&bytes).to_bytes());
    env.attach_signature(sig).expect("sig");
    let env_json = env.to_json().expect("json");
    card::card_body(RECORD_AGREEMENT_RECEIPT, AGREEMENT_RECEIPT_VERSION, &env_json)
        .expect("card body")
}

#[tokio::test]
async fn decline_reaches_the_other_node() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let CrossQuoteFixture { _dir_x, _dir_y, node_x, node_y, conv_x, conv_y, req_x, .. } =
        setup_cross_quote_fixture().await;

    let quote_res = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "enquiry_record_id": req_x,
                "expires_in_secs": 3600,
                "terms": simple_service_terms("Bike brake repair", 4500),
            }),
        )
        .await;
    let q_msg = quote_res["message_id"].as_str().unwrap();
    let quote_rec_id = quote_res["record_id"].as_str().unwrap().to_string();
    let quote_id = quote_res["quote_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(&node_y, q_msg).await, "quote delivered");

    node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;

    let dec_res = node_x
        .rpc_ok("deal.decline", json!({ "declines": quote_rec_id, "note": "not today" }))
        .await;
    assert_eq!(dec_res["declined"], true);
    assert_eq!(dec_res["declines"], quote_rec_id);
    assert_eq!(dec_res["deal"], quote_id);

    let y_declined = wait_until(Duration::from_secs(20), || async {
        node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;
        let q = node_y.rpc_ok("quote.get", json!({ "quote_id": quote_id })).await;
        q["declined_at_secs"].is_number()
    })
    .await;
    assert!(y_declined, "Y files decline card from X");

    node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;

    let q_x = node_x.rpc_ok("quote.get", json!({ "quote_id": quote_id })).await;
    assert!(q_x["declined_at_secs"].is_number());
    assert_eq!(q_x["decline_note"], "not today");

    let q_y = node_y.rpc_ok("quote.get", json!({ "quote_id": quote_id })).await;
    assert!(q_y["declined_at_secs"].is_number());
    assert_eq!(q_y["decline_note"], "not today");

    let thread_x = node_x.rpc_ok("transaction.thread", json!({ "conversation": conv_x })).await;
    let card_x =
        thread_x["cards"].as_array().unwrap().iter().find(|c| c["card_type"] == "quote").unwrap();
    assert_eq!(card_x["deal_state"], "declined");
    assert_eq!(card_x["declined"], true);

    let thread_y = node_y.rpc_ok("transaction.thread", json!({ "conversation": conv_y })).await;
    let card_y =
        thread_y["cards"].as_array().unwrap().iter().find(|c| c["card_type"] == "quote").unwrap();
    assert_eq!(card_y["deal_state"], "declined");
    assert_eq!(card_y["declined"], true);

    let rep = node_x
        .rpc_ok("deal.decline", json!({ "declines": quote_rec_id, "note": "not today" }))
        .await;
    assert_eq!(rep["record_id"], dec_res["record_id"]);
}

#[tokio::test]
async fn enquiry_decline_and_withdrawal_reach_the_other_node() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let CrossQuoteFixture { _dir_x, _dir_y, node_x, node_y, conv_x, conv_y, req_x, req_y } =
        setup_cross_quote_fixture().await;

    // Case 1: Enquiry decline by quote author.
    // Node X sent enquiry req_x to Node Y.
    // Node Y (the quote author / recipient of enquiry) declines req_x.
    let dec_y =
        node_y.rpc_ok("deal.decline", json!({ "declines": req_x, "note": "cannot service" })).await;
    assert_eq!(dec_y["declined"], true);

    let x_saw_decline = wait_until(Duration::from_secs(20), || async {
        node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;
        let thread_x = node_x.rpc_ok("transaction.thread", json!({ "conversation": conv_x })).await;
        let card = thread_x["cards"].as_array().unwrap().iter().find(|c| c["record_id"] == req_x);
        if let Some(c) = card
            && c["declined"] == true
        {
            let enquiry_id = c["data"]["enquiry_id"].as_str().unwrap();
            let enq = node_x.rpc_ok("enquiry.get", json!({ "enquiry_id": enquiry_id })).await;
            enq["declined_at_secs"].is_number() && enq["decline_note"] == "cannot service"
        } else {
            false
        }
    })
    .await;
    assert!(x_saw_decline, "X sees enquiry declined by quote author Y");

    // Case 2: Enquiry withdrawal by its author.
    // Node Y sent enquiry req_y to Node X.
    // Node Y (the enquiry author) withdraws/declines req_y.
    let dec_x =
        node_y.rpc_ok("deal.decline", json!({ "declines": req_y, "note": "nevermind" })).await;
    assert_eq!(dec_x["declined"], true);

    let x_saw_withdrawal = wait_until(Duration::from_secs(20), || async {
        node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;
        let thread_x = node_x.rpc_ok("transaction.thread", json!({ "conversation": conv_x })).await;
        let card = thread_x["cards"].as_array().unwrap().iter().find(|c| c["record_id"] == req_y);
        if let Some(c) = card
            && c["declined"] == true
        {
            let enquiry_id = c["data"]["enquiry_id"].as_str().unwrap();
            let enq = node_x.rpc_ok("enquiry.get", json!({ "enquiry_id": enquiry_id })).await;
            enq["declined_at_secs"].is_number() && enq["decline_note"] == "nevermind"
        } else {
            false
        }
    })
    .await;
    assert!(x_saw_withdrawal, "X sees enquiry withdrawn by author Y");

    // Also verify on Y's thread in conv_y
    node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;
    let thread_y = node_y.rpc_ok("transaction.thread", json!({ "conversation": conv_y })).await;
    let card_y_req_x =
        thread_y["cards"].as_array().unwrap().iter().find(|c| c["record_id"] == req_x).unwrap();
    assert_eq!(card_y_req_x["declined"], true);
}

#[tokio::test]
async fn refused_superseded_acceptance_shows_on_that_version() {
    let _guard = common::serial_guard().await;
    let _ = ring::default_provider().install_default();
    if !roym_artifacts_present() {
        eprintln!("skipping: Roym wasm/UI artifacts not built (`mise run build:roym`)");
        return;
    }

    let CrossQuoteFixture { _dir_x, _dir_y, node_x, node_y, conv_x, conv_y, req_x, .. } =
        setup_cross_quote_fixture().await;

    let q1_res = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "enquiry_record_id": req_x,
                "expires_in_secs": 3600,
                "terms": simple_service_terms("Brake repair v1", 4000),
            }),
        )
        .await;
    let q1_rec_id = q1_res["record_id"].as_str().unwrap().to_string();
    let quote_id = q1_res["quote_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(&node_y, q1_res["message_id"].as_str().unwrap()).await);
    node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;

    let q1_view = node_x.rpc_ok("quote.get", json!({ "quote_id": quote_id })).await;
    let q1_env: Envelope = Envelope::from_json(q1_view["envelope"].as_str().unwrap()).unwrap();
    let q1_payload: QuotePayload = serde_json::from_value(q1_env.payload).unwrap();

    let q2_res = node_y
        .rpc_ok(
            "quote.set",
            json!({
                "enquiry_record_id": req_x,
                "quote_id": quote_id,
                "expires_in_secs": 3600,
                "terms": simple_service_terms("Brake repair v2", 5000),
            }),
        )
        .await;
    let q2_rec_id = q2_res["record_id"].as_str().unwrap().to_string();
    assert!(wait_delivered(&node_y, q2_res["message_id"].as_str().unwrap()).await);
    node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;

    let accept_err =
        node_x.rpc_err("agreement.accept", json!({ "quote_record_id": q1_rec_id })).await;
    assert!(accept_err["message"].as_str().unwrap().contains("quote-superseded"));

    let card_body = make_hand_signed_acceptance(
        &node_x,
        &q1_rec_id,
        &q1_env.issuer,
        &q1_payload.counterparty_did,
        q1_payload.terms,
        q1_env.issued_at_secs + 1,
    );
    let sent = node_x
        .rpc_ok(
            "conversation.send",
            json!({
                "conversation": conv_x,
                "body": card_body,
                "content_type": CARD_CONTENT_TYPE,
            }),
        )
        .await;
    assert!(wait_delivered(&node_x, sent["message_id"].as_str().unwrap()).await);

    node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;

    let x_refused = wait_until(Duration::from_secs(20), || async {
        node_x.rpc_ok("transaction.sync", json!({ "conversation": conv_x })).await;
        let agr = node_x.rpc_ok("agreement.get", json!({ "quote_record_id": q1_rec_id })).await;
        agr["state"] == "not-formed" && agr["state_reason"] == "quote-superseded"
    })
    .await;
    assert!(x_refused, "X files refusal snapshot with quote-superseded on version 1");

    let accept_v2 =
        node_x.rpc_ok("agreement.accept", json!({ "quote_record_id": q2_rec_id })).await;
    assert_eq!(accept_v2["role"], "counterparty");
    assert!(wait_delivered(&node_x, accept_v2["message_id"].as_str().unwrap()).await);

    node_y.rpc_ok("transaction.sync", json!({ "conversation": conv_y })).await;
    let y_agr2 = node_y.rpc_ok("agreement.get", json!({ "quote_record_id": q2_rec_id })).await;
    assert_eq!(y_agr2["state"], "agreed");
    assert_eq!(y_agr2["pair"]["state"], "complete");
}

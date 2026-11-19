//! What a party says it read when it accepts a quote: `viewed_lang` and
//! `viewed_via_mt` in the signed acceptance, on both builds.

use serde_json::{Value, json};
use syneroym_roym_core::{
    deal::{Answers, DealTerms},
    testing::sd::{PlainQuote, conceal_quote},
    transaction,
};
use syneroym_signed_record::Envelope;

use super::{
    deal_fixtures::{deliver_peer_card_with_disclosures, setup_peer_request},
    fixtures::*,
    helpers::*,
};

fn hi_text(hi: &str, en: &str) -> Value {
    json!({
        "lang": "hi",
        "text": hi,
        "translations": [
            { "lang": "en", "text": en }
        ]
    })
}

/// A quote the peer signed with `terms.scope.lang` Hindi and one English author
/// translation on each text field.
fn peer_quote_hindi_with_english(conv: &str, req_record_id: &str) -> (String, String, Vec<String>) {
    let now = wall_now();
    let quote_id = transaction::derive_quote_id(conv, &peer_did(), 1).unwrap();
    let terms_val = json!({
        "currency": "EUR",
        "quote_expires_at_secs": now + 3600,
        "commitments": [
            {
                "commitment_id": "c1",
                "giver": "quote-author",
                "resource_kind": "goods-or-service",
                "kind": "fulfilment",
                "after": [],
                "description": hi_text("बगीचे की मरम्मत", "Garden repair"),
                "declared_value_minor": 50000,
                "location": { "where": "at-receiver", "address": "123 Flower St" }
            },
            {
                "commitment_id": "c2",
                "giver": "counterparty",
                "resource_kind": "currency",
                "kind": "currency",
                "after": ["c1"],
                "description": hi_text("EUR 500.00", "EUR 500.00"),
                "declared_value_minor": 50000,
                "payee": "Garden Services Ltd",
                "payment_methods": ["card", "cash"],
                "tax_minor": 5000,
                "fees_minor": 2000
            }
        ],
        "scope": hi_text("बगीचे की मरम्मत", "Garden repair"),
        "cancellation_terms": hi_text("रद्द करने की शर्तें", "Cancel 24h prior for full refund"),
        "refund_terms": hi_text("वापसी की शर्तें", "Full refund if unsatisfactory"),
        "dispute_path": hi_text("विवाद निवारण", "Contact disputes@example.com")
    });
    let terms: DealTerms = serde_json::from_value(terms_val).unwrap();
    let plain = PlainQuote {
        quote_id: quote_id.clone(),
        conversation: conv.to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: req_record_id.to_string() },
        counterparty_did: owner_did(),
        terms,
    };
    let (payload, disclosures) = conceal_quote(plain, 1);
    let payload_val = serde_json::to_value(&payload).unwrap();
    let env_json = sign_as_peer(
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &quote_id,
        payload_val,
        Some(now + 3600),
        now,
    );
    let record_id = Envelope::from_json(&env_json).unwrap().record_id().unwrap();
    (record_id, env_json, disclosures)
}

/// The peer's quote, filed on both builds. Returns the quote's record id.
async fn synced_quote(h: &Harness, name: &str) -> String {
    let conv = format!("conv-237-{name}");
    let request = both_rpc(
        h,
        "enquiry.set",
        json!({
            "conversation": conv,
            "description": { "lang": "en", "text": "Counterparty deal" },
            "data_use_notice": transaction::DEFAULT_DATA_USE_NOTICE,
        }),
    )
    .await
    .1;
    let req_record_id = request["result"]["record_id"].as_str().unwrap();
    let (quote_record_id, quote_env, disclosures) =
        peer_quote_hindi_with_english(&conv, req_record_id);
    deliver_peer_card_with_disclosures(
        h,
        &format!("m-quote-237-{name}"),
        &conv,
        &peer_did(),
        transaction::RECORD_QUOTE,
        transaction::QUOTE_VERSION,
        &quote_env,
        &disclosures,
    )
    .await;
    both_rpc(h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;
    quote_record_id
}

/// The answer of `agreement.accept` without the ids each build mints for
/// itself (the receipt's envelope carries the clock).
fn without_minted_ids(answer: &Value) -> Value {
    let mut answer = answer.clone();
    if let Some(result) = answer.get_mut("result").and_then(Value::as_object_mut) {
        for key in ["message_id", "record_id"] {
            result.remove(key);
        }
    }
    stripped(&answer)
}

/// `(viewed_lang, viewed_via_mt)` of the signed half a build holds.
fn viewed_in(half: &Value) -> (Value, Value) {
    let payload = Envelope::from_json(half["envelope"].as_str().unwrap()).unwrap().payload;
    (payload["viewed_lang"].clone(), payload["viewed_via_mt"].clone())
}

/// Accepts a fresh quote with `params` added, checks both builds answer
/// alike, and returns what the counterparty half says was read.
async fn accept_and_read_back(h: &Harness, name: &str, params: Value) -> (Value, Value) {
    let quote = synced_quote(h, name).await;
    let mut input = json!({ "quote_record_id": quote });
    input.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
    let (wasm, native) = both_rpc(h, "agreement.accept", input).await;
    assert_eq!(without_minted_ids(&wasm), without_minted_ids(&native));
    assert_eq!(wasm["result"]["role"], "counterparty", "{wasm}");

    let (wasm, native) = both_rpc(h, "agreement.get", json!({ "quote_record_id": quote })).await;
    let (read_wasm, read_native) =
        (viewed_in(&wasm["result"]["counterparty"]), viewed_in(&native["result"]["counterparty"]));
    assert_eq!(read_wasm, read_native);
    read_wasm
}

async fn refused(h: &Harness, quote: &str, params: Value) -> Value {
    let mut input = json!({ "quote_record_id": quote });
    input.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
    let (wasm, native) = both_rpc(h, "agreement.accept", input).await;
    assert_eq!(wasm, native);
    wasm["error"].clone()
}

#[tokio::test]
async fn scenario_237_accept_records_what_was_viewed_parity() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    // Each case starts a new conversation with the same peer.
    both_rpc(&h, "contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 100 }))
        .await;

    let said = |lang: &str, via_mt: bool| (json!(lang), json!(via_mt));
    assert_eq!(accept_and_read_back(&h, "none", json!({})).await, said("hi", false));
    assert_eq!(
        accept_and_read_back(&h, "en", json!({ "viewed_lang": "en" })).await,
        said("en", false)
    );
    assert_eq!(
        accept_and_read_back(&h, "ta-mt", json!({ "viewed_lang": "ta", "viewed_via_mt": true }))
            .await,
        said("ta", true)
    );

    // These are refused before anything is signed, so one quote serves all.
    let quote = synced_quote(&h, "refused").await;
    let error = refused(&h, &quote, json!({ "viewed_lang": "ta" })).await;
    assert_eq!(
        (error["code"].as_i64(), &error["data"]),
        (Some(-32602), &json!({ "field": "viewed_lang" }))
    );
    let error = refused(&h, &quote, json!({ "viewed_lang": "hi", "viewed_via_mt": true })).await;
    assert_eq!(
        (error["code"].as_i64(), &error["data"]),
        (Some(-32602), &json!({ "field": "viewed_via_mt" }))
    );
    let error = refused(&h, &quote, json!({ "viewed_lang": "HI" })).await;
    assert_eq!(error["data"], json!({ "field": "viewed_lang" }));
    assert!(error["message"].as_str().unwrap().contains("hi"), "{error}");

    // A repeated accept is checked too: bad values are refused, and not
    // answered as "already accepted".
    let accepted = synced_quote(&h, "repeat").await;
    let (first, _) = both_rpc(&h, "agreement.accept", json!({ "quote_record_id": accepted })).await;
    assert_eq!(first["result"]["role"], "counterparty", "{first}");
    let error = refused(&h, &accepted, json!({ "viewed_lang": "ta" })).await;
    assert_eq!(
        (error["code"].as_i64(), &error["data"]),
        (Some(-32602), &json!({ "field": "viewed_lang" }))
    );
}

/// The quote author's node countersigns with the original language, and the
/// pair is complete although the two halves say different things.
#[tokio::test]
async fn author_countersigns_the_original_when_the_peer_read_a_translation() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;
    let conv = open_conv(&h, &peer_did()).await;
    let (req_rec_id, _) = setup_peer_request(&h, &conv).await;

    let params = json!({
        "enquiry_record_id": req_rec_id,
        "expires_in_secs": 3600,
        "terms": {
            "currency": "EUR",
            "commitments": [
                {
                    "giver": "quote-author",
                    "resource_kind": "goods-or-service",
                    "kind": "fulfilment",
                    "after": [],
                    "description": hi_text("बगीचे की मरम्मत", "Garden repair"),
                    "declared_value_minor": 50000,
                    "location": { "where": "at-receiver", "address": "123 Flower St" }
                },
                {
                    "giver": "counterparty",
                    "resource_kind": "currency",
                    "kind": "currency",
                    "after": ["c1"],
                    "description": hi_text("EUR 500.00", "EUR 500.00"),
                    "declared_value_minor": 50000,
                    "payee": "Garden Services Ltd",
                    "payment_methods": ["card", "cash"],
                    "tax_minor": 5000,
                    "fees_minor": 2000
                }
            ],
            "scope": hi_text("बगीचे की मरम्मत", "Garden repair"),
            "cancellation_terms": hi_text("रद्द करने की शर्तें", "Cancel 24h prior for full refund"),
            "refund_terms": hi_text("वापसी की शर्तें", "Full refund if unsatisfactory"),
            "dispute_path": hi_text("विवाद निवारण", "Contact disputes@example.com")
        }
    });
    let (quote_set, _) = both_rpc(&h, "quote.set", params).await;
    let quote_record_id = quote_set["result"]["record_id"].as_str().unwrap().to_string();
    let (quote, _) =
        both_rpc(&h, "quote.get", json!({ "quote_id": quote_set["result"]["quote_id"] })).await;
    let terms = Envelope::from_json(quote["result"]["envelope"].as_str().unwrap()).unwrap().payload
        ["terms"]
        .clone();

    let (agr_id, receipt) = peer_counterparty_receipt(
        &conv,
        &quote_record_id,
        &peer_did(),
        &owner_did(),
        terms,
        1_001_000,
    );
    deliver_peer_card(
        &h,
        &format!("m-agr-{agr_id}"),
        &conv,
        &peer_did(),
        transaction::RECORD_AGREEMENT_RECEIPT,
        transaction::AGREEMENT_RECEIPT_VERSION,
        &receipt,
    )
    .await;
    both_rpc(&h, "transaction.sync", json!({ "conversation": conv, "full": true })).await;

    let (wasm, native) =
        both_rpc(&h, "agreement.get", json!({ "quote_record_id": quote_record_id })).await;
    for answer in [&wasm, &native] {
        let result = &answer["result"];
        assert_eq!(result["pair"]["state"], "complete", "{answer}");
        assert_eq!(viewed_in(&result["counterparty"]), (json!("en"), json!(false)));
        assert_eq!(viewed_in(&result["quote_author"]), (json!("hi"), json!(false)));
    }
}

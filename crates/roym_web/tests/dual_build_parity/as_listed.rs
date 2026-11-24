//! Dual-build parity tests for transaction driving and as-listed agreements.

use std::time::{Duration, Instant};

use serde_json::json;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_roym_core::transaction;

use super::{fixtures::*, helpers::*};

#[tokio::test]
async fn scenario_252_transaction_drive_with_two_changed_conversations() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    // Initial drive to register collections and capture the baseline event cursor.
    both_rpc(&h, "transaction.drive", json!({})).await;

    let peer1 = Identity::from_bytes(&[11; 32]);
    let peer2 = Identity::from_bytes(&[12; 32]);

    let peer1_did = derive_did_key(&peer1.public_key());
    let peer2_did = derive_did_key(&peer2.public_key());

    let conv1 = open_conv(&h, &peer1_did).await;
    let conv2 = open_conv(&h, &peer2_did).await;

    let (_, _, req1) = peer_signed_request_with_identity(&peer1, &conv1, 1, 1_000_000);
    let (_, _, req2) = peer_signed_request_with_identity(&peer2, &conv2, 1, 1_000_000);

    deliver_peer_card(
        &h,
        "m-req-1",
        &conv1,
        &peer1_did,
        transaction::RECORD_ENQUIRY,
        transaction::ENQUIRY_VERSION,
        &req1,
    )
    .await;

    deliver_peer_card(
        &h,
        "m-req-2",
        &conv2,
        &peer2_did,
        transaction::RECORD_ENQUIRY,
        transaction::ENQUIRY_VERSION,
        &req2,
    )
    .await;

    let (w, n) = both_rpc(&h, "transaction.drive", json!({})).await;
    assert_eq!(w["result"]["synced"], 2, "wasm synced: {w}");
    assert_eq!(n["result"]["synced"], 2, "native synced: {n}");
    assert_eq!(w["result"]["pending"], 0);
    assert_eq!(n["result"]["pending"], 0);
    assert_eq!(w["result"]["again"], false);
    assert_eq!(n["result"]["again"], false);

    let (tw1, tn1) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv1 })).await;
    let (tw2, tn2) = both_rpc(&h, "transaction.thread", json!({ "conversation": conv2 })).await;
    assert_eq!(tw1["result"]["cards"].as_array().unwrap().len(), 1);
    assert_eq!(tn1["result"]["cards"].as_array().unwrap().len(), 1);
    assert_eq!(tw2["result"]["cards"].as_array().unwrap().len(), 1);
    assert_eq!(tn2["result"]["cards"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_stop_rule_measurement_200_agreements_8_conversations() {
    let h = harness().await;
    enrol_signing(&h, "conversation").await;
    enrol_signing(&h, "transaction").await;

    // Initial drive ensures collections exist before seeding agreements
    both_rpc(&h, "transaction.drive", json!({})).await;

    for i in 0..200 {
        let agr_id = format!("rec_agr_{i}");
        let row = json!({
            "quote_record_id": agr_id,
            "quote_id": format!("q_{i}"),
            "conversation": format!("conv_unrelated_{i}"),
            "counterparty_did": peer_did(),
            "quote_author_did": h.owner_did,
            "terms": sample_quote_terms(),
            "state": "agreed",
            "seq": 1,
            "commitments": [],
            "updated_at_secs": 1_000_000,
        });
        seed_row(&h, true, "transaction", "agreements", &agr_id, row.clone()).await;
        seed_row(&h, false, "transaction", "agreements", &agr_id, row).await;
    }

    for i in 0..8 {
        let peer = Identity::from_bytes(&[20 + i as u8; 32]);
        let peer_did = derive_did_key(&peer.public_key());
        let conv = open_conv(&h, &peer_did).await;
        let (_, _, req) = peer_signed_request_with_identity(&peer, &conv, 1, 1_000_000);
        deliver_peer_card(
            &h,
            &format!("m-card-{i}"),
            &conv,
            &peer_did,
            transaction::RECORD_ENQUIRY,
            transaction::ENQUIRY_VERSION,
            &req,
        )
        .await;
    }

    let t0 = Instant::now();
    let (w, n) = both_rpc(&h, "transaction.drive", json!({})).await;
    let elapsed = t0.elapsed();

    eprintln!(
        "STOP_RULE_MEASUREMENT: transaction.drive took {} ms (elapsed: {:?})",
        elapsed.as_millis(),
        elapsed
    );

    assert_eq!(w["result"]["synced"], 8, "wasm synced: {w}");
    assert_eq!(n["result"]["synced"], 8, "native synced: {n}");
    assert!(elapsed < Duration::from_secs(2), "stop rule: must run under 2s, took {:?}", elapsed);
}

/// Signs a listing owned by `key` whose conversation address is the peer's
/// own DID, so a direct conversation to that peer matches it.
fn peer_listing(key: &Identity, peer_did: &str, model: &str, now: u64) -> (String, String) {
    use syneroym_roym_core::{
        deal::LocalText,
        lang::LangTag,
        listing::{
            self, AddressDisclosure, ListingDealTerms, ListingPayload, ListingStatus,
            LocationTerms, OpenTo, PaymentModel, PaymentTerms, RelationshipTerms, ServiceLocation,
        },
        record::{Envelope, RECORD_LISTING, RecordDraft},
    };

    let issuer = derive_did_key(&key.public_key());
    let listing_id = listing::derive_listing_id(&issuer, "peer-hedges").expect("listing id");
    let text = |body: &str| LocalText::plain(LangTag::parse_canonical("en").expect("lang"), body);
    let payload = ListingPayload {
        listing_id: listing_id.clone(),
        slug: "peer-hedges".to_string(),
        title: "Peer hedges".to_string(),
        summary: "Neat hedges.".to_string(),
        lang: LangTag::parse_canonical("en").expect("lang"),
        translations: Vec::new(),
        categories: vec!["gardening".to_string()],
        conversation_address: peer_did.to_string(),
        status: ListingStatus::Active,
        extension: None,
        booking: None,
        payment: Some(PaymentTerms {
            currency: "EUR".to_string(),
            model: match model {
                "fixed" => PaymentModel::Fixed,
                _ => PaymentModel::PerHour,
            },
            amount_minor: Some(3_500),
            tax_included: true,
            fees_minor: None,
            methods: vec!["cash".to_string()],
            payee: "A. Gardener".to_string(),
        }),
        product: None,
        service: None,
        location: Some(LocationTerms {
            where_: ServiceLocation::AtGiver,
            service_area: Vec::new(),
            address_disclosure: AddressDisclosure::OnAgreement,
        }),
        relationship: Some(RelationshipTerms { open_to: OpenTo::Anyone, member_of: None }),
        acceptance: syneroym_roym_core::listing::Acceptance::ConfirmFirst,
        not_limited: true,
        instant_cap: syneroym_roym_core::listing::DEFAULT_INSTANT_CAP,
        deal_terms: Some(ListingDealTerms {
            cancellation_terms: text("Free until 24 hours before"),
            refund_terms: text("Full refund"),
            dispute_path: text("Write to us first"),
            quote_valid_secs: 3_600,
            payment_timing: None,
        }),
        service_record: None,
    };
    let draft = RecordDraft {
        version: listing::LISTING_VERSION,
        record_type: RECORD_LISTING.to_string(),
        subject: listing_id.clone(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, now).expect("envelope");
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).expect("signature");
    (env.to_json().expect("json"), listing_id)
}

#[tokio::test]
async fn scenario_256_enquiry_set_as_listed_not_complete_parity() {
    let h = harness().await;
    enrol_signing(&h, "transaction").await;
    let now = 1_000_000;
    let peer = Identity::from_bytes(&[31; 32]);
    let peer_did = derive_did_key(&peer.public_key());
    let conv = open_conv(&h, &peer_did).await;
    let (envelope, _) = peer_listing(&peer, &peer_did, "per-hour", now);

    let params = json!({
        "conversation": conv,
        "as_listed": {
            "envelope": envelope,
            "valid_for_secs": 3600,
            "viewed_lang": "en",
            "viewed_via_mt": false
        },
        "data_use_notice": "This data stays on the two nodes."
    });
    let (w, n) = both_rpc(&h, "enquiry.set", params).await;
    assert_eq!(w, n, "both builds must refuse the same way");
    assert_eq!(w["error"]["data"]["code"], "as-listed-not-complete", "{w}");
    assert_eq!(w["error"]["data"]["reason"], "price-not-fixed", "{w}");
}

#[tokio::test]
async fn scenario_259_deal_as_listed_status_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    enrol_signing(&h, "transaction").await;
    let mut params = full_listing_params("as-listed-status", "Hedge trimming");
    params["acceptance"] = json!("instant");
    params["not_limited"] = json!(true);
    params["instant_cap"] = json!(3);
    params["deal_terms"] = json!({
        "cancellation_terms": { "lang": "en", "text": "Free until 24 hours before" },
        "refund_terms": { "lang": "en", "text": "Full refund" },
        "dispute_path": { "lang": "en", "text": "Write to us first" },
        "quote_valid_secs": 3600
    });
    let (set, _) = both_rpc(&h, "listing.set", params).await;
    let listing_id = set["result"]["listing_id"].as_str().expect("listing id").to_string();

    let (w, n) = both_rpc(&h, "deal.as-listed-status", json!({ "listing_id": listing_id })).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["acceptance"], "instant", "{w}");
    assert_eq!(w["result"]["instant_cap"], 3);
    assert_eq!(w["result"]["signing_ready"], true);
    assert_eq!(w["result"]["open_instant"], json!([]));
    assert_eq!(w["result"]["in_flight_instant"], 0);
}

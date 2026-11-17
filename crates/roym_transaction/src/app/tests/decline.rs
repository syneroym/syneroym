//! Service tests for signed deal-decline: idempotency and refusal of invalid
//! signers or records.

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    deal::{Commitment, CommitmentKind, DealTerms, Details, ExtensionRef, LocalText, ResourceKind},
    lang::LangTag,
    listing::derive_slot_id,
    record::{Envelope, RECORD_DEAL_DECLINE, RecordDraft},
    transaction::{
        Answers, DEAL_DECLINE_VERSION, DealDeclinePayload, DealRole, QUOTE_VERSION, QuotePayload,
        TimeWindow, derive_quote_id,
    },
};

use crate::app::{
    CARDS, CardRow, QUOTE_HISTORY, QUOTES, RecordPointerRow,
    decline_ops::{self, DealDeclineInput},
    get_row, put_bytes, put_row,
    sync::{self, SyncInput},
    tests::fixture,
};

fn sample_commitment(id: &str, listing_id: &str, start_secs: u64, end_secs: u64) -> Commitment {
    let slot_id = derive_slot_id(listing_id, start_secs, end_secs).expect("slot id");
    Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::ServiceInSlot,
        kind: CommitmentKind::Fulfilment,
        extension: Some(ExtensionRef { id: "scheduling".to_string(), version: 1 }),
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("lang"),
            "Slot service",
        ),
        declared_value_minor: 5000,
        details: Some(Details {
            extension: "scheduling".to_string(),
            schema: "scheduling/slot@1".to_string(),
            data: json!({ "listing_id": listing_id, "slot_id": slot_id }),
            extra: vec![],
        }),
        window: Some(TimeWindow { earliest_secs: start_secs, latest_secs: end_secs }),
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }
}

fn make_quote_envelope(
    author: &Identity,
    cp_did: &str,
    now: u64,
) -> (String, String, QuotePayload) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let quote_id = derive_quote_id("conv_1", &author_did, 1).expect("quote id");
    let terms = DealTerms {
        currency: "EUR".to_string(),
        commitments: vec![sample_commitment("c1", "list_1", now + 1000, now + 4000)],
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Scope"),
        cancellation_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "None"),
        refund_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "None"),
        dispute_path: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "None"),
        quote_expires_at_secs: now + 3600,
    };
    let payload = QuotePayload {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence: 1,
        counterparty_did: cp_did.to_string(),
        terms,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
    };
    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: "quote".to_string(),
        subject: "conv_1".to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: Some(now + 3600),
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, author_did, None, now).expect("env");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let rec_id = env.record_id().expect("rec id");
    (env.to_json().expect("json"), rec_id, payload)
}

fn make_decline_envelope(
    signer: &Identity,
    declines: &str,
    deal: &str,
    conv: &str,
    now: u64,
) -> (String, String) {
    let signer_did = substrate::derive_did_key(&signer.public_key());
    let payload = DealDeclinePayload {
        declines: declines.to_string(),
        deal: deal.to_string(),
        conversation: conv.to_string(),
        reason: None,
    };
    let draft = RecordDraft {
        version: DEAL_DECLINE_VERSION,
        record_type: RECORD_DEAL_DECLINE.to_string(),
        subject: declines.to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, signer_did, None, now).expect("env");
    let sig = signer.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let rec_id = env.record_id().expect("rec id");
    (env.to_json().expect("json"), rec_id)
}

#[tokio::test]
async fn repeated_decline_returns_the_same_record() {
    let f = fixture();
    let now = f.host.now();
    let author = Identity::generate().expect("author");
    let author_did = substrate::derive_did_key(&author.public_key());

    // f is counterparty
    let (q_json, q_rec_id, quote) = make_quote_envelope(&author, &f.master_did, now);
    put_bytes(&f.host, QUOTE_HISTORY, &q_rec_id, q_json.as_bytes()).await.unwrap();

    let ptr = RecordPointerRow {
        envelope: q_json,
        record_id: q_rec_id.clone(),
        id: quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: author_did,
        mine: false,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(f.master_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, QUOTES, &quote.quote_id, &ptr).await.unwrap();

    let out1 = decline_ops::decline_at(
        &f.host,
        DealDeclineInput { declines: q_rec_id.clone(), note: Some("not today".to_string()) },
        now,
    )
    .await
    .unwrap();
    assert!(out1.declined);

    let out2 = decline_ops::decline_at(
        &f.host,
        DealDeclineInput { declines: q_rec_id.clone(), note: Some("not today".to_string()) },
        now + 10,
    )
    .await
    .unwrap();
    assert_eq!(out1.record_id, out2.record_id);
    assert_eq!(out1.declines, out2.declines);
    assert_eq!(out1.deal, out2.deal);
}

#[tokio::test]
async fn decline_filing_checks_refuse_invalid_signers_or_records() {
    let f = fixture();
    let now = f.host.now();
    let third_party = Identity::generate().expect("third_party");

    // f is quote author
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let (q_json, q_rec_id, quote) = make_quote_envelope(&f.master, &cp_did, now);
    put_bytes(&f.host, QUOTE_HISTORY, &q_rec_id, q_json.as_bytes()).await.unwrap();

    let ptr = RecordPointerRow {
        envelope: q_json,
        record_id: q_rec_id.clone(),
        id: quote.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: f.master_did.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(cp_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, QUOTES, &quote.quote_id, &ptr).await.unwrap();

    // 1. Third party signs decline for quote -> refused: only counterparty can
    //    decline
    let (bad_dec_json, _) =
        make_decline_envelope(&third_party, &q_rec_id, &quote.quote_id, "conv_1", now);
    let bad_card_body = card::card_body(RECORD_DEAL_DECLINE, DEAL_DECLINE_VERSION, &bad_dec_json)
        .expect("card body");

    f.host.on_call("conversation", "conversation.changes", move |_| {
        Ok(json!({
            "result": {
                "messages": [
                    {
                        "id": "msg_bad_dec",
                        "conversation": "conv_1",
                        "author": "did:key:third",
                        "direction": "incoming",
                        "sender_timestamp_ms": (now * 1000) as i64,
                        "content_type": CARD_CONTENT_TYPE,
                        "languages": ["en"],
                        "body_encoding": "utf8",
                        "restored": false,
                        "state": "delivered",
                        "stored_at_secs": now as i64,
                        "visible_seq": 1,
                        "verified": true,
                        "body": bad_card_body,
                    }
                ],
                "last_seq": 1,
                "kind": "direct"
            }
        })
        .to_string())
    });

    let sync_res =
        sync::sync(&f.host, SyncInput { conversation: "conv_1".to_string(), full: false })
            .await
            .unwrap();
    assert_eq!(sync_res.refused, 1);

    let card: CardRow = get_row(&f.host, CARDS, "msg_bad_dec").await.unwrap().unwrap();
    assert_eq!(card.reason.as_deref(), Some("only the counterparty can decline this quote"));
}

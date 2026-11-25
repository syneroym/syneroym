use serde_json::{Value, json};
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    conversation::Direction,
    deal::{Commitment, CommitmentKind, DealTerms, Details, ExtensionRef, LocalText, ResourceKind},
    lang::LangTag,
    listing::derive_slot_id,
    progress::{self, AGREEMENT_PROGRESS_VERSION, FormedAs, RECORD_AGREEMENT_PROGRESS},
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RECORD_QUOTE, RecordDraft},
    testing::{
        TestHost,
        sd::{PlainQuote, conceal_quote},
    },
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, Answers, DealRole, QUOTE_VERSION,
        SignedTerms, TimeWindow, derive_quote_id, verify_quote,
    },
};

use super::fixture;
use crate::app::{
    AGREEMENTS, AgreementRow, CARDS, CardRow,
    agreement_ops::{self, AgreementAcceptInput, QuoteRecordInput},
    disclosures::{self, HeldQuote},
    get_row,
    quote_ops::store_received_quote,
    sync::{self, SyncInput, receipts::file_progress_card},
};

pub(super) fn slot_commitment(
    id: &str,
    listing_id: &str,
    start_secs: u64,
    end_secs: u64,
) -> (Commitment, String) {
    let slot_id = derive_slot_id(listing_id, start_secs, end_secs).expect("derive slot id");
    let commitment = Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::Counterparty,
        resource_kind: ResourceKind::ServiceInSlot,
        kind: CommitmentKind::Fulfilment,
        extension: Some(ExtensionRef { id: "scheduling".to_string(), version: 1 }),
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Piano lesson in slot",
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
    };
    (commitment, slot_id)
}

pub(super) fn author_service_commitment(id: &str) -> Commitment {
    Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::GoodsOrService,
        kind: CommitmentKind::Fulfilment,
        extension: None,
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Tuning service",
        ),
        declared_value_minor: 5000,
        details: None,
        window: None,
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }
}

pub(super) fn build_and_sign_quote(
    author: &Identity,
    counterparty_did: &str,
    commitments: Vec<Commitment>,
    expires_at_secs: u64,
    now: u64,
) -> (HeldQuote, String, String) {
    quote_answering(author, counterparty_did, "rec_enq_1", commitments, expires_at_secs, now)
}

/// A quote signed by `author` that answers the enquiry `enquiry_record_id`.
pub(super) fn quote_answering(
    author: &Identity,
    counterparty_did: &str,
    enquiry_record_id: &str,
    commitments: Vec<Commitment>,
    expires_at_secs: u64,
    now: u64,
) -> (HeldQuote, String, String) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let quote_id = derive_quote_id("conv_1", &author_did, 1).expect("derive quote id");
    let terms = DealTerms {
        currency: "EUR".to_string(),
        commitments,
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("parse lang"), "Lesson deal"),
        cancellation_terms: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "24h notice",
        ),
        refund_terms: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Full refund",
        ),
        dispute_path: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Arbitration",
        ),
        quote_expires_at_secs: expires_at_secs,
    };
    let plain = PlainQuote {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: enquiry_record_id.to_string() },
        counterparty_did: counterparty_did.to_string(),
        terms: terms.clone(),
    };
    let (payload, disclosures) = conceal_quote(plain, 1);

    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: "conv_1".to_string(),
        payload: serde_json::to_value(&payload).expect("serialize payload"),
        expires_at_secs: Some(expires_at_secs),
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, author_did, None, now).expect("draft envelope");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach signature");
    let record_id = env.record_id().expect("record id");
    let envelope_json = env.to_json().expect("envelope json");
    let verdict = verify_quote(&envelope_json, now);
    let held = HeldQuote { verdict, signed: payload, terms, disclosures };
    (held, record_id, envelope_json)
}

fn make_progress_card(
    author: &Identity,
    agreement: &str,
    counterparty_did: &str,
    terms: &DealTerms,
    state_reason: Option<&str>,
    now: u64,
) -> (CardRow, String) {
    make_progress_card_with_formed_as(
        author,
        agreement,
        counterparty_did,
        terms,
        state_reason,
        FormedAs::Quote,
        now,
    )
}

fn make_progress_card_with_formed_as(
    author: &Identity,
    agreement: &str,
    counterparty_did: &str,
    terms: &DealTerms,
    state_reason: Option<&str>,
    formed_as: FormedAs,
    now: u64,
) -> (CardRow, String) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let progress_payload = progress::open(
        agreement.to_string(),
        "conv_1".to_string(),
        counterparty_did.to_string(),
        author_did.clone(),
        terms,
        formed_as,
        state_reason.map(str::to_string),
    );

    let draft = RecordDraft {
        version: AGREEMENT_PROGRESS_VERSION,
        record_type: RECORD_AGREEMENT_PROGRESS.to_string(),
        subject: agreement.to_string(),
        payload: serde_json::to_value(&progress_payload).expect("serialize payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, author_did, None, now).expect("draft progress");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach signature");
    let envelope_json = env.to_json().expect("envelope json");

    let card_row = CardRow {
        message_id: "msg_progress_1".to_string(),
        conversation: "conv_1".to_string(),
        direction: Direction::Incoming,
        sender_timestamp_ms: (now * 1000) as i64,
        card_type: "agreement-progress".to_string(),
        version: AGREEMENT_PROGRESS_VERSION,
        known: true,
        verified: true,
        expired: false,
        repeat: false,
        reason: None,
        issuer: None,
        record_id: None,
        revocation_status: None,
        data: None,
        stored_at_secs: now,
        declined: None,
        deal_state: None,
        version_count: None,
        agreement_payee: None,
        agreement_payment_methods: vec![],
    };

    (card_row, envelope_json)
}

#[tokio::test]
async fn authors_own_accept_is_refused() {
    let f = fixture();
    let now = f.host.now();
    let other_user = "did:key:z6MkuOtherCounterpartyUser00000000000000000";

    let (quote, record_id, envelope_json) = build_and_sign_quote(
        &f.master,
        other_user,
        vec![author_service_commitment("c1")],
        now + 3600,
        now,
    );

    let verdict = verify_quote(&envelope_json, now);
    assert!(verdict.verified, "verdict reason: {:?}", verdict.reason);
    store_received_quote(&f.host, &envelope_json, &verdict, &quote.signed, now, &f.master_did)
        .await
        .expect("store quote");
    disclosures::store(&f.host, &record_id, RECORD_QUOTE, &quote.disclosures, now)
        .await
        .expect("store disclosures");

    let input =
        AgreementAcceptInput { quote_record_id: record_id, viewed_lang: None, viewed_via_mt: None };
    let result = agreement_ops::accept_at(&f.host, input, now).await;
    let err = result.expect_err("quote author cannot accept own quote");
    assert_eq!(err.error.as_ref().unwrap().message, "writer-countersigns");
    assert_eq!(err.error.as_ref().unwrap().data, Some(json!("writer-countersigns")));
}

#[tokio::test]
async fn repeated_accept_returns_the_same_half() {
    let f = fixture();
    let now = f.host.now();
    let author = Identity::generate().expect("generate author");
    let (c1, slot_id) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);

    let (quote, record_id, envelope_json) =
        build_and_sign_quote(&author, &f.master_did, vec![c1], now + 3600, now);

    let verdict = verify_quote(&envelope_json, now);
    assert!(verdict.verified, "verdict reason: {:?}", verdict.reason);
    store_received_quote(&f.host, &envelope_json, &verdict, &quote.signed, now, &f.master_did)
        .await
        .expect("store quote");
    disclosures::store(&f.host, &record_id, RECORD_QUOTE, &quote.disclosures, now)
        .await
        .expect("store disclosures");

    let input1 = AgreementAcceptInput {
        quote_record_id: record_id.clone(),
        viewed_lang: None,
        viewed_via_mt: None,
    };
    let out1 = agreement_ops::accept_at(&f.host, input1, now).await.expect("first accept succeeds");

    let input2 = AgreementAcceptInput {
        quote_record_id: record_id.clone(),
        viewed_lang: None,
        viewed_via_mt: None,
    };
    let out2 =
        agreement_ops::accept_at(&f.host, input2, now).await.expect("second accept succeeds");

    assert_eq!(out1.record_id, out2.record_id);
    assert_eq!(out1.pair, out2.pair);

    assert!(f.scheduling.is_held(&record_id, "c1"));
    assert_eq!(f.scheduling.held_seats(&slot_id), 1, "a repeat accept holds one seat");
}

#[tokio::test]
async fn counterparty_holds_its_own_seat_before_signing() {
    let f = fixture();
    let now = f.host.now();
    let author = Identity::generate().expect("generate author");
    let (c1, slot_id) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);

    let (quote, record_id, envelope_json) =
        build_and_sign_quote(&author, &f.master_did, vec![c1], now + 3600, now);

    let verdict = verify_quote(&envelope_json, now);
    assert!(verdict.verified, "verdict reason: {:?}", verdict.reason);
    store_received_quote(&f.host, &envelope_json, &verdict, &quote.signed, now, &f.master_did)
        .await
        .expect("store quote");
    disclosures::store(&f.host, &record_id, RECORD_QUOTE, &quote.disclosures, now)
        .await
        .expect("store disclosures");

    let input = AgreementAcceptInput {
        quote_record_id: record_id.clone(),
        viewed_lang: None,
        viewed_via_mt: None,
    };
    agreement_ops::accept_at(&f.host, input, now).await.expect("accept holds seat and signs");

    assert!(f.scheduling.is_held(&record_id, "c1"), "the extension holds the seat");
    assert_eq!(f.scheduling.held_seats(&slot_id), 1);
    assert_eq!(f.scheduling.calls("reservation.reserve"), 1);
}

#[tokio::test]
async fn taken_seat_answers_reserve_failed_and_signs_nothing() {
    let f = fixture();
    let now = f.host.now();
    let author = Identity::generate().expect("generate author");
    let (c1, slot_id) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);

    f.scheduling.take_seat(&slot_id, "agr_earlier_winner");

    let (quote, record_id, envelope_json) =
        build_and_sign_quote(&author, &f.master_did, vec![c1], now + 3600, now);

    let verdict = verify_quote(&envelope_json, now);
    assert!(verdict.verified, "verdict reason: {:?}", verdict.reason);
    store_received_quote(&f.host, &envelope_json, &verdict, &quote.signed, now, &f.master_did)
        .await
        .expect("store quote");
    disclosures::store(&f.host, &record_id, RECORD_QUOTE, &quote.disclosures, now)
        .await
        .expect("store disclosures");

    let input = AgreementAcceptInput {
        quote_record_id: record_id.clone(),
        viewed_lang: None,
        viewed_via_mt: None,
    };
    let err = agreement_ops::accept_at(&f.host, input, now)
        .await
        .expect_err("taken seat must reject accept");
    let error = err.error.as_ref().unwrap();
    assert_eq!(error.message, "reserve-failed");
    assert_eq!(error.data.as_ref().unwrap()["reason"], "slot-taken");
    assert!(!f.scheduling.is_held(&record_id, "c1"), "nothing is held for a refused reserve");

    let agr = get_row::<AgreementRow, _>(&f.host, "agreements", &record_id)
        .await
        .expect("query agreement");
    assert!(
        agr.as_ref().and_then(|a| a.counterparty.as_ref()).is_none(),
        "no counterparty half must be stored when seat reservation fails"
    );
}

#[tokio::test]
async fn counterparty_releases_holds_on_not_formed_and_expiry() {
    let f = fixture();
    let now = f.host.now();
    let author = Identity::generate().expect("generate author");

    // Part 1: Hold released when author notifies not-formed
    let (c1, slot_id1) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (quote1, rec_id1, env1) =
        build_and_sign_quote(&author, &f.master_did, vec![c1], now + 3600, now);
    let v1 = verify_quote(&env1, now);
    store_received_quote(&f.host, &env1, &v1, &quote1.signed, now, &f.master_did)
        .await
        .expect("store quote 1");
    disclosures::store(&f.host, &rec_id1, RECORD_QUOTE, &quote1.disclosures, now)
        .await
        .expect("store disclosures 1");

    let input1 = AgreementAcceptInput {
        quote_record_id: rec_id1.clone(),
        viewed_lang: None,
        viewed_via_mt: None,
    };
    agreement_ops::accept_at(&f.host, input1, now).await.expect("accept 1");
    assert!(f.scheduling.is_held(&rec_id1, "c1"));

    let (card_row, snap_env) = make_progress_card(
        &author,
        &rec_id1,
        &f.master_did,
        &quote1.terms,
        Some("slot-taken"),
        now + 10,
    );
    let res =
        file_progress_card(&f.host, card_row, &snap_env, &[], "conv_1", now + 10, &f.master_did)
            .await
            .expect("file progress card");
    assert!(res.filed, "file_progress_card failed: {res:?}");
    assert!(!res.refused, "file_progress_card was refused: {res:?}");

    assert!(f.scheduling.is_released(&rec_id1, "c1"), "not-formed snapshot must release the hold");
    assert_eq!(f.scheduling.held_seats(&slot_id1), 0, "not-formed snapshot must free the seat");

    // Part 2: Hold released on quote expiry
    let author2 = Identity::generate().expect("generate author 2");
    let (c2, slot_id2) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (quote2, rec_id2, env2) =
        build_and_sign_quote(&author2, &f.master_did, vec![c2], now + 300, now);
    let v2 = verify_quote(&env2, now);
    assert!(v2.verified, "v2 reason: {:?}", v2.reason);
    store_received_quote(&f.host, &env2, &v2, &quote2.signed, now, &f.master_did)
        .await
        .expect("store quote 2");
    disclosures::store(&f.host, &rec_id2, RECORD_QUOTE, &quote2.disclosures, now)
        .await
        .expect("store disclosures 2");

    let input2 = AgreementAcceptInput {
        quote_record_id: rec_id2.clone(),
        viewed_lang: None,
        viewed_via_mt: None,
    };
    agreement_ops::accept_at(&f.host, input2, now).await.expect("accept 2");

    let get_input = QuoteRecordInput { quote_record_id: rec_id2.clone() };
    agreement_ops::get_at(&f.host, get_input, now + 350).await.expect("get after expiry");

    assert!(f.scheduling.is_released(&rec_id2, "c1"), "expired quote must release the hold");
    assert_eq!(f.scheduling.held_seats(&slot_id2), 0, "expired quote must free the seat");
}

#[tokio::test]
async fn file_repeated_card_stores_repeat_true() {
    let f = fixture();
    let author = Identity::generate().expect("generate author");
    let now = 1_000_000;
    let (c1, _slot_id) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (quote, rec_id, env) =
        build_and_sign_quote(&author, &f.master_did, vec![c1], now + 300, now);
    let v = verify_quote(&env, now);
    store_received_quote(&f.host, &env, &v, &quote.signed, now, &f.master_did)
        .await
        .expect("store quote");
    disclosures::store(&f.host, &rec_id, RECORD_QUOTE, &quote.disclosures, now)
        .await
        .expect("store disclosures");

    let (card_row1, snap_env) = make_progress_card(
        &author,
        &rec_id,
        &f.master_did,
        &quote.terms,
        Some("slot-taken"),
        now + 10,
    );
    let res1 =
        file_progress_card(&f.host, card_row1, &snap_env, &[], "conv_1", now + 10, &f.master_did)
            .await
            .expect("file first progress card");
    assert!(res1.filed, "res1: {:?}", res1);

    let first_stored: CardRow = get_row(&f.host, CARDS, "msg_progress_1")
        .await
        .expect("query first card")
        .expect("first card exists");
    assert!(!first_stored.repeat, "first filed card must not be repeat");

    let mut card_row2 = make_progress_card(
        &author,
        &rec_id,
        &f.master_did,
        &quote.terms,
        Some("slot-taken"),
        now + 10,
    )
    .0;
    card_row2.message_id = "msg_snap_repeat".to_string();

    let res2 =
        file_progress_card(&f.host, card_row2, &snap_env, &[], "conv_1", now + 10, &f.master_did)
            .await
            .expect("file second progress card");
    assert!(res2.filed);

    let second_stored: CardRow = get_row(&f.host, CARDS, "msg_snap_repeat")
        .await
        .expect("query second card")
        .expect("second card exists");
    assert!(second_stored.repeat, "second filed card must have repeat: true");
}

pub(super) fn make_author_countersigned_half_card(
    author: &Identity,
    author_did: &str,
    counterparty_did: &str,
    q_rec_id: &str,
    terms: &SignedTerms,
    issued_at: u64,
) -> String {
    let author_payload = AgreementReceiptPayload {
        quote_record_id: q_rec_id.to_string(),
        counterparty_did: counterparty_did.to_string(),
        quote_author_did: author_did.to_string(),
        role: DealRole::QuoteAuthor,
        terms: terms.clone(),
        viewed_lang: LangTag::parse_canonical("en").expect("lang"),
        viewed_via_mt: false,
    };
    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: q_rec_id.to_string(),
        payload: serde_json::to_value(&author_payload).expect("payload json"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, author_did.to_string(), None, issued_at).expect("env");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let author_half_env = env.to_json().expect("env json");
    card::card_body(RECORD_AGREEMENT_RECEIPT, AGREEMENT_RECEIPT_VERSION, &author_half_env, &[])
        .expect("card body")
}

pub(super) fn mock_author_half_delivered_at(
    host: &TestHost,
    author_did: String,
    delivery_time: u64,
    card_body: String,
) {
    host.on_call("conversation", "conversation.changes", move |params: String| {
        let p: Value = serde_json::from_str(&params).unwrap_or(Value::Null);
        let after_seq = p.get("after_seq").and_then(Value::as_u64).unwrap_or(0);
        if after_seq == 0 {
            Ok(json!({
                "result": {
                    "messages": [
                        {
                            "id": "msg_author_half",
                            "conversation": "conv_1",
                            "author": author_did,
                            "direction": "incoming",
                            "sender_timestamp_ms": (delivery_time * 1000) as i64,
                            "content_type": CARD_CONTENT_TYPE,
                            "languages": ["en"],
                            "body_encoding": "utf8",
                            "restored": false,
                            "state": "delivered",
                            "stored_at_secs": delivery_time as i64,
                            "visible_seq": 1,
                            "verified": true,
                            "body": card_body,
                        }
                    ],
                    "last_seq": 1,
                    "kind": "direct"
                }
            })
            .to_string())
        } else {
            Ok(json!({
                "result": {
                    "messages": [],
                    "last_seq": 1,
                    "kind": "direct"
                }
            })
            .to_string())
        }
    });
}

#[tokio::test]
async fn counterparty_files_author_countersigned_half_delivered_after_expiry() {
    let f = fixture();
    let author = Identity::generate().expect("author identity");
    let author_did = substrate::derive_did_key(&author.public_key());
    let now = 1_000_000;
    let (c1, _slot_id) = slot_commitment("c1", "listing_123", now + 1_000, now + 4_600);
    // Quote expires at now + 300
    let (quote_payload, q_rec_id, q_env) =
        build_and_sign_quote(&author, &f.master_did, vec![c1], now + 300, now);
    let v = verify_quote(&q_env, now);
    store_received_quote(&f.host, &q_env, &v, &quote_payload.signed, now, &f.master_did)
        .await
        .expect("store quote");
    disclosures::store(&f.host, &q_rec_id, RECORD_QUOTE, &quote_payload.disclosures, now)
        .await
        .expect("store disclosures");

    // Counterparty accepts at now + 10 (before expiry)
    let acc_res = agreement_ops::accept_at(
        &f.host,
        AgreementAcceptInput {
            quote_record_id: q_rec_id.clone(),
            viewed_lang: None,
            viewed_via_mt: None,
        },
        now + 10,
    )
    .await
    .expect("counterparty accepts");
    assert_eq!(acc_res.role, DealRole::Counterparty);

    // Quote author signs countersigned receipt half at now + 50 (before expiry)
    let card_body = make_author_countersigned_half_card(
        &author,
        &author_did,
        &f.master_did,
        &q_rec_id,
        &quote_payload.signed.terms,
        now + 50,
    );

    // Author's half is delivered to counterparty at now + 400 (after expiry now +
    // 300)
    mock_author_half_delivered_at(&f.host, author_did, now + 400, card_body);

    let sync_res = sync::sync_at(
        &f.host,
        SyncInput { conversation: "conv_1".to_string(), full: false },
        now + 400,
    )
    .await
    .expect("sync at now + 400");
    assert_eq!(sync_res.filed, 1);
    assert_eq!(sync_res.refused, 0);

    // Stored card verified is true and no refusal reason
    let card: CardRow =
        get_row(&f.host, CARDS, "msg_author_half").await.expect("get card").expect("card exists");
    assert!(card.verified);
    assert!(card.reason.is_none());

    // Agreement row now has both halves and agreed_at_secs set to the author's
    // issued_at
    let agr: AgreementRow =
        get_row(&f.host, AGREEMENTS, &q_rec_id).await.expect("get agr").expect("agr exists");
    assert!(agr.quote_author.is_some());
    assert_eq!(agr.agreed_at_secs, Some(now + 50));
}

#[tokio::test]
async fn a_snapshot_whose_formed_as_changed_is_refused() {
    let f = fixture();
    let now = f.host.now();
    let author = Identity::generate().expect("generate author");
    let (quote, rec_id, env) = build_and_sign_quote(
        &author,
        &f.master_did,
        vec![author_service_commitment("c1")],
        now + 3600,
        now,
    );
    let v = verify_quote(&env, now);
    store_received_quote(&f.host, &env, &v, &quote.signed, now, &f.master_did)
        .await
        .expect("store quote");
    disclosures::store(&f.host, &rec_id, RECORD_QUOTE, &quote.disclosures, now)
        .await
        .expect("store disclosures");
    agreement_ops::accept_at(
        &f.host,
        AgreementAcceptInput {
            quote_record_id: rec_id.clone(),
            viewed_lang: None,
            viewed_via_mt: None,
        },
        now,
    )
    .await
    .expect("accept");

    let (card, snap_env) = make_progress_card_with_formed_as(
        &author,
        &rec_id,
        &f.master_did,
        &quote.terms,
        None,
        FormedAs::AsListedInstant,
        now + 10,
    );
    let res = file_progress_card(&f.host, card, &snap_env, &[], "conv_1", now + 10, &f.master_did)
        .await
        .expect("file progress card");
    assert!(res.filed && !res.refused, "{res:?}");

    let (card, snap_env) = make_progress_card_with_formed_as(
        &author,
        &rec_id,
        &f.master_did,
        &quote.terms,
        None,
        FormedAs::Quote,
        now + 20,
    );
    let res = file_progress_card(&f.host, card, &snap_env, &[], "conv_1", now + 20, &f.master_did)
        .await
        .expect("file progress card");
    assert!(res.refused, "{res:?}");
    let stored: CardRow =
        get_row(&f.host, CARDS, "msg_progress_1").await.expect("get card").expect("card exists");
    assert_eq!(stored.reason.as_deref(), Some("formed-as-changed"));
}

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    conversation::Direction,
    deal::{Commitment, CommitmentKind, DealTerms, Details, ExtensionRef, LocalText, ResourceKind},
    lang::LangTag,
    listing::derive_slot_id,
    progress::{self, AGREEMENT_PROGRESS_VERSION, RECORD_AGREEMENT_PROGRESS},
    record::{Envelope, RECORD_QUOTE, RecordDraft},
    transaction::{
        Answers, DealRole, QUOTE_VERSION, QuotePayload, TimeWindow, derive_quote_id, verify_quote,
    },
};

use super::fixture;
use crate::app::{
    AgreementRow, CardRow, LEDGER,
    agreement_ops::{self, AgreementAcceptInput, QuoteRecordInput},
    get_row,
    hold::{HoldLedgerRow, SeatLedgerRow, hold_id, seat_id},
    quote_ops::store_received_quote,
    sync::receipts::file_progress_card,
};

fn slot_commitment(
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

fn author_service_commitment(id: &str) -> Commitment {
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

fn build_and_sign_quote(
    author: &Identity,
    counterparty_did: &str,
    commitments: Vec<Commitment>,
    expires_at_secs: u64,
    now: u64,
) -> (QuotePayload, String, String) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let quote_id = derive_quote_id("conv_1", &author_did, 1).expect("derive quote id");
    let payload = QuotePayload {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence: 1,
        counterparty_did: counterparty_did.to_string(),
        terms: DealTerms {
            currency: "EUR".to_string(),
            commitments,
            home_exchange: None,
            listing_id: None,
            scope: LocalText::plain(
                LangTag::parse_canonical("en").expect("parse lang"),
                "Lesson deal",
            ),
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
        },
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
    };

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
    (payload, record_id, envelope_json)
}

fn make_progress_card(
    author: &Identity,
    agreement: &str,
    counterparty_did: &str,
    terms: &DealTerms,
    state_reason: Option<&str>,
    now: u64,
) -> (CardRow, String) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let progress_payload = progress::open(
        agreement.to_string(),
        "conv_1".to_string(),
        counterparty_did.to_string(),
        author_did.clone(),
        terms,
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
        reason: None,
        issuer: None,
        record_id: None,
        revocation_status: None,
        data: None,
        stored_at_secs: now,
        declined: None,
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

    let (payload, record_id, envelope_json) = build_and_sign_quote(
        &f.master,
        other_user,
        vec![author_service_commitment("c1")],
        now + 3600,
        now,
    );

    let verdict = verify_quote(&envelope_json, now);
    assert!(verdict.verified, "verdict reason: {:?}", verdict.reason);
    store_received_quote(&f.host, &envelope_json, &verdict, &payload, now, &f.master_did)
        .await
        .expect("store quote");

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

    let (payload, record_id, envelope_json) =
        build_and_sign_quote(&author, &f.master_did, vec![c1], now + 3600, now);

    let verdict = verify_quote(&envelope_json, now);
    assert!(verdict.verified, "verdict reason: {:?}", verdict.reason);
    store_received_quote(&f.host, &envelope_json, &verdict, &payload, now, &f.master_did)
        .await
        .expect("store quote");

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

    let hold = get_row::<HoldLedgerRow, _>(&f.host, LEDGER, &hold_id(&record_id, "c1"))
        .await
        .expect("query ledger")
        .expect("hold row exists");
    assert_eq!(hold.seat, 1);
    assert_eq!(hold.slot_id, slot_id);
}

#[tokio::test]
async fn counterparty_holds_its_own_seat_before_signing() {
    let f = fixture();
    let now = f.host.now();
    let author = Identity::generate().expect("generate author");
    let (c1, slot_id) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);

    let (payload, record_id, envelope_json) =
        build_and_sign_quote(&author, &f.master_did, vec![c1], now + 3600, now);

    let verdict = verify_quote(&envelope_json, now);
    assert!(verdict.verified, "verdict reason: {:?}", verdict.reason);
    store_received_quote(&f.host, &envelope_json, &verdict, &payload, now, &f.master_did)
        .await
        .expect("store quote");

    let input = AgreementAcceptInput {
        quote_record_id: record_id.clone(),
        viewed_lang: None,
        viewed_via_mt: None,
    };
    agreement_ops::accept_at(&f.host, input, now).await.expect("accept holds seat and signs");

    let hold = get_row::<HoldLedgerRow, _>(&f.host, LEDGER, &hold_id(&record_id, "c1"))
        .await
        .expect("query hold")
        .expect("hold row must be recorded in ledger");
    assert_eq!(hold.seat, 1);
    assert_eq!(hold.slot_id, slot_id);

    let seat = get_row::<SeatLedgerRow, _>(&f.host, LEDGER, &seat_id(&slot_id, 1))
        .await
        .expect("query seat")
        .expect("seat row must be recorded in ledger");
    assert_eq!(seat.agreement, record_id);
    assert_eq!(seat.commitment_id, "c1");
}

#[tokio::test]
async fn taken_seat_answers_reserve_failed_and_signs_nothing() {
    let f = fixture();
    let now = f.host.now();
    let author = Identity::generate().expect("generate author");
    let (c1, slot_id) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);

    let existing_seat = SeatLedgerRow {
        kind: "seat".to_string(),
        slot_id: slot_id.clone(),
        seat: 1,
        agreement: "agr_earlier_winner".to_string(),
        commitment_id: "c1".to_string(),
        created_at_secs: now - 100,
    };
    f.host.set_storage_row(LEDGER, &seat_id(&slot_id, 1), &existing_seat);

    let (payload, record_id, envelope_json) =
        build_and_sign_quote(&author, &f.master_did, vec![c1], now + 3600, now);

    let verdict = verify_quote(&envelope_json, now);
    assert!(verdict.verified, "verdict reason: {:?}", verdict.reason);
    store_received_quote(&f.host, &envelope_json, &verdict, &payload, now, &f.master_did)
        .await
        .expect("store quote");

    let input = AgreementAcceptInput {
        quote_record_id: record_id.clone(),
        viewed_lang: None,
        viewed_via_mt: None,
    };
    let err = agreement_ops::accept_at(&f.host, input, now)
        .await
        .expect_err("taken seat must reject accept");
    assert_eq!(err.error.as_ref().unwrap().message, "reserve-failed");

    let hold = get_row::<HoldLedgerRow, _>(&f.host, LEDGER, &hold_id(&record_id, "c1"))
        .await
        .expect("query hold");
    assert!(hold.is_none(), "no hold row should remain for failed reservation");

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
    let (payload1, rec_id1, env1) =
        build_and_sign_quote(&author, &f.master_did, vec![c1], now + 3600, now);
    let v1 = verify_quote(&env1, now);
    store_received_quote(&f.host, &env1, &v1, &payload1, now, &f.master_did)
        .await
        .expect("store quote 1");

    let input1 = AgreementAcceptInput {
        quote_record_id: rec_id1.clone(),
        viewed_lang: None,
        viewed_via_mt: None,
    };
    agreement_ops::accept_at(&f.host, input1, now).await.expect("accept 1");
    assert!(
        get_row::<HoldLedgerRow, _>(&f.host, LEDGER, &hold_id(&rec_id1, "c1"))
            .await
            .unwrap()
            .is_some()
    );

    let (card_row, snap_env) = make_progress_card(
        &author,
        &rec_id1,
        &f.master_did,
        &payload1.terms,
        Some("slot-taken"),
        now + 10,
    );
    let res = file_progress_card(&f.host, card_row, &snap_env, "conv_1", now + 10, &f.master_did)
        .await
        .expect("file progress card");
    assert!(res.filed, "file_progress_card failed: {res:?}");
    assert!(!res.refused, "file_progress_card was refused: {res:?}");

    let hold1 = get_row::<HoldLedgerRow, _>(&f.host, LEDGER, &hold_id(&rec_id1, "c1"))
        .await
        .expect("query hold 1");
    assert!(hold1.is_none(), "not-formed snapshot must release the hold");
    let seat1 = get_row::<SeatLedgerRow, _>(&f.host, LEDGER, &seat_id(&slot_id1, 1))
        .await
        .expect("query seat 1");
    assert!(seat1.is_none(), "not-formed snapshot must release the seat");

    // Part 2: Hold released on quote expiry
    let author2 = Identity::generate().expect("generate author 2");
    let (c2, slot_id2) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (payload2, rec_id2, env2) =
        build_and_sign_quote(&author2, &f.master_did, vec![c2], now + 300, now);
    let v2 = verify_quote(&env2, now);
    assert!(v2.verified, "v2 reason: {:?}", v2.reason);
    store_received_quote(&f.host, &env2, &v2, &payload2, now, &f.master_did)
        .await
        .expect("store quote 2");

    let input2 = AgreementAcceptInput {
        quote_record_id: rec_id2.clone(),
        viewed_lang: None,
        viewed_via_mt: None,
    };
    agreement_ops::accept_at(&f.host, input2, now).await.expect("accept 2");

    let get_input = QuoteRecordInput { quote_record_id: rec_id2.clone() };
    agreement_ops::get_at(&f.host, get_input, now + 350).await.expect("get after expiry");

    let hold2 = get_row::<HoldLedgerRow, _>(&f.host, LEDGER, &hold_id(&rec_id2, "c1"))
        .await
        .expect("query hold 2");
    assert!(hold2.is_none(), "expired quote must release counterparty hold");
    let seat2 = get_row::<SeatLedgerRow, _>(&f.host, LEDGER, &seat_id(&slot_id2, 1))
        .await
        .expect("query seat 2");
    assert!(seat2.is_none(), "expired quote must release seat");
}

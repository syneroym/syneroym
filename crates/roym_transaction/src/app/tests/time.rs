use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card,
    deal::{
        Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind, SettlementRole,
        transition::{CommitmentState, TRACK_WINDOW_SECS, track_end},
    },
    fulfilment::FULFILMENT_RECEIPT_VERSION,
    lang::LangTag,
    progress::{self as core_progress, AgreementProgressPayload, AgreementState},
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RECORD_FULFILMENT_RECEIPT, RecordDraft},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, Answers, DealRole, ReceiptHalf,
        SignedTerms, TimeWindow, derive_quote_id,
    },
};

use super::{TestFixture, fixture};
use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRef, AgreementRow, FULFILMENTS, FulfilmentsRow,
    decide::{SnapshotRow, snapshot_id},
    ensure_collections,
    fulfilment_ops::{self, FulfilmentSignInput, fulfilment_id},
    get_row, progress, progress_ops, put_row, sync,
};

fn make_service_commitment(id: &str, window: Option<TimeWindow>) -> Commitment {
    Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::GoodsOrService,
        kind: CommitmentKind::Fulfilment,
        extension: None,
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Piano tuning service",
        ),
        declared_value_minor: 5000,
        details: None,
        window,
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }
}

fn make_receipt_half(
    signer: &Identity,
    role: DealRole,
    quote_record_id: &str,
    terms: &SignedTerms,
    counterparty_did: &str,
    quote_author_did: &str,
    now: u64,
) -> ReceiptHalf {
    let signer_did = substrate::derive_did_key(&signer.public_key());
    let payload = AgreementReceiptPayload {
        quote_record_id: quote_record_id.to_string(),
        counterparty_did: counterparty_did.to_string(),
        quote_author_did: quote_author_did.to_string(),
        role,
        terms: terms.clone(),
        viewed_lang: LangTag::parse_canonical("en").expect("parse lang"),
        viewed_via_mt: false,
    };
    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: quote_record_id.to_string(),
        payload: serde_json::to_value(&payload).expect("payload to value"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, signer_did.clone(), None, now).expect("draft envelope");
    let sig = signer.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach signature");
    let envelope_json = env.to_json().expect("envelope to json");
    let record_id = env.record_id().expect("record id");
    ReceiptHalf { record_id, envelope: envelope_json, issuer: signer_did, issued_at_secs: now }
}

async fn store_open_snapshot(
    fix: &TestFixture,
    quote_record_id: &str,
    terms: &DealTerms,
    counterparty_did: &str,
    agreed_at: u64,
) -> AgreementProgressPayload {
    let snap1_payload = core_progress::open(
        quote_record_id.to_string(),
        "conv_1".to_string(),
        counterparty_did.to_string(),
        fix.master_did.clone(),
        terms,
        None,
    );
    let (snap1_env, snap1_rec_id) =
        progress::sign(&fix.host, &snap1_payload).await.expect("sign snapshot 1");
    let snap_key = snapshot_id(quote_record_id, 1);
    let snap_row = SnapshotRow {
        seq: 1,
        agreement: quote_record_id.to_string(),
        conversation: "conv_1".to_string(),
        snapshot: snap1_payload.clone(),
        envelope: snap1_env,
        record_id: snap1_rec_id,
        message_id: None,
        stored_at_secs: agreed_at,
    };
    put_row(&fix.host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await.expect("put snapshot 1");
    snap1_payload
}

async fn setup_agreed_deal(
    fix: &TestFixture,
    counterparty: &Identity,
    window: Option<TimeWindow>,
    agreed_at: u64,
) -> (String, DealTerms, AgreementProgressPayload) {
    ensure_collections(&fix.host).await.expect("ensure collections");

    let counterparty_did = substrate::derive_did_key(&counterparty.public_key());
    let quote_record_id = "rec_quote_time_test_1".to_string();
    let quote_id = derive_quote_id("conv_1", &fix.master_did, 1).expect("derive quote id");

    let terms = DealTerms {
        currency: "EUR".to_string(),
        commitments: vec![make_service_commitment("c1", window)],
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
        quote_expires_at_secs: agreed_at + 3600,
    };

    let plain = syneroym_roym_core::testing::sd::PlainQuote {
        quote_id: quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
        counterparty_did: counterparty_did.clone(),
        terms: terms.clone(),
    };
    let (payload, disclosures) = syneroym_roym_core::testing::sd::conceal_quote(plain, 1);
    crate::app::disclosures::store(
        &fix.host,
        &quote_record_id,
        syneroym_roym_core::record::RECORD_QUOTE,
        &disclosures,
        agreed_at,
    )
    .await
    .unwrap();

    let author_half = make_receipt_half(
        &fix.master,
        DealRole::QuoteAuthor,
        &quote_record_id,
        &payload.terms,
        &counterparty_did,
        &fix.master_did,
        agreed_at,
    );
    let counterparty_half = make_receipt_half(
        counterparty,
        DealRole::Counterparty,
        &quote_record_id,
        &payload.terms,
        &counterparty_did,
        &fix.master_did,
        agreed_at,
    );

    let snap1_payload =
        store_open_snapshot(fix, &quote_record_id, &terms, &counterparty_did, agreed_at).await;

    let agr_row = AgreementRow {
        quote_record_id: quote_record_id.clone(),
        quote_id,
        conversation: "conv_1".to_string(),
        quote_author_did: fix.master_did.clone(),
        counterparty_did,
        terms: terms.clone(),
        state: Some(AgreementState::Agreed),
        state_reason: None,
        cancel_reason: None,
        seq: Some(1),
        commitments: snap1_payload.commitments.clone(),
        agreed_at_secs: Some(agreed_at),
        counterparty: Some(counterparty_half),
        quote_author: Some(author_half),
        updated_at_secs: agreed_at,
    };
    put_row(&fix.host, AGREEMENTS, &quote_record_id, &agr_row).await.expect("put agreement row");

    (quote_record_id, terms, snap1_payload)
}

#[tokio::test]
async fn unsettled_commitment_ends_unconfirmed_after_its_track_end() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");

    let agreed_at = 1_700_000_000;

    // Case A: without giver's half -> claimed is Some(false)
    let (agreement_a, _terms_a, _) = setup_agreed_deal(&fix, &counterparty, None, agreed_at).await;
    let expected_track_end_a = track_end(None, agreed_at);
    assert_eq!(expected_track_end_a, agreed_at + TRACK_WINDOW_SECS);

    // Ticking past track end advances the unsettled commitment to EndedUnconfirmed
    let tick_after_a = expected_track_end_a + 1;
    progress_ops::tick_conversation_at(&fix.host, "conv_1", tick_after_a)
        .await
        .expect("tick conversation after track end");

    let output_a = progress_ops::progress_at(
        &fix.host,
        AgreementRef { agreement: agreement_a.clone() },
        tick_after_a,
    )
    .await
    .expect("progress output");

    assert_eq!(output_a.snapshots.len(), 2);
    let latest_a = &output_a.snapshots[0];
    assert_eq!(latest_a.seq, 2);
    assert_eq!(latest_a.snapshot.state, AgreementState::Closed);
    assert_eq!(latest_a.snapshot.commitments[0].state, CommitmentState::EndedUnconfirmed);
    assert_eq!(latest_a.snapshot.commitments[0].claimed, Some(false));
    assert_eq!(latest_a.snapshot.commitments[0].reason, None);

    // Case B: with giver's half -> claimed is Some(true)
    let fix_b = fixture();
    let window_b = TimeWindow { earliest_secs: agreed_at + 1000, latest_secs: agreed_at + 2000 };
    let (agreement_b, terms_b, _) =
        setup_agreed_deal(&fix_b, &counterparty, Some(window_b), agreed_at).await;
    let expected_track_end_b = track_end(terms_b.commitments[0].window.as_ref(), agreed_at);
    assert_eq!(expected_track_end_b, agreed_at + 2000 + TRACK_WINDOW_SECS);

    // Author (giver) signs fulfilment before track end
    let sign_time = agreed_at + 500;
    fulfilment_ops::sign_at(
        &fix_b.host,
        FulfilmentSignInput {
            agreement: agreement_b.clone(),
            commitment_id: "c1".to_string(),
            settlement_note: None,
        },
        sign_time,
    )
    .await
    .expect("author sign fulfilment");

    // After giver's half, commitment is Claimed in snapshot 2
    let prog_signed = progress_ops::progress_at(
        &fix_b.host,
        AgreementRef { agreement: agreement_b.clone() },
        sign_time,
    )
    .await
    .expect("progress after sign");
    assert_eq!(prog_signed.snapshots.len(), 2);
    assert_eq!(prog_signed.snapshots[0].snapshot.commitments[0].state, CommitmentState::Claimed);

    // Ticking past track end advances the claimed commitment to EndedUnconfirmed
    // with claimed: true
    let tick_after_b = expected_track_end_b + 1;
    progress_ops::tick_conversation_at(&fix_b.host, "conv_1", tick_after_b)
        .await
        .expect("tick conversation b");

    let output_b = progress_ops::progress_at(
        &fix_b.host,
        AgreementRef { agreement: agreement_b.clone() },
        tick_after_b,
    )
    .await
    .expect("progress output b");

    assert_eq!(output_b.snapshots.len(), 3);
    let latest_b = &output_b.snapshots[0];
    assert_eq!(latest_b.seq, 3);
    assert_eq!(latest_b.snapshot.state, AgreementState::Closed);
    assert_eq!(latest_b.snapshot.commitments[0].state, CommitmentState::EndedUnconfirmed);
    assert_eq!(latest_b.snapshot.commitments[0].claimed, Some(true));
    assert_eq!(latest_b.snapshot.commitments[0].reason, None);
}

#[tokio::test]
async fn tick_before_the_end_changes_nothing() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");

    let agreed_at = 1_700_000_000;
    let (agreement, _, _) = setup_agreed_deal(&fix, &counterparty, None, agreed_at).await;

    // Tick well before the track window ends (30 days)
    let tick_early = agreed_at + 100;
    progress_ops::tick_conversation_at(&fix.host, "conv_1", tick_early).await.expect("tick early");

    let agr: AgreementRow = get_row(&fix.host, AGREEMENTS, &agreement)
        .await
        .expect("get agreement row")
        .expect("agreement exists");

    assert_eq!(agr.state, Some(AgreementState::Agreed));
    assert_eq!(agr.seq, Some(1));
    assert_eq!(agr.commitments[0].state, CommitmentState::Promised);

    let prog = progress_ops::progress_at(
        &fix.host,
        AgreementRef { agreement: agreement.clone() },
        tick_early,
    )
    .await
    .expect("progress output");

    assert_eq!(prog.snapshots.len(), 1);
    assert_eq!(prog.snapshots[0].seq, 1);
    assert_eq!(prog.snapshots[0].snapshot.state, AgreementState::Agreed);
}

#[tokio::test]
async fn a_late_half_after_the_end_is_kept_and_changes_nothing() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let counterparty_did = substrate::derive_did_key(&counterparty.public_key());

    let agreed_at = 1_700_000_000;
    let (agreement, _, _) = setup_agreed_deal(&fix, &counterparty, None, agreed_at).await;
    let expected_track_end = track_end(None, agreed_at);

    // Tick past track end so the agreement ends unconfirmed and closes
    let tick_after = expected_track_end + 1;
    progress_ops::tick_conversation_at(&fix.host, "conv_1", tick_after)
        .await
        .expect("tick past end");

    let agr_closed: AgreementRow = get_row(&fix.host, AGREEMENTS, &agreement)
        .await
        .expect("get closed agreement")
        .expect("agreement exists");
    assert_eq!(agr_closed.state, Some(AgreementState::Closed));
    assert_eq!(agr_closed.seq, Some(2));
    assert_eq!(agr_closed.commitments[0].state, CommitmentState::EndedUnconfirmed);

    // Counterparty (receiver) signs a fulfilment receipt late (after the agreement
    // is closed)
    let late_now = tick_after + 500;
    let plain_fulfilment = syneroym_roym_core::testing::sd::PlainFulfilment {
        agreement: agreement.clone(),
        conversation: "conv_1".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: fix.master_did.clone(),
        role: SettlementRole::Receiver,
        settlement_note: None,
    };
    let (half_payload, half_disclosures) =
        syneroym_roym_core::testing::sd::conceal_fulfilment(plain_fulfilment, 1);

    let draft = RecordDraft {
        version: FULFILMENT_RECEIPT_VERSION,
        record_type: RECORD_FULFILMENT_RECEIPT.to_string(),
        subject: agreement.clone(),
        payload: serde_json::to_value(&half_payload).expect("payload to value"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, counterparty_did.clone(), None, late_now)
        .expect("draft envelope");
    let sig = counterparty.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach signature");
    let env_json = env.to_json().expect("env to json");
    let late_record_id = env.record_id().expect("record id");

    let card_body = card::card_body(
        "fulfilment-receipt",
        FULFILMENT_RECEIPT_VERSION,
        &env_json,
        &half_disclosures,
    )
    .expect("card body");

    let message = json!({
        "id": "msg_late_receiver_half",
        "conversation": "conv_1",
        "sender": counterparty_did,
        "direction": "incoming",
        "body": card_body,
        "created_at_ms": late_now * 1000,
    });

    let file_res =
        sync::file_incoming_card(&fix.host, &message, "conv_1", late_now, &fix.master_did)
            .await
            .expect("file incoming card");

    // The late card is filed and recorded as evidence
    assert!(file_res.filed);
    assert!(!file_res.refused);

    let f_id = fulfilment_id(&agreement, "c1");
    let fulfilments: FulfilmentsRow = get_row(&fix.host, FULFILMENTS, &f_id)
        .await
        .expect("get fulfilments")
        .expect("fulfilments row exists");
    assert_eq!(fulfilments.counterparty.as_ref().map(|h| &h.record_id), Some(&late_record_id));

    // But the agreement progress is completely unchanged: remains Closed, seq 2
    let agr_still_closed: AgreementRow = get_row(&fix.host, AGREEMENTS, &agreement)
        .await
        .expect("get agreement row")
        .expect("agreement exists");
    assert_eq!(agr_still_closed.state, Some(AgreementState::Closed));
    assert_eq!(agr_still_closed.seq, Some(2));
    assert_eq!(agr_still_closed.commitments[0].state, CommitmentState::EndedUnconfirmed);
    assert_eq!(agr_still_closed.commitments[0].claimed, Some(false));

    let snap3_key = snapshot_id(&agreement, 3);
    let snap3: Option<SnapshotRow> =
        get_row(&fix.host, AGREEMENT_PROGRESS, &snap3_key).await.expect("get snap 3");
    assert!(snap3.is_none());

    let prog = progress_ops::progress_at(
        &fix.host,
        AgreementRef { agreement: agreement.clone() },
        late_now + 10,
    )
    .await
    .expect("progress output");
    assert_eq!(prog.snapshots.len(), 2);
    assert_eq!(prog.snapshots[0].seq, 2);
    assert_eq!(prog.snapshots[0].snapshot.commitments[0].state, CommitmentState::EndedUnconfirmed);
    assert_eq!(prog.snapshots[0].snapshot.commitments[0].claimed, Some(false));
}

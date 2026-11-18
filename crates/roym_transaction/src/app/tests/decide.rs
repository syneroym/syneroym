use serde_json::json;
use syneroym_app_host::types::proxy::ProxyError;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    deal::{
        Commitment, CommitmentKind, DealTerms, Details, ExtensionRef, LocalText, ResourceKind,
        SettlementRole, SignedTerms, transition::CommitmentState,
    },
    lang::LangTag,
    listing::derive_slot_id,
    progress::{AgreementEvent, AgreementProgressPayload, AgreementState, CommitmentProgress},
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RECORD_QUOTE, RecordDraft},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, Answers, DealRole, QUOTE_VERSION,
        ReceiptHalf, TimeWindow, derive_quote_id,
    },
};

use super::{TestFixture, fixture};
use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, LEDGER,
    decide::{
        self, CountersignRow, DecisionRow, DecisionState, SnapshotRow, countersign_id, decision_id,
        snapshot_id,
    },
    disclosures::HeldQuote,
    get_row,
    hold::{HoldLedgerRow, Reservations, SeatLedger, SeatLedgerRow, hold_id, seat_id},
    progress_ops,
};

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
            "Author Service",
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

fn author_slot_commitment(
    id: &str,
    listing_id: &str,
    start_secs: u64,
    end_secs: u64,
) -> (Commitment, String) {
    let slot_id = derive_slot_id(listing_id, start_secs, end_secs).expect("derive slot id");
    let commitment = Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::ServiceInSlot,
        kind: CommitmentKind::Fulfilment,
        extension: Some(ExtensionRef { id: "scheduling".to_string(), version: 1 }),
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Author Slot Service",
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

fn build_and_sign_author_quote(
    author: &Identity,
    counterparty_did: &str,
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
    let plain = syneroym_roym_core::testing::sd::PlainQuote {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
        counterparty_did: counterparty_did.to_string(),
        terms: terms.clone(),
    };
    let (payload, disclosures) = syneroym_roym_core::testing::sd::conceal_quote(plain, 1);

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
    let verdict = syneroym_roym_core::transaction::verify_quote(&envelope_json, now);
    let held = HeldQuote { verdict, signed: payload, terms, disclosures };
    (held, record_id, envelope_json)
}

fn make_counterparty_receipt(
    cp: &Identity,
    author_did: &str,
    quote_record_id: &str,
    terms: &SignedTerms,
    now: u64,
) -> (AgreementReceiptPayload, String, String) {
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let payload = AgreementReceiptPayload {
        quote_record_id: quote_record_id.to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: author_did.to_string(),
        role: DealRole::Counterparty,
        terms: terms.clone(),
        viewed_lang: LangTag::parse_canonical("en").expect("lang"),
        viewed_via_mt: false,
    };

    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: quote_record_id.to_string(),
        payload: serde_json::to_value(&payload).expect("serialize receipt"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, cp_did, None, now).expect("draft receipt env");
    let sig = cp.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach receipt sig");
    let record_id = env.record_id().expect("record id");
    let envelope_json = env.to_json().expect("receipt json");
    (payload, record_id, envelope_json)
}

fn unformed_agreement_row(
    quote_rec_id: &str,
    quote: &HeldQuote,
    author_did: &str,
    cp_did: &str,
    cp_rec_id: &str,
    cp_envelope: &str,
    now: u64,
) -> AgreementRow {
    AgreementRow {
        quote_record_id: quote_rec_id.to_string(),
        quote_id: quote.signed.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_did.to_string(),
        quote_author_did: author_did.to_string(),
        terms: quote.terms.clone(),
        agreed_at_secs: None,
        counterparty: Some(ReceiptHalf {
            envelope: cp_envelope.to_string(),
            record_id: cp_rec_id.to_string(),
            issuer: cp_did.to_string(),
            issued_at_secs: now,
        }),
        quote_author: None,
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: vec![],
        updated_at_secs: now,
    }
}

fn setup_service_agreement(f: &TestFixture) -> (HeldQuote, String, AgreementRow, u64) {
    let cp = Identity::generate().expect("generate cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let now = f.host.now();

    let (quote, quote_rec_id, _) = build_and_sign_author_quote(
        &f.master,
        &cp_did,
        vec![author_service_commitment("c1")],
        now + 3600,
        now,
    );
    let (_cp_receipt, cp_rec_id, cp_envelope) =
        make_counterparty_receipt(&cp, &f.master_did, &quote_rec_id, &quote.signed.terms, now);

    let row = unformed_agreement_row(
        &quote_rec_id,
        &quote,
        &f.master_did,
        &cp_did,
        &cp_rec_id,
        &cp_envelope,
        now,
    );
    (quote, quote_rec_id, row, now)
}

#[tokio::test]
async fn writer_countersigns_in_sync_and_sends_snapshot_one() {
    let f = fixture();
    let (quote, quote_rec_id, mut row, now) = setup_service_agreement(&f);

    let res = decide::run(&f.host, &mut row, &quote, now).await;
    assert_eq!(res, Ok(true));

    // Verify countersign row
    let cs_key = countersign_id(&quote_rec_id);
    let cs: Option<CountersignRow> = get_row(&f.host, LEDGER, &cs_key).await.expect("get cs");
    assert!(cs.is_some(), "countersign row must exist");
    let cs = cs.unwrap();
    assert_eq!(cs.agreement, quote_rec_id);

    // Verify snapshot 1
    let snap_key = snapshot_id(&quote_rec_id, 1);
    let snap: Option<SnapshotRow> =
        get_row(&f.host, AGREEMENT_PROGRESS, &snap_key).await.expect("get snap");
    assert!(snap.is_some(), "snapshot 1 must exist");
    let snap = snap.unwrap();
    assert_eq!(snap.snapshot.state, AgreementState::Agreed);
    assert_eq!(snap.seq, 1);

    // Verify decision row
    let dec_key = decision_id(&quote.signed.quote_id);
    let dec: Option<DecisionRow> = get_row(&f.host, LEDGER, &dec_key).await.expect("get dec");
    assert!(dec.is_some());
    assert_eq!(dec.unwrap().state, DecisionState::Agreed);

    // Verify agreed_at_secs set from countersign half
    assert_eq!(row.agreed_at_secs, Some(cs.half.issued_at_secs));
    assert_eq!(row.state, Some(AgreementState::Agreed));
}

#[tokio::test]
async fn losing_the_last_seat_ends_not_formed() {
    let f = fixture();
    let cp = Identity::generate().expect("generate cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let now = f.host.now();

    let (commitment, slot_id) = author_slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let (quote, quote_rec_id, _) =
        build_and_sign_author_quote(&f.master, &cp_did, vec![commitment], now + 3600, now);
    let (_cp_receipt, cp_rec_id, cp_envelope) =
        make_counterparty_receipt(&cp, &f.master_did, &quote_rec_id, &quote.signed.terms, now);

    // Take the seat beforehand so capacity 1 is exhausted
    SeatLedger
        .reserve(&f.host, "other_agreement", "c1", &slot_id, now)
        .await
        .expect("reserve initial seat");
    SeatLedger.confirm(&f.host, "other_agreement", "c1").await.expect("confirm initial seat");

    let mut row = unformed_agreement_row(
        &quote_rec_id,
        &quote,
        &f.master_did,
        &cp_did,
        &cp_rec_id,
        &cp_envelope,
        now,
    );

    let res = decide::run(&f.host, &mut row, &quote, now).await;
    assert_eq!(res, Ok(false));

    // Decision row is not-formed with reason slot-taken
    let dec_key = decision_id(&quote.signed.quote_id);
    let dec: Option<DecisionRow> = get_row(&f.host, LEDGER, &dec_key).await.expect("get dec");
    assert!(dec.is_some());
    let dec = dec.unwrap();
    assert_eq!(dec.state, DecisionState::NotFormed);
    assert_eq!(dec.reason.as_deref(), Some("slot-taken"));

    // Snapshot 1 is not-formed
    let snap_key = snapshot_id(&quote_rec_id, 1);
    let snap: Option<SnapshotRow> = get_row(&f.host, AGREEMENT_PROGRESS, &snap_key).await.unwrap();
    assert!(snap.is_some());
    let snap = snap.unwrap();
    assert_eq!(snap.snapshot.state, AgreementState::NotFormed);
    assert_eq!(snap.snapshot.state_reason.as_deref(), Some("slot-taken"));
    assert_eq!(row.state, Some(AgreementState::NotFormed));
}

#[tokio::test]
async fn repeat_branch_reuses_stored_snapshot_one() {
    let f = fixture();
    let (quote, quote_rec_id, mut row, now) = setup_service_agreement(&f);

    // Pre-populate decision row as deciding
    let dec_key = decision_id(&quote.signed.quote_id);
    let dec = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Deciding,
        agreement: quote_rec_id.clone(),
        quote_id: quote.signed.quote_id.clone(),
        reason: None,
        created_at_secs: now,
    };
    crate::app::put_row(&f.host, LEDGER, &dec_key, &dec).await.unwrap();

    // First run succeeds
    let res = decide::run(&f.host, &mut row, &quote, now).await;
    assert_eq!(res, Ok(true));
    assert_eq!(row.state, Some(AgreementState::Agreed));

    // A second run on the repeat path reuses the stored snapshot 1 even past expiry
    let res_repeat = decide::run(&f.host, &mut row, &quote, now + 7200).await;
    assert_eq!(res_repeat, Ok(true));
    assert_eq!(row.state, Some(AgreementState::Agreed));
}

#[tokio::test]
async fn send_error_stops_decision_row_and_retries_on_next_sync() {
    let f = fixture();
    let (quote, _quote_rec_id, mut row, now) = setup_service_agreement(&f);

    // 1. Transport error triggers retry (returns Err)
    f.host.on_call("conversation", "send", |_| {
        Err(ProxyError::Transport("connection reset by peer".to_string()))
    });

    let res = decide::run(&f.host, &mut row, &quote, now).await;
    assert!(res.is_err(), "transport error must bubble up as Err for retry");

    // 2. Lasting error keeps decision row deciding and returns Ok(false)
    f.host.on_call("conversation", "send", |_| {
        Ok(serde_json::json!({
            "error": {
                "code": 403,
                "message": "rate limit exceeded"
            }
        })
        .to_string())
    });

    let res2 = decide::run(&f.host, &mut row, &quote, now).await;
    assert_eq!(res2, Ok(false));

    let dec_key = decision_id(&quote.signed.quote_id);
    let dec: Option<DecisionRow> = get_row(&f.host, LEDGER, &dec_key).await.expect("get dec");
    assert!(dec.is_some());
    assert_eq!(
        dec.unwrap().state,
        DecisionState::Deciding,
        "lasting refusal must leave decision row deciding"
    );
}

#[tokio::test]
async fn release_both_seat_and_hold_in_one_batch() {
    let f = fixture();
    let now = f.host.now();

    SeatLedger.reserve(&f.host, "agr_batch", "c1", "slot_batch", now).await.expect("reserve");

    let h_key = hold_id("agr_batch", "c1");
    let s_key = seat_id("slot_batch", 1);

    let h: Option<HoldLedgerRow> = get_row(&f.host, LEDGER, &h_key).await.expect("get hold");
    let s: Option<SeatLedgerRow> = get_row(&f.host, LEDGER, &s_key).await.expect("get seat");
    assert!(h.is_some());
    assert!(s.is_some());

    SeatLedger.release(&f.host, "agr_batch", "c1").await.expect("release");

    let h_after: Option<HoldLedgerRow> = get_row(&f.host, LEDGER, &h_key).await.expect("get hold");
    let s_after: Option<SeatLedgerRow> = get_row(&f.host, LEDGER, &s_key).await.expect("get seat");
    assert!(h_after.is_none(), "hold row must be released");
    assert!(s_after.is_none(), "seat row must be released");
}

#[tokio::test]
async fn repeat_past_expiry_without_countersign_leaves_row_deciding() {
    let f = fixture();
    let cp = Identity::generate().expect("generate cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let now = f.host.now();

    let (quote, quote_rec_id, _) = build_and_sign_author_quote(
        &f.master,
        &cp_did,
        vec![author_service_commitment("c1")],
        now + 100,
        now,
    );
    let (_cp_receipt, cp_rec_id, cp_envelope) =
        make_counterparty_receipt(&cp, &f.master_did, &quote_rec_id, &quote.signed.terms, now);

    let mut row = unformed_agreement_row(
        &quote_rec_id,
        &quote,
        &f.master_did,
        &cp_did,
        &cp_rec_id,
        &cp_envelope,
        now,
    );

    // Pre-populate decision row as deciding
    let dec_key = decision_id(&quote.signed.quote_id);
    let dec = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Deciding,
        agreement: quote_rec_id.clone(),
        quote_id: quote.signed.quote_id.clone(),
        reason: None,
        created_at_secs: now,
    };
    crate::app::put_row(&f.host, LEDGER, &dec_key, &dec).await.unwrap();

    // Now advance clock past quote expiry
    let expired_now = now + 500;
    let res = decide::run(&f.host, &mut row, &quote, expired_now).await;
    assert_eq!(res, Ok(false));

    // Decision row remains deciding
    let dec_after: Option<DecisionRow> = get_row(&f.host, LEDGER, &dec_key).await.expect("get dec");
    assert!(dec_after.is_some());
    assert_eq!(dec_after.unwrap().state, DecisionState::Deciding);
}

async fn seed_snapshot_history(
    host: &syneroym_roym_core::testing::TestHost,
    quote_rec_id: &str,
    cp_did: &str,
    master_did: &str,
    now: u64,
) {
    let snap1 = AgreementProgressPayload {
        agreement: quote_rec_id.to_string(),
        conversation: "conv_1".to_string(),
        seq: 1,
        counterparty_did: cp_did.to_string(),
        quote_author_did: master_did.to_string(),
        state: AgreementState::Agreed,
        state_reason: None,
        cancel_reason: None,
        commitments: vec![CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Promised,
            reason: None,
            claimed: Some(false),
            halves: vec![],
        }],
    };
    let snap1_row = SnapshotRow {
        seq: 1,
        agreement: quote_rec_id.to_string(),
        conversation: "conv_1".to_string(),
        snapshot: snap1,
        envelope: "{}".to_string(),
        record_id: "rec_snap_1".to_string(),
        message_id: Some("msg_1".to_string()),
        stored_at_secs: now,
    };
    crate::app::put_row(host, AGREEMENT_PROGRESS, &snapshot_id(quote_rec_id, 1), &snap1_row)
        .await
        .unwrap();

    let snap2 = AgreementProgressPayload {
        agreement: quote_rec_id.to_string(),
        conversation: "conv_1".to_string(),
        seq: 2,
        counterparty_did: cp_did.to_string(),
        quote_author_did: master_did.to_string(),
        state: AgreementState::Agreed,
        state_reason: None,
        cancel_reason: None,
        commitments: vec![CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Claimed,
            reason: None,
            claimed: Some(true),
            halves: vec![],
        }],
    };
    let snap2_row = SnapshotRow {
        seq: 2,
        agreement: quote_rec_id.to_string(),
        conversation: "conv_1".to_string(),
        snapshot: snap2,
        envelope: "{}".to_string(),
        record_id: "rec_snap_2".to_string(),
        message_id: Some("msg_2".to_string()),
        stored_at_secs: now,
    };
    crate::app::put_row(host, AGREEMENT_PROGRESS, &snapshot_id(quote_rec_id, 2), &snap2_row)
        .await
        .unwrap();
}

#[tokio::test]
async fn transition_retries_and_advances_when_snapshot_key_already_exists() {
    let f = fixture();
    let cp = Identity::generate().expect("generate cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let now = f.host.now();

    let (quote, quote_rec_id, _) = build_and_sign_author_quote(
        &f.master,
        &cp_did,
        vec![author_service_commitment("c1")],
        now + 3600,
        now,
    );

    let row = AgreementRow {
        quote_record_id: quote_rec_id.clone(),
        quote_id: quote.signed.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: f.master_did.clone(),
        terms: quote.terms.clone(),
        agreed_at_secs: Some(now),
        counterparty: None,
        quote_author: None,
        state: Some(AgreementState::Agreed),
        state_reason: None,
        cancel_reason: None,
        seq: Some(1),
        commitments: vec![CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Promised,
            reason: None,
            claimed: Some(false),
            halves: vec![],
        }],
        updated_at_secs: now,
    };
    crate::app::put_row(&f.host, AGREEMENTS, &quote_rec_id, &row).await.unwrap();

    seed_snapshot_history(&f.host, &quote_rec_id, &cp_did, &f.master_did, now).await;

    // Now call transition_at with GiverHalf. Attempt 0 tries seq 2, finds it exists
    // via create, retries by rolling forward from stored seq 2, and
    // successfully creates seq 3!
    let updated = progress_ops::transition_at(
        &f.host,
        &quote_rec_id,
        AgreementEvent::Half {
            commitment_id: "c1".to_string(),
            role: SettlementRole::Giver,
            record_id: "rec_giver_half".to_string(),
        },
        now,
    )
    .await
    .expect("transition_at should succeed by rolling forward over existing snapshot");

    assert_eq!(updated.seq, Some(3));
    let snap3: Option<SnapshotRow> =
        get_row(&f.host, AGREEMENT_PROGRESS, &snapshot_id(&quote_rec_id, 3)).await.unwrap();
    assert!(snap3.is_some(), "snapshot 3 must be stored");
}

#[tokio::test]
async fn repeat_after_the_agreement_moved_keeps_its_state() {
    let f = fixture();
    let (quote, quote_rec_id, mut row, now) = setup_service_agreement(&f);
    crate::app::put_row(&f.host, AGREEMENTS, &quote_rec_id, &row).await.unwrap();

    let agreed = decide::run(&f.host, &mut row, &quote, now).await.expect("initial decide");
    assert!(agreed);
    assert_eq!(row.state, Some(AgreementState::Agreed));
    assert_eq!(row.seq, Some(1));

    let cancelled = progress_ops::transition_at(
        &f.host,
        &quote_rec_id,
        AgreementEvent::Cancel { reason: syneroym_roym_core::progress::CancelReason::Other },
        now,
    )
    .await
    .expect("cancel transition");
    assert_eq!(cancelled.state, Some(AgreementState::Cancelled));
    assert_eq!(cancelled.seq, Some(2));

    let res = decide::run(&f.host, &mut row, &quote, now).await.expect("repeat decide");
    assert!(res);

    let stored: Option<AgreementRow> = get_row(&f.host, AGREEMENTS, &quote_rec_id).await.unwrap();
    let agr = stored.expect("agreement exists");
    assert_eq!(agr.state, Some(AgreementState::Cancelled));
    assert_eq!(agr.seq, Some(2));
}

#[tokio::test]
async fn crash_before_the_decision_row_is_final_is_finished_on_the_next_sync() {
    let f = fixture();
    let (quote, quote_rec_id, mut row, now) = setup_service_agreement(&f);
    crate::app::put_row(&f.host, AGREEMENTS, &quote_rec_id, &row).await.unwrap();

    // Inject crash when writing to agreements collection
    f.host.crash_on_write("agreements", &quote_rec_id);

    let err = decide::run(&f.host, &mut row, &quote, now)
        .await
        .expect_err("initial decide must fail due to injected crash");
    assert!(err.contains("injected crash on write to agreements"));

    f.host.restart();

    let res =
        decide::run(&f.host, &mut row, &quote, now).await.expect("next sync finishes decision");
    assert!(res);

    let stored: Option<AgreementRow> = get_row(&f.host, AGREEMENTS, &quote_rec_id).await.unwrap();
    let agr = stored.expect("agreement must be stored");
    assert_eq!(agr.state, Some(AgreementState::Agreed));
    assert_eq!(agr.seq, Some(1));
    assert!(agr.quote_author.is_some());

    let dec: Option<DecisionRow> =
        get_row(&f.host, LEDGER, &decision_id(&quote.signed.quote_id)).await.unwrap();
    assert_eq!(dec.expect("decision row exists").state, DecisionState::Agreed);
}

#[tokio::test]
async fn cancel_before_the_decision_row_is_final_is_kept() {
    let f = fixture();
    let (quote, quote_rec_id, mut row, now) = setup_service_agreement(&f);
    crate::app::put_row(&f.host, AGREEMENTS, &quote_rec_id, &row).await.unwrap();

    // Inject crash when writing to agreements collection on initial decide
    f.host.crash_on_write("agreements", &quote_rec_id);

    let err = decide::run(&f.host, &mut row, &quote, now)
        .await
        .expect_err("initial decide must fail due to injected crash");
    assert!(err.contains("injected crash on write to agreements"));

    f.host.restart();

    let cancelled = progress_ops::transition_at(
        &f.host,
        &quote_rec_id,
        AgreementEvent::Cancel { reason: syneroym_roym_core::progress::CancelReason::Other },
        now,
    )
    .await
    .expect("cancel transition");
    assert_eq!(cancelled.state, Some(AgreementState::Cancelled));
    assert_eq!(cancelled.seq, Some(2));

    let res = decide::run(&f.host, &mut row, &quote, now).await.expect("repeat decide");
    assert!(res);

    let stored: Option<AgreementRow> = get_row(&f.host, AGREEMENTS, &quote_rec_id).await.unwrap();
    let agr = stored.expect("agreement exists");
    assert_eq!(agr.state, Some(AgreementState::Cancelled));
    assert_eq!(agr.seq, Some(2));
    assert!(agr.quote_author.is_some());
    assert!(agr.agreed_at_secs.is_some());

    let dec_key = decision_id(&quote.signed.quote_id);
    let dec: Option<DecisionRow> = get_row(&f.host, LEDGER, &dec_key).await.unwrap();
    assert_eq!(dec.expect("decision exists").state, DecisionState::Agreed);
}

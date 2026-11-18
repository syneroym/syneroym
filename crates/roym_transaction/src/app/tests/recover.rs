//! Service tests for writer decision recovery and crash points.

use serde_json::{Value, json};
use syneroym_app_host::AppHost;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    deal::{Commitment, CommitmentKind, DealTerms, Details, ExtensionRef, LocalText, ResourceKind},
    lang::LangTag,
    listing::derive_slot_id,
    progress::AgreementState,
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RECORD_QUOTE, RecordDraft},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, Answers, DealRole, QUOTE_VERSION,
        ReceiptHalf, SignedTerms, TimeWindow, derive_quote_id,
    },
};

use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, CARDS, CardRow, Direction, LEDGER, QUOTE_HISTORY,
    QUOTES, RecordPointerRow, SYNC_STATE, SyncStateRow,
    decide::{
        self, CountersignRow, DecisionRow, DecisionState, RefusedRow, SnapshotRow, countersign_id,
        decision_id, refused_id, snapshot_id,
    },
    disclosures::{self, HeldQuote},
    get_row,
    hold::{HoldLedgerRow, Reservations, ReserveOutcome, SeatLedger, hold_id, seat_id},
    put_bytes, put_card, put_row,
    sync::{self, SyncInput},
    tests::{TestFixture, fixture},
};

fn slot_commitment(
    id: &str,
    listing_id: &str,
    start_secs: u64,
    end_secs: u64,
) -> (Commitment, String) {
    let slot_id = derive_slot_id(listing_id, start_secs, end_secs).expect("slot id");
    let commitment = Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::ServiceInSlot,
        kind: CommitmentKind::Fulfilment,
        extension: Some(ExtensionRef { id: "scheduling".to_string(), version: 1 }),
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Slot service",
        ),
        declared_value_minor: 5000,
        details: Some(Details {
            extension: "scheduling".to_string(),
            schema: "scheduling/slot@1".to_string(),
            data: serde_json::json!({
                "listing_id": listing_id,
                "slot_id": slot_id,
            }),
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

fn build_quote(
    author: &Identity,
    counterparty_did: &str,
    commitments: Vec<Commitment>,
    expires_at_secs: u64,
    now: u64,
) -> (HeldQuote, String, String) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let quote_id = derive_quote_id("conv_1", &author_did, 1).expect("quote id");
    let terms = DealTerms {
        currency: "EUR".to_string(),
        commitments,
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Service"),
        cancellation_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "24h"),
        refund_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Full"),
        dispute_path: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Arb"),
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
        payload: serde_json::to_value(&payload).expect("serialize quote"),
        expires_at_secs: Some(expires_at_secs),
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, author_did, None, now).expect("env");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let q_rec_id = env.record_id().expect("rec id");
    let env_json = env.to_json().expect("env json");
    let verdict = syneroym_roym_core::transaction::verify_quote(&env_json, now);
    let held = HeldQuote { verdict, signed: payload, terms, disclosures };
    (held, q_rec_id, env_json)
}

fn make_receipt(
    cp: &Identity,
    author_did: &str,
    quote_record_id: &str,
    terms: &SignedTerms,
    now: u64,
) -> (ReceiptHalf, String) {
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
    let (mut env, bytes) =
        Envelope::unsigned(draft, cp_did.clone(), None, now).expect("receipt env");
    let sig = cp.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("receipt sig");
    let record_id = env.record_id().expect("record id");
    let envelope_json = env.to_json().expect("receipt json");

    (
        ReceiptHalf {
            envelope: envelope_json,
            record_id: record_id.clone(),
            issuer: cp_did,
            issued_at_secs: now,
        },
        record_id,
    )
}

async fn setup_test_deal_with_env(
    f: &TestFixture,
    commitments: Vec<Commitment>,
    expiry_secs: u64,
) -> (HeldQuote, String, String, AgreementRow, u64) {
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let now = f.host.now();

    let (quote, q_rec_id, q_env_json) =
        build_quote(&f.master, &cp_did, commitments, expiry_secs, now);
    let (cp_half, _) = make_receipt(&cp, &f.master_did, &q_rec_id, &quote.signed.terms, now);

    put_bytes(&f.host, QUOTE_HISTORY, &q_rec_id, q_env_json.as_bytes()).await.unwrap();
    disclosures::store(&f.host, &q_rec_id, RECORD_QUOTE, &quote.disclosures, now).await.unwrap();

    let row = AgreementRow {
        quote_record_id: q_rec_id.clone(),
        quote_id: quote.signed.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_did,
        quote_author_did: f.master_did.clone(),
        terms: quote.terms.clone(),
        agreed_at_secs: None,
        counterparty: Some(cp_half),
        quote_author: None,
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: vec![],
        updated_at_secs: now,
    };
    (quote, q_rec_id, q_env_json, row, now)
}

async fn setup_test_deal(
    f: &TestFixture,
    commitments: Vec<Commitment>,
    expiry_secs: u64,
) -> (HeldQuote, String, AgreementRow, u64) {
    let (quote, q_rec_id, _, row, now) =
        setup_test_deal_with_env(f, commitments, expiry_secs).await;
    (quote, q_rec_id, row, now)
}

async fn file_card_and_move_cursor(f: &TestFixture, conv: &str, msg_id: &str, now: u64) {
    let card_row = CardRow {
        message_id: msg_id.to_string(),
        conversation: conv.to_string(),
        direction: Direction::Incoming,
        sender_timestamp_ms: (now * 1000) as i64,
        card_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        version: 1,
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
        version_count: None,
        deal_state: None,
        agreement_payee: None,
        agreement_payment_methods: vec![],
    };
    put_card(&f.host, &card_row).await.expect("put card");
    put_row(&f.host, SYNC_STATE, conv, &SyncStateRow { last_seq: 1 })
        .await
        .expect("put sync state");
}

struct RefusingConfirmReservations;

impl Reservations for RefusingConfirmReservations {
    async fn reserve<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment_id: &str,
        slot_id: &str,
        now: u64,
    ) -> Result<ReserveOutcome, String> {
        SeatLedger.reserve(host, agreement, commitment_id, slot_id, now).await
    }

    async fn confirm<H: AppHost>(
        &self,
        _host: &H,
        _agreement: &str,
        _commitment_id: &str,
    ) -> Result<(), String> {
        Err("slot-expired".to_string())
    }

    async fn release<H: AppHost>(
        &self,
        host: &H,
        agreement: &str,
        commitment_id: &str,
    ) -> Result<(), String> {
        SeatLedger.release(host, agreement, commitment_id).await
    }
}

#[tokio::test]
async fn crash_after_the_fence_is_finished_on_the_next_sync() {
    let f = fixture();
    let now = f.host.now();
    let (comm, _) = slot_commitment("c1", "listing_1", now + 1000, now + 4600);
    let (quote, q_rec_id, q_env_json, row, now) =
        setup_test_deal_with_env(&f, vec![comm], now + 3600).await;

    let ptr = RecordPointerRow {
        envelope: q_env_json,
        record_id: q_rec_id.clone(),
        id: quote.signed.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: f.master_did.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(quote.signed.counterparty_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f.host, QUOTES, &quote.signed.quote_id, &ptr).await.unwrap();

    let cp_half = row.counterparty.as_ref().unwrap();
    let card_body = card::card_body(
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &cp_half.envelope,
        &[],
    )
    .expect("card body");

    let card_body_clone = card_body.clone();
    let cp_did = quote.signed.counterparty_did.clone();
    f.host.on_call("conversation", "conversation.changes", move |params| {
        let p: Value = serde_json::from_str(&params).unwrap_or(Value::Null);
        let after_seq = p.get("after_seq").and_then(Value::as_u64).unwrap_or(0);
        if after_seq == 0 {
            Ok(json!({
                "result": {
                    "messages": [
                        {
                            "id": "msg_accept_1",
                            "conversation": "conv_1",
                            "author": cp_did,
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
                            "body": card_body_clone,
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

    // Failing write after fence: on the first reserve write to ledger (seat:)
    f.host.fail_write("ledger", "seat:");

    // First sync delivers the acceptance receipt half.
    // decide::run fails on seat: write (after writing the fence), but sync catches
    // it because dec.state == Deciding, files the card and advances last_seq to
    // 1.
    sync::sync_at(&f.host, SyncInput { conversation: "conv_1".to_string(), full: false }, now)
        .await
        .expect("sync files card despite decide write failure");

    assert!(f.host.was_failed_write_hit(), "failing write must have fired during sync");

    // Check card is stored and sync cursor moved despite decide::run write failure
    let card: Option<CardRow> = get_row(&f.host, CARDS, "msg_accept_1").await.unwrap();
    assert!(card.is_some(), "card must be stored despite decide::run write failure");
    let state: Option<SyncStateRow> = get_row(&f.host, SYNC_STATE, "conv_1").await.unwrap();
    assert_eq!(state.unwrap().last_seq, 1, "sync cursor must be moved to 1");

    // The recovery sweep at the end of sync finishes the deciding row to Agreed
    let dec_key = decision_id(&quote.signed.quote_id);
    let dec_after: DecisionRow = get_row(&f.host, LEDGER, &dec_key).await.unwrap().unwrap();
    assert_eq!(dec_after.state, DecisionState::Agreed);

    let agr_after: AgreementRow = get_row(&f.host, AGREEMENTS, &q_rec_id).await.unwrap().unwrap();
    assert_eq!(agr_after.state, Some(AgreementState::Agreed));
    assert_eq!(agr_after.seq, Some(1));
}

#[tokio::test]
async fn crash_after_each_reserve_is_finished_on_the_next_sync() {
    let f = fixture();
    let now = f.host.now();
    let (comm1, _) = slot_commitment("c1", "listing_1", now + 1000, now + 4600);
    let (comm2, _) = slot_commitment("c2", "listing_2", now + 5000, now + 8600);
    let (quote, q_rec_id, mut row, now) = setup_test_deal(&f, vec![comm1, comm2], now + 3600).await;
    put_row(&f.host, AGREEMENTS, &q_rec_id, &row).await.unwrap();
    file_card_and_move_cursor(&f, "conv_1", "msg_accept_2", now).await;

    // Crash on writing second hold
    f.host.crash_on_write("ledger", format!("hold:{}:c2", q_rec_id));

    let err = decide::run(&f.host, &mut row, &quote, now)
        .await
        .expect_err("fault must fire on second reserve");
    assert!(err.contains("injected crash"));

    f.host.restart();

    sync::sync_at(
        &f.host,
        SyncInput { conversation: "conv_1".to_string(), full: false },
        now + 100,
    )
    .await
    .unwrap();

    let dec_key = decision_id(&quote.signed.quote_id);
    let dec_after: DecisionRow = get_row(&f.host, LEDGER, &dec_key).await.unwrap().unwrap();
    assert_eq!(dec_after.state, DecisionState::Agreed);
}

#[tokio::test]
async fn crash_after_the_countersign_row_is_finished_on_the_next_sync() {
    let f = fixture();
    let now = f.host.now();
    let (comm, _) = slot_commitment("c1", "listing_1", now + 1000, now + 4600);
    let (quote, q_rec_id, mut row, now) = setup_test_deal(&f, vec![comm], now + 3600).await;
    put_row(&f.host, AGREEMENTS, &q_rec_id, &row).await.unwrap();
    file_card_and_move_cursor(&f, "conv_1", "msg_accept_3", now).await;

    // Crash after countersign stored: when writing snapshot row
    f.host.crash_on_write("agreement_progress", "snapshot:");

    let err = decide::run(&f.host, &mut row, &quote, now)
        .await
        .expect_err("fault must fire on snapshot write");
    assert!(err.contains("injected crash"));

    let cs_key = countersign_id(&q_rec_id);
    let cs: Option<CountersignRow> = get_row(&f.host, LEDGER, &cs_key).await.unwrap();
    assert!(cs.is_some(), "countersign row must be stored before crash");

    f.host.restart();

    sync::sync_at(
        &f.host,
        SyncInput { conversation: "conv_1".to_string(), full: false },
        now + 100,
    )
    .await
    .unwrap();

    let dec_key = decision_id(&quote.signed.quote_id);
    let dec_after: DecisionRow = get_row(&f.host, LEDGER, &dec_key).await.unwrap().unwrap();
    assert_eq!(dec_after.state, DecisionState::Agreed);
}

#[tokio::test]
async fn crash_after_the_snapshot_row_is_finished_on_the_next_sync() {
    let f = fixture();
    let now = f.host.now();
    let (comm, _) = slot_commitment("c1", "listing_1", now + 1000, now + 4600);
    let (quote, q_rec_id, mut row, now) = setup_test_deal(&f, vec![comm], now + 3600).await;
    put_row(&f.host, AGREEMENTS, &q_rec_id, &row).await.unwrap();
    file_card_and_move_cursor(&f, "conv_1", "msg_accept_4", now).await;

    // Crash after snapshot stored: on agreement row write
    f.host.crash_on_write("agreements", &q_rec_id);

    let err = decide::run(&f.host, &mut row, &quote, now)
        .await
        .expect_err("fault must fire on agreements write");
    assert!(err.contains("injected crash"));

    let snap_key = snapshot_id(&q_rec_id, 1);
    let snap: Option<SnapshotRow> = get_row(&f.host, AGREEMENT_PROGRESS, &snap_key).await.unwrap();
    assert!(snap.is_some(), "snapshot row must be stored before crash");

    f.host.restart();

    sync::sync_at(
        &f.host,
        SyncInput { conversation: "conv_1".to_string(), full: false },
        now + 100,
    )
    .await
    .unwrap();

    let dec_key = decision_id(&quote.signed.quote_id);
    let dec_after: DecisionRow = get_row(&f.host, LEDGER, &dec_key).await.unwrap().unwrap();
    assert_eq!(dec_after.state, DecisionState::Agreed);
}

#[tokio::test]
async fn crash_between_agreement_row_and_decision_row_is_finished_on_the_next_sync() {
    let f = fixture();
    let now = f.host.now();
    let (comm, _) = slot_commitment("c1", "listing_1", now + 1000, now + 4600);
    let (quote, q_rec_id, mut row, now) = setup_test_deal(&f, vec![comm], now + 3600).await;
    put_row(&f.host, AGREEMENTS, &q_rec_id, &row).await.unwrap();
    file_card_and_move_cursor(&f, "conv_1", "msg_accept_5", now).await;

    // Skip 1 write (the fence write to decision_id), and crash on the 2nd write
    // (mark_decision_agreed)
    let dec_target = decision_id(&quote.signed.quote_id);
    f.host.crash_on_write_skip("ledger", &dec_target, 1);

    let err = decide::run(&f.host, &mut row, &quote, now)
        .await
        .expect_err("fault must fire on decision write");
    assert!(err.contains("injected crash"));

    // Agreement row is already agreed seq 1
    let agr_stored: Option<AgreementRow> = get_row(&f.host, AGREEMENTS, &q_rec_id).await.unwrap();
    assert_eq!(agr_stored.unwrap().state, Some(AgreementState::Agreed));

    // But decision row remains deciding
    let dec_key = decision_id(&quote.signed.quote_id);
    let dec: Option<DecisionRow> = get_row(&f.host, LEDGER, &dec_key).await.unwrap();
    assert_eq!(dec.unwrap().state, DecisionState::Deciding);

    f.host.restart();

    sync::sync_at(
        &f.host,
        SyncInput { conversation: "conv_1".to_string(), full: false },
        now + 100,
    )
    .await
    .unwrap();

    let dec_after: DecisionRow = get_row(&f.host, LEDGER, &dec_key).await.unwrap().unwrap();
    assert_eq!(dec_after.state, DecisionState::Agreed);
}

#[tokio::test]
async fn crash_after_fence_with_clock_moved_past_expiry_ends_not_formed() {
    let f = fixture();
    let now = f.host.now();
    let (comm, _) = slot_commitment("c1", "listing_1", now + 100, now + 200);
    let (quote, q_rec_id, mut row, now) = setup_test_deal(&f, vec![comm], now + 300).await;
    put_row(&f.host, AGREEMENTS, &q_rec_id, &row).await.unwrap();
    file_card_and_move_cursor(&f, "conv_1", "msg_accept_6", now).await;

    f.host.crash_on_write("ledger", "seat:");

    let err =
        decide::run(&f.host, &mut row, &quote, now).await.expect_err("fault must fire on reserve");
    assert!(err.contains("injected crash"));

    f.host.restart();
    // Clock moved past expiry
    f.host.set_now(now + 350);

    sync::sync_at(
        &f.host,
        SyncInput { conversation: "conv_1".to_string(), full: false },
        now + 350,
    )
    .await
    .unwrap();

    let dec_key = decision_id(&quote.signed.quote_id);
    let dec: DecisionRow = get_row(&f.host, LEDGER, &dec_key).await.unwrap().unwrap();
    assert_eq!(dec.state, DecisionState::NotFormed);
    assert_eq!(dec.reason.as_deref(), Some("quote-expired"));

    let agr: AgreementRow = get_row(&f.host, AGREEMENTS, &q_rec_id).await.unwrap().unwrap();
    assert_eq!(agr.state, Some(AgreementState::NotFormed));
    assert_eq!(agr.state_reason.as_deref(), Some("quote-expired"));
}

#[tokio::test]
async fn crash_after_countersign_stored_with_clock_past_expiry_checks_no_refused_row() {
    let f = fixture();
    let now = f.host.now();
    let (comm, _) = slot_commitment("c1", "listing_1", now + 100, now + 200);
    let (quote, q_rec_id, mut row, now) = setup_test_deal(&f, vec![comm], now + 300).await;
    put_row(&f.host, AGREEMENTS, &q_rec_id, &row).await.unwrap();
    file_card_and_move_cursor(&f, "conv_1", "msg_accept_7", now).await;

    // Crash on snapshot write (countersign is already written!)
    f.host.crash_on_write("agreement_progress", "snapshot:");

    let err =
        decide::run(&f.host, &mut row, &quote, now).await.expect_err("fault must fire on snapshot");
    assert!(err.contains("injected crash"));

    f.host.restart();
    // Clock moved past expiry
    f.host.set_now(now + 350);

    sync::sync_at(
        &f.host,
        SyncInput { conversation: "conv_1".to_string(), full: false },
        now + 350,
    )
    .await
    .unwrap();

    let dec_key = decision_id(&quote.signed.quote_id);
    let dec_after: DecisionRow = get_row(&f.host, LEDGER, &dec_key).await.unwrap().unwrap();
    assert_eq!(dec_after.state, DecisionState::Agreed);

    // An agreed deal gets no refusal row
    let cp_rec_id = row.counterparty.unwrap().record_id;
    let ref_row: Option<RefusedRow> =
        get_row(&f.host, LEDGER, &refused_id(&cp_rec_id)).await.unwrap();
    assert!(ref_row.is_none(), "agreed countersigned deal must not have a refused row");
}

#[tokio::test]
async fn refused_confirm_stub_ends_not_formed() {
    let f = fixture();
    let now = f.host.now();
    let (comm, _) = slot_commitment("c1", "listing_1", now + 1000, now + 4600);
    let (quote, q_rec_id, mut row, now) = setup_test_deal(&f, vec![comm], now + 3600).await;
    put_row(&f.host, AGREEMENTS, &q_rec_id, &row).await.unwrap();

    let res =
        decide::run_with_reservations(&f.host, &mut row, &quote, RefusingConfirmReservations, now)
            .await;
    assert_eq!(res, Ok(false));

    let dec_key = decision_id(&quote.signed.quote_id);
    let dec: DecisionRow = get_row(&f.host, LEDGER, &dec_key).await.unwrap().unwrap();
    assert_eq!(dec.state, DecisionState::NotFormed);
    assert_eq!(dec.reason.as_deref(), Some("slot-expired"));

    // Holds are released
    let h_key = hold_id(&q_rec_id, "c1");
    let hold: Option<HoldLedgerRow> = get_row(&f.host, LEDGER, &h_key).await.unwrap();
    assert!(hold.is_none(), "holds must be released after confirm refusal");
}

#[tokio::test]
async fn refused_confirm_with_crash_after_not_formed_snapshot_stays_not_formed_on_rerun() {
    let f = fixture();
    let now = f.host.now();
    let (comm, slot_id) = slot_commitment("c1", "listing_1", now + 1000, now + 4600);
    let (quote, q_rec_id, mut row, now) = setup_test_deal(&f, vec![comm], now + 3600).await;
    put_row(&f.host, AGREEMENTS, &q_rec_id, &row).await.unwrap();

    // Crash right after not-formed snapshot stored: on agreement row write
    f.host.crash_on_write("agreements", &q_rec_id);

    let err =
        decide::run_with_reservations(&f.host, &mut row, &quote, RefusingConfirmReservations, now)
            .await
            .expect_err("fault must fire");
    assert!(err.contains("injected crash"));

    // Snapshot 1 is stored as not-formed
    let snap_key = snapshot_id(&q_rec_id, 1);
    let snap: Option<SnapshotRow> = get_row(&f.host, AGREEMENT_PROGRESS, &snap_key).await.unwrap();
    assert!(snap.is_some());
    assert_eq!(snap.unwrap().snapshot.state, AgreementState::NotFormed);

    f.host.restart();

    // Another party holds and releases the slot; slot is free again
    let s_key = seat_id(&slot_id, 1);
    let seat: Option<serde_json::Value> = get_row(&f.host, LEDGER, &s_key).await.unwrap();
    assert!(seat.is_none(), "seat was released");

    // Repeat half delivery runs with real reservations (slot is free)
    let res = decide::run(&f.host, &mut row, &quote, now).await;
    assert_eq!(res, Ok(false));

    // Stays not-formed! Does not re-reserve!
    let dec_key = decision_id(&quote.signed.quote_id);
    let dec: DecisionRow = get_row(&f.host, LEDGER, &dec_key).await.unwrap().unwrap();
    assert_eq!(dec.state, DecisionState::NotFormed);
    assert_eq!(dec.reason.as_deref(), Some("slot-expired"));
}

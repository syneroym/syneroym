use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    clock,
    deal::{DealTerms, LocalText},
    lang::LangTag,
    progress::{self, AgreementState, RECORD_AGREEMENT_PROGRESS},
    record::{Envelope, RECORD_QUOTE, RecordDraft},
    transaction::{
        Answers, QUOTE_VERSION, QuotePayload, ReceiptHalf, derive_quote_id, verify_quote,
    },
};

use super::helpers::{build_and_sign_quote, make_counterparty_receipt, slot_commitment};
use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, CardRow, Direction, LEDGER,
    agreement_ops::{self, AgreementAcceptInput},
    decide::{
        self, CountersignRow, DecisionRow, DecisionState, RefusedRow, SnapshotRow, countersign_id,
        decision_id, refused_id, snapshot_id,
    },
    get_row,
    hold::{HoldLedgerRow, hold_id},
    put_row,
    quote_ops::store_received_quote,
    sync::receipts::file_progress_card,
    tests::{TestFixture, fixture},
};

#[tokio::test]
async fn two_syncs_send_one_refusal() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("cp identity");
    let counterparty_did = substrate::derive_did_key(&counterparty.public_key());
    let now = clock::now_secs();

    // Author creates quote expiring at now + 100
    let (quote_payload, q_rec_id, _) =
        build_and_sign_quote(&fix.master, &counterparty_did, 1, None, now + 100, now);

    let (cp_rec_id, cp_env) = make_counterparty_receipt(
        &counterparty,
        &fix.master_did,
        &q_rec_id,
        &quote_payload.terms,
        now,
    );

    let mut row = AgreementRow {
        quote_record_id: q_rec_id.clone(),
        quote_id: quote_payload.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: fix.master_did.clone(),
        terms: quote_payload.terms.clone(),
        agreed_at_secs: None,
        counterparty: Some(ReceiptHalf {
            record_id: cp_rec_id.clone(),
            envelope: cp_env,
            issuer: counterparty_did,
            issued_at_secs: now,
        }),
        quote_author: None,
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: vec![],
        updated_at_secs: now,
    };

    // First sync happens after expiry (now + 200)
    let res1 = decide::run(&fix.host, &mut row, &quote_payload, now + 200).await;
    assert_eq!(res1, Ok(false));

    // Refusal snapshot 1 was stored
    let snap_key = snapshot_id(&q_rec_id, 1);
    let snap: Option<SnapshotRow> =
        get_row(&fix.host, AGREEMENT_PROGRESS, &snap_key).await.expect("get snap");
    assert!(snap.is_some());
    let s = snap.unwrap();
    assert_eq!(s.snapshot.state, AgreementState::NotFormed);
    assert_eq!(s.snapshot.state_reason.as_deref(), Some("quote-expired"));

    // Refused row was created in LEDGER
    let ref_key = refused_id(&cp_rec_id);
    let ref_row: Option<RefusedRow> =
        get_row(&fix.host, LEDGER, &ref_key).await.expect("get refused");
    assert!(ref_row.is_some());
    assert_eq!(ref_row.unwrap().code, "quote-expired");

    // Decision row was NOT created
    let dec_key = decision_id(&quote_payload.quote_id);
    let dec: Option<DecisionRow> = get_row(&fix.host, LEDGER, &dec_key).await.expect("get dec");
    assert!(dec.is_none());

    // Second sync runs at now + 210: skips because refused row exists
    let res2 = decide::run(&fix.host, &mut row, &quote_payload, now + 210).await;
    assert_eq!(res2, Ok(false));
}

#[tokio::test]
async fn a_countersigned_half_before_expiry_still_forms_the_agreement() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("cp identity");
    let counterparty_did = substrate::derive_did_key(&counterparty.public_key());
    let now = clock::now_secs();

    // Quote expires at now + 100
    let (quote_payload, q_rec_id, _) =
        build_and_sign_quote(&fix.master, &counterparty_did, 1, None, now + 100, now);

    let (cp_rec_id, cp_env) = make_counterparty_receipt(
        &counterparty,
        &fix.master_did,
        &q_rec_id,
        &quote_payload.terms,
        now + 10,
    );

    let mut row = AgreementRow {
        quote_record_id: q_rec_id.clone(),
        quote_id: quote_payload.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: fix.master_did.clone(),
        terms: quote_payload.terms.clone(),
        agreed_at_secs: None,
        counterparty: Some(ReceiptHalf {
            record_id: cp_rec_id,
            envelope: cp_env,
            issuer: counterparty_did,
            issued_at_secs: now + 10,
        }),
        quote_author: None,
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: vec![],
        updated_at_secs: now + 10,
    };

    // Stored countersign row was issued before expiry at now + 20
    let cs_key = countersign_id(&q_rec_id);
    let cs_row = CountersignRow {
        kind: "countersign".to_string(),
        agreement: q_rec_id.clone(),
        half: ReceiptHalf {
            envelope: "{}".to_string(),
            record_id: "rec_cs_1".to_string(),
            issuer: fix.master_did.clone(),
            issued_at_secs: now + 20,
        },
        message_id: Some("msg_cs_1".to_string()),
        created_at_secs: now + 20,
    };
    put_row(&fix.host, LEDGER, &cs_key, &cs_row).await.expect("put cs");

    // Decision fence exists in deciding state
    let dec_key = decision_id(&quote_payload.quote_id);
    let dec_fence = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Deciding.as_str().to_string(),
        agreement: q_rec_id.clone(),
        quote_id: quote_payload.quote_id.clone(),
        reason: None,
        created_at_secs: now + 20,
    };
    put_row(&fix.host, LEDGER, &dec_key, &dec_fence).await.expect("put dec");

    // Time passes expiry (now + 200 > now + 100). Recovery runs decide::run:
    let res = decide::run(&fix.host, &mut row, &quote_payload, now + 200).await;
    assert_eq!(res, Ok(true));
    assert_eq!(row.state, Some(AgreementState::Agreed));

    let dec: DecisionRow =
        get_row(&fix.host, LEDGER, &dec_key).await.expect("get dec").expect("dec exists");
    assert_eq!(dec.state, DecisionState::Agreed.as_str());
}

fn make_signed_refusal_snapshot(
    author: &Identity,
    author_did: &str,
    counterparty_did: &str,
    q_rec_id: &str,
    terms: &DealTerms,
    reason: &str,
    now: u64,
) -> String {
    let snap_payload = progress::open(
        q_rec_id.to_string(),
        "conv_1".to_string(),
        counterparty_did.to_string(),
        author_did.to_string(),
        terms,
        Some(reason.to_string()),
    );
    let snap_draft = RecordDraft {
        version: progress::AGREEMENT_PROGRESS_VERSION,
        record_type: RECORD_AGREEMENT_PROGRESS.to_string(),
        subject: q_rec_id.to_string(),
        payload: serde_json::to_value(&snap_payload).expect("serialize snap"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut snap_env, snap_bytes) =
        Envelope::unsigned(snap_draft, author_did.to_string(), None, now).expect("snap env");
    let snap_sig = author.sign(&snap_bytes).to_bytes();
    snap_env.attach_signature(z32::encode(&snap_sig)).expect("attach sig");
    snap_env.to_json().expect("snap json")
}

async fn store_sample_slot_quote(
    fix: &TestFixture,
    author: &Identity,
    author_did: &str,
    now: u64,
) -> (QuotePayload, String) {
    let (slot_comm, _) = slot_commitment("c1", "listing_99", now + 1000, now + 4600);
    let quote_payload = QuotePayload {
        quote_id: derive_quote_id("conv_1", author_did, 1).expect("quote id"),
        conversation: "conv_1".to_string(),
        sequence: 1,
        counterparty_did: fix.master_did.clone(),
        terms: DealTerms {
            currency: "EUR".to_string(),
            commitments: vec![slot_comm],
            home_exchange: None,
            listing_id: None,
            scope: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Lesson"),
            cancellation_terms: LocalText::plain(
                LangTag::parse_canonical("en").expect("lang"),
                "24h",
            ),
            refund_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Full"),
            dispute_path: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Arb"),
            quote_expires_at_secs: now + 3600,
        },
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
    };

    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: "conv_1".to_string(),
        payload: serde_json::to_value(&quote_payload).expect("serialize quote"),
        expires_at_secs: Some(now + 3600),
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, author_did.to_string(), None, now).expect("env");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let q_rec_id = env.record_id().expect("rec id");
    let env_json = env.to_json().expect("env json");

    let v_q = verify_quote(&env_json, now);
    store_received_quote(&fix.host, &env_json, &v_q, &quote_payload, now, &fix.master_did)
        .await
        .expect("store quote");
    (quote_payload, q_rec_id)
}

#[tokio::test]
async fn counterparty_releases_holds_on_refusal_snapshot() {
    let fix = fixture();
    let author = Identity::generate().expect("author identity");
    let author_did = substrate::derive_did_key(&author.public_key());
    let now = clock::now_secs();

    let (quote_payload, q_rec_id) = store_sample_slot_quote(&fix, &author, &author_did, now).await;

    // Counterparty accepts, which places a hold on the slot
    agreement_ops::accept_at(
        &fix.host,
        AgreementAcceptInput {
            quote_record_id: q_rec_id.clone(),
            viewed_lang: None,
            viewed_via_mt: Some(false),
        },
        now + 10,
    )
    .await
    .expect("accept");

    // Verify hold exists
    let h_key = hold_id(&q_rec_id, "c1");
    let hold_before: Option<HoldLedgerRow> =
        get_row(&fix.host, LEDGER, &h_key).await.expect("get hold");
    assert!(hold_before.is_some(), "hold must be active after accept");

    // Author sends not-formed refusal snapshot with reason quote-superseded
    let snap_json = make_signed_refusal_snapshot(
        &author,
        &author_did,
        &fix.master_did,
        &q_rec_id,
        &quote_payload.terms,
        "quote-superseded",
        now + 20,
    );

    let card_row = CardRow {
        message_id: "m_snap_refuse".to_string(),
        conversation: "conv_1".to_string(),
        direction: Direction::Incoming,
        sender_timestamp_ms: ((now + 20) * 1000) as i64,
        card_type: "agreement-progress".to_string(),
        version: 1,
        known: true,
        verified: true,
        expired: false,
        reason: None,
        issuer: None,
        record_id: None,
        revocation_status: None,
        data: None,
        stored_at_secs: now + 20,
        declined: None,
        version_count: None,
        deal_state: None,
        agreement_payee: None,
        agreement_payment_methods: vec![],
    };

    let res =
        file_progress_card(&fix.host, card_row, &snap_json, "conv_1", now + 20, &fix.master_did)
            .await
            .expect("file progress card");
    assert!(res.filed);

    // Verify hold is released
    let hold_after: Option<HoldLedgerRow> =
        get_row(&fix.host, LEDGER, &h_key).await.expect("get hold after");
    assert!(hold_after.is_none(), "not-formed refusal must release the hold");

    let agr: AgreementRow =
        get_row(&fix.host, AGREEMENTS, &q_rec_id).await.expect("get agr").expect("agr exists");
    assert_eq!(agr.state, Some(AgreementState::NotFormed));
    assert_eq!(agr.state_reason.as_deref(), Some("quote-superseded"));
}

#[tokio::test]
async fn acceptance_when_quote_already_agreed_is_refused() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp identity");
    let cp_did = substrate::derive_did_key(&cp.public_key());
    let now = clock::now_secs();

    let (quote_payload, q_rec_id, _) =
        build_and_sign_quote(&fix.master, &cp_did, 1, None, now + 3600, now);

    let (cp_rec_id, cp_env) =
        make_counterparty_receipt(&cp, &fix.master_did, &q_rec_id, &quote_payload.terms, now);

    let mut row = AgreementRow {
        quote_record_id: q_rec_id.clone(),
        quote_id: quote_payload.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: fix.master_did.clone(),
        terms: quote_payload.terms.clone(),
        agreed_at_secs: None,
        counterparty: Some(ReceiptHalf {
            record_id: cp_rec_id.clone(),
            envelope: cp_env,
            issuer: cp_did,
            issued_at_secs: now,
        }),
        quote_author: None,
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: vec![],
        updated_at_secs: now,
    };

    // Another agreement was already formed for this quote_id
    let dec_key = decision_id(&quote_payload.quote_id);
    let existing_dec = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Agreed.as_str().to_string(),
        agreement: "other_quote_record_id".to_string(),
        quote_id: quote_payload.quote_id.clone(),
        reason: None,
        created_at_secs: now - 10,
    };
    put_row(&fix.host, LEDGER, &dec_key, &existing_dec).await.expect("put dec");

    // Acceptance of this version runs decide::run:
    let res = decide::run(&fix.host, &mut row, &quote_payload, now).await;
    assert_eq!(res, Ok(false));

    // Refusal snapshot 1 was stored with quote-already-agreed
    let snap_key = snapshot_id(&q_rec_id, 1);
    let snap: Option<SnapshotRow> =
        get_row(&fix.host, AGREEMENT_PROGRESS, &snap_key).await.expect("get snap");
    assert!(snap.is_some());
    let s = snap.unwrap();
    assert_eq!(s.snapshot.state, AgreementState::NotFormed);
    assert_eq!(s.snapshot.state_reason.as_deref(), Some("quote-already-agreed"));

    // Refused row created
    let ref_key = refused_id(&cp_rec_id);
    let ref_row: Option<RefusedRow> = get_row(&fix.host, LEDGER, &ref_key).await.expect("get ref");
    assert!(ref_row.is_some());
    assert_eq!(ref_row.unwrap().code, "quote-already-agreed");

    // Decision row still points to the other agreement!
    let dec_after: DecisionRow =
        get_row(&fix.host, LEDGER, &dec_key).await.expect("get dec").unwrap();
    assert_eq!(dec_after.agreement, "other_quote_record_id");
}

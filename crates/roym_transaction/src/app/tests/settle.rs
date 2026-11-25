use std::sync::Arc;

use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    backup::ImportInput,
    clock,
    conversation::Direction,
    deal::{
        Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind, SettlementRole,
        transition::CommitmentState,
    },
    disclosure::{FULFILMENT_SET, reveal_hidden_strict},
    fulfilment::{FULFILMENT_RECEIPT_VERSION, FulfilmentHidden, FulfilmentReceiptPayload},
    lang::LangTag,
    method::NoParams,
    progress::{self as core_progress, AgreementProgressPayload, AgreementState, FormedAs},
    record::{
        Envelope, RECORD_AGREEMENT_RECEIPT, RECORD_FULFILMENT_RECEIPT, RECORD_QUOTE, RecordDraft,
    },
    testing::sd::{PlainFulfilment, PlainQuote, conceal_fulfilment, conceal_quote},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, Answers, DealRole, QUOTE_VERSION,
        ReceiptHalf, SignedTerms, derive_quote_id,
    },
};
use z32;

use super::{fixture, fixture_with_master};
use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRef, AgreementRow, CardRow, FULFILMENTS,
    FulfilmentsRow, QUOTE_HISTORY, SendState, backup,
    decide::{SnapshotRow, snapshot_id},
    disclosures, ensure_collections,
    fulfilment_ops::{self, FulfilmentSignInput, fulfilment_id},
    get_row,
    payment_ops::{self, PaymentAckInput, PaymentRequestInput},
    progress, progress_ops, put_bytes, put_row,
    sync::settle_receipts::file_fulfilment_card,
};

pub(super) fn make_service_commitment(id: &str) -> Commitment {
    Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::GoodsOrService,
        kind: CommitmentKind::Fulfilment,
        extension: None,
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Piano lesson",
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

pub(super) fn make_currency_commitment(id: &str, author_did: &str) -> Commitment {
    Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::Counterparty,
        resource_kind: ResourceKind::Currency,
        kind: CommitmentKind::Currency,
        extension: None,
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Payment for lesson",
        ),
        declared_value_minor: 5000,
        details: None,
        window: None,
        location: None,
        payee: Some(author_did.to_string()),
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }
}

pub(super) fn sample_note(text: &str) -> LocalText {
    LocalText::plain(LangTag::parse_canonical("en").expect("parse lang"), text)
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
    fix: &super::TestFixture,
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
        FormedAs::Quote,
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
    put_row(&fix.host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await.expect("put snapshot row");
    snap1_payload
}

pub(super) fn incoming_card_row(
    message_id: &str,
    conversation: &str,
    card_type: &str,
    version: u32,
    issuer: &str,
    now: u64,
) -> CardRow {
    CardRow {
        message_id: message_id.to_string(),
        conversation: conversation.to_string(),
        direction: Direction::Incoming,
        sender_timestamp_ms: (now * 1000) as i64,
        card_type: card_type.to_string(),
        version,
        known: true,
        verified: true,
        expired: false,
        repeat: false,
        reason: None,
        issuer: Some(issuer.to_string()),
        record_id: None,
        revocation_status: None,
        data: None,
        stored_at_secs: now,
        declined: None,
        deal_state: None,
        agreement_payee: None,
        agreement_payment_methods: Vec::new(),
        version_count: None,
    }
}

pub(super) fn make_signed_fulfilment_envelope(
    counterparty: &Identity,
    cp_did: &str,
    agreement: &str,
    payload: &FulfilmentReceiptPayload,
    issued_at_secs: u64,
) -> String {
    let draft = RecordDraft {
        version: FULFILMENT_RECEIPT_VERSION,
        record_type: RECORD_FULFILMENT_RECEIPT.to_string(),
        subject: agreement.to_string(),
        payload: serde_json::to_value(payload).expect("payload to value"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, cp_did.to_string(), None, issued_at_secs)
        .expect("draft envelope");
    let sig = counterparty.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach signature");
    env.to_json().expect("envelope to json")
}

pub(super) async fn setup_agreed_deal(
    fix: &super::TestFixture,
    counterparty: &Identity,
    commitments: Vec<Commitment>,
    agreed_at: u64,
) -> (String, DealTerms) {
    ensure_collections(&fix.host).await.expect("ensure collections");

    let counterparty_did = substrate::derive_did_key(&counterparty.public_key());
    let quote_id = derive_quote_id("conv_1", &fix.master_did, 1).expect("derive quote id");

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
        quote_expires_at_secs: agreed_at + 3600,
    };

    let plain = PlainQuote {
        quote_id: quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
        counterparty_did: counterparty_did.clone(),
        terms: terms.clone(),
    };
    let (payload, disclosures) = conceal_quote(plain, 1);
    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: "conv_1".to_string(),
        payload: serde_json::to_value(&payload).expect("serialize quote payload"),
        expires_at_secs: Some(agreed_at + 3600),
        supersedes: None,
    };
    let (mut q_env, q_bytes) =
        Envelope::unsigned(draft, fix.master_did.clone(), None, agreed_at).expect("draft quote");
    let sig = fix.master.sign(&q_bytes).to_bytes();
    q_env.attach_signature(z32::encode(&sig)).expect("attach signature");
    let quote_record_id = q_env.record_id().unwrap();
    let q_json = q_env.to_json().expect("json");
    put_bytes(&fix.host, QUOTE_HISTORY, &quote_record_id, q_json.as_bytes()).await.unwrap();
    disclosures::store(&fix.host, &quote_record_id, RECORD_QUOTE, &disclosures, agreed_at)
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
        formed_as: None,
        quote_id,
        conversation: "conv_1".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: fix.master_did.clone(),
        terms: terms.clone(),
        agreed_at_secs: Some(agreed_at),
        counterparty: Some(counterparty_half),
        quote_author: Some(author_half),
        state: Some(AgreementState::Agreed),
        state_reason: None,
        cancel_reason: None,
        seq: Some(1),
        commitments: snap1_payload.commitments,
        updated_at_secs: agreed_at,
    };
    put_row(&fix.host, AGREEMENTS, &quote_record_id, &agr_row).await.expect("put agreement row");

    (quote_record_id, terms)
}

#[tokio::test]
async fn receiver_half_settles_with_a_settlement_note() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (agreement, _terms) =
        setup_agreed_deal(&fix, &counterparty, vec![make_service_commitment("c1")], agreed_at)
            .await;

    // Giver signs fulfilment
    let out_giver = fulfilment_ops::sign_at(
        &fix.host,
        FulfilmentSignInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            settlement_note: None,
        },
        agreed_at + 10,
    )
    .await
    .expect("author sign fulfilment");
    assert!(!out_giver.record_id.is_empty());

    // Counterparty receiver delivers fulfilment card with a settlement note
    let cp_did = substrate::derive_did_key(&counterparty.public_key());
    let note_text = "Piano was tuned excellently and sounds wonderful";
    let plain_fulfilment = PlainFulfilment {
        agreement: agreement.clone(),
        conversation: "conv_1".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: fix.master_did.clone(),
        role: SettlementRole::Receiver,
        settlement_note: Some(sample_note(note_text)),
    };
    let (rc_payload, rc_disclosures) = conceal_fulfilment(plain_fulfilment, 1);
    let envelope_json = make_signed_fulfilment_envelope(
        &counterparty,
        &cp_did,
        &agreement,
        &rc_payload,
        agreed_at + 20,
    );

    let card_row = incoming_card_row(
        "msg_fulfil_rc",
        "conv_1",
        RECORD_FULFILMENT_RECEIPT,
        FULFILMENT_RECEIPT_VERSION,
        &cp_did,
        agreed_at + 20,
    );

    let file_res = file_fulfilment_card(
        &fix.host,
        card_row,
        &envelope_json,
        &rc_disclosures,
        "conv_1",
        agreed_at + 20,
        &fix.master_did,
    )
    .await
    .expect("file fulfilment card");
    assert!(file_res.filed);
    assert!(!file_res.refused);

    // Verify fulfilment row holds receiver half with settlement note
    let f_id = fulfilment_id(&agreement, "c1");
    let f_row: Option<FulfilmentsRow> = get_row(&fix.host, FULFILMENTS, &f_id).await.unwrap();
    assert!(f_row.is_some());
    let r_half = f_row.unwrap().counterparty.expect("counterparty half stored");
    let disc = disclosures::load(&fix.host, &r_half.record_id).await.unwrap().unwrap();
    let env_parsed = Envelope::from_json(&r_half.envelope).unwrap();
    let parsed: FulfilmentReceiptPayload = serde_json::from_value(env_parsed.payload).unwrap();
    let revealed: FulfilmentHidden =
        reveal_hidden_strict(&parsed.sd, &FULFILMENT_SET, &disc).expect("reveal fulfilment");
    assert_eq!(revealed.settlement_note.map(|n| n.text), Some(note_text.to_string()));

    // Progress snapshot advances to Fulfilled and Closed
    let prog = progress_ops::progress_at(
        &fix.host,
        AgreementRef { agreement: agreement.clone() },
        agreed_at + 30,
    )
    .await
    .expect("get progress");
    assert_eq!(prog.snapshots[0].snapshot.commitments[0].state, CommitmentState::Fulfilled);
    assert_eq!(prog.snapshots[0].snapshot.state, AgreementState::Closed);
}

#[tokio::test]
async fn one_payment_per_currency_commitment() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (agreement, _terms) = setup_agreed_deal(
        &fix,
        &counterparty,
        vec![make_service_commitment("c1"), make_currency_commitment("c2", &fix.master_did)],
        agreed_at,
    )
    .await;

    // 1. Author requests payment for currency commitment c2
    let req1 = payment_ops::request(
        &fix.host,
        PaymentRequestInput {
            agreement: agreement.clone(),
            commitment_id: "c2".to_string(),
            note: Some(sample_note("Invoice #101")),
        },
    )
    .await
    .expect("first payment request");
    assert!(!req1.record_id.is_empty());
    assert_ne!(req1.state, SendState::AlreadyRecorded);

    // 2. Second request returns AlreadyRecorded with the same record_id
    let req2 = payment_ops::request(
        &fix.host,
        PaymentRequestInput {
            agreement: agreement.clone(),
            commitment_id: "c2".to_string(),
            note: Some(sample_note("Invoice #101 retry")),
        },
    )
    .await
    .expect("second payment request");
    assert_eq!(req2.record_id, req1.record_id);
    assert_eq!(req2.state, SendState::AlreadyRecorded);

    // 3. Author acknowledges payment for c2
    let ack1 = payment_ops::acknowledge(
        &fix.host,
        PaymentAckInput {
            agreement: agreement.clone(),
            commitment_id: "c2".to_string(),
            method: Some("sepa-instant".to_string()),
            reference: Some("TX-9988".to_string()),
            observed_at_secs: Some(agreed_at + 50),
            settlement_note: None,
            supersedes: None,
        },
    )
    .await
    .expect("first payment ack");
    assert!(!ack1.record_id.is_empty());

    // 4. Second ack without supersedes returns AlreadyRecorded with the same
    //    record_id
    let ack2 = payment_ops::acknowledge(
        &fix.host,
        PaymentAckInput {
            agreement: agreement.clone(),
            commitment_id: "c2".to_string(),
            method: Some("sepa-instant".to_string()),
            reference: Some("TX-9988".to_string()),
            observed_at_secs: Some(agreed_at + 60),
            settlement_note: None,
            supersedes: None,
        },
    )
    .await
    .expect("second payment ack");
    assert_eq!(ack2.record_id, ack1.record_id);
    assert_eq!(ack2.state, SendState::AlreadyRecorded);
}

#[tokio::test]
async fn repeated_sign_and_ack_return_the_same_record() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (agreement, _terms) =
        setup_agreed_deal(&fix, &counterparty, vec![make_service_commitment("c1")], agreed_at)
            .await;

    // Repeated fulfilment sign
    let sign1 = fulfilment_ops::sign_at(
        &fix.host,
        FulfilmentSignInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            settlement_note: Some(sample_note("first note")),
        },
        agreed_at + 10,
    )
    .await
    .expect("sign 1");

    let sign2 = fulfilment_ops::sign_at(
        &fix.host,
        FulfilmentSignInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            settlement_note: Some(sample_note("different note")),
        },
        agreed_at + 20,
    )
    .await
    .expect("sign 2");

    assert_eq!(sign1.record_id, sign2.record_id);
    assert_eq!(sign2.state, SendState::AlreadyRecorded);
}

fn make_fulfilment_card_row(msg_id: &str, issuer: &str, at: u64) -> CardRow {
    CardRow {
        message_id: msg_id.to_string(),
        conversation: "conv_1".to_string(),
        direction: Direction::Incoming,
        sender_timestamp_ms: (at * 1000) as i64,
        card_type: RECORD_FULFILMENT_RECEIPT.to_string(),
        version: FULFILMENT_RECEIPT_VERSION,
        known: true,
        verified: true,
        expired: false,
        repeat: false,
        reason: None,
        issuer: Some(issuer.to_string()),
        record_id: None,
        revocation_status: None,
        data: None,
        stored_at_secs: at,
        declined: None,
        deal_state: None,
        agreement_payee: None,
        agreement_payment_methods: Vec::new(),
        version_count: None,
    }
}

fn sign_bad_fulfilment_env(
    signer: &Identity,
    signer_did: &str,
    agreement: &str,
    cp_did: &str,
    master_did: &str,
    role: SettlementRole,
    at: u64,
) -> (String, Vec<String>) {
    let plain = PlainFulfilment {
        agreement: agreement.to_string(),
        conversation: "conv_1".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: cp_did.to_string(),
        quote_author_did: master_did.to_string(),
        role,
        settlement_note: None,
    };
    let (payload, disclosures) = conceal_fulfilment(plain, 1);
    let draft = RecordDraft {
        version: FULFILMENT_RECEIPT_VERSION,
        record_type: RECORD_FULFILMENT_RECEIPT.to_string(),
        subject: agreement.to_string(),
        payload: serde_json::to_value(&payload).expect("payload to value"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, signer_did.to_string(), None, at).expect("draft envelope");
    let sig = signer.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach signature");
    (env.to_json().expect("envelope to json"), disclosures)
}

#[tokio::test]
async fn a_half_from_the_wrong_party_is_not_filed() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (agreement, _terms) =
        setup_agreed_deal(&fix, &counterparty, vec![make_service_commitment("c1")], agreed_at)
            .await;

    let stranger = Identity::generate().expect("generate stranger");
    let stranger_did = substrate::derive_did_key(&stranger.public_key());
    let cp_did = substrate::derive_did_key(&counterparty.public_key());

    // 1. A stranger (neither author nor counterparty) signs a fulfilment card
    let (bad_env_json, bad_disc) = sign_bad_fulfilment_env(
        &stranger,
        &stranger_did,
        &agreement,
        &cp_did,
        &fix.master_did,
        SettlementRole::Giver,
        agreed_at + 20,
    );
    let card_row = make_fulfilment_card_row("msg_bad_stranger", &stranger_did, agreed_at + 20);

    let res = file_fulfilment_card(
        &fix.host,
        card_row,
        &bad_env_json,
        &bad_disc,
        "conv_1",
        agreed_at + 20,
        &fix.master_did,
    )
    .await
    .expect("file bad card");

    assert!(res.refused);

    let f_id = fulfilment_id(&agreement, "c1");
    let f_row: Option<FulfilmentsRow> = get_row(&fix.host, FULFILMENTS, &f_id).await.unwrap();
    assert!(
        f_row.is_none()
            || (f_row.as_ref().unwrap().quote_author.is_none()
                && f_row.as_ref().unwrap().counterparty.is_none())
    );

    // 2. Counterparty signs a fulfilment card claiming Giver role (when author is
    //    giver)
    let (bad_role_env_json, bad_role_disc) = sign_bad_fulfilment_env(
        &counterparty,
        &cp_did,
        &agreement,
        &cp_did,
        &fix.master_did,
        SettlementRole::Giver,
        agreed_at + 25,
    );
    let card_row2 = make_fulfilment_card_row("msg_bad_role", &cp_did, agreed_at + 25);

    let res2 = file_fulfilment_card(
        &fix.host,
        card_row2,
        &bad_role_env_json,
        &bad_role_disc,
        "conv_1",
        agreed_at + 25,
        &fix.master_did,
    )
    .await
    .expect("file bad role card");

    assert!(res2.refused);
}

#[tokio::test]
async fn second_fulfilment_receipt_from_same_party_is_refused_and_stores_no_disclosures() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("generate counterparty");
    let agreed_at = clock::now_secs();
    let (agreement, _terms) =
        setup_agreed_deal(&fix, &counterparty, vec![make_service_commitment("c1")], agreed_at)
            .await;
    let cp_did = substrate::derive_did_key(&counterparty.public_key());

    // 1. Counterparty files a first valid fulfilment receipt (receiver role)
    let (env1, disc1) = sign_bad_fulfilment_env(
        &counterparty,
        &cp_did,
        &agreement,
        &cp_did,
        &fix.master_did,
        SettlementRole::Receiver,
        agreed_at + 10,
    );
    let card1 = make_fulfilment_card_row("msg_f1", &cp_did, agreed_at + 10);
    let res1 = file_fulfilment_card(
        &fix.host,
        card1,
        &env1,
        &disc1,
        "conv_1",
        agreed_at + 10,
        &fix.master_did,
    )
    .await
    .expect("file first fulfilment receipt");
    assert!(res1.filed && !res1.refused);

    let rec_id1 = Envelope::from_json(&env1).unwrap().record_id().unwrap();
    let stored1 = disclosures::load(&fix.host, &rec_id1).await.unwrap();
    assert!(stored1.is_some());

    // 2. Counterparty sends a second, different fulfilment receipt for the same
    //    commitment
    let (env2, disc2) = sign_bad_fulfilment_env(
        &counterparty,
        &cp_did,
        &agreement,
        &cp_did,
        &fix.master_did,
        SettlementRole::Receiver,
        agreed_at + 20,
    );
    let rec_id2 = Envelope::from_json(&env2).unwrap().record_id().unwrap();
    assert_ne!(rec_id1, rec_id2);

    let card2 = make_fulfilment_card_row("msg_f2", &cp_did, agreed_at + 20);
    let res2 = file_fulfilment_card(
        &fix.host,
        card2,
        &env2,
        &disc2,
        "conv_1",
        agreed_at + 20,
        &fix.master_did,
    )
    .await
    .expect("file second fulfilment receipt");
    assert!(res2.refused);

    // No disclosures row should exist for the refused second receipt
    let stored2 = disclosures::load(&fix.host, &rec_id2).await.unwrap();
    assert!(stored2.is_none(), "disclosures row must not exist for refused second receipt");

    // Export and import must succeed
    let bundle = backup::export(&fix.host, NoParams {}).await.expect("export bundle");
    let f2 = fixture_with_master(Arc::clone(&fix.master));
    let import_out = backup::import(&f2.host, ImportInput { bundle }).await.expect("import bundle");
    assert!(import_out.imported);
}

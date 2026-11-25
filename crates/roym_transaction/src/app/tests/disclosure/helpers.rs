use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    deal::{Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind},
    lang::LangTag,
    progress::{self as core_progress, AgreementProgressPayload, AgreementState, FormedAs},
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RECORD_QUOTE, RecordDraft},
    testing::sd::{PlainQuote, conceal_quote},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, Answers, DealRole, QUOTE_VERSION,
        QuotePayload, ReceiptHalf, SignedTerms, derive_quote_id,
    },
};
use z32;

pub(crate) use crate::app::tests::refuse::helpers::{
    build_and_sign_enquiry, build_and_sign_enquiry_conv, build_and_sign_quote,
    make_counterparty_receipt, sample_terms_input, slot_commitment,
};
use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, QUOTE_HISTORY,
    decide::{SnapshotRow, snapshot_id},
    disclosures, ensure_collections, progress, put_bytes, put_row,
    quote_ops::DealTermsInput,
    tests::TestFixture,
};

pub(crate) fn sample_deal_terms(_now: u64) -> DealTermsInput {
    sample_terms_input("General lesson terms")
}

pub(crate) fn sample_service_commitment(id: &str) -> Commitment {
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

pub(crate) fn sample_currency_commitment(id: &str, author_did: &str) -> Commitment {
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

pub(crate) fn make_receipt_half(
    signer: &Identity,
    delegation: Option<String>,
    role: DealRole,
    quote_record_id: &str,
    terms: &SignedTerms,
    parties: (&str, &str),
    now: u64,
) -> ReceiptHalf {
    let (counterparty_did, quote_author_did) = parties;
    let issuer_did = match &delegation {
        Some(_) => quote_author_did.to_string(),
        None => substrate::derive_did_key(&signer.public_key()),
    };
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
        payload: serde_json::to_value(&payload).expect("serialize receipt payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, issuer_did.clone(), delegation, now).expect("draft envelope");
    let sig = signer.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach signature");
    let envelope_json = env.to_json().expect("envelope to json");
    let record_id = env.record_id().expect("record id");
    ReceiptHalf { record_id, envelope: envelope_json, issuer: issuer_did, issued_at_secs: now }
}

async fn store_test_quote_record(
    fix: &TestFixture,
    payload: &QuotePayload,
    agreed_at: u64,
) -> (String, String) {
    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: "conv_1".to_string(),
        payload: serde_json::to_value(payload).expect("serialize quote payload"),
        expires_at_secs: Some(agreed_at + 3600),
        supersedes: None,
    };
    let (mut q_env, q_bytes) =
        Envelope::unsigned(draft, fix.master_did.clone(), Some(fix.cert_json.clone()), agreed_at)
            .expect("draft quote");
    let sig = fix.service_key.sign(&q_bytes).to_bytes();
    q_env.attach_signature(z32::encode(&sig)).expect("attach signature");
    let q_json = q_env.to_json().expect("json");
    let record_id = q_env.record_id().expect("record id");
    put_bytes(&fix.host, QUOTE_HISTORY, &record_id, q_json.as_bytes()).await.unwrap();
    (record_id, q_json)
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

pub(crate) async fn setup_agreed_currency_deal(
    fix: &TestFixture,
    counterparty: &Identity,
    agreed_at: u64,
) -> (String, DealTerms) {
    ensure_collections(&fix.host).await.expect("ensure collections");

    let counterparty_did = substrate::derive_did_key(&counterparty.public_key());
    let quote_id = derive_quote_id("conv_1", &fix.master_did, 1).expect("derive quote id");

    let commitments =
        vec![sample_service_commitment("c1"), sample_currency_commitment("c2", &fix.master_did)];

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
    let (quote_record_id, _q_json) = store_test_quote_record(fix, &payload, agreed_at).await;
    disclosures::store(&fix.host, &quote_record_id, RECORD_QUOTE, &disclosures, agreed_at)
        .await
        .unwrap();

    let author_half = make_receipt_half(
        &fix.service_key,
        Some(fix.cert_json.clone()),
        DealRole::QuoteAuthor,
        &quote_record_id,
        &payload.terms,
        (&counterparty_did, &fix.master_did),
        agreed_at,
    );

    let counterparty_half = make_receipt_half(
        counterparty,
        None,
        DealRole::Counterparty,
        &quote_record_id,
        &payload.terms,
        (&counterparty_did, &fix.master_did),
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

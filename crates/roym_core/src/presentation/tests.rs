use serde_json::Value;
use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;
use crate::{
    deal::{
        Answers, Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind,
        transition::CommitmentState,
    },
    lang::LangTag,
    progress::{
        AGREEMENT_PROGRESS_VERSION, AgreementProgressPayload, AgreementState, CancelReason,
        CommitmentProgress,
    },
    record::{RECORD_AGREEMENT_PROGRESS, RECORD_AGREEMENT_RECEIPT, RECORD_QUOTE},
    testing::sd::{PlainQuote, conceal_quote},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, DealRole, QUOTE_VERSION, TimeWindow,
        derive_quote_id,
    },
};

fn lang_en() -> LangTag {
    LangTag::parse("en").expect("valid tag")
}

fn sign_test_record(
    key: &Identity,
    record_type: &str,
    version: u32,
    subject: &str,
    payload: Value,
    expires_at_secs: Option<u64>,
    now_secs: u64,
) -> String {
    let issuer = substrate::derive_did_key(&key.public_key());
    let draft = RecordDraft {
        version,
        record_type: record_type.to_string(),
        subject: subject.to_string(),
        payload,
        expires_at_secs,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, issuer, None, now_secs).expect("envelope draft");
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).expect("attach sig");
    env.to_json().expect("to json")
}

fn sample_two_commitment_terms() -> DealTerms {
    DealTerms {
        currency: "INR".to_string(),
        quote_expires_at_secs: 10_000,
        home_exchange: None,
        commitments: vec![
            Commitment {
                commitment_id: "c1".to_string(),
                giver: DealRole::QuoteAuthor,
                resource_kind: ResourceKind::GoodsOrService,
                kind: CommitmentKind::Fulfilment,
                extension: None,
                after: vec![],
                description: LocalText::plain(lang_en(), "Guitar lesson"),
                declared_value_minor: 50_000,
                details: None,
                window: Some(TimeWindow { earliest_secs: 5_000, latest_secs: 8_000 }),
                location: None,
                payee: None,
                payment_methods: None,
                tax_minor: None,
                fees_minor: None,
                eep: None,
            },
            Commitment {
                commitment_id: "c2".to_string(),
                giver: DealRole::Counterparty,
                resource_kind: ResourceKind::Currency,
                kind: CommitmentKind::Currency,
                extension: None,
                after: vec!["c1".to_string()],
                description: LocalText::plain(lang_en(), "Lesson fee"),
                declared_value_minor: 50_000,
                details: None,
                window: None,
                location: None,
                payee: Some("did:key:zAuthorPayee".to_string()),
                payment_methods: Some(vec!["upi".to_string()]),
                tax_minor: Some(2_000),
                fees_minor: Some(1_000),
                eep: None,
            },
        ],
        listing_id: None,
        scope: LocalText::plain(lang_en(), "60 minute music instruction"),
        cancellation_terms: LocalText::plain(lang_en(), "24 hour notice"),
        refund_terms: LocalText::plain(lang_en(), "Full refund if cancelled in time"),
        dispute_path: LocalText::plain(lang_en(), "Community mediation"),
    }
}

struct TestDealFixture {
    author_key: Identity,
    author_did: String,
    counterparty_did: String,
    quote_record_id: String,
    quote_env: String,
    author_half_env: String,
    cp_half_env: String,
    disclosures: Vec<String>,
    terms: crate::deal::sd::SignedTerms,
}

fn setup_test_deal(terms: DealTerms) -> TestDealFixture {
    setup_test_deal_seeded(terms, 100)
}

fn setup_test_deal_seeded(terms: DealTerms, seed: u64) -> TestDealFixture {
    let author_key = Identity::generate().expect("key");
    let counterparty_key = Identity::generate().expect("key");
    let author_did = substrate::derive_did_key(&author_key.public_key());
    let counterparty_did = substrate::derive_did_key(&counterparty_key.public_key());

    let conv = "conv_music_deal_1234";
    let quote_id = derive_quote_id(conv, &author_did, 1).expect("quote id");
    let plain_q = PlainQuote {
        quote_id: quote_id.clone(),
        conversation: conv.to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_777".to_string() },
        counterparty_did: counterparty_did.clone(),
        terms,
    };
    let (quote_payload, disclosures) = conceal_quote(plain_q, seed);

    let quote_env = sign_test_record(
        &author_key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote_id,
        serde_json::to_value(&quote_payload).expect("json"),
        Some(quote_payload.terms.quote_expires_at_secs),
        1_000,
    );

    let q_parsed = Envelope::from_json(&quote_env).expect("parse quote env");
    let quote_record_id = q_parsed.record_id().expect("rec id");

    let author_half = AgreementReceiptPayload {
        quote_record_id: quote_record_id.clone(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: author_did.clone(),
        role: DealRole::QuoteAuthor,
        terms: quote_payload.terms.clone(),
        viewed_lang: lang_en(),
        viewed_via_mt: false,
    };
    let author_half_env = sign_test_record(
        &author_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &quote_record_id,
        serde_json::to_value(&author_half).expect("json"),
        None,
        1_050,
    );

    let cp_half = AgreementReceiptPayload {
        quote_record_id: quote_record_id.clone(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: author_did.clone(),
        role: DealRole::Counterparty,
        terms: quote_payload.terms.clone(),
        viewed_lang: lang_en(),
        viewed_via_mt: false,
    };
    let cp_half_env = sign_test_record(
        &counterparty_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &quote_record_id,
        serde_json::to_value(&cp_half).expect("json"),
        None,
        1_060,
    );

    TestDealFixture {
        author_key,
        author_did,
        counterparty_did,
        quote_record_id,
        quote_env,
        author_half_env,
        cp_half_env,
        disclosures,
        terms: quote_payload.terms,
    }
}

#[test]
fn agreement_with_price_hidden_and_scope_shown_verifies() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // Disclose only scope from terms (salt, "scope", LocalText)
    let scope_disc = fix
        .disclosures
        .iter()
        .find(|d| {
            let dec = syneroym_signed_record::decode_disclosure(d).expect("decode");
            dec.name == "scope"
        })
        .expect("scope disclosure")
        .clone();

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.author_half_env, fix.cp_half_env],
        disclosures: vec![scope_disc],
    };

    let view = verify_presentation(&presentation, 1_100).expect("verify presentation");
    assert_eq!(view.deal.agreement, fix.quote_record_id);
    assert_eq!(view.records.len(), 2);
    assert!(view.records.iter().all(|r| r.verified));

    // Scope is shown
    let scope_val = view.terms.get("scope").expect("scope field in terms");
    assert_eq!(scope_val.get("text").and_then(Value::as_str), Some("60 minute music instruction"));

    // Price / declared values are hidden
    assert!(view.hidden.contains(&"terms.commitments[c1].declared_value_minor".to_string()));
    assert!(view.hidden.contains(&"terms.commitments[c2].declared_value_minor".to_string()));
    assert!(!view.hidden.contains(&"terms.scope".to_string()));
}

#[test]
fn quote_alone_with_price_hidden_verifies() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    let scope_disc = fix
        .disclosures
        .iter()
        .find(|d| {
            let dec = syneroym_signed_record::decode_disclosure(d).expect("decode");
            dec.name == "scope"
        })
        .expect("scope disclosure")
        .clone();

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env],
        disclosures: vec![scope_disc],
    };

    let view = verify_presentation(&presentation, 1_100).expect("verify quote presentation");
    assert_eq!(view.deal.agreement, fix.quote_record_id);
    assert!(view.deal.quote_id.is_some());
    assert_eq!(view.records.len(), 1);
    assert_eq!(view.records[0].record_type, RECORD_QUOTE);
    assert!(view.hidden.contains(&"terms.commitments[c1].declared_value_minor".to_string()));
}

#[test]
fn quote_and_half_with_different_terms_are_refused() {
    let fix1 = setup_test_deal(sample_two_commitment_terms());

    let mut terms2 = sample_two_commitment_terms();
    terms2.currency = "USD".to_string();
    let fix2 = setup_test_deal_seeded(terms2, 200);

    let author_half = AgreementReceiptPayload {
        quote_record_id: fix1.quote_record_id.clone(),
        counterparty_did: fix1.counterparty_did.clone(),
        quote_author_did: fix1.author_did.clone(),
        role: DealRole::QuoteAuthor,
        terms: fix2.terms,
        viewed_lang: lang_en(),
        viewed_via_mt: false,
    };
    let author_half_env = sign_test_record(
        &fix1.author_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &fix1.quote_record_id,
        serde_json::to_value(&author_half).expect("json"),
        None,
        1_050,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix1.quote_env, author_half_env],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100).expect_err("mismatched terms must fail");
    assert_eq!(err.code(), "terms-mismatch");
}

#[test]
fn snapshot_bound_to_the_quote_authors_half_is_the_writers() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    let progress_payload = AgreementProgressPayload {
        agreement: fix.quote_record_id.clone(),
        conversation: "conv_music_deal_1234".to_string(),
        counterparty_did: fix.counterparty_did.clone(),
        quote_author_did: fix.author_did.clone(),
        seq: 1,
        state: AgreementState::Agreed,
        state_reason: None,
        cancel_reason: None,
        commitments: vec![
            CommitmentProgress {
                commitment_id: "c1".to_string(),
                state: CommitmentState::Promised,
                reason: None,
                claimed: Some(false),
                halves: vec![],
            },
            CommitmentProgress {
                commitment_id: "c2".to_string(),
                state: CommitmentState::Promised,
                reason: None,
                claimed: Some(false),
                halves: vec![],
            },
        ],
    };

    // Signed with author_key (the quote author)
    let snap_env = sign_test_record(
        &fix.author_key,
        RECORD_AGREEMENT_PROGRESS,
        AGREEMENT_PROGRESS_VERSION,
        &fix.quote_record_id,
        serde_json::to_value(&progress_payload).expect("json"),
        None,
        1_200,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.author_half_env, fix.cp_half_env, snap_env],
        disclosures: vec![],
    };

    let view = verify_presentation(&presentation, 1_300).expect("verify with snapshot");
    assert_eq!(view.writer, WriterCheck::Checked);
}

#[test]
fn snapshot_from_another_key_is_writer_not_checked() {
    let fix = setup_test_deal(sample_two_commitment_terms());
    let other_key = Identity::generate().expect("key");

    let progress_payload = AgreementProgressPayload {
        agreement: fix.quote_record_id.clone(),
        conversation: "conv_music_deal_1234".to_string(),
        counterparty_did: fix.counterparty_did.clone(),
        quote_author_did: fix.author_did.clone(),
        seq: 1,
        state: AgreementState::Agreed,
        state_reason: None,
        cancel_reason: None,
        commitments: vec![
            CommitmentProgress {
                commitment_id: "c1".to_string(),
                state: CommitmentState::Promised,
                reason: None,
                claimed: Some(false),
                halves: vec![],
            },
            CommitmentProgress {
                commitment_id: "c2".to_string(),
                state: CommitmentState::Promised,
                reason: None,
                claimed: Some(false),
                halves: vec![],
            },
        ],
    };

    let snap_env = sign_test_record(
        &other_key,
        RECORD_AGREEMENT_PROGRESS,
        AGREEMENT_PROGRESS_VERSION,
        &fix.quote_record_id,
        serde_json::to_value(&progress_payload).expect("json"),
        None,
        1_200,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.author_half_env, fix.cp_half_env, snap_env],
        disclosures: vec![],
    };

    let view = verify_presentation(&presentation, 1_300).expect("verify with snapshot");
    assert_eq!(view.writer, WriterCheck::NotChecked);
}

#[test]
fn presentation_without_disclosures_shows_kinds_and_outcomes() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    let progress_payload = AgreementProgressPayload {
        agreement: fix.quote_record_id.clone(),
        conversation: "conv_music_deal_1234".to_string(),
        counterparty_did: fix.counterparty_did.clone(),
        quote_author_did: fix.author_did.clone(),
        seq: 2,
        state: AgreementState::Cancelled,
        state_reason: None,
        cancel_reason: Some(CancelReason::CannotDeliver),
        commitments: vec![
            CommitmentProgress {
                commitment_id: "c1".to_string(),
                state: CommitmentState::Fulfilled,
                reason: Some("teacher-cancelled".to_string()),
                claimed: Some(true),
                halves: vec![],
            },
            CommitmentProgress {
                commitment_id: "c2".to_string(),
                state: CommitmentState::Promised,
                reason: None,
                claimed: Some(false),
                halves: vec![],
            },
        ],
    };

    let snap_env = sign_test_record(
        &fix.author_key,
        RECORD_AGREEMENT_PROGRESS,
        AGREEMENT_PROGRESS_VERSION,
        &fix.quote_record_id,
        serde_json::to_value(&progress_payload).expect("json"),
        None,
        1_200,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.author_half_env, fix.cp_half_env, snap_env],
        disclosures: vec![],
    };

    let view = verify_presentation(&presentation, 1_300).expect("verify presentation");
    assert_eq!(view.commitments.len(), 2);

    let c1 = &view.commitments[0];
    assert_eq!(c1.id, "c1");
    assert_eq!(c1.kind, CommitmentKind::Fulfilment);
    assert_eq!(c1.giver, DealRole::QuoteAuthor);
    assert_eq!(c1.state, Some(CommitmentState::Fulfilled));
    assert_eq!(c1.claimed, Some(true));
    assert_eq!(c1.reason.as_deref(), Some("teacher-cancelled"));

    let c2 = &view.commitments[1];
    assert_eq!(c2.id, "c2");
    assert_eq!(c2.kind, CommitmentKind::Currency);
    assert_eq!(c2.giver, DealRole::Counterparty);
    assert_eq!(c2.claimed, Some(false));
}

#[test]
fn records_of_two_deals_are_refused() {
    let fix1 = setup_test_deal(sample_two_commitment_terms());
    let fix2 = setup_test_deal(sample_two_commitment_terms());

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix1.author_half_env, fix2.cp_half_env],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100).expect_err("different deals must fail");
    assert_eq!(err.code(), "multiple-deals");
}

#[test]
fn a_disclosure_of_another_object_is_refused() {
    let fix1 = setup_test_deal(sample_two_commitment_terms());
    let fix2 = setup_test_deal_seeded(sample_two_commitment_terms(), 500);

    // Disclose scope from deal 2 while presenting deal 1
    let foreign_disc = fix2.disclosures[0].clone();

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix1.author_half_env, fix1.cp_half_env],
        disclosures: vec![foreign_disc],
    };

    let err = verify_presentation(&presentation, 1_100).expect_err("foreign disclosure must fail");
    assert_eq!(err.code(), "disclosure-unknown-digest");
}

#[test]
fn a_cross_field_rule_with_a_field_hidden_is_not_checked() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // Present with no disclosures: all cross-field rules should be marked
    // not_checked
    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.author_half_env, fix.cp_half_env],
        disclosures: vec![],
    };

    let view = verify_presentation(&presentation, 1_100).expect("verify presentation");
    assert!(!view.not_checked.is_empty());
    assert!(view.not_checked.contains(&"agreement_receipt.viewed_via_mt_scope_lang".to_string()));
    assert!(view.not_checked.contains(&"deal_terms.all_texts_same_language".to_string()));
    assert!(view.not_checked.contains(&"terms.commitments[c2].tax_fees_within_value".to_string()));
}

#[test]
fn more_than_64_records_are_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());
    let mut records = Vec::new();
    for _ in 0..65 {
        records.push(fix.author_half_env.clone());
    }

    let presentation = Presentation { presentation_version: 1, records, disclosures: vec![] };

    let err = verify_presentation(&presentation, 1_100).expect_err("65 records must fail");
    assert_eq!(err.code(), "record-count");
}

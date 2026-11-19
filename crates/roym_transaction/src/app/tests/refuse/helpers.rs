use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    deal::{Commitment, CommitmentKind, DealTerms, Details, ExtensionRef, LocalText, ResourceKind},
    lang::LangTag,
    listing::derive_slot_id,
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RECORD_ENQUIRY, RECORD_QUOTE, RecordDraft},
    testing::sd::{PlainQuote, conceal_quote},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, Answers, DealRole, ENQUIRY_VERSION,
        EnquiryPayload, QUOTE_VERSION, SignedTerms, TimeWindow, derive_enquiry_id, derive_quote_id,
        verify_quote,
    },
};

use crate::app::{
    disclosures::HeldQuote,
    quote_ops::{CommitmentInput, DealTermsInput},
};

pub(crate) fn sample_commitment(id: &str) -> Commitment {
    Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::GoodsOrService,
        kind: CommitmentKind::Fulfilment,
        extension: None,
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Repair service",
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

pub(crate) fn slot_commitment(
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

pub(crate) fn sample_terms_input(scope_text: &str) -> DealTermsInput {
    DealTermsInput {
        currency: "EUR".to_string(),
        home_exchange: None,
        commitments: vec![CommitmentInput {
            giver: DealRole::QuoteAuthor,
            resource_kind: ResourceKind::GoodsOrService,
            kind: CommitmentKind::Fulfilment,
            extension: None,
            after: vec![],
            description: LocalText::plain(
                LangTag::parse_canonical("en").expect("parse lang"),
                scope_text,
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
        }],
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("parse lang"), scope_text),
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
    }
}

pub(crate) fn build_and_sign_enquiry(enquirer: &Identity, now: u64) -> (String, String) {
    build_and_sign_enquiry_conv(enquirer, "conv_1", now)
}

pub(crate) fn build_and_sign_enquiry_conv(
    enquirer: &Identity,
    conv: &str,
    now: u64,
) -> (String, String) {
    let enquirer_did = substrate::derive_did_key(&enquirer.public_key());
    let enquiry_id = derive_enquiry_id(conv, &enquirer_did, 1).expect("derive enquiry id");
    let payload = EnquiryPayload {
        enquiry_id,
        conversation: conv.to_string(),
        sequence: 1,
        categories: vec!["repair".to_string()],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Need fix",
        ),
        data_use_notice: "Standard notice".to_string(),
        window: None,
        area: None,
        listing_id: None,
    };
    let draft = RecordDraft {
        version: ENQUIRY_VERSION,
        record_type: RECORD_ENQUIRY.to_string(),
        subject: conv.to_string(),
        payload: serde_json::to_value(&payload).expect("serialize enquiry"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, enquirer_did, None, now).expect("draft envelope");
    let sig = enquirer.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("attach signature");
    let rec_id = env.record_id().expect("record id");
    let env_json = env.to_json().expect("envelope json");
    (rec_id, env_json)
}

pub(crate) fn build_and_sign_quote(
    author: &Identity,
    counterparty_did: &str,
    sequence: u32,
    supersedes: Option<String>,
    expires_at_secs: u64,
    now: u64,
) -> (HeldQuote, String, String) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let quote_id = derive_quote_id("conv_1", &author_did, sequence).expect("derive quote id");
    let terms = DealTerms {
        currency: "EUR".to_string(),
        commitments: vec![sample_commitment("c1")],
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("parse lang"), "Repair job"),
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
        sequence,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
        counterparty_did: counterparty_did.to_string(),
        terms: terms.clone(),
    };
    let (payload, disclosures) = conceal_quote(plain, 1);

    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: "conv_1".to_string(),
        payload: serde_json::to_value(&payload).expect("serialize quote"),
        expires_at_secs: Some(expires_at_secs),
        supersedes,
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

pub(crate) fn make_counterparty_receipt(
    cp: &Identity,
    author_did: &str,
    quote_record_id: &str,
    terms: &SignedTerms,
    now: u64,
) -> (String, String) {
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
    (record_id, envelope_json)
}

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    clock,
    deal::{Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind},
    lang::LangTag,
    record::{Envelope, RECORD_ENQUIRY, RECORD_QUOTE, RecordDraft},
    transaction::{
        Answers, DealRole, ENQUIRY_VERSION, EnquiryPayload, QUOTE_VERSION, QuotePayload,
        derive_enquiry_id, derive_quote_id, verify_quote,
    },
};

use super::fixture;
use crate::app::{
    AgreementRow, ENQUIRY_HISTORY, LEDGER,
    agreement_ops::{self, AgreementAcceptInput},
    decline_ops, put_bytes,
    quote_ops::{self, CommitmentInput, DealTermsInput, QuoteSetInput, store_received_quote},
};

fn sample_commitment(id: &str) -> Commitment {
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

fn sample_terms_input(scope_text: &str) -> DealTermsInput {
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

fn build_and_sign_enquiry(enquirer: &Identity, now: u64) -> (String, String) {
    let enquirer_did = substrate::derive_did_key(&enquirer.public_key());
    let enquiry_id = derive_enquiry_id("conv_1", &enquirer_did, 1).expect("derive enquiry id");
    let payload = EnquiryPayload {
        enquiry_id,
        conversation: "conv_1".to_string(),
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
        subject: "conv_1".to_string(),
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

fn build_and_sign_quote(
    author: &Identity,
    counterparty_did: &str,
    sequence: u32,
    supersedes: Option<String>,
    expires_at_secs: u64,
    now: u64,
) -> (QuotePayload, String, String) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let quote_id = derive_quote_id("conv_1", &author_did, sequence).expect("derive quote id");
    let payload = QuotePayload {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence,
        counterparty_did: counterparty_did.to_string(),
        terms: DealTerms {
            currency: "EUR".to_string(),
            commitments: vec![sample_commitment("c1")],
            home_exchange: None,
            listing_id: None,
            scope: LocalText::plain(
                LangTag::parse_canonical("en").expect("parse lang"),
                "Repair job",
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
    (payload, record_id, envelope_json)
}

#[tokio::test]
async fn accepting_a_superseded_version_is_refused() {
    let fix = fixture();
    let author = Identity::generate().expect("author identity");
    let counterparty_did = fix.master_did.clone();
    let now = clock::now_secs();

    // Author creates quote v1
    let (pay_q1, rec_q1, env_q1) =
        build_and_sign_quote(&author, &counterparty_did, 1, None, now + 3600, now);
    let v_q1 = verify_quote(&env_q1, now);
    assert!(v_q1.verified, "v_q1 must verify: {:?}", v_q1.reason);
    store_received_quote(&fix.host, &env_q1, &v_q1, &pay_q1, now, &counterparty_did)
        .await
        .expect("store quote v1");

    // Author creates quote v2 superseding v1 (same quote_id sequence 1)
    let (pay_q2, rec_q2, env_q2) = build_and_sign_quote(
        &author,
        &counterparty_did,
        1,
        Some(rec_q1.clone()),
        now + 3600,
        now + 10,
    );
    let v_q2 = verify_quote(&env_q2, now + 10);
    assert!(v_q2.verified, "v_q2 must verify: {:?}", v_q2.reason);
    store_received_quote(&fix.host, &env_q2, &v_q2, &pay_q2, now + 10, &counterparty_did)
        .await
        .expect("store quote v2");

    // Counterparty tries to accept superseded v1
    let accept_v1_err = agreement_ops::accept_at(
        &fix.host,
        AgreementAcceptInput {
            quote_record_id: rec_q1,
            viewed_lang: None,
            viewed_via_mt: Some(false),
        },
        now + 20,
    )
    .await
    .unwrap_err();

    let err_str = serde_json::to_string(&accept_v1_err).expect("err json");
    assert!(err_str.contains("quote-superseded"));

    // Counterparty accepts latest v2
    let accept_v2 = agreement_ops::accept_at(
        &fix.host,
        AgreementAcceptInput {
            quote_record_id: rec_q2.clone(),
            viewed_lang: None,
            viewed_via_mt: Some(false),
        },
        now + 20,
    )
    .await
    .expect("accept v2 succeeds");

    assert_eq!(accept_v2.quote_record_id, rec_q2);
}

#[tokio::test]
async fn revision_after_acceptance_is_refused() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("counterparty identity");
    let counterparty_did = substrate::derive_did_key(&counterparty.public_key());
    let now = clock::now_secs();

    // Store enquiry from counterparty in ENQUIRY_HISTORY
    let (enq_rec, enq_env) = build_and_sign_enquiry(&counterparty, now);
    put_bytes(&fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes())
        .await
        .expect("store enquiry");

    // Author (this node) sets quote v1
    let quote_res = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Fixing sink"),
        },
        now,
    )
    .await
    .expect("set quote v1");

    let quote_id = quote_res.quote_id.clone();
    let q_rec_id = quote_res.record_id.clone();

    // Counterparty accepts quote v1 (simulate receiving acceptance half)
    let (mut half_env, half_bytes) = Envelope::unsigned(
        RecordDraft {
            version: syneroym_roym_core::transaction::AGREEMENT_RECEIPT_VERSION,
            record_type: syneroym_roym_core::record::RECORD_AGREEMENT_RECEIPT.to_string(),
            subject: q_rec_id.clone(),
            payload: json!({
                "quote_record_id": q_rec_id,
                "role": "counterparty",
                "viewed_lang": "en",
                "viewed_via_mt": false,
            }),
            expires_at_secs: None,
            supersedes: None,
        },
        counterparty_did.clone(),
        None,
        now + 10,
    )
    .expect("draft half");
    let sig = counterparty.sign(&half_bytes).to_bytes();
    half_env.attach_signature(z32::encode(&sig)).expect("attach sig");
    let half_rec = half_env.record_id().expect("rec id");
    let half_json = half_env.to_json().expect("json");

    let agr_row = AgreementRow {
        quote_record_id: q_rec_id.clone(),
        quote_id: quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: fix.master_did.clone(),
        terms: sample_terms_input("Fixing sink").into_deal_terms(now + 3600),
        agreed_at_secs: None,
        counterparty: Some(syneroym_roym_core::transaction::ReceiptHalf {
            record_id: half_rec,
            envelope: half_json,
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
    crate::app::put_row(&fix.host, crate::app::AGREEMENTS, &q_rec_id, &agr_row)
        .await
        .expect("put agr");

    // Author tries to revise the quote
    let revise_err = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec,
            quote_id: Some(quote_id),
            expires_in_secs: 3600,
            terms: sample_terms_input("Fixing sink and pipe"),
        },
        now + 20,
    )
    .await
    .unwrap_err();

    let err_str = serde_json::to_string(&revise_err).expect("err json");
    assert!(err_str.contains("quote-already-accepted"));
}

#[tokio::test]
async fn revision_after_an_end_is_refused() {
    let fix = fixture();
    let counterparty = Identity::generate().expect("counterparty identity");
    let now = clock::now_secs();

    // Store enquiry from counterparty in ENQUIRY_HISTORY
    let (enq_rec, enq_env) = build_and_sign_enquiry(&counterparty, now);
    put_bytes(&fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes())
        .await
        .expect("store enquiry");

    // Author sets quote v1
    let quote_res = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Fixing sink"),
        },
        now,
    )
    .await
    .expect("set quote v1");

    let quote_id = quote_res.quote_id.clone();

    // Test case 1: Deal expired by clock
    let revise_expired_err = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: Some(quote_id.clone()),
            expires_in_secs: 3600,
            terms: sample_terms_input("Fixing sink revision"),
        },
        now + 4000, // past expiry (3600s)
    )
    .await
    .unwrap_err();

    let err_exp_str = serde_json::to_string(&revise_expired_err).expect("err json");
    assert!(err_exp_str.contains("quote-ended"));

    // Test case 2: Deal ended by decline
    let (enq_rec2, enq_env2) = build_and_sign_enquiry(&counterparty, now);
    put_bytes(&fix.host, ENQUIRY_HISTORY, &enq_rec2, enq_env2.as_bytes())
        .await
        .expect("store enquiry 2");

    let quote_res2 = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec2.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Job 2"),
        },
        now,
    )
    .await
    .expect("set quote v2");

    let quote_id2 = quote_res2.quote_id.clone();
    let q2_rec_id = quote_res2.record_id.clone();

    // Mark declined
    let dec_key = decline_ops::decline_id(&quote_id2);
    crate::app::put_row(
        &fix.host,
        LEDGER,
        &dec_key,
        &decline_ops::DeclineRow {
            kind: "decline".to_string(),
            deal: quote_id2.clone(),
            declines: q2_rec_id,
            record_id: "rec_dec".to_string(),
            envelope: "{}".to_string(),
            created_at_secs: now,
        },
    )
    .await
    .expect("put decline");

    let revise_declined_err = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec2,
            quote_id: Some(quote_id2),
            expires_in_secs: 3600,
            terms: sample_terms_input("Job 2 revised"),
        },
        now + 10,
    )
    .await
    .unwrap_err();

    let err_dec_str = serde_json::to_string(&revise_declined_err).expect("err json");
    assert!(err_dec_str.contains("quote-ended"));
}

//! Tests for presentation building, third-party verification, and disclosed
//! single-record verify.

use std::sync::Arc;

use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    deal::{DealTerms, LocalText},
    disclosure::{Salt, decode_disclosure, disclosure_digest, encode_disclosure},
    lang::LangTag,
    method::DisclosedVerifyInput,
    record::{Envelope, RECORD_QUOTE, RecordDraft},
    testing::sd::{PlainQuote, conceal_quote},
    transaction::{Answers, QUOTE_VERSION, derive_quote_id},
};

use crate::app::{
    QUOTE_HISTORY, disclosures, get_bytes,
    present_ops::{self, PresentInclude, PresentInput},
    quote_ops,
    tests::{TestFixture, disclosure::helpers, fixture},
};

async fn setup_present_test() -> (TestFixture, String, String, Vec<String>) {
    let fix = fixture();
    let cp_key = Arc::new(Identity::generate().expect("key"));
    let (quote_record_id, _terms) = helpers::setup_agreed_currency_deal(&fix, &cp_key, 1_000).await;

    let quote_bytes = get_bytes(&fix.host, QUOTE_HISTORY, &quote_record_id)
        .await
        .unwrap()
        .expect("quote bytes present");
    let quote_env = String::from_utf8(quote_bytes).expect("utf8");

    let discs =
        disclosures::load(&fix.host, &quote_record_id).await.unwrap().expect("disclosures present");

    (fix, quote_record_id, quote_env, discs)
}

#[tokio::test]
async fn present_shows_only_the_named_paths() {
    let (fix, quote_record_id, _quote_env, _discs) = setup_present_test().await;

    let input = PresentInput {
        record_id: quote_record_id.clone(),
        include: PresentInclude::default(),
        show: vec!["terms.scope".to_string()],
    };

    let pres = present_ops::present(&fix.host, input).await.expect("present succeeds");
    assert_eq!(pres.presentation_version, 1);
    assert!(!pres.records.is_empty());
    assert_eq!(pres.disclosures.len(), 1);

    let dec = decode_disclosure(&pres.disclosures[0]).expect("decode disclosure");
    assert_eq!(dec.name, "scope");

    let view = present_ops::verify_presentation(&fix.host, pres)
        .await
        .expect("verify presentation succeeds");

    assert_eq!(view.deal.agreement, quote_record_id);
    let scope_val = view.terms.get("scope").expect("scope is shown");
    assert_eq!(scope_val["text"].as_str(), Some("Lesson deal"));

    assert!(view.hidden.contains(&"terms.cancellation_terms".to_string()));
    assert!(view.hidden.contains(&"terms.commitments[c1].declared_value_minor".to_string()));
    assert!(view.hidden.contains(&"terms.commitments[c2].declared_value_minor".to_string()));
    assert!(!view.hidden.contains(&"terms.scope".to_string()));
}

#[tokio::test]
async fn present_refuses_a_path_that_is_not_hidden() {
    let (fix, quote_record_id, _quote_env, _discs) = setup_present_test().await;

    let input_visible = PresentInput {
        record_id: quote_record_id.clone(),
        include: PresentInclude::default(),
        show: vec!["terms.currency".to_string()],
    };

    let err = present_ops::present(&fix.host, input_visible)
        .await
        .expect_err("visible field path must fail");
    let err_obj = err.error.as_ref().expect("error object");
    assert_eq!(err_obj.code, -32602);
    assert!(err_obj.message.contains("terms.currency"));

    let input_nonexistent = PresentInput {
        record_id: quote_record_id.clone(),
        include: PresentInclude::default(),
        show: vec!["terms.commitments[c1].nonexistent".to_string()],
    };

    let err2 = present_ops::present(&fix.host, input_nonexistent)
        .await
        .expect_err("nonexistent field path must fail");
    let err2_obj = err2.error.as_ref().expect("error object");
    assert_eq!(err2_obj.code, -32602);
    assert!(err2_obj.message.contains("terms.commitments[c1].nonexistent"));
}

#[tokio::test]
async fn verify_with_disclosures_shows_them() {
    let (fix, _quote_record_id, quote_env, discs) = setup_present_test().await;

    let scope_disc = discs
        .iter()
        .find(|d| {
            let dec = decode_disclosure(d).expect("decode");
            dec.name == "scope"
        })
        .expect("scope disclosure")
        .clone();

    let input = DisclosedVerifyInput { envelope: quote_env, disclosures: vec![scope_disc] };

    let verdict = quote_ops::verify(&fix.host, input).await.expect("verify succeeds");
    assert!(verdict.verdict.verified);
    assert!(verdict.disclosure_refused.is_none());

    let shown = verdict.shown.expect("shown object present");
    assert!(shown.get("scope").is_some());
    assert!(!verdict.hidden.contains(&"terms.scope".to_string()));
    assert!(verdict.hidden.contains(&"terms.commitments[c1].declared_value_minor".to_string()));
}

#[tokio::test]
async fn verify_with_a_bad_disclosure_keeps_verified_and_names_the_problem() {
    let (fix, _quote_record_id, quote_env, _discs) = setup_present_test().await;

    let (_agr_id, terms) = helpers::setup_agreed_currency_deal(
        &fix,
        &Arc::new(Identity::generate().expect("key")),
        2_000,
    )
    .await;
    let quote_id = derive_quote_id("conv_1", &fix.master_did, 2).expect("quote id");
    let plain_q = PlainQuote {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence: 2,
        answers: Answers::Enquiry { record_id: "rec_foreign".to_string() },
        counterparty_did: "did:key:zCounterpartyForeign".to_string(),
        terms,
    };
    let (_foreign_payload, foreign_discs) = conceal_quote(plain_q, 999);
    let bad_disclosure = foreign_discs[0].clone();

    let input = DisclosedVerifyInput { envelope: quote_env, disclosures: vec![bad_disclosure] };

    let verdict = quote_ops::verify(&fix.host, input).await.expect("verify returns verdict");
    assert!(verdict.verdict.verified, "envelope signature must remain verified");
    assert!(verdict.shown.is_none(), "no fields shown when disclosure refused");
    assert_eq!(
        verdict.disclosure_refused,
        Some("disclosure-unknown-digest".to_string()),
        "problem must be named"
    );
    assert!(verdict.hidden.contains(&"terms.scope".to_string()));
}

fn tamper_quote_commitment_disclosure_for_verify(
    fix: &TestFixture,
    cid: &str,
    field_name: &str,
    tampered_val: serde_json::Value,
) -> (String, String) {
    let cp_key = Arc::new(Identity::generate().expect("key"));
    let cp_did = substrate::derive_did_key(&cp_key.public_key());
    let quote_id = derive_quote_id("conv_1", &fix.master_did, 1).expect("quote id");
    let terms = DealTerms {
        currency: "INR".to_string(),
        listing_id: None,
        quote_expires_at_secs: 10_000,
        home_exchange: None,
        commitments: vec![
            helpers::sample_service_commitment("c1"),
            helpers::sample_currency_commitment("c2", &fix.master_did),
        ],
        scope: LocalText::plain(LangTag::parse_canonical("en").unwrap(), "Lesson deal"),
        cancellation_terms: LocalText::plain(LangTag::parse_canonical("en").unwrap(), "Standard"),
        refund_terms: LocalText::plain(LangTag::parse_canonical("en").unwrap(), "Full refund"),
        dispute_path: LocalText::plain(LangTag::parse_canonical("en").unwrap(), "Mediation"),
    };
    let plain_q = PlainQuote {
        quote_id: quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
        counterparty_did: cp_did,
        terms,
    };
    let (mut quote_payload, disclosures) = conceal_quote(plain_q, 100);

    let comm = quote_payload
        .terms
        .commitments
        .iter_mut()
        .find(|c| c.commitment_id == cid)
        .expect("commitment");

    let old_disc_idx = disclosures
        .iter()
        .position(|d| {
            let dec = decode_disclosure(d).expect("decode");
            if dec.name != field_name {
                return false;
            }
            let dig = disclosure_digest(d);
            comm.sd.contains(&dig)
        })
        .expect("field disclosure found in commitment");

    let old_disc = &disclosures[old_disc_idx];
    let old_dec = decode_disclosure(old_disc).expect("decode");
    let old_digest = disclosure_digest(old_disc);

    let salt = Salt::from_encoded(&old_dec.salt).expect("salt");
    let new_disc = encode_disclosure(&salt, field_name, &tampered_val).expect("encode");
    let new_digest = disclosure_digest(&new_disc);

    let sd_pos = comm.sd.iter().position(|d| d == &old_digest).expect("digest in _sd");
    comm.sd[sd_pos] = new_digest;
    comm.sd.sort();

    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: quote_id,
        payload: serde_json::to_value(&quote_payload).expect("payload"),
        expires_at_secs: Some(10_000),
        supersedes: None,
    };
    let (mut q_env, q_bytes) =
        Envelope::unsigned(draft, fix.master_did.clone(), None, 1_000).expect("envelope");
    let sig = fix.master.sign(&q_bytes).to_bytes();
    q_env.attach_signature(z32::encode(&sig)).expect("attach signature");
    let quote_env = q_env.to_json().expect("json");
    (quote_env, new_disc)
}

#[tokio::test]
async fn quote_verify_refuses_invalid_revealed_fields() {
    let fix = fixture();

    // 1. String declared value
    let (q_str, d_str) = tamper_quote_commitment_disclosure_for_verify(
        &fix,
        "c2",
        "declared_value_minor",
        serde_json::json!("free"),
    );
    let verdict = quote_ops::verify(
        &fix.host,
        DisclosedVerifyInput { envelope: q_str, disclosures: vec![d_str] },
    )
    .await
    .unwrap();
    assert!(verdict.verdict.verified);
    assert_eq!(verdict.disclosure_refused, Some("field-invalid".to_string()));

    // 2. Negative declared value
    let (q_neg, d_neg) = tamper_quote_commitment_disclosure_for_verify(
        &fix,
        "c2",
        "declared_value_minor",
        serde_json::json!(-10),
    );
    let verdict = quote_ops::verify(
        &fix.host,
        DisclosedVerifyInput { envelope: q_neg, disclosures: vec![d_neg] },
    )
    .await
    .unwrap();
    assert!(verdict.verdict.verified);
    assert_eq!(verdict.disclosure_refused, Some("field-invalid".to_string()));

    // 3. Declared value over max
    let (q_max, d_max) = tamper_quote_commitment_disclosure_for_verify(
        &fix,
        "c2",
        "declared_value_minor",
        serde_json::json!(1_000_000_000_000_001i64),
    );
    let verdict = quote_ops::verify(
        &fix.host,
        DisclosedVerifyInput { envelope: q_max, disclosures: vec![d_max] },
    )
    .await
    .unwrap();
    assert!(verdict.verdict.verified);
    assert_eq!(verdict.disclosure_refused, Some("field-invalid".to_string()));

    // 4. Bad payment methods
    let (q_pm, d_pm) = tamper_quote_commitment_disclosure_for_verify(
        &fix,
        "c2",
        "payment_methods",
        serde_json::json!(vec!["invalid token!"]),
    );
    let verdict = quote_ops::verify(
        &fix.host,
        DisclosedVerifyInput { envelope: q_pm, disclosures: vec![d_pm] },
    )
    .await
    .unwrap();
    assert!(verdict.verdict.verified);
    assert_eq!(verdict.disclosure_refused, Some("field-invalid".to_string()));
}

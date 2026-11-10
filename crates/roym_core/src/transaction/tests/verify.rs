use syneroym_identity::{Identity, substrate};

use super::*;

#[test]
fn verify_quote_refuses_a_payload_whose_quote_id_was_derived_from_a_different_issuer() {
    let real_signer = Identity::generate().unwrap();
    let wrong_key = Identity::generate().unwrap();
    let wrong_issuer = substrate::derive_did_key(&wrong_key.public_key());

    let counterparty_key = Identity::generate().unwrap();
    let counterparty_did = substrate::derive_did_key(&counterparty_key.public_key());

    // Derive quote_id using wrong_issuer
    let quote = sample_quote(&wrong_issuer, &counterparty_did);
    let env = sign_envelope(
        &real_signer,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        serde_json::to_value(&quote).unwrap(),
        Some(quote.terms.quote_expires_at_secs),
        1000,
    );

    let verdict = verify_quote(&env, 1000);
    assert!(!verdict.verified);
    assert_eq!(
        verdict.reason.as_deref(),
        Some("quote_id is not derivable from the signature's own issuer")
    );
}

#[test]
fn verify_agreement_receipt_refuses_a_half_whose_role_does_not_match_its_issuer() {
    let counterparty_key = Identity::generate().unwrap();
    let counterparty_did = substrate::derive_did_key(&counterparty_key.public_key());
    let quote_author_key = Identity::generate().unwrap();
    let quote_author_did = substrate::derive_did_key(&quote_author_key.public_key());

    // A receipt saying role is Counterparty, but signed by quote_author_key!
    let payload = AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        counterparty_did: counterparty_did.clone(),
        quote_author_did: quote_author_did.clone(),
        role: DealRole::Counterparty,
        terms: sample_terms(),
    };
    let author_signs_counterparty_half = sign_envelope(
        &quote_author_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &payload.quote_record_id,
        serde_json::to_value(&payload).unwrap(),
        None,
        1000,
    );
    let verdict = verify_agreement_receipt(&author_signs_counterparty_half, 1000);
    assert!(!verdict.verified);
    assert_eq!(
        verdict.reason.as_deref(),
        Some("counterparty receipt issuer does not match counterparty_did")
    );

    // Subject mismatch
    let env_subject_mismatch = sign_envelope(
        &counterparty_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        "rec_wrong_subject",
        serde_json::to_value(&payload).unwrap(),
        None,
        1000,
    );
    let verdict_subj = verify_agreement_receipt(&env_subject_mismatch, 1000);
    assert!(!verdict_subj.verified);
    assert_eq!(
        verdict_subj.reason.as_deref(),
        Some("envelope subject does not match quote_record_id")
    );

    // Valid counterparty half
    let env_valid = sign_envelope(
        &counterparty_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &payload.quote_record_id,
        serde_json::to_value(&payload).unwrap(),
        None,
        1000,
    );
    let verdict_ok = verify_agreement_receipt(&env_valid, 1000);
    assert!(verdict_ok.verified);

    // A receipt saying role is QuoteAuthor, but signed by counterparty_key!
    let payload_quote_author = AgreementReceiptPayload { role: DealRole::QuoteAuthor, ..payload };
    let counterparty_signs_author_half = sign_envelope(
        &counterparty_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &payload_quote_author.quote_record_id,
        serde_json::to_value(&payload_quote_author).unwrap(),
        None,
        1000,
    );
    let verdict_prov_bad = verify_agreement_receipt(&counterparty_signs_author_half, 1000);
    assert!(!verdict_prov_bad.verified);
    assert_eq!(
        verdict_prov_bad.reason.as_deref(),
        Some("quote author receipt issuer does not match quote_author_did")
    );

    // Valid quote author half signed by quote_author_key
    let env_quote_author_valid = sign_envelope(
        &quote_author_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &payload_quote_author.quote_record_id,
        serde_json::to_value(&payload_quote_author).unwrap(),
        None,
        1000,
    );
    let verdict_prov_ok = verify_agreement_receipt(&env_quote_author_valid, 1000);
    assert!(verdict_prov_ok.verified);
}

#[test]
fn expired_quote_envelope_verifies_with_expired_flag() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let counterparty_key = Identity::generate().unwrap();
    let counterparty_did = substrate::derive_did_key(&counterparty_key.public_key());

    let mut quote = sample_quote(&issuer, &counterparty_did);
    let quote_expiry = 1500;
    quote.terms.quote_expires_at_secs = quote_expiry;
    let env = sign_envelope(
        &key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        serde_json::to_value(&quote).unwrap(),
        Some(quote_expiry),
        1000,
    );

    // Before expiry: verified = true, expired = false
    let v_before = verify_quote(&env, 1400);
    assert!(v_before.verified);
    assert!(!v_before.expired);
    assert!(v_before.payload.is_some());

    // Exactly at or after expiry: verified = true, expired = true
    let v_after = verify_quote(&env, 2000);
    assert!(v_after.verified);
    assert!(v_after.expired);
    assert!(v_after.payload.is_some());
    assert_eq!(v_after.payload.unwrap().quote_id, quote.quote_id);
}

#[test]
fn verify_request_happy_and_mismatch() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let req = sample_request(&issuer);

    let env = sign_envelope(
        &key,
        RECORD_REQUEST,
        REQUEST_VERSION,
        &req.request_id,
        serde_json::to_value(&req).unwrap(),
        None,
        1000,
    );

    let v = verify_request(&env, 1000);
    assert!(v.verified);
    assert!(!v.expired);
    assert_eq!(v.payload.as_ref().unwrap().request_id, req.request_id);

    // Sign with mismatching issuer
    let wrong_key = Identity::generate().unwrap();
    let env_mismatch = sign_envelope(
        &wrong_key,
        RECORD_REQUEST,
        REQUEST_VERSION,
        &req.request_id,
        serde_json::to_value(&req).unwrap(),
        None,
        1000,
    );
    let v_bad = verify_request(&env_mismatch, 1000);
    assert!(!v_bad.verified);
    assert_eq!(
        v_bad.reason.as_deref(),
        Some("request_id is not derivable from the signature's own issuer")
    );
}

#[test]
fn verify_quote_refuses_envelope_whose_expiry_mismatches_terms() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let counterparty_key = Identity::generate().unwrap();
    let counterparty_did = substrate::derive_did_key(&counterparty_key.public_key());

    let quote = sample_quote(&issuer, &counterparty_did);
    // terms say quote_expires_at_secs = 15000, but envelope says 5000
    let env = sign_envelope(
        &key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        serde_json::to_value(&quote).unwrap(),
        Some(5000),
        1000,
    );
    let verdict = verify_quote(&env, 1000);
    assert!(!verdict.verified);
    assert_eq!(
        verdict.reason.as_deref(),
        Some("envelope expires_at_secs does not match quote_expires_at_secs in terms")
    );

    // Envelope has no expiry at all
    let env_no_exp = sign_envelope(
        &key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        serde_json::to_value(&quote).unwrap(),
        None,
        1000,
    );
    let verdict_no_exp = verify_quote(&env_no_exp, 1000);
    assert!(!verdict_no_exp.verified);
    assert_eq!(
        verdict_no_exp.reason.as_deref(),
        Some("envelope expires_at_secs does not match quote_expires_at_secs in terms")
    );
}

#[test]
fn verify_quote_refuses_same_counterparty_and_quote_author_did() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());

    // counterparty_did is the quote author issuer itself
    let quote = sample_quote(&issuer, &issuer);
    let env = sign_envelope(
        &key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        serde_json::to_value(&quote).unwrap(),
        Some(quote.terms.quote_expires_at_secs),
        1000,
    );
    let verdict = verify_quote(&env, 1000);
    assert!(!verdict.verified);
    assert_eq!(verdict.reason.as_deref(), Some("counterparty_did cannot be the quote issuer"));
}

#[test]
fn verifiers_refuse_wrong_record_type() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let req = sample_request(&issuer);
    let env = sign_envelope(
        &key,
        RECORD_REQUEST,
        REQUEST_VERSION,
        &req.request_id,
        serde_json::to_value(&req).unwrap(),
        None,
        1000,
    );

    let quote_verdict = verify_quote(&env, 1000);
    assert!(!quote_verdict.verified);
    assert_eq!(quote_verdict.reason.as_deref(), Some("not a quote record this build understands"));

    let receipt_verdict = verify_agreement_receipt(&env, 1000);
    assert!(!receipt_verdict.verified);
    assert_eq!(
        receipt_verdict.reason.as_deref(),
        Some("not an agreement-receipt record this build understands")
    );
}

#[test]
fn verify_request_refuses_expiry() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let req = sample_request(&issuer);
    let env = sign_envelope(
        &key,
        RECORD_REQUEST,
        REQUEST_VERSION,
        &req.request_id,
        serde_json::to_value(&req).unwrap(),
        Some(20000),
        1000,
    );
    let verdict = verify_request(&env, 1000);
    assert!(!verdict.verified);
    assert_eq!(verdict.reason.as_deref(), Some("a request may not declare an expiry"));
}

#[test]
fn verify_agreement_receipt_refuses_expiry() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let receipt = AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        counterparty_did: issuer.clone(),
        quote_author_did: "did:key:quoteauthor123".to_string(),
        role: DealRole::Counterparty,
        terms: sample_terms(),
    };
    let env = sign_envelope(
        &key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &receipt.quote_record_id,
        serde_json::to_value(&receipt).unwrap(),
        Some(20000),
        1000,
    );
    let verdict = verify_agreement_receipt(&env, 1000);
    assert!(!verdict.verified);
    assert_eq!(verdict.reason.as_deref(), Some("an agreement receipt may not declare an expiry"));
}

#[test]
fn verify_quote_refuses_lifetime_outside_bounds() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let mut quote = sample_quote(&issuer, "did:key:counterparty123");

    // Lifetime too short (< MIN_QUOTE_LIFETIME_SECS = 300)
    quote.terms.quote_expires_at_secs = 1100; // 1100 - 1000 = 100s
    let env_short = sign_envelope(
        &key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        serde_json::to_value(&quote).unwrap(),
        Some(quote.terms.quote_expires_at_secs),
        1000,
    );
    let v_short = verify_quote(&env_short, 1000);
    assert!(!v_short.verified);
    assert_eq!(v_short.reason.as_deref(), Some("quote lifetime outside permitted bounds"));

    // Lifetime too long (> MAX_QUOTE_LIFETIME_SECS = 90 * 86400 = 7_776_000)
    quote.terms.quote_expires_at_secs = 1000 + 8_000_000;
    let env_long = sign_envelope(
        &key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote.quote_id,
        serde_json::to_value(&quote).unwrap(),
        Some(quote.terms.quote_expires_at_secs),
        1000,
    );
    let v_long = verify_quote(&env_long, 1000);
    assert!(!v_long.verified);
    assert_eq!(v_long.reason.as_deref(), Some("quote lifetime outside permitted bounds"));
}

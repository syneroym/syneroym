use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;

fn sample_terms() -> AgreedTerms {
    AgreedTerms {
        scope: "Install garden fence".to_string(),
        currency: "EUR".to_string(),
        amount_minor: 50000,
        tax_minor: 5000,
        fees_minor: 2000,
        payment_methods: vec!["card".to_string(), "cash".to_string()],
        payee: "Garden Services Ltd".to_string(),
        payment_timing: PaymentTiming::AfterWork,
        schedule: Some(TimeWindow { earliest_secs: 10000, latest_secs: 20000 }),
        location: QuoteLocation {
            where_: ServiceLocation::AtCustomer,
            area: None,
            address: Some("123 Flower St".to_string()),
        },
        cancellation_terms: "Cancel 24h prior for full refund".to_string(),
        refund_terms: "Full refund if unsatisfactory".to_string(),
        dispute_path: "Contact disputes@example.com".to_string(),
        quote_expires_at_secs: 15000,
    }
}

fn sample_request(issuer: &str) -> RequestPayload {
    let conversation = "conv_123456";
    let sequence = 1;
    let request_id = derive_request_id(conversation, issuer, sequence).unwrap();
    RequestPayload {
        request_id,
        conversation: conversation.to_string(),
        sequence,
        listing_id: Some("lst_abc123".to_string()),
        categories: vec!["gardening".to_string()],
        description: "Need fence installed".to_string(),
        area: None,
        window: Some(TimeWindow { earliest_secs: 10000, latest_secs: 20000 }),
        data_use_notice: DEFAULT_DATA_USE_NOTICE.to_string(),
    }
}

fn sample_quote(issuer: &str, consumer_did: &str) -> QuotePayload {
    let conversation = "conv_123456";
    let sequence = 1;
    let quote_id = derive_quote_id(conversation, issuer, sequence).unwrap();
    QuotePayload {
        quote_id,
        conversation: conversation.to_string(),
        sequence,
        request_record_id: "rec_req123".to_string(),
        listing_id: Some("lst_abc123".to_string()),
        consumer_did: consumer_did.to_string(),
        terms: sample_terms(),
    }
}

fn sign_envelope(
    key: &Identity,
    record_type: &str,
    version: u32,
    subject: &str,
    payload: serde_json::Value,
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
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, now_secs).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    env.to_json().unwrap()
}

#[test]
fn time_window_validate() {
    let valid = TimeWindow { earliest_secs: 100, latest_secs: 200 };
    assert!(valid.validate().is_ok());
    let equal = TimeWindow { earliest_secs: 100, latest_secs: 100 };
    assert!(equal.validate().is_ok());
    let invalid = TimeWindow { earliest_secs: 200, latest_secs: 100 };
    assert!(matches!(invalid.validate(), Err(TransactionError::InvalidTimeWindow { .. })));
}

#[test]
fn quote_location_validate() {
    let at_cust = QuoteLocation {
        where_: ServiceLocation::AtCustomer,
        area: None,
        address: Some("123 Main St".to_string()),
    };
    assert!(at_cust.validate().is_ok());

    let remote_with_addr = QuoteLocation {
        where_: ServiceLocation::Remote,
        area: None,
        address: Some("123 Main St".to_string()),
    };
    assert_eq!(remote_with_addr.validate(), Err(TransactionError::AddressNotApplicable));

    let long_addr = QuoteLocation {
        where_: ServiceLocation::AtCustomer,
        area: None,
        address: Some("a".repeat(MAX_ADDRESS_LEN + 1)),
    };
    assert_eq!(long_addr.validate(), Err(TransactionError::AddressTooLong));
}

#[test]
fn agreed_terms_validate_rules() {
    let mut t = sample_terms();
    assert!(t.validate().is_ok());

    // Scope empty / too long
    t.scope = "   ".to_string();
    assert_eq!(t.validate(), Err(TransactionError::ScopeEmpty));
    t.scope = "a".repeat(MAX_SCOPE_LEN + 1);
    assert_eq!(t.validate(), Err(TransactionError::ScopeTooLong));
    t.scope = "Valid scope".to_string();

    // Currency unknown
    t.currency = "XYZ".to_string();
    assert!(matches!(t.validate(), Err(TransactionError::CurrencyUnknown(_))));
    t.currency = "EUR".to_string();

    // Negative amounts
    t.amount_minor = -1;
    assert!(matches!(t.validate(), Err(TransactionError::NegativeAmount(-1))));
    t.amount_minor = 50000;

    t.tax_minor = -1;
    assert!(matches!(t.validate(), Err(TransactionError::NegativeTax(-1))));
    t.tax_minor = 5000;

    t.fees_minor = -1;
    assert!(matches!(t.validate(), Err(TransactionError::NegativeFees(-1))));
    t.fees_minor = 2000;

    // Breakdown exceeds total (including checked_add overflow)
    t.tax_minor = 40000;
    t.fees_minor = 20000; // 40000 + 20000 = 60000 > 50000
    assert!(matches!(t.validate(), Err(TransactionError::BreakdownExceedsTotal { .. })));
    t.tax_minor = i64::MAX;
    t.fees_minor = 1;
    assert!(matches!(t.validate(), Err(TransactionError::BreakdownExceedsTotal { .. })));
    t.tax_minor = 5000;
    t.fees_minor = 2000;

    // Payee
    t.payee = "".to_string();
    assert_eq!(t.validate(), Err(TransactionError::PayeeEmpty));
    t.payee = "p".repeat(MAX_PAYEE_LEN + 1);
    assert_eq!(t.validate(), Err(TransactionError::PayeeTooLong));
    t.payee = "Valid Payee".to_string();

    // Payment methods
    t.payment_methods = (0..MAX_PAYMENT_METHODS + 1).map(|i| format!("m{i}")).collect();
    assert_eq!(t.validate(), Err(TransactionError::TooManyPaymentMethods));
    t.payment_methods = vec!["x".repeat(MAX_PAYMENT_METHOD_LEN + 1)];
    assert_eq!(t.validate(), Err(TransactionError::PaymentMethodTooLong));
    t.payment_methods = vec!["card".to_string()];

    // Cancellation terms
    t.cancellation_terms = "".to_string();
    assert_eq!(t.validate(), Err(TransactionError::CancellationTermsEmpty));
    t.cancellation_terms = "x".repeat(MAX_TERMS_TEXT_LEN + 1);
    assert_eq!(t.validate(), Err(TransactionError::CancellationTermsTooLong));
    t.cancellation_terms = "Valid cancel".to_string();

    // Refund terms
    t.refund_terms = "".to_string();
    assert_eq!(t.validate(), Err(TransactionError::RefundTermsEmpty));
    t.refund_terms = "x".repeat(MAX_TERMS_TEXT_LEN + 1);
    assert_eq!(t.validate(), Err(TransactionError::RefundTermsTooLong));
    t.refund_terms = "Valid refund".to_string();

    // Dispute path
    t.dispute_path = "".to_string();
    assert_eq!(t.validate(), Err(TransactionError::DisputePathEmpty));
    t.dispute_path = "x".repeat(MAX_TERMS_TEXT_LEN + 1);
    assert_eq!(t.validate(), Err(TransactionError::DisputePathTooLong));
    t.dispute_path = "Valid dispute".to_string();

    // Quote expiry zero
    t.quote_expires_at_secs = 0;
    assert_eq!(t.validate(), Err(TransactionError::QuoteExpiryZero));
    t.quote_expires_at_secs = 1000;
    assert!(t.validate().is_ok());
}

#[test]
fn request_payload_validate_rules() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let mut r = sample_request(&issuer);
    assert!(r.validate().is_ok());

    r.request_id = "".to_string();
    assert_eq!(r.validate(), Err(TransactionError::RequestIdEmpty));
    r.request_id = "req_123".to_string();

    r.conversation = "".to_string();
    assert_eq!(r.validate(), Err(TransactionError::ConversationEmpty));
    r.conversation = "conv_123".to_string();

    r.sequence = 0;
    assert_eq!(r.validate(), Err(TransactionError::InvalidSequence(0)));
    r.sequence = 1;

    r.description = "   ".to_string();
    assert_eq!(r.validate(), Err(TransactionError::DescriptionEmpty));
    r.description = "d".repeat(MAX_DESCRIPTION_LEN + 1);
    assert_eq!(r.validate(), Err(TransactionError::DescriptionTooLong));
    r.description = "Valid description".to_string();

    r.categories = (0..MAX_CATEGORIES + 1).map(|i| format!("c{i}")).collect();
    assert_eq!(r.validate(), Err(TransactionError::TooManyCategories));
    r.categories = vec!["UPPERCASE".to_string()];
    assert!(matches!(r.validate(), Err(TransactionError::InvalidCategory(_))));
    r.categories = vec!["valid-cat".to_string()];

    r.data_use_notice = "".to_string();
    assert_eq!(r.validate(), Err(TransactionError::DataUseNoticeEmpty));
    r.data_use_notice = "n".repeat(MAX_NOTICE_LEN + 1);
    assert_eq!(r.validate(), Err(TransactionError::DataUseNoticeTooLong));
    r.data_use_notice = DEFAULT_DATA_USE_NOTICE.to_string();
    assert!(r.validate().is_ok());
}

#[test]
fn quote_payload_validate_rules() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let consumer_key = Identity::generate().unwrap();
    let consumer_did = substrate::derive_did_key(&consumer_key.public_key());

    let mut q = sample_quote(&issuer, &consumer_did);
    assert!(q.validate().is_ok());

    q.quote_id = "".to_string();
    assert_eq!(q.validate(), Err(TransactionError::QuoteIdEmpty));
    q.quote_id = "quo_123".to_string();

    q.conversation = "".to_string();
    assert_eq!(q.validate(), Err(TransactionError::ConversationEmpty));
    q.conversation = "conv_123".to_string();

    q.sequence = 0;
    assert_eq!(q.validate(), Err(TransactionError::InvalidSequence(0)));
    q.sequence = 1;

    q.request_record_id = "bad_prefix".to_string();
    assert!(matches!(q.validate(), Err(TransactionError::InvalidRecordId(_))));
    q.request_record_id = "rec_req123".to_string();

    q.consumer_did = "not-a-did".to_string();
    assert!(matches!(q.validate(), Err(TransactionError::InvalidDid(_))));
    q.consumer_did = consumer_did;
    assert!(q.validate().is_ok());
}

#[test]
fn agreement_receipt_payload_validate_rules() {
    let consumer_key = Identity::generate().unwrap();
    let consumer_did = substrate::derive_did_key(&consumer_key.public_key());
    let provider_key = Identity::generate().unwrap();
    let provider_did = substrate::derive_did_key(&provider_key.public_key());

    let mut a = AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        consumer_did: consumer_did.clone(),
        provider_did: provider_did.clone(),
        role: Role::Consumer,
        terms: sample_terms(),
    };
    assert!(a.validate().is_ok());

    a.quote_record_id = "bad_prefix".to_string();
    assert!(matches!(a.validate(), Err(TransactionError::InvalidRecordId(_))));
    a.quote_record_id = "rec_quote123".to_string();

    a.consumer_did = "invalid".to_string();
    assert!(matches!(a.validate(), Err(TransactionError::InvalidDid(_))));
    a.consumer_did = consumer_did.clone();

    a.provider_did = "invalid".to_string();
    assert!(matches!(a.validate(), Err(TransactionError::InvalidDid(_))));
    a.provider_did = provider_did.clone();

    a.consumer_did = provider_did;
    assert_eq!(a.validate(), Err(TransactionError::SamePartyDid));
    a.consumer_did = consumer_did;
    assert!(a.validate().is_ok());
}

#[test]
fn derive_request_id_and_quote_id_are_stable_and_differ_on_any_input_change() {
    let id1 = derive_request_id("c1", "did:key:1", 1).unwrap();
    let id2 = derive_request_id("c1", "did:key:1", 1).unwrap();
    assert_eq!(id1, id2);
    assert!(id1.starts_with(REQUEST_ID_PREFIX));

    // Changing conversation
    assert_ne!(id1, derive_request_id("c2", "did:key:1", 1).unwrap());
    // Changing issuer
    assert_ne!(id1, derive_request_id("c1", "did:key:2", 1).unwrap());
    // Changing sequence
    assert_ne!(id1, derive_request_id("c1", "did:key:1", 2).unwrap());

    // Quote id
    let q1 = derive_quote_id("c1", "did:key:1", 1).unwrap();
    let q2 = derive_quote_id("c1", "did:key:1", 1).unwrap();
    assert_eq!(q1, q2);
    assert!(q1.starts_with(QUOTE_ID_PREFIX));
    assert_ne!(id1, q1);
    assert_ne!(q1, derive_quote_id("c2", "did:key:1", 1).unwrap());
}

#[test]
fn verify_quote_refuses_a_payload_whose_quote_id_was_derived_from_a_different_issuer() {
    let real_signer = Identity::generate().unwrap();
    let wrong_key = Identity::generate().unwrap();
    let wrong_issuer = substrate::derive_did_key(&wrong_key.public_key());

    let consumer_key = Identity::generate().unwrap();
    let consumer_did = substrate::derive_did_key(&consumer_key.public_key());

    // Derive quote_id using wrong_issuer
    let quote = sample_quote(&wrong_issuer, &consumer_did);
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
    let consumer_key = Identity::generate().unwrap();
    let consumer_did = substrate::derive_did_key(&consumer_key.public_key());
    let provider_key = Identity::generate().unwrap();
    let provider_did = substrate::derive_did_key(&provider_key.public_key());

    // A receipt saying role is Consumer, but signed by provider_key!
    let payload = AgreementReceiptPayload {
        quote_record_id: "rec_quote123".to_string(),
        consumer_did: consumer_did.clone(),
        provider_did: provider_did.clone(),
        role: Role::Consumer,
        terms: sample_terms(),
    };
    let env_provider_signing_consumer_half = sign_envelope(
        &provider_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &payload.quote_record_id,
        serde_json::to_value(&payload).unwrap(),
        None,
        1000,
    );
    let verdict = verify_agreement_receipt(&env_provider_signing_consumer_half, 1000);
    assert!(!verdict.verified);
    assert_eq!(
        verdict.reason.as_deref(),
        Some("consumer receipt issuer does not match consumer_did")
    );

    // Subject mismatch
    let env_subject_mismatch = sign_envelope(
        &consumer_key,
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

    // Valid consumer half
    let env_valid = sign_envelope(
        &consumer_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &payload.quote_record_id,
        serde_json::to_value(&payload).unwrap(),
        None,
        1000,
    );
    let verdict_ok = verify_agreement_receipt(&env_valid, 1000);
    assert!(verdict_ok.verified);

    // A receipt saying role is Provider, but signed by consumer_key!
    let payload_provider = AgreementReceiptPayload { role: Role::Provider, ..payload };
    let env_consumer_signing_provider_half = sign_envelope(
        &consumer_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &payload_provider.quote_record_id,
        serde_json::to_value(&payload_provider).unwrap(),
        None,
        1000,
    );
    let verdict_prov_bad = verify_agreement_receipt(&env_consumer_signing_provider_half, 1000);
    assert!(!verdict_prov_bad.verified);
    assert_eq!(
        verdict_prov_bad.reason.as_deref(),
        Some("provider receipt issuer does not match provider_did")
    );

    // Valid provider half signed by provider_key
    let env_provider_valid = sign_envelope(
        &provider_key,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &payload_provider.quote_record_id,
        serde_json::to_value(&payload_provider).unwrap(),
        None,
        1000,
    );
    let verdict_prov_ok = verify_agreement_receipt(&env_provider_valid, 1000);
    assert!(verdict_prov_ok.verified);
}

#[test]
fn halves_agree_table_test() {
    let consumer_did = "did:key:zConsumer".to_string();
    let provider_did = "did:key:zProvider".to_string();
    let terms = sample_terms();

    let base_consumer = AgreementReceiptPayload {
        quote_record_id: "rec_quo123".to_string(),
        consumer_did: consumer_did.clone(),
        provider_did: provider_did.clone(),
        role: Role::Consumer,
        terms: terms.clone(),
    };
    let base_provider = AgreementReceiptPayload {
        quote_record_id: "rec_quo123".to_string(),
        consumer_did: consumer_did.clone(),
        provider_did: provider_did.clone(),
        role: Role::Provider,
        terms: terms.clone(),
    };

    // Identical except role -> true
    assert!(halves_agree(&base_consumer, &base_provider));
    assert!(halves_agree(&base_provider, &base_consumer));

    // Same role -> false
    assert!(!halves_agree(&base_consumer, &base_consumer));
    assert!(!halves_agree(&base_provider, &base_provider));

    // Differ in quote_record_id -> false
    let mut diff_quote = base_provider.clone();
    diff_quote.quote_record_id = "rec_diff".to_string();
    assert!(!halves_agree(&base_consumer, &diff_quote));

    // Differ in consumer_did -> false
    let mut diff_cdid = base_provider.clone();
    diff_cdid.consumer_did = "did:key:zOther".to_string();
    assert!(!halves_agree(&base_consumer, &diff_cdid));

    // Differ in provider_did -> false
    let mut diff_pdid = base_provider.clone();
    diff_pdid.provider_did = "did:key:zOther".to_string();
    assert!(!halves_agree(&base_consumer, &diff_pdid));

    // Differ in terms (e.g. amount) -> false
    let mut diff_terms = base_provider.clone();
    diff_terms.terms.amount_minor = 99999;
    assert!(!halves_agree(&base_consumer, &diff_terms));
}

#[test]
fn expired_quote_envelope_verifies_with_expired_flag() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let consumer_key = Identity::generate().unwrap();
    let consumer_did = substrate::derive_did_key(&consumer_key.public_key());

    let mut quote = sample_quote(&issuer, &consumer_did);
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
fn agreed_terms_validate_refuses_xyz_as_currency() {
    let mut t = sample_terms();
    t.currency = "XYZ".to_string();
    assert!(matches!(t.validate(), Err(TransactionError::CurrencyUnknown(c)) if c == "XYZ"));
}

#[test]
fn pair_state_transitions() {
    let half_c = ReceiptHalf {
        envelope: "{}".to_string(),
        record_id: "rec_c".to_string(),
        issuer: "did:key:c".to_string(),
        issued_at_secs: 1000,
    };
    let half_p = ReceiptHalf {
        envelope: "{}".to_string(),
        record_id: "rec_p".to_string(),
        issuer: "did:key:p".to_string(),
        issued_at_secs: 1000,
    };

    assert_eq!(pair_state(None, None), PairState::None);
    assert_eq!(pair_state(Some(&half_c), None), PairState::Half { role: Role::Consumer });
    assert_eq!(pair_state(None, Some(&half_p)), PairState::Half { role: Role::Provider });
    assert_eq!(pair_state(Some(&half_c), Some(&half_p)), PairState::Complete);
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
    let consumer_key = Identity::generate().unwrap();
    let consumer_did = substrate::derive_did_key(&consumer_key.public_key());

    let quote = sample_quote(&issuer, &consumer_did);
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
fn verify_quote_refuses_same_consumer_and_provider_did() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());

    // consumer_did is the provider issuer itself
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
    assert_eq!(verdict.reason.as_deref(), Some("consumer_did cannot be the quote issuer"));
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
        consumer_did: issuer.clone(),
        provider_did: "did:key:provider123".to_string(),
        role: Role::Consumer,
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
    let mut quote = sample_quote(&issuer, "did:key:consumer123");

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

#[test]
fn the_ui_notices_match_this_crate() {
    use std::{fs, path::PathBuf};

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let messages_path = manifest_dir.join("../roym_web/ui/src/screens/messages.ts");
    assert!(messages_path.exists(), "missing ../roym_web/ui/src/screens/messages.ts");

    let content = fs::read_to_string(&messages_path)
        .expect("Failed to read ../roym_web/ui/src/screens/messages.ts");

    fn parse_ts_const_string(content: &str, const_name: &str) -> String {
        let start_idx = content
            .find(const_name)
            .and_then(|idx| content[idx..].find('='))
            .map(|offset| content.find(const_name).unwrap() + offset)
            .unwrap_or_else(|| panic!("{const_name} assignment not found"));
        let slice = &content[start_idx + 1..];
        let mut in_quote = false;
        let mut quote_char = ' ';
        let mut end_idx = slice.len();
        for (i, c) in slice.char_indices() {
            if in_quote {
                if c == quote_char {
                    in_quote = false;
                }
            } else if c == '"' || c == '\'' {
                in_quote = true;
                quote_char = c;
            } else if c == ';' {
                end_idx = i;
                break;
            }
        }
        let expr = &slice[..end_idx];
        let mut result = String::new();
        for part in expr.split('+') {
            let trimmed = part.trim();
            if (trimmed.starts_with('"') && trimmed.ends_with('"'))
                || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
            {
                result.push_str(&trimmed[1..trimmed.len() - 1]);
            }
        }
        result
    }

    let ui_data_use = parse_ts_const_string(&content, "DEFAULT_DATA_USE_NOTICE");
    assert_eq!(
        ui_data_use, DEFAULT_DATA_USE_NOTICE,
        "UI DEFAULT_DATA_USE_NOTICE must match Rust DEFAULT_DATA_USE_NOTICE"
    );

    let ui_address_disc = parse_ts_const_string(&content, "ADDRESS_DISCLOSURE_NOTICE");
    assert_eq!(
        ui_address_disc, ADDRESS_DISCLOSURE_NOTICE,
        "UI ADDRESS_DISCLOSURE_NOTICE must match Rust ADDRESS_DISCLOSURE_NOTICE"
    );
}

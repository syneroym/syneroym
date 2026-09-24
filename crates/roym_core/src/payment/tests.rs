use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;
use crate::{
    listing::ServiceLocation,
    transaction::{PaymentTiming, QuoteLocation},
};

fn sample_terms() -> AgreedTerms {
    AgreedTerms {
        scope: "All work".to_string(),
        currency: "USD".to_string(),
        amount_minor: 10_000,
        tax_minor: 0,
        fees_minor: 0,
        payment_methods: vec!["card".to_string(), "cash".to_string()],
        payee: "Test Payee".to_string(),
        payment_timing: PaymentTiming::BeforeWork,
        schedule: None,
        location: QuoteLocation { where_: ServiceLocation::Remote, area: None, address: None },
        cancellation_terms: "Standard cancellation".to_string(),
        refund_terms: "Standard refund".to_string(),
        dispute_path: "Contact provider".to_string(),
        quote_expires_at_secs: 2000,
    }
}

fn sample_request() -> PaymentRequestPayload {
    PaymentRequestPayload {
        agreement: "rec_quote123".to_string(),
        conversation: "conv_123".to_string(),
        consumer_did: "did:key:zConsumer".to_string(),
        provider_did: "did:key:zProvider".to_string(),
        currency: "USD".to_string(),
        amount_minor: 10_000,
        note: Some("Initial payment".to_string()),
    }
}

fn sample_ack(role: Role) -> PaymentAcknowledgementPayload {
    PaymentAcknowledgementPayload {
        agreement: "rec_quote123".to_string(),
        conversation: "conv_123".to_string(),
        consumer_did: "did:key:zConsumer".to_string(),
        provider_did: "did:key:zProvider".to_string(),
        role,
        currency: "USD".to_string(),
        amount_minor: 10_000,
        observed_at_secs: 1000,
        method: Some("card".to_string()),
        reference: Some("ref_tx_999".to_string()),
    }
}

#[test]
fn payment_request_validation() {
    let mut req = sample_request();
    assert!(req.validate().is_ok());

    req.agreement = "bad_prefix".to_string();
    assert!(matches!(req.validate(), Err(PaymentError::InvalidAgreement(_))));
    req.agreement = "rec_quote123".to_string();

    req.consumer_did = "bad_did".to_string();
    assert!(matches!(req.validate(), Err(PaymentError::InvalidConsumerDid(_))));
    req.consumer_did = "did:key:zConsumer".to_string();

    req.provider_did = req.consumer_did.clone();
    assert!(matches!(req.validate(), Err(PaymentError::SamePartyDid)));
    req.provider_did = "did:key:zProvider".to_string();

    req.currency = "INVALID".to_string();
    assert!(matches!(req.validate(), Err(PaymentError::UnknownCurrency(_))));
    req.currency = "USD".to_string();

    req.amount_minor = -1;
    assert!(matches!(req.validate(), Err(PaymentError::NegativeAmount(-1))));
    req.amount_minor = 10_000;

    let long_note = "n".repeat(MAX_NOTE_LEN + 1);
    req.note = Some(long_note);
    assert!(matches!(req.validate(), Err(PaymentError::NoteTooLong)));
}

#[test]
fn payment_acknowledgement_validation() {
    let mut ack = sample_ack(Role::Consumer);
    assert!(ack.validate().is_ok());

    ack.observed_at_secs = 0;
    assert!(matches!(ack.validate(), Err(PaymentError::ZeroObservedTime)));
    ack.observed_at_secs = 1000;

    ack.method = Some(String::new());
    assert!(matches!(ack.validate(), Err(PaymentError::InvalidPaymentMethod)));
    ack.method = Some("card".to_string());

    let long_ref = "r".repeat(MAX_REFERENCE_LEN + 1);
    ack.reference = Some(long_ref);
    assert!(matches!(ack.validate(), Err(PaymentError::ReferenceTooLong)));
}

#[test]
fn test_matches_terms() {
    let terms = sample_terms();
    assert!(matches_terms("USD", 10_000, Some("card"), &terms));
    assert!(matches_terms("USD", 10_000, None, &terms));
    assert!(!matches_terms("EUR", 10_000, Some("card"), &terms));
    assert!(!matches_terms("USD", 5_000, Some("card"), &terms));
    assert!(!matches_terms("USD", 10_000, Some("crypto"), &terms));
}

#[test]
fn test_acknowledgements_agree() {
    let consumer_ack = sample_ack(Role::Consumer);
    let provider_ack = sample_ack(Role::Provider);
    assert!(acknowledgements_agree(&consumer_ack, &provider_ack));

    let mut mismatched = provider_ack.clone();
    mismatched.amount_minor = 20_000;
    assert!(!acknowledgements_agree(&consumer_ack, &mismatched));
}

#[test]
fn test_is_valid_correction() {
    let old = sample_ack(Role::Consumer);
    let mut new = old.clone();
    new.reference = Some("corrected_ref".to_string());
    new.method = Some("cash".to_string());
    new.observed_at_secs = 1005;
    assert!(is_valid_correction(&old, &new));

    let mut bad_correction = old.clone();
    bad_correction.amount_minor = 50_000;
    assert!(!is_valid_correction(&old, &bad_correction));
}

#[test]
fn verify_payment_envelopes() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());

    // Payment request
    let mut req = sample_request();
    req.provider_did = issuer.clone();
    let draft = RecordDraft {
        version: PAYMENT_REQUEST_VERSION,
        record_type: RECORD_PAYMENT_REQUEST.to_string(),
        subject: req.agreement.clone(),
        payload: serde_json::to_value(&req).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer.clone(), None, 1000).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let env_json = env.to_json().unwrap();

    let verdict = verify_payment_request(&env_json, 1000);
    assert!(verdict.verified);
    assert_eq!(verdict.payload, Some(req));

    // Payment ack
    let mut ack = sample_ack(Role::Provider);
    ack.provider_did = issuer.clone();
    let ack_draft = RecordDraft {
        version: PAYMENT_ACKNOWLEDGEMENT_VERSION,
        record_type: RECORD_PAYMENT_ACKNOWLEDGEMENT.to_string(),
        subject: ack.agreement.clone(),
        payload: serde_json::to_value(&ack).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut ack_env, ack_bytes) =
        Envelope::unsigned(ack_draft, issuer.clone(), None, 1000).unwrap();
    let ack_sig = z32::encode(&key.sign(&ack_bytes).to_bytes());
    ack_env.attach_signature(ack_sig).unwrap();
    let ack_env_json = ack_env.to_json().unwrap();

    let ack_verdict = verify_payment_acknowledgement(&ack_env_json, 1000);
    assert!(ack_verdict.verified);
    assert_eq!(ack_verdict.payload, Some(ack));
}

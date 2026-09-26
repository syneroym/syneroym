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

fn sample_receipt(role: Role) -> FulfilmentReceiptPayload {
    FulfilmentReceiptPayload {
        agreement: "rec_quote123".to_string(),
        conversation: "conv_123".to_string(),
        consumer_did: "did:key:zConsumer".to_string(),
        provider_did: "did:key:zProvider".to_string(),
        role,
        terms: sample_terms(),
    }
}

#[test]
fn fulfilment_receipt_validation() {
    let mut receipt = sample_receipt(Role::Consumer);
    assert!(receipt.validate().is_ok());

    receipt.agreement = "bad_prefix".to_string();
    assert!(matches!(receipt.validate(), Err(FulfilmentError::InvalidAgreement(_))));
    receipt.agreement = "rec_quote123".to_string();

    receipt.consumer_did = "bad_did".to_string();
    assert!(matches!(receipt.validate(), Err(FulfilmentError::InvalidConsumerDid(_))));
    receipt.consumer_did = "did:key:zConsumer".to_string();

    receipt.provider_did = receipt.consumer_did.clone();
    assert!(matches!(receipt.validate(), Err(FulfilmentError::SamePartyDid)));
    receipt.provider_did = "did:key:zProvider".to_string();

    receipt.conversation = String::new();
    assert!(matches!(receipt.validate(), Err(FulfilmentError::ConversationEmpty)));
}

#[test]
fn test_fulfilment_halves_agree() {
    let consumer_half = sample_receipt(Role::Consumer);
    let provider_half = sample_receipt(Role::Provider);
    assert!(fulfilment_halves_agree(&consumer_half, &provider_half));

    let mut mismatched = provider_half.clone();
    mismatched.terms.amount_minor = 20_000;
    assert!(!fulfilment_halves_agree(&consumer_half, &mismatched));

    let same_role = consumer_half.clone();
    assert!(!fulfilment_halves_agree(&consumer_half, &same_role));
}

#[test]
fn verify_fulfilment_envelopes() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());

    let mut receipt = sample_receipt(Role::Provider);
    receipt.provider_did = issuer.clone();
    let draft = RecordDraft {
        version: FULFILMENT_RECEIPT_VERSION,
        record_type: RECORD_FULFILMENT_RECEIPT.to_string(),
        subject: receipt.agreement.clone(),
        payload: serde_json::to_value(&receipt).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer.clone(), None, 1000).unwrap();
    let sig = z32::encode(&key.sign(&bytes).to_bytes());
    env.attach_signature(sig).unwrap();
    let env_json = env.to_json().unwrap();

    let verdict = verify_fulfilment_receipt(&env_json, 1000);
    assert!(verdict.verified);
    assert_eq!(verdict.payload, Some(receipt));
}

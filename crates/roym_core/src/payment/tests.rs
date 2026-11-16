use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;
use crate::{
    deal::{Commitment, CommitmentKind, ResourceKind},
    lang::LangTag,
    transaction::DealRole,
};

fn sample_commitment() -> Commitment {
    Commitment {
        commitment_id: "c1".to_string(),
        giver: DealRole::Counterparty,
        resource_kind: ResourceKind::Currency,
        kind: CommitmentKind::Currency,
        extension: None,
        after: Vec::new(),
        description: LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "USD 100.00".to_string(),
            translations: Vec::new(),
        },
        declared_value_minor: 10_000,
        details: None,
        window: None,
        location: None,
        payee: Some("Test Payee".to_string()),
        payment_methods: Some(vec!["card".to_string(), "cash".to_string()]),
        tax_minor: Some(0),
        fees_minor: Some(0),
        eep: None,
    }
}

fn sample_request() -> PaymentRequestPayload {
    PaymentRequestPayload {
        agreement: "rec_quote123".to_string(),
        conversation: "conv_123".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: "did:key:zCounterparty".to_string(),
        quote_author_did: "did:key:zQuoteAuthor".to_string(),
        note: Some(LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Initial payment".to_string(),
            translations: Vec::new(),
        }),
    }
}

fn sample_ack(role: SettlementRole) -> PaymentAcknowledgementPayload {
    PaymentAcknowledgementPayload {
        agreement: "rec_quote123".to_string(),
        conversation: "conv_123".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: "did:key:zCounterparty".to_string(),
        quote_author_did: "did:key:zQuoteAuthor".to_string(),
        role,
        observed_at_secs: 1000,
        method: Some("card".to_string()),
        reference: Some("ref_tx_999".to_string()),
        settlement_note: None,
    }
}

#[test]
fn payment_request_validation() {
    let mut req = sample_request();
    assert!(req.validate().is_ok());

    req.agreement = "bad_prefix".to_string();
    assert!(matches!(req.validate(), Err(PaymentError::InvalidAgreement(_))));
    req.agreement = "rec_quote123".to_string();

    req.counterparty_did = "bad_did".to_string();
    assert!(matches!(req.validate(), Err(PaymentError::InvalidCounterpartyDid(_))));
    req.counterparty_did = "did:key:zCounterparty".to_string();

    req.quote_author_did = req.counterparty_did.clone();
    assert!(matches!(req.validate(), Err(PaymentError::SamePartyDid)));
    req.quote_author_did = "did:key:zQuoteAuthor".to_string();

    req.commitment_id = String::new();
    assert!(matches!(req.validate(), Err(PaymentError::CommitmentIdEmpty)));
    req.commitment_id = "c1".to_string();

    let long_note = "n".repeat(MAX_NOTE_LEN + 1);
    req.note = Some(LocalText {
        lang: LangTag::parse("en").unwrap(),
        text: long_note,
        translations: Vec::new(),
    });
    assert!(matches!(req.validate(), Err(PaymentError::Note(_))));
}

#[test]
fn payment_acknowledgement_validation() {
    let mut ack = sample_ack(SettlementRole::Giver);
    assert!(ack.validate().is_ok());

    ack.observed_at_secs = 0;
    assert!(matches!(ack.validate(), Err(PaymentError::ZeroObservedTime)));
    ack.observed_at_secs = 1000;

    ack.method = Some(String::new());
    assert!(matches!(ack.validate(), Err(PaymentError::MethodShape(_))));
    ack.method = Some("Cash".to_string());
    assert!(matches!(
        ack.validate(),
        Err(PaymentError::MethodShape(m)) if m == "Cash"
    ));
    ack.method = Some("card".to_string());

    let long_ref = "r".repeat(MAX_REFERENCE_LEN + 1);
    ack.reference = Some(long_ref);
    assert!(matches!(ack.validate(), Err(PaymentError::ReferenceTooLong)));
}

#[test]
fn test_commitment_accepts() {
    let commitment = sample_commitment();
    assert!(commitment_accepts(&commitment, "card"));
    assert!(commitment_accepts(&commitment, "cash"));
    assert!(!commitment_accepts(&commitment, "crypto"));
}

#[test]
fn test_acknowledgements_agree() {
    let giver_ack = sample_ack(SettlementRole::Giver);
    let receiver_ack = sample_ack(SettlementRole::Receiver);
    assert!(acknowledgements_agree(&giver_ack, &receiver_ack));

    let mut mismatched = receiver_ack.clone();
    mismatched.commitment_id = "c2".to_string();
    assert!(!acknowledgements_agree(&giver_ack, &mismatched));
}

#[test]
fn test_is_valid_correction() {
    let old = sample_ack(SettlementRole::Giver);
    let mut new = old.clone();
    new.reference = Some("corrected_ref".to_string());
    new.method = Some("cash".to_string());
    new.observed_at_secs = 1005;
    assert!(is_valid_correction(&old, &new));

    let mut bad_correction = old.clone();
    bad_correction.commitment_id = "c2".to_string();
    assert!(!is_valid_correction(&old, &bad_correction));
}

#[test]
fn verify_payment_envelopes() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());

    // Payment request
    let mut req = sample_request();
    req.quote_author_did = issuer.clone();
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
    let mut ack = sample_ack(SettlementRole::Receiver);
    ack.quote_author_did = issuer.clone();
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

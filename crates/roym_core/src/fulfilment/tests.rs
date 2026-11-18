use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;
use crate::deal::SettlementRole;

fn sample_receipt(role: SettlementRole) -> FulfilmentReceiptPayload {
    let plain = crate::testing::sd::PlainFulfilment {
        agreement: "rec_quote123".to_string(),
        conversation: "conv_123".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: "did:key:zCounterparty".to_string(),
        quote_author_did: "did:key:zQuoteAuthor".to_string(),
        role,
        settlement_note: None,
    };
    crate::testing::sd::conceal_fulfilment(plain, 12).0
}

#[test]
fn fulfilment_receipt_validation() {
    let mut receipt = sample_receipt(SettlementRole::Giver);
    assert!(receipt.validate().is_ok());

    receipt.agreement = "bad_prefix".to_string();
    assert!(matches!(receipt.validate(), Err(FulfilmentError::InvalidAgreement(_))));
    receipt.agreement = "rec_quote123".to_string();

    receipt.counterparty_did = "bad_did".to_string();
    assert!(matches!(receipt.validate(), Err(FulfilmentError::InvalidCounterpartyDid(_))));
    receipt.counterparty_did = "did:key:zCounterparty".to_string();

    receipt.quote_author_did = receipt.counterparty_did.clone();
    assert!(matches!(receipt.validate(), Err(FulfilmentError::SamePartyDid)));
    receipt.quote_author_did = "did:key:zQuoteAuthor".to_string();

    receipt.conversation = String::new();
    assert!(matches!(receipt.validate(), Err(FulfilmentError::ConversationEmpty)));
    receipt.conversation = "conv_123".to_string();

    receipt.commitment_id = String::new();
    assert!(matches!(receipt.validate(), Err(FulfilmentError::CommitmentIdEmpty)));
    receipt.commitment_id = "c1".to_string();

    receipt.sd = Vec::new();
    assert!(matches!(receipt.validate(), Err(FulfilmentError::SdShape(_))));
}

#[test]
fn test_fulfilment_halves_agree() {
    let counterparty_half = sample_receipt(SettlementRole::Receiver);
    let quote_author_half = sample_receipt(SettlementRole::Giver);
    assert!(fulfilment_halves_agree(&counterparty_half, &quote_author_half));

    let mut mismatched = quote_author_half.clone();
    mismatched.commitment_id = "c2".to_string();
    assert!(!fulfilment_halves_agree(&counterparty_half, &mismatched));

    let same_role = counterparty_half.clone();
    assert!(!fulfilment_halves_agree(&counterparty_half, &same_role));
}

#[test]
fn verify_fulfilment_envelopes() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());

    let mut receipt = sample_receipt(SettlementRole::Giver);
    receipt.quote_author_did = issuer.clone();
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

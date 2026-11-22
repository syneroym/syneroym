use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;

fn sample() -> CommitmentFailurePayload {
    CommitmentFailurePayload {
        agreement: "rec_quote_1".to_string(),
        conversation: "conv_1".to_string(),
        commitment_id: "c1".to_string(),
        reason: FailureReason::ReservationLost,
    }
}

fn sign(
    payload: &serde_json::Value,
    subject: &str,
    record_type: &str,
    expires_at_secs: Option<u64>,
) -> String {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let draft = RecordDraft {
        version: COMMITMENT_FAILURE_VERSION,
        record_type: record_type.to_string(),
        subject: subject.to_string(),
        payload: payload.clone(),
        expires_at_secs,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, 1000).unwrap();
    env.attach_signature(z32::encode(&key.sign(&bytes).to_bytes())).unwrap();
    env.to_json().unwrap()
}

fn sign_sample(subject: &str) -> String {
    sign(&serde_json::to_value(sample()).unwrap(), subject, RECORD_COMMITMENT_FAILURE, None)
}

#[test]
fn a_good_payload_validates() {
    assert!(sample().validate().is_ok());
}

#[test]
fn each_bad_field_is_refused_with_its_own_error() {
    let mut p = sample();
    p.agreement = "bad".to_string();
    assert!(matches!(p.validate(), Err(CommitmentFailureError::InvalidAgreement(_))));

    let mut p = sample();
    p.conversation = "  ".to_string();
    assert_eq!(p.validate(), Err(CommitmentFailureError::ConversationEmpty));

    let mut p = sample();
    p.commitment_id = String::new();
    assert_eq!(p.validate(), Err(CommitmentFailureError::CommitmentIdEmpty));
}

#[test]
fn the_reason_is_written_in_kebab_case() {
    let json = serde_json::to_value(sample()).unwrap();
    assert_eq!(json["reason"], "reservation-lost");
    assert_eq!(FailureReason::ReservationLost.as_str(), "reservation-lost");
}

#[test]
fn an_unknown_reason_and_an_unknown_field_do_not_parse() {
    let mut json = serde_json::to_value(sample()).unwrap();
    json["reason"] = "out-of-stock".into();
    assert!(serde_json::from_value::<CommitmentFailurePayload>(json).is_err());

    let mut json = serde_json::to_value(sample()).unwrap();
    json["note"] = "x".into();
    assert!(serde_json::from_value::<CommitmentFailurePayload>(json).is_err());
}

#[test]
fn a_good_envelope_verifies() {
    let verdict = verify_commitment_failure(&sign_sample("rec_quote_1"), 1000);
    assert!(verdict.verified, "{:?}", verdict.reason);
    let payload = verdict.payload.unwrap();
    assert_eq!(payload, sample());
    assert!(verdict.signer_did.is_some());
}

#[test]
fn the_subject_must_equal_the_agreement() {
    let verdict = verify_commitment_failure(&sign_sample("rec_other"), 1000);
    assert!(!verdict.verified);
    assert!(verdict.reason.unwrap().contains("subject does not match"));
}

#[test]
fn a_tampered_envelope_is_refused() {
    let tampered = sign_sample("rec_quote_1").replace("reservation-lost", "reservation-lust");
    assert!(!verify_commitment_failure(&tampered, 1000).verified);
}

#[test]
fn a_record_of_another_type_is_refused() {
    let payload = serde_json::to_value(sample()).unwrap();
    let envelope = sign(&payload, "rec_quote_1", "deal-decline", None);
    let verdict = verify_commitment_failure(&envelope, 1000);
    assert!(!verdict.verified);
    assert!(verdict.reason.unwrap().contains("not a commitment-failure record"));
}

#[test]
fn an_expiry_is_refused() {
    let payload = serde_json::to_value(sample()).unwrap();
    let envelope = sign(&payload, "rec_quote_1", RECORD_COMMITMENT_FAILURE, Some(2000));
    let verdict = verify_commitment_failure(&envelope, 1000);
    assert!(!verdict.verified);
    assert!(verdict.reason.unwrap().contains("may not declare an expiry"));
}

#[test]
fn a_payload_that_does_not_validate_is_refused() {
    let mut payload = sample();
    payload.commitment_id = String::new();
    let envelope = sign(
        &serde_json::to_value(payload).unwrap(),
        "rec_quote_1",
        RECORD_COMMITMENT_FAILURE,
        None,
    );
    assert!(!verify_commitment_failure(&envelope, 1000).verified);
}

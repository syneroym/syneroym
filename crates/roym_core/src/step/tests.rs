use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_signed_record::{Envelope, RecordDraft};

use super::*;
use crate::{
    deal::SettlementRole,
    disclosure::{STEP_SET, reveal_hidden_strict},
    testing::sd::{self, PlainStep},
};

fn plain(data: serde_json::Value) -> PlainStep {
    PlainStep {
        agreement: "rec_quote123".to_string(),
        conversation: "conv_123".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: "did:key:zCounterparty".to_string(),
        quote_author_did: "did:key:zQuoteAuthor".to_string(),
        role: SettlementRole::Giver,
        extension: ExtensionRef { id: "scheduling".to_string(), version: 1 },
        step_type: "scheduling/started".to_string(),
        data,
    }
}

fn sample() -> StepPayload {
    sd::conceal_step(plain(json!({})), 7).0
}

fn sign(payload: &StepPayload, key: &Identity, issuer: &str, record_type: &str) -> String {
    let draft = RecordDraft {
        version: STEP_VERSION,
        record_type: record_type.to_string(),
        subject: payload.agreement.clone(),
        payload: serde_json::to_value(payload).unwrap(),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, issuer.to_string(), None, 1000).unwrap();
    env.attach_signature(z32::encode(&key.sign(&bytes).to_bytes())).unwrap();
    env.to_json().unwrap()
}

#[test]
fn a_good_step_validates() {
    assert!(sample().validate().is_ok());
}

#[test]
fn each_bad_field_is_refused_with_its_own_error() {
    let mut p = sample();
    p.agreement = "bad".to_string();
    assert!(matches!(p.validate(), Err(StepError::InvalidAgreement(_))));

    let mut p = sample();
    p.counterparty_did = "bad".to_string();
    assert!(matches!(p.validate(), Err(StepError::InvalidCounterpartyDid(_))));

    let mut p = sample();
    p.quote_author_did = "bad".to_string();
    assert!(matches!(p.validate(), Err(StepError::InvalidQuoteAuthorDid(_))));

    let mut p = sample();
    p.quote_author_did = p.counterparty_did.clone();
    assert!(matches!(p.validate(), Err(StepError::SamePartyDid)));

    let mut p = sample();
    p.conversation.clear();
    assert!(matches!(p.validate(), Err(StepError::ConversationEmpty)));

    let mut p = sample();
    p.commitment_id.clear();
    assert!(matches!(p.validate(), Err(StepError::CommitmentIdEmpty)));

    let mut p = sample();
    p.sd.clear();
    assert!(matches!(p.validate(), Err(StepError::SdShape(_))));
}

#[test]
fn the_extension_reference_needs_an_id_and_a_version() {
    let mut p = sample();
    p.extension.id = "Not Valid".to_string();
    assert!(matches!(p.validate(), Err(StepError::InvalidExtension(_))));

    let mut p = sample();
    p.extension.version = 0;
    assert!(matches!(p.validate(), Err(StepError::InvalidExtension(_))));
}

#[test]
fn the_step_type_needs_the_extension_id_prefix_and_a_limit() {
    for bad in ["started", "other/started", "scheduling/", "scheduling-x/started"] {
        let mut p = sample();
        p.step_type = bad.to_string();
        assert!(
            matches!(p.validate(), Err(StepError::InvalidStepType(_))),
            "{bad} should be refused"
        );
    }
    let mut p = sample();
    p.step_type = format!("scheduling/{}", "a".repeat(MAX_STEP_TYPE_LEN));
    assert!(matches!(p.validate(), Err(StepError::InvalidStepType(_))));

    let mut p = sample();
    p.step_type = format!("scheduling/{}", "a".repeat(MAX_STEP_TYPE_LEN - "scheduling/".len()));
    assert!(p.validate().is_ok());
}

#[test]
fn error_codes_are_stable() {
    assert_eq!(StepError::SamePartyDid.code(), "same-party-did");
    assert_eq!(StepError::DataNotObject.code(), "details-invalid");
    assert_eq!(StepError::DataTooLarge { bytes: 1 }.code(), "step-data-too-large");
}

#[test]
fn verify_accepts_a_good_envelope_and_refuses_a_tampered_or_wrong_type_one() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let mut payload = sample();
    payload.quote_author_did = issuer.clone();

    let good = sign(&payload, &key, &issuer, RECORD_STEP);
    let verdict = verify_step(&good, 1000);
    assert!(verdict.verified, "{:?}", verdict.reason);
    assert_eq!(verdict.payload, Some(payload.clone()));

    let tampered = good.replacen("scheduling/started", "scheduling/finished", 1);
    assert!(!verify_step(&tampered, 1000).verified);

    let wrong_type = sign(&payload, &key, &issuer, "fulfilment-receipt");
    let verdict = verify_step(&wrong_type, 1000);
    assert!(!verdict.verified);
    assert_eq!(verdict.reason.as_deref(), Some("not a step record this build understands"));
}

#[test]
fn verify_refuses_an_issuer_that_is_not_a_party() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let payload = sample();
    let envelope = sign(&payload, &key, &issuer, RECORD_STEP);
    let verdict = verify_step(&envelope, 1000);
    assert!(!verdict.verified);
    assert_eq!(verdict.reason.as_deref(), Some("step issuer is not a party to the agreement"));
}

#[test]
fn the_data_is_one_hidden_field_that_reveals_strictly() {
    let data = json!({ "note": "left at the door" });
    let (payload, disclosures) = sd::conceal_step(plain(data.clone()), 3);
    assert_eq!(disclosures.len(), 1);
    let hidden: StepHidden = reveal_hidden_strict(&payload.sd, &STEP_SET, &disclosures).unwrap();
    assert_eq!(hidden.data, data);

    let (_, other) = sd::conceal_step(plain(data), 4);
    assert!(reveal_hidden_strict::<StepHidden>(&payload.sd, &STEP_SET, &other).is_err());
    assert!(reveal_hidden_strict::<StepHidden>(&payload.sd, &STEP_SET, &[]).is_err());
}

#[test]
fn data_at_the_limit_passes_and_one_byte_more_fails() {
    // `{"k":"<fill>"}` is 8 bytes longer than the fill.
    let fill = |n: usize| json!({ "k": "a".repeat(n) });
    let at_limit = fill(MAX_STEP_DATA_BYTES - 8);
    assert_eq!(serde_json::to_vec(&at_limit).unwrap().len(), MAX_STEP_DATA_BYTES);
    assert!(StepHidden { data: at_limit }.validate().is_ok());

    let over = fill(MAX_STEP_DATA_BYTES - 7);
    assert_eq!(
        StepHidden { data: over }.validate(),
        Err(StepError::DataTooLarge { bytes: MAX_STEP_DATA_BYTES + 1 })
    );
}

#[test]
fn data_must_be_an_object() {
    for bad in [json!([]), json!("x"), json!(1), json!(null)] {
        assert_eq!(StepHidden { data: bad }.validate(), Err(StepError::DataNotObject));
    }
}

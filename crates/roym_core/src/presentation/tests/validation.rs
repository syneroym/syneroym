use serde_json::{Value, json};
use syneroym_signed_record::{Salt, decode_disclosure, disclosure_digest, encode_disclosure};

use super::*;
use crate::{
    method::{all_terms_hidden_paths, reveal_disclosed_terms},
    presentation::{Presentation, verify_presentation},
    record::RECORD_QUOTE,
    transaction::{QUOTE_VERSION, QuotePayload, derive_quote_id},
};

fn tamper_quote_commitment_disclosure(
    cid: &str,
    field_name: &str,
    tampered_val: Value,
) -> (String, String, SignedTerms) {
    let mut fix = setup_test_deal(sample_two_commitment_terms());
    let comm =
        fix.terms.commitments.iter_mut().find(|c| c.commitment_id == cid).expect("commitment");

    let old_disc_idx = fix
        .disclosures
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

    let old_disc = &fix.disclosures[old_disc_idx];
    let old_dec = decode_disclosure(old_disc).expect("decode");
    let old_digest = disclosure_digest(old_disc);

    let salt = Salt::from_encoded(&old_dec.salt).expect("salt");
    let new_disc = encode_disclosure(&salt, field_name, &tampered_val).expect("encode");
    let new_digest = disclosure_digest(&new_disc);

    let sd_pos = comm.sd.iter().position(|d| d == &old_digest).expect("digest in _sd");
    comm.sd[sd_pos] = new_digest;
    comm.sd.sort();

    let quote_payload = QuotePayload {
        quote_id: derive_quote_id("conv_music_deal_1234", &fix.author_did, 1).unwrap(),
        conversation: "conv_music_deal_1234".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_777".to_string() },
        counterparty_did: fix.counterparty_did.clone(),
        terms: fix.terms.clone(),
    };
    let quote_env = sign_test_record(
        &fix.author_key,
        RECORD_QUOTE,
        QUOTE_VERSION,
        &quote_payload.quote_id,
        serde_json::to_value(&quote_payload).unwrap(),
        Some(quote_payload.terms.quote_expires_at_secs),
        1_000,
    );
    (quote_env, new_disc, fix.terms)
}

#[test]
fn declared_value_as_string_is_refused() {
    let (quote_env, bad_disc, signed_terms) =
        tamper_quote_commitment_disclosure("c2", "declared_value_minor", json!("free"));

    let pres = Presentation {
        presentation_version: 1,
        records: vec![quote_env],
        disclosures: vec![bad_disc.clone()],
    };
    let err = verify_presentation(&pres, 1_100).expect_err("string declared value must be refused");
    assert_eq!(err.code(), "field-invalid");

    let (val, hidden, refused) = reveal_disclosed_terms(&signed_terms, &[bad_disc]);
    assert!(val.is_none());
    assert_eq!(refused, Some("field-invalid".to_string()));
    assert!(!hidden.is_empty());
}

#[test]
fn declared_value_negative_is_refused() {
    let (quote_env, bad_disc, signed_terms) =
        tamper_quote_commitment_disclosure("c2", "declared_value_minor", json!(-10));

    let pres = Presentation {
        presentation_version: 1,
        records: vec![quote_env],
        disclosures: vec![bad_disc.clone()],
    };
    let err =
        verify_presentation(&pres, 1_100).expect_err("negative declared value must be refused");
    assert_eq!(err.code(), "field-invalid");

    let (val, _hidden, refused) = reveal_disclosed_terms(&signed_terms, &[bad_disc]);
    assert!(val.is_none());
    assert_eq!(refused, Some("field-invalid".to_string()));
}

#[test]
fn declared_value_over_max_is_refused() {
    let over_max = json!(1_000_000_000_000_001i64);
    let (quote_env, bad_disc, signed_terms) =
        tamper_quote_commitment_disclosure("c2", "declared_value_minor", over_max);

    let pres = Presentation {
        presentation_version: 1,
        records: vec![quote_env],
        disclosures: vec![bad_disc.clone()],
    };
    let err =
        verify_presentation(&pres, 1_100).expect_err("over max declared value must be refused");
    assert_eq!(err.code(), "field-invalid");

    let (val, _hidden, refused) = reveal_disclosed_terms(&signed_terms, &[bad_disc]);
    assert!(val.is_none());
    assert_eq!(refused, Some("field-invalid".to_string()));
}

#[test]
fn declared_value_over_i64_max_is_refused() {
    let over_i64 = serde_json::from_str("9223372036854775808").unwrap();
    let (quote_env, bad_disc, signed_terms) =
        tamper_quote_commitment_disclosure("c2", "declared_value_minor", over_i64);

    let pres = Presentation {
        presentation_version: 1,
        records: vec![quote_env],
        disclosures: vec![bad_disc.clone()],
    };
    let err = verify_presentation(&pres, 1_100)
        .expect_err("over i64::MAX declared value must be refused");
    assert_eq!(err.code(), "field-invalid");

    let (val, _hidden, refused) = reveal_disclosed_terms(&signed_terms, &[bad_disc]);
    assert!(val.is_none());
    assert_eq!(refused, Some("field-invalid".to_string()));
}

#[test]
fn payment_methods_invalid_token_is_refused() {
    let bad_methods = json!(vec!["invalid token with space!"]);
    let (quote_env, bad_disc, signed_terms) =
        tamper_quote_commitment_disclosure("c2", "payment_methods", bad_methods);

    let pres = Presentation {
        presentation_version: 1,
        records: vec![quote_env],
        disclosures: vec![bad_disc.clone()],
    };
    let err = verify_presentation(&pres, 1_100)
        .expect_err("invalid token in payment_methods must be refused");
    assert_eq!(err.code(), "field-invalid");

    let (val, _hidden, refused) = reveal_disclosed_terms(&signed_terms, &[bad_disc]);
    assert!(val.is_none());
    assert_eq!(refused, Some("field-invalid".to_string()));
}

#[test]
fn payment_methods_too_many_items_is_refused() {
    let too_many: Vec<String> = (0..17).map(|i| format!("method_{i}")).collect();
    let (quote_env, bad_disc, signed_terms) =
        tamper_quote_commitment_disclosure("c2", "payment_methods", json!(too_many));

    let pres = Presentation {
        presentation_version: 1,
        records: vec![quote_env],
        disclosures: vec![bad_disc.clone()],
    };
    let err = verify_presentation(&pres, 1_100)
        .expect_err("more than 16 payment_methods must be refused");
    assert_eq!(err.code(), "field-invalid");

    let (val, _hidden, refused) = reveal_disclosed_terms(&signed_terms, &[bad_disc]);
    assert!(val.is_none());
    assert_eq!(refused, Some("field-invalid".to_string()));
}

#[test]
fn refusal_path_hidden_list_equals_all_hidden_list_with_no_disclosures() {
    let fix = setup_test_deal(sample_two_commitment_terms());
    let (shown_none, hidden_refused, err) =
        reveal_disclosed_terms(&fix.terms, &["bogus-disclosure".to_string()]);
    assert!(shown_none.is_none());
    assert!(err.is_some());

    let (shown_empty, hidden_empty, err_none) = reveal_disclosed_terms(&fix.terms, &[]);
    assert!(shown_empty.is_some());
    assert!(err_none.is_none());

    assert_eq!(hidden_refused, hidden_empty);
    assert_eq!(hidden_refused, all_terms_hidden_paths(&fix.terms));
    assert!(hidden_refused.contains(&"terms.listing_id".to_string()));
}

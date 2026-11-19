use serde_json::{Map, json};
use syneroym_signed_record::{encode_base64url, encode_disclosure, seeded_salt};

use super::*;

#[test]
fn a_disclosure_whose_digest_is_not_in_the_record_is_refused() {
    let mut salts = (0..).map(|n| seeded_salt(1, n));
    let mut obj = Map::new();
    obj.insert("currency".to_string(), json!("INR"));
    obj.insert("scope".to_string(), json!({"lang": "en", "text": "Cleaning"}));

    let disclosures = conceal_object(&mut obj, &TERMS_SET, &mut salts).unwrap();
    assert_eq!(disclosures.len(), TERMS_SET.hidden.len());

    // Build extra disclosure not referenced by `obj`
    let extra_salt = seeded_salt(99, 99);
    let extra_disc = encode_disclosure(&extra_salt, "scope", &json!("extra")).unwrap();

    let mut with_extra = disclosures.clone();
    with_extra.push(extra_disc);

    let index = DisclosureIndex::build(&with_extra, with_extra.len()).unwrap();
    let revealed = reveal_object(&obj, &TERMS_SET, &index).unwrap();

    let err = check_all_used(&index, &revealed.used).unwrap_err();
    assert_eq!(err.code(), "disclosure-unknown-digest");
}

#[test]
fn a_disclosure_listed_twice_in_the_card_is_refused() {
    let salt = seeded_salt(2, 0);
    let disc = encode_disclosure(&salt, "note", &json!("hello")).unwrap();

    let duplicate_list = vec![disc.clone(), disc];
    let err = DisclosureIndex::build(&duplicate_list, 5).unwrap_err();
    assert_eq!(err.code(), "digest-repeated");
}

#[test]
fn a_name_that_is_not_hidden_here_is_refused() {
    let salt = seeded_salt(3, 0);
    // Name "random_field" is not in PAYMENT_REQUEST_SET.hidden
    let disc = encode_disclosure(&salt, "random_field", &json!("val")).unwrap();

    let index = DisclosureIndex::build(&[disc], 5).unwrap();
    let mut obj = Map::new();
    let digest = index.digests().next().unwrap();
    obj.insert("_sd".to_string(), json!([digest]));

    let err = reveal_object(&obj, &PAYMENT_REQUEST_SET, &index).unwrap_err();
    assert_eq!(err.code(), "disclosure-name");
}

#[test]
fn a_name_equal_to_a_visible_field_is_refused() {
    let salt = seeded_salt(4, 0);
    // Name "currency" is in TERMS_SET.visible, not hidden
    let disc = encode_disclosure(&salt, "currency", &json!("INR")).unwrap();

    let index = DisclosureIndex::build(&[disc], 5).unwrap();
    let mut obj = Map::new();
    let digest = index.digests().next().unwrap();
    let mut digests = vec![
        digest.clone(),
        format!("{:-<42}1", ""),
        format!("{:-<42}2", ""),
        format!("{:-<42}3", ""),
        format!("{:-<42}4", ""),
    ];
    digests.sort();
    obj.insert("_sd".to_string(), json!(digests));

    let err = reveal_object(&obj, &TERMS_SET, &index).unwrap_err();
    assert_eq!(err.code(), "disclosure-visible-name");
}

#[test]
fn a_salt_of_another_length_is_refused() {
    // 12-byte salt instead of 16-byte
    let bad_salt_str = encode_base64url(&[1u8; 12]);
    let tuple = (bad_salt_str, "note", json!("test"));
    let json_bytes = serde_json::to_vec(&tuple).unwrap();
    let disc = encode_base64url(&json_bytes);

    let index = DisclosureIndex::build(&[disc], 5).unwrap();
    let err = index.check_salts().unwrap_err();
    assert_eq!(err.code(), "salt-length");
    assert_eq!(err, SdError::SaltLength(12));
}

#[test]
fn a_salt_used_twice_in_one_record_is_refused() {
    let salt = seeded_salt(5, 0);
    let disc1 = encode_disclosure(&salt, "note", &json!("first")).unwrap();
    let disc2 = encode_disclosure(&salt, "method", &json!("cash")).unwrap();

    let index = DisclosureIndex::build(&[disc1, disc2], 5).unwrap();
    let err = index.check_salts().unwrap_err();
    assert_eq!(err.code(), "salt-repeated");
}

#[test]
fn an_unclassified_field_cannot_be_concealed() {
    let mut salts = (0..).map(|n| seeded_salt(6, n));
    let mut obj = Map::new();
    obj.insert("currency".to_string(), json!("INR"));
    obj.insert("unclassified_secret".to_string(), json!("peekaboo"));

    let err = conceal_object(&mut obj, &TERMS_SET, &mut salts).unwrap_err();
    assert_eq!(err.code(), "unclassified-field");
    assert_eq!(err, SdError::UnclassifiedField("unclassified_secret".to_string()));
}

#[test]
fn field_paths_parse_and_print() {
    let paths = [
        "terms.scope",
        "terms.listing_id",
        "terms.commitments[c1].description",
        "terms.commitments[c16].declared_value_minor",
        "rec_12345.note",
        "rec_abcde.settlement_note",
    ];

    for s in paths {
        let parsed = FieldPath::parse(s).expect("parse valid field path");
        assert_eq!(parsed.to_string(), s);
    }

    assert!(FieldPath::parse("invalid_no_dot").is_err());
    assert!(FieldPath::parse("terms.").is_err());
    assert!(FieldPath::parse("terms.commitments[].description").is_err());
}

#[test]
fn too_many_disclosures_are_refused_before_any_decode() {
    let disclosures = vec!["disc1".to_string(), "disc2".to_string(), "disc3".to_string()];
    let err = DisclosureIndex::build(&disclosures, 2).unwrap_err();
    assert_eq!(err.code(), "disclosure-count-too-high");
}

#[test]
fn one_oversized_disclosure_is_refused_before_decode() {
    let oversized = "a".repeat(MAX_RECORD_DISCLOSURE_BYTES + 1);
    let err = DisclosureIndex::build(&[oversized], 10).unwrap_err();
    assert_eq!(err.code(), "quote-too-large");
}

use serde_json::json;

use super::*;

fn envelope() -> String {
    json!({ "envelope_version": 1, "record_type": "listing" }).to_string()
}

#[test]
fn a_forwarded_record_body_round_trips_through_its_bytes() {
    let body = RecordForwardBody::new(record::RECORD_LISTING, &envelope()).unwrap();
    let bytes = body.to_bytes().unwrap();
    let read = RecordForwardBody::parse(&bytes).unwrap();
    assert_eq!(read, body);
    assert_eq!(read.record_forward_version, 1);
    assert_eq!(read.record_type, "listing");
}

#[test]
fn every_type_the_product_offers_is_accepted() {
    for record_type in RECORD_FORWARD_TYPES {
        assert!(RecordForwardBody::new(record_type, &envelope()).is_ok(), "{record_type}");
    }
}

#[test]
fn a_type_that_is_not_offered_is_refused() {
    let err = RecordForwardBody::new("enquiry", &envelope()).unwrap_err();
    assert_eq!(err, RecordForwardError::UnsupportedRecordType("enquiry".to_string()));
}

#[test]
fn another_body_version_is_refused() {
    let bytes = json!({
        "record_forward_version": 2,
        "record_type": "listing",
        "envelope": envelope(),
    })
    .to_string();
    let err = RecordForwardBody::parse(bytes.as_bytes()).unwrap_err();
    assert_eq!(err, RecordForwardError::UnsupportedVersion(2));
}

#[test]
fn a_body_over_the_size_limit_is_refused_before_it_parses() {
    let bytes = vec![b'{'; MAX_CARD_BODY_BYTES + 1];
    let err = RecordForwardBody::parse(&bytes).unwrap_err();
    assert_eq!(err, RecordForwardError::TooLarge);
}

#[test]
fn an_envelope_over_the_size_limit_is_refused_on_the_send_path() {
    let too_long = "x".repeat(MAX_CARD_BODY_BYTES + 1);
    let err = RecordForwardBody::new(record::RECORD_LISTING, &too_long).unwrap_err();
    assert_eq!(err, RecordForwardError::TooLarge);
}

#[test]
fn an_envelope_that_is_not_a_json_object_is_refused_on_the_send_path() {
    for bad in ["not json", "[1, 2]", "\"text\""] {
        let err = RecordForwardBody::new(record::RECORD_QUOTE, bad).unwrap_err();
        assert!(
            matches!(err, RecordForwardError::Malformed(_)),
            "{bad} must be refused, got {err:?}"
        );
    }
}

#[test]
fn a_body_that_is_not_json_is_malformed() {
    let err = RecordForwardBody::parse(b"not json").unwrap_err();
    assert!(matches!(err, RecordForwardError::Malformed(_)), "{err:?}");
}

#[test]
fn a_body_missing_a_field_is_malformed() {
    let bytes = json!({ "record_forward_version": 1, "record_type": "quote" }).to_string();
    let err = RecordForwardBody::parse(bytes.as_bytes()).unwrap_err();
    assert!(matches!(err, RecordForwardError::Malformed(_)), "{err:?}");
}

#[test]
fn a_body_from_a_later_version_keeps_its_unknown_fields_harmless() {
    let bytes = json!({
        "record_forward_version": 1,
        "record_type": "profile",
        "envelope": envelope(),
        "added_later": true,
    })
    .to_string();
    let read = RecordForwardBody::parse(bytes.as_bytes()).unwrap();
    assert_eq!(read.record_type, "profile");
}

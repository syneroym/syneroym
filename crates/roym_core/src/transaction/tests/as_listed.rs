//! Tests for the enquiry's ask to agree as listed.

use syneroym_identity::Identity;

use super::*;
use crate::{
    as_listed::{AsListed, MAX_CONSENT_SECS, SlotChoice},
    listing,
};

const LISTING_ID: &str = "lst_as_listed";
const WINDOW: TimeWindow = TimeWindow { earliest_secs: 10_000, latest_secs: 13_600 };

fn consent(valid_until_secs: u64) -> AsListed {
    AsListed {
        record_id: "rec_listing_version".to_string(),
        slot: None,
        variant: None,
        valid_until_secs,
    }
}

fn payload(issuer: &str, as_listed: Option<AsListed>) -> EnquiryPayload {
    EnquiryPayload {
        enquiry_id: derive_enquiry_id("conv_1", issuer, 1).expect("enquiry id"),
        conversation: "conv_1".to_string(),
        sequence: 1,
        listing_id: Some(LISTING_ID.to_string()),
        categories: Vec::new(),
        description: None,
        as_listed,
        area: None,
        window: None,
        data_use_notice: DEFAULT_DATA_USE_NOTICE.to_string(),
    }
}

#[test]
fn as_listed_needs_a_listing() {
    let mut p = payload("did:key:zAsker", Some(consent(2_000)));
    p.listing_id = None;
    assert_eq!(p.validate().expect_err("refused").code(), Some("as-listed-needs-listing"));
}

#[test]
fn variant_is_reserved_and_refused() {
    let mut as_listed = consent(2_000);
    as_listed.variant = Some("variant-a".to_string());
    let p = payload("did:key:zAsker", Some(as_listed));
    assert_eq!(p.validate().expect_err("refused").code(), Some("variant-not-supported"));
}

#[test]
fn a_slot_id_must_match_its_window() {
    let derived = listing::derive_slot_id(LISTING_ID, WINDOW.earliest_secs, WINDOW.latest_secs)
        .expect("slot id");
    let mut as_listed = consent(2_000);
    as_listed.slot = Some(SlotChoice { slot_id: derived, window: WINDOW });
    assert!(payload("did:key:zAsker", Some(as_listed.clone())).validate().is_ok());

    let mut wrong = as_listed;
    wrong.slot.as_mut().expect("slot").slot_id = "slot_other".to_string();
    let p = payload("did:key:zAsker", Some(wrong));
    assert_eq!(p.validate().expect_err("refused").code(), Some("slot-differs"));
}

#[test]
fn an_enquiry_without_a_description_needs_as_listed() {
    let p = payload("did:key:zAsker", None);
    assert_eq!(p.validate().expect_err("refused").code(), Some("description-required"));

    let p = payload("did:key:zAsker", Some(consent(2_000)));
    assert!(p.validate().is_ok());
}

#[test]
fn a_signed_as_listed_enquiry_with_no_description_verifies() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());
    let p = payload(&issuer, Some(consent(1_300)));
    let env = sign_envelope(
        &key,
        RECORD_ENQUIRY,
        ENQUIRY_VERSION,
        &p.enquiry_id,
        serde_json::to_value(&p).expect("json"),
        None,
        1_000,
    );
    let verdict = verify_enquiry(&env, 1_000);
    assert!(verdict.verified, "{:?}", verdict.reason);
    assert_eq!(verdict.payload.expect("payload").as_listed, Some(consent(1_300)));
}

#[test]
fn the_consent_window_is_bounded_by_the_issue_time() {
    let key = Identity::generate().unwrap();
    let issuer = substrate::derive_did_key(&key.public_key());

    for valid_until_secs in [1_000, 1_001 + MAX_CONSENT_SECS] {
        let p = payload(&issuer, Some(consent(valid_until_secs)));
        let env = sign_envelope(
            &key,
            RECORD_ENQUIRY,
            ENQUIRY_VERSION,
            &p.enquiry_id,
            serde_json::to_value(&p).expect("json"),
            None,
            1_000,
        );
        let verdict = verify_enquiry(&env, 1_000);
        assert!(!verdict.verified, "{valid_until_secs} must be refused");
    }

    let p = payload(&issuer, Some(consent(1_000 + MAX_CONSENT_SECS)));
    let env = sign_envelope(
        &key,
        RECORD_ENQUIRY,
        ENQUIRY_VERSION,
        &p.enquiry_id,
        serde_json::to_value(&p).expect("json"),
        None,
        1_000,
    );
    assert!(verify_enquiry(&env, 1_000).verified);
}

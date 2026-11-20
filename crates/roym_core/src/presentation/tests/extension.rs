use serde_json::json;
use syneroym_signed_record::{decode_disclosure, disclosure_digest};

use super::*;
use crate::{
    deal::{Details, ExtensionRef},
    extension::{SCHEDULING_EXTENSION, SCHEDULING_SLOT_SCHEMA},
    listing,
};

const NOT_CHECKED: &str = "terms.commitments[c1].details_matches_extension";

fn slot_terms() -> DealTerms {
    let mut terms = sample_two_commitment_terms();
    let slot_id = listing::derive_slot_id("lis_slot_1", 5_000, 8_000).expect("slot");
    let c1 = &mut terms.commitments[0];
    c1.resource_kind = ResourceKind::ServiceInSlot;
    c1.extension = Some(ExtensionRef { id: SCHEDULING_EXTENSION.to_string(), version: 1 });
    c1.details = Some(Details {
        extension: SCHEDULING_EXTENSION.to_string(),
        schema: SCHEDULING_SLOT_SCHEMA.to_string(),
        data: json!({ "listing_id": "lis_slot_1", "slot_id": slot_id }),
        extra: vec![],
    });
    terms
}

fn with_extension(id: &str, schema: &str, data: serde_json::Value) -> DealTerms {
    let mut terms = sample_two_commitment_terms();
    let c1 = &mut terms.commitments[0];
    c1.extension = Some(ExtensionRef { id: id.to_string(), version: 1 });
    c1.details = Some(Details {
        extension: id.to_string(),
        schema: schema.to_string(),
        data,
        extra: vec![],
    });
    terms
}

/// A presentation of the quote that shows only the named fields of `c1`.
fn showing(fix: &TestDealFixture, names: &[&str]) -> Presentation {
    let c1 = &fix.terms.commitments[0];
    let disclosures = fix
        .disclosures
        .iter()
        .filter(|d| {
            let name = decode_disclosure(d).expect("decode").name;
            names.contains(&name.as_str()) && c1.sd.contains(&disclosure_digest(d))
        })
        .cloned()
        .collect();
    Presentation { presentation_version: 1, records: vec![fix.quote_env.clone()], disclosures }
}

#[test]
fn scheduling_with_a_good_window_is_checked() {
    let fix = setup_test_deal(slot_terms());
    let view = verify_presentation(&showing(&fix, &["details", "window"]), 1_100).unwrap();
    assert!(!view.not_checked.contains(&NOT_CHECKED.to_string()));
}

#[test]
fn hidden_details_are_not_checked() {
    let fix = setup_test_deal(slot_terms());
    let view = verify_presentation(&showing(&fix, &[]), 1_100).unwrap();
    assert!(view.not_checked.contains(&NOT_CHECKED.to_string()));
}

#[test]
fn a_hidden_window_leaves_the_slot_check_open() {
    let fix = setup_test_deal(slot_terms());
    let view = verify_presentation(&showing(&fix, &["details"]), 1_100).unwrap();
    assert!(view.not_checked.contains(&NOT_CHECKED.to_string()));
}

#[test]
fn an_unknown_extension_with_visible_details_is_verified_but_not_checked() {
    let fix = setup_test_deal(with_extension("future", "future/thing@1", json!({ "a": 1 })));
    let view = verify_presentation(&showing(&fix, &["details"]), 1_100).unwrap();
    assert!(view.not_checked.contains(&NOT_CHECKED.to_string()));
}

#[test]
fn one_shot_details_that_fail_the_schema_are_refused() {
    let fix = setup_test_deal(with_extension("one-shot", "one-shot/order@1", json!({ "x": 1 })));
    let err = verify_presentation(&showing(&fix, &["details"]), 1_100).unwrap_err();
    assert_eq!(err.code(), "field-invalid");
}

#[test]
fn one_shot_details_that_pass_the_schema_are_checked() {
    let fix = setup_test_deal(with_extension("one-shot", "one-shot/order@1", json!({})));
    let view = verify_presentation(&showing(&fix, &["details"]), 1_100).unwrap();
    assert!(!view.not_checked.contains(&NOT_CHECKED.to_string()));
}

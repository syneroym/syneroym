use syneroym_signed_record::seeded_salt;

use super::*;
use crate::extension::{self, ExtensionCheck, Registry, registry};

fn one_shot_commitment(id: &str, giver: DealRole) -> Commitment {
    let mut c = sample_service_commitment(id, giver, 100);
    c.extension = Some(ExtensionRef { id: "one-shot".to_string(), version: 1 });
    c.details = Some(Details {
        extension: "one-shot".to_string(),
        schema: "one-shot/order@1".to_string(),
        data: json!({}),
        extra: vec![],
    });
    c
}

fn unknown_commitment(id: &str, giver: DealRole) -> Commitment {
    let mut c = sample_service_commitment(id, giver, 100);
    c.extension = Some(ExtensionRef { id: "future".to_string(), version: 3 });
    c.details = Some(Details {
        extension: "future".to_string(),
        schema: "future/thing@2".to_string(),
        data: json!({ "anything": [1, 2, 3], "goes": true }),
        extra: vec![],
    });
    c
}

fn code_of(c: Commitment) -> &'static str {
    sample_deal_terms(vec![c]).validate().unwrap_err().code()
}

#[test]
fn one_shot_goods_commitment_is_checked() {
    let c = one_shot_commitment("c1", DealRole::QuoteAuthor);
    let terms = sample_deal_terms(vec![c]);
    terms.validate().unwrap();
    assert_eq!(terms.check_extensions(registry()).unwrap(), [ExtensionCheck::Checked]);
}

#[test]
fn one_shot_is_refused_where_it_does_not_belong() {
    let mut on_slot = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    on_slot.extension = Some(ExtensionRef { id: "one-shot".to_string(), version: 1 });
    on_slot.details = one_shot_commitment("c1", DealRole::QuoteAuthor).details;
    assert_eq!(code_of(on_slot), "commitment-kind-not-supported");

    let mut on_currency = sample_currency_commitment("c1", DealRole::QuoteAuthor, 100);
    on_currency.extension = Some(ExtensionRef { id: "one-shot".to_string(), version: 1 });
    on_currency.details = one_shot_commitment("c1", DealRole::QuoteAuthor).details;
    assert_eq!(code_of(on_currency), "commitment-kind-not-supported");
}

#[test]
fn one_shot_with_the_wrong_schema_or_data_is_refused() {
    let mut wrong_schema = one_shot_commitment("c1", DealRole::QuoteAuthor);
    wrong_schema.details.as_mut().unwrap().schema = SCHEDULING_SLOT_SCHEMA.to_string();
    // The schema name must start with the extension's id, so this is a shape
    // error; a schema of the right form that the manifest lacks is a
    // mismatch.
    assert_eq!(code_of(wrong_schema), "extension-shape");

    let mut other_schema = one_shot_commitment("c1", DealRole::QuoteAuthor);
    other_schema.details.as_mut().unwrap().schema = "one-shot/other@1".to_string();
    assert_eq!(code_of(other_schema), "details-mismatch");

    let mut with_data = one_shot_commitment("c1", DealRole::QuoteAuthor);
    with_data.details.as_mut().unwrap().data = json!({ "colour": "red" });
    assert_eq!(code_of(with_data), "details-invalid");
}

#[test]
fn a_slot_commitment_with_bad_fields_is_refused() {
    let mut extra_key = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    extra_key.details.as_mut().unwrap().data["note"] = json!("x");
    assert_eq!(code_of(extra_key), "details-invalid");

    let mut not_a_code = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    not_a_code.details.as_mut().unwrap().data["listing_id"] = json!("has a space");
    assert_eq!(code_of(not_a_code), "details-invalid");

    let mut no_window = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    no_window.window = None;
    assert_eq!(code_of(no_window), "slot-details");

    let mut wrong_slot = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    wrong_slot.details.as_mut().unwrap().data["slot_id"] = json!("slot_wrong");
    assert_eq!(code_of(wrong_slot), "slot-details");
}

#[test]
fn unknown_extension_is_not_checked() {
    let terms = sample_deal_terms(vec![unknown_commitment("c1", DealRole::QuoteAuthor)]);
    terms.validate().unwrap();
    assert_eq!(terms.check_extensions(registry()).unwrap(), [ExtensionCheck::NotChecked]);

    let mut on_currency = sample_currency_commitment("c1", DealRole::QuoteAuthor, 100);
    on_currency.extension = Some(ExtensionRef { id: "future".to_string(), version: 3 });
    on_currency.details = unknown_commitment("c1", DealRole::QuoteAuthor).details;
    assert_eq!(code_of(on_currency), "commitment-kind-not-supported");

    let mut other_id = unknown_commitment("c1", DealRole::QuoteAuthor);
    other_id.details.as_mut().unwrap().extension = "another".to_string();
    assert_eq!(code_of(other_id), "details-mismatch");

    let mut no_details = unknown_commitment("c1", DealRole::QuoteAuthor);
    no_details.details = None;
    assert_eq!(code_of(no_details), "details-mismatch");
}

#[test]
fn a_known_extension_with_another_version_is_not_checked() {
    let mut c = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    c.extension = Some(ExtensionRef { id: SCHEDULING_EXTENSION.to_string(), version: 2 });
    let terms = sample_deal_terms(vec![c]);
    terms.validate().unwrap();
    assert_eq!(terms.check_extensions(registry()).unwrap(), [ExtensionCheck::NotChecked]);
}

#[test]
fn check_extensions_reports_each_commitment() {
    let terms = sample_deal_terms(vec![
        sample_slot_commitment("c1", DealRole::QuoteAuthor, 100),
        one_shot_commitment("c2", DealRole::QuoteAuthor),
        unknown_commitment("c3", DealRole::QuoteAuthor),
        sample_currency_commitment("c4", DealRole::Counterparty, 100),
    ]);
    terms.validate().unwrap();
    assert_eq!(
        terms.check_extensions(registry()).unwrap(),
        [
            ExtensionCheck::Checked,
            ExtensionCheck::Checked,
            ExtensionCheck::NotChecked,
            ExtensionCheck::None
        ]
    );
}

#[test]
fn a_commitment_naming_an_insight_is_not_checked() {
    let insight =
        json!({ "id": "digest", "version": 1, "kind": "insight", "reads": ["event-log"] });
    let with_insight = Registry::parse(&[&insight.to_string()]).unwrap();
    let mut c = unknown_commitment("c1", DealRole::QuoteAuthor);
    c.extension = Some(ExtensionRef { id: "digest".to_string(), version: 1 });
    c.details.as_mut().unwrap().extension = "digest".to_string();
    c.details.as_mut().unwrap().schema = "digest/x@1".to_string();
    assert_eq!(
        extension::check_commitment_extension(&with_insight, &c).unwrap(),
        ExtensionCheck::NotChecked
    );
}

fn extra_cases() -> Vec<(Vec<DetailsExtra>, &'static str)> {
    let item = |label: &str, value: &str| DetailsExtra {
        label: label.to_string(),
        value: value.to_string(),
    };
    vec![
        ((0..17).map(|i| item(&format!("l{i}"), "v")).collect(), "17 items"),
        (vec![item(&"l".repeat(65), "v")], "65 byte label"),
        (vec![item("l", &"v".repeat(513))], "513 byte value"),
    ]
}

#[test]
fn extra_limits_hold_for_every_extension() {
    for (extra, what) in extra_cases() {
        for mut c in [
            one_shot_commitment("c1", DealRole::QuoteAuthor),
            unknown_commitment("c1", DealRole::QuoteAuthor),
            sample_slot_commitment("c1", DealRole::QuoteAuthor, 100),
        ] {
            c.details.as_mut().unwrap().extra = extra.clone();
            assert_eq!(code_of(c), "extension-shape", "{what}");
        }
    }
    let mut at_the_limit = one_shot_commitment("c1", DealRole::QuoteAuthor);
    at_the_limit.details.as_mut().unwrap().extra = (0..16)
        .map(|i| DetailsExtra { label: format!("{i}{}", "l".repeat(62)), value: "v".repeat(512) })
        .collect();
    sample_deal_terms(vec![at_the_limit]).validate().unwrap();
}

#[test]
fn the_extension_and_schema_names_have_their_form() {
    for (id, schema) in [("Future", "Future/a@1"), ("future", "future/a"), ("future", "other/a@1")]
    {
        let mut c = unknown_commitment("c1", DealRole::QuoteAuthor);
        c.extension = Some(ExtensionRef { id: id.to_string(), version: 1 });
        let details = c.details.as_mut().unwrap();
        details.extension = id.to_string();
        details.schema = schema.to_string();
        assert_eq!(code_of(c), "extension-shape", "{id} {schema}");
    }
    let mut version_zero = unknown_commitment("c1", DealRole::QuoteAuthor);
    version_zero.extension.as_mut().unwrap().version = 0;
    assert_eq!(code_of(version_zero), "extension-shape");
}

fn conceal(terms: &DealTerms, seed: u64) -> crate::deal::sd::ConcealedTerms {
    let mut salts = (0..).map(|n| seeded_salt(seed, n));
    terms.conceal(&mut salts).unwrap()
}

#[test]
fn reveal_strict_refuses_a_slot_id_that_does_not_match_its_window() {
    let mut c = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    c.details.as_mut().unwrap().data["slot_id"] = json!("slot_wrong");
    let concealed = conceal(&sample_deal_terms(vec![c]), 31);
    let err = crate::deal::sd::reveal_terms_strict(&concealed.signed, &concealed.disclosures)
        .unwrap_err();
    assert!(err.to_string().contains("does not match"), "{err}");
}

#[test]
fn reveal_strict_reveals_a_quote_with_an_unknown_extension() {
    let terms = sample_deal_terms(vec![unknown_commitment("c1", DealRole::QuoteAuthor)]);
    let concealed = conceal(&terms, 32);
    let revealed =
        crate::deal::sd::reveal_terms_strict(&concealed.signed, &concealed.disclosures).unwrap();
    assert_eq!(revealed, terms);
}

#[test]
fn visible_check_lets_an_unknown_extension_pass_and_refuses_a_misplaced_known_one() {
    let unknown = sample_deal_terms(vec![unknown_commitment("c1", DealRole::QuoteAuthor)]);
    conceal(&unknown, 33).signed.validate_visible().unwrap();

    let mut misplaced = one_shot_commitment("c1", DealRole::QuoteAuthor);
    misplaced.resource_kind = ResourceKind::ServiceInSlot;
    let signed = conceal(&sample_deal_terms(vec![misplaced]), 34).signed;
    assert_eq!(signed.validate_visible().unwrap_err().code(), "commitment-kind-not-supported");
}

/// Each case breaks a kind rule. `validate` and `validate_visible` must both
/// refuse it with the same code.
#[test]
fn the_kind_rules_hold_on_both_paths() {
    let mut slot_without_extension = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    slot_without_extension.extension = None;
    slot_without_extension.details = None;

    let mut goods_as_currency = sample_service_commitment("c1", DealRole::QuoteAuthor, 100);
    goods_as_currency.kind = CommitmentKind::Currency;

    let mut currency_as_fulfilment = sample_currency_commitment("c1", DealRole::QuoteAuthor, 100);
    currency_as_fulfilment.kind = CommitmentKind::Fulfilment;

    let cases = [
        (slot_without_extension, "commitment-kind-not-supported"),
        (goods_as_currency, "kind-mismatch"),
        (currency_as_fulfilment, "kind-mismatch"),
    ];
    for (n, (c, want)) in cases.into_iter().enumerate() {
        let terms = sample_deal_terms(vec![c]);
        assert_eq!(terms.validate().unwrap_err().code(), want, "validate, case {n}");
        let signed = conceal(&terms, 40 + n as u64).signed;
        assert_eq!(signed.validate_visible().unwrap_err().code(), want, "visible, case {n}");
    }
}

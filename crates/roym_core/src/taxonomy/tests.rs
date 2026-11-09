use super::*;

#[test]
fn base_taxonomy_parses_and_validates() {
    let taxonomy = Taxonomy::parse(BASE_JSON).expect("base taxonomy must be valid");
    assert!(
        taxonomy.categories.len() >= 50 && taxonomy.categories.len() <= 150,
        "category count {} out of range 50..=150",
        taxonomy.categories.len()
    );
    assert!(!taxonomy.units.is_empty());
    assert!(!taxonomy.payment_methods.is_empty());
    assert!(!taxonomy.attributes.is_empty());
}

#[test]
fn refuses_invalid_json() {
    let err = Taxonomy::parse("not json").unwrap_err();
    assert!(matches!(err, TaxonomyError::Parse(_)));
}

#[test]
fn refuses_unknown_field() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["unit"],
        "payment_methods": ["cash"],
        "attributes": [],
        "extra_field": 123
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(err, TaxonomyError::Parse(_)));
}

#[test]
fn refuses_bad_category_code_shape() {
    let json = r#"{
        "categories": [{ "code": "Plumbing" }],
        "units": [],
        "payment_methods": [],
        "attributes": []
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::CodeShape {
            kind: "category",
            code
        } if code == "Plumbing"
    ));

    let too_long = "a".repeat(65);
    let json_long = format!(
        r#"{{
            "categories": [{{ "code": "{too_long}" }}],
            "units": [],
            "payment_methods": [],
            "attributes": []
        }}"#
    );
    let err = Taxonomy::parse(&json_long).unwrap_err();
    assert!(matches!(err, TaxonomyError::CodeShape { kind: "category", .. }));

    let json_space = r#"{
        "categories": [{ "code": "a b" }],
        "units": [],
        "payment_methods": [],
        "attributes": []
    }"#;
    let err = Taxonomy::parse(json_space).unwrap_err();
    assert!(matches!(err, TaxonomyError::CodeShape { kind: "category", .. }));
}

#[test]
fn refuses_bad_unit_code_shape() {
    let too_long = "u".repeat(33);
    let json = format!(
        r#"{{
            "categories": [{{ "code": "cat" }}, {{ "code": "sub", "parent": "cat" }}],
            "units": ["{too_long}"],
            "payment_methods": [],
            "attributes": []
        }}"#
    );
    let err = Taxonomy::parse(&json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::CodeShape {
            kind: "unit",
            code
        } if code == too_long
    ));
}

#[test]
fn refuses_bad_payment_method_code_shape() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["piece"],
        "payment_methods": ["Cash"],
        "attributes": []
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::CodeShape {
            kind: "payment method",
            code
        } if code == "Cash"
    ));
}

#[test]
fn refuses_bad_attribute_code_shape() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["piece"],
        "payment_methods": ["cash"],
        "attributes": [
            { "code": "Bad-Attr", "categories": ["cat"], "value": { "type": "yes-no" } }
        ]
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::CodeShape {
            kind: "attribute",
            code
        } if code == "Bad-Attr"
    ));
}

#[test]
fn refuses_bad_choice_code_shape() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["piece"],
        "payment_methods": ["cash"],
        "attributes": [
            { "code": "attr", "categories": ["cat"], "value": { "type": "choice", "choices": ["Valid", "bad choice"] } }
        ]
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::CodeShape {
            kind: "choice",
            code
        } if code == "Valid"
    ));
}

#[test]
fn refuses_duplicate_category() {
    let json = r#"{
        "categories": [
            { "code": "cat" },
            { "code": "sub", "parent": "cat" },
            { "code": "cat" }
        ],
        "units": [],
        "payment_methods": [],
        "attributes": []
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::Duplicate {
            kind: "category",
            code
        } if code == "cat"
    ));
}

#[test]
fn refuses_duplicate_unit() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["kg", "kg"],
        "payment_methods": [],
        "attributes": []
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::Duplicate {
            kind: "unit",
            code
        } if code == "kg"
    ));
}

#[test]
fn refuses_duplicate_payment_method() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["kg"],
        "payment_methods": ["cash", "cash"],
        "attributes": []
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::Duplicate {
            kind: "payment method",
            code
        } if code == "cash"
    ));
}

#[test]
fn refuses_duplicate_attribute() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["kg"],
        "payment_methods": ["cash"],
        "attributes": [
            { "code": "attr", "categories": ["cat"], "value": { "type": "yes-no" } },
            { "code": "attr", "categories": ["cat"], "value": { "type": "yes-no" } }
        ]
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::Duplicate {
            kind: "attribute",
            code
        } if code == "attr"
    ));
}

#[test]
fn refuses_duplicate_choice() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["kg"],
        "payment_methods": ["cash"],
        "attributes": [
            { "code": "attr", "categories": ["cat"], "value": { "type": "choice", "choices": ["opt", "opt"] } }
        ]
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::Duplicate {
            kind: "choice",
            code
        } if code == "opt"
    ));
}

#[test]
fn refuses_missing_parent() {
    let json = r#"{
        "categories": [
            { "code": "child", "parent": "nonexistent" }
        ],
        "units": [],
        "payment_methods": [],
        "attributes": []
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::BadParent {
            code,
            parent
        } if code == "child" && parent == "nonexistent"
    ));
}

#[test]
fn refuses_parent_that_has_a_parent() {
    let json = r#"{
        "categories": [
            { "code": "grandparent" },
            { "code": "parent", "parent": "grandparent" },
            { "code": "child", "parent": "parent" }
        ],
        "units": [],
        "payment_methods": [],
        "attributes": []
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::BadParent {
            code,
            parent
        } if code == "child" && parent == "parent"
    ));
}

#[test]
fn refuses_child_before_group() {
    let json = r#"{
        "categories": [
            { "code": "child", "parent": "group" },
            { "code": "group" }
        ],
        "units": [],
        "payment_methods": [],
        "attributes": []
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::ParentAfterChild {
            code,
            parent
        } if code == "child" && parent == "group"
    ));
}

#[test]
fn refuses_group_without_children() {
    let json = r#"{
        "categories": [
            { "code": "group-alone" }
        ],
        "units": [],
        "payment_methods": [],
        "attributes": []
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::GroupWithoutChildren(code) if code == "group-alone"
    ));
}

#[test]
fn refuses_unknown_attribute_category() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["kg"],
        "payment_methods": ["cash"],
        "attributes": [
            { "code": "attr", "categories": ["unknown-cat"], "value": { "type": "yes-no" } }
        ]
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::UnknownCategory {
            attribute,
            category
        } if attribute == "attr" && category == "unknown-cat"
    ));
}

#[test]
fn refuses_unknown_number_unit() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["kg"],
        "payment_methods": ["cash"],
        "attributes": [
            { "code": "attr", "categories": ["cat"], "value": { "type": "number", "unit": "unknown-unit" } }
        ]
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::UnknownUnit {
            attribute,
            unit
        } if attribute == "attr" && unit == "unknown-unit"
    ));
}

#[test]
fn refuses_attribute_with_empty_categories() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["kg"],
        "payment_methods": ["cash"],
        "attributes": [
            { "code": "attr", "categories": [], "value": { "type": "yes-no" } }
        ]
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::Empty(attr) if attr == "attr"
    ));
}

#[test]
fn refuses_choice_attribute_with_empty_choices() {
    let json = r#"{
        "categories": [{ "code": "cat" }, { "code": "sub", "parent": "cat" }],
        "units": ["kg"],
        "payment_methods": ["cash"],
        "attributes": [
            { "code": "attr", "categories": ["cat"], "value": { "type": "choice", "choices": [] } }
        ]
    }"#;
    let err = Taxonomy::parse(json).unwrap_err();
    assert!(matches!(
        err,
        TaxonomyError::Empty(attr) if attr == "attr"
    ));
}

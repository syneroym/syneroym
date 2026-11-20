use serde_json::{Value, json};

use super::{Field, FieldType, check_fields};

fn field(name: &str, field_type: FieldType) -> Field {
    Field {
        name: name.to_string(),
        field_type,
        required: false,
        max_len: matches!(field_type, FieldType::Text | FieldType::LongText | FieldType::Code)
            .then_some(8),
        min: None,
        max: None,
        choices: None,
    }
}

fn one(field: Field, value: Value) -> Result<(), String> {
    let name = field.name.clone();
    check_fields(&[field], &json!({ name: value })).map_err(|e| e.0)
}

#[test]
fn text_and_long_text() {
    for kind in [FieldType::Text, FieldType::LongText] {
        assert!(one(field("f", kind), json!("ok")).is_ok());
        assert!(one(field("f", kind), json!("123456789")).is_err(), "too long");
        assert!(one(field("f", kind), json!(5)).is_err(), "not a string");
    }
    let mut required = field("f", FieldType::Text);
    required.required = true;
    assert!(one(required, json!("  ")).is_err(), "blank when required");
}

#[test]
fn code() {
    assert!(one(field("f", FieldType::Code), json!("a_b.c:-1")).is_ok());
    assert!(one(field("f", FieldType::Code), json!("has space")).is_err());
    assert!(one(field("f", FieldType::Code), json!("")).is_err());
    assert!(one(field("f", FieldType::Code), json!("a".repeat(9))).is_err());
}

#[test]
fn number() {
    let mut f = field("f", FieldType::Number);
    f.min = Some(-2);
    f.max = Some(5);
    assert!(one(f.clone(), json!(-2)).is_ok());
    assert!(one(f.clone(), json!(5)).is_ok());
    assert!(one(f.clone(), json!(6)).is_err());
    assert!(one(f.clone(), json!(-3)).is_err());
    assert!(one(f, json!(1.5)).is_err(), "a float is not an integer");
}

#[test]
fn money_and_duration_are_not_negative() {
    for kind in [FieldType::Money, FieldType::Duration] {
        assert!(one(field("f", kind), json!(0)).is_ok());
        assert!(one(field("f", kind), json!(12_345)).is_ok());
        assert!(one(field("f", kind), json!(-1)).is_err());
        assert!(one(field("f", kind), json!(1.0)).is_err());
        assert!(one(field("f", kind), json!("5")).is_err());
    }
}

#[test]
fn date_time() {
    assert!(one(field("f", FieldType::DateTime), json!(1_700_000_000)).is_ok());
    assert!(one(field("f", FieldType::DateTime), json!(-1)).is_err());
    assert!(one(field("f", FieldType::DateTime), json!("2026-01-01")).is_err());
}

#[test]
fn choice() {
    let mut f = field("f", FieldType::Choice);
    f.choices = Some(vec!["a".to_string(), "b-c".to_string()]);
    assert!(one(f.clone(), json!("b-c")).is_ok());
    assert!(one(f.clone(), json!("z")).is_err());
    assert!(one(f, json!(1)).is_err());
}

#[test]
fn the_object_rules() {
    let mut required = field("must", FieldType::Code);
    required.required = true;
    let fields = [required, field("may", FieldType::Code)];
    assert!(check_fields(&fields, &json!({ "must": "a" })).is_ok());
    assert!(check_fields(&fields, &json!({ "must": "a", "may": "b" })).is_ok());
    assert!(check_fields(&fields, &json!({ "may": "b" })).is_err(), "required is missing");
    assert!(check_fields(&fields, &json!({ "must": "a", "extra": 1 })).is_err(), "unknown key");
    assert!(check_fields(&fields, &json!([])).is_err(), "not an object");
    assert!(check_fields(&fields, &json!(null)).is_err());
    assert!(check_fields(&[], &json!({})).is_ok());
}

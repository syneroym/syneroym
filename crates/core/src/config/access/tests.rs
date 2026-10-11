use serde::{Deserialize, Serialize};

use super::CoordinatorAccess;
use crate::config::roles::CoordinatorRole;

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
struct Wrap {
    access: CoordinatorAccess,
}

#[test]
fn accepted_forms_parse_correctly() {
    let wrap = toml::from_str::<Wrap>(r#"access = "everyone""#).unwrap();
    assert_eq!(wrap.access, CoordinatorAccess::Everyone);

    let wrap = toml::from_str::<Wrap>(r#"access = "passport""#).unwrap();
    assert_eq!(wrap.access, CoordinatorAccess::Passport);

    let wrap = toml::from_str::<Wrap>(r#"access = ["a", "b"]"#).unwrap();
    assert_eq!(wrap.access, CoordinatorAccess::List(vec!["a".into(), "b".into()]));

    let wrap = toml::from_str::<Wrap>(r#"access = []"#).unwrap();
    assert_eq!(wrap.access, CoordinatorAccess::List(vec![]));

    let wrap = toml::from_str::<Wrap>(r#"access = { list = ["a"], passport = true }"#).unwrap();
    assert_eq!(wrap.access, CoordinatorAccess::ListOrPassport(vec!["a".into()]));
}

#[test]
fn missing_key_gives_everyone() {
    let role = toml::from_str::<CoordinatorRole>("").unwrap();
    assert_eq!(role.access, CoordinatorAccess::Everyone);
}

#[test]
fn typo_gives_error_naming_typo_and_expected() {
    let err = toml::from_str::<Wrap>(r#"access = "everyon""#).unwrap_err().to_string();
    assert!(err.contains("everyon"), "expected error to contain typo: {err}");
    assert!(err.contains("everyone"), "expected error to name expected values: {err}");
}

#[test]
fn unknown_string_gives_error() {
    let err = toml::from_str::<Wrap>(r#"access = "anyone""#).unwrap_err().to_string();
    assert!(err.contains("anyone"), "expected error to contain bad string: {err}");
    assert!(err.contains("everyone"), "expected error to name expected values: {err}");
}

#[test]
fn table_with_passport_false_gives_error_mentioning_list_form() {
    let err = toml::from_str::<Wrap>(r#"access = { list = ["a"], passport = false }"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("plain list form"), "expected error: {err}");
}

#[test]
fn table_missing_keys_gives_error() {
    let err = toml::from_str::<Wrap>(r#"access = { list = ["a"] }"#).unwrap_err().to_string();
    assert!(err.contains("missing field `passport`"), "expected error: {err}");

    let err = toml::from_str::<Wrap>(r#"access = { passport = true }"#).unwrap_err().to_string();
    assert!(err.contains("missing field `list`"), "expected error: {err}");
}

#[test]
fn table_unknown_key_gives_error() {
    let err = toml::from_str::<Wrap>(r#"access = { lists = ["a"], passport = true }"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown field `lists`"), "expected error: {err}");
}

#[test]
fn non_string_or_seq_or_map_gives_error() {
    let err = toml::from_str::<Wrap>(r#"access = 1"#).unwrap_err().to_string();
    assert!(err.contains("invalid type: integer"), "expected error: {err}");
}

#[test]
fn serialize_round_trip_for_all_variants() {
    let cases = vec![
        CoordinatorAccess::Everyone,
        CoordinatorAccess::Passport,
        CoordinatorAccess::List(vec!["endpoint1".into(), "endpoint2".into()]),
        CoordinatorAccess::ListOrPassport(vec!["endpoint1".into()]),
    ];

    for case in cases {
        let wrap = Wrap { access: case };
        let serialized = toml::to_string(&wrap).unwrap();
        let parsed = toml::from_str::<Wrap>(&serialized).unwrap();
        assert_eq!(wrap, parsed);
    }
}

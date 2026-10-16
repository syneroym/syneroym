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

#[test]
fn helper_methods_and_check_tls() {
    let everyone = CoordinatorAccess::Everyone;
    let passport = CoordinatorAccess::Passport;
    let list = CoordinatorAccess::List(vec!["ep1".into(), "ep2".into()]);
    let list_or_passport = CoordinatorAccess::ListOrPassport(vec!["ep1".into()]);

    assert!(!everyone.accepts_passports());
    assert!(passport.accepts_passports());
    assert!(!list.accepts_passports());
    assert!(list_or_passport.accepts_passports());

    assert_eq!(everyone.list(), <&[String]>::default());
    assert_eq!(passport.list(), <&[String]>::default());
    assert_eq!(list.list(), &["ep1".to_string(), "ep2".to_string()]);
    assert_eq!(list_or_passport.list(), &["ep1".to_string()]);

    assert!(everyone.check_tls(false).is_ok());
    assert!(everyone.check_tls(true).is_ok());
    assert!(list.check_tls(false).is_ok());
    assert!(list.check_tls(true).is_ok());

    assert!(passport.check_tls(false).is_err());
    assert!(passport.check_tls(true).is_ok());
    assert!(list_or_passport.check_tls(false).is_err());
    assert!(list_or_passport.check_tls(true).is_ok());
}

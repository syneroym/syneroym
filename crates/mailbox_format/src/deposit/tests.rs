use serde_json::json;

use super::*;
use crate::test_support::{Member, seal_between};

fn valid_deposit() -> Deposit {
    let (sealed, _) = seal_between(&Member::new(3_600), &Member::new(3_600));
    sealed.deposit
}

#[test]
fn deposit_id_is_32_lowercase_hex_and_round_trips() {
    let id = DepositId::random().unwrap();
    let text = id.to_string();
    assert_eq!(text.len(), 32);
    assert!(text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    assert_eq!(text.parse::<DepositId>().unwrap(), id);
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, format!("\"{text}\""));
    assert_eq!(serde_json::from_str::<DepositId>(&json).unwrap(), id);
}

#[test]
fn deposit_id_refuses_upper_case_and_wrong_length() {
    let text = "ab".repeat(16);
    assert!(text.parse::<DepositId>().is_ok());
    assert!(text.to_uppercase().parse::<DepositId>().is_err());
    assert!("ab".repeat(15).parse::<DepositId>().is_err());
    assert!("ab".repeat(17).parse::<DepositId>().is_err());
    assert!("zz".repeat(16).parse::<DepositId>().is_err());
}

#[test]
fn sealed_bytes_serialize_as_base64() {
    let sealed = SealedBytes::new(vec![1, 2, 3, 255]);
    assert_eq!(serde_json::to_string(&sealed).unwrap(), "\"AQID/w==\"");
    assert_eq!(serde_json::from_str::<SealedBytes>("\"AQID/w==\"").unwrap(), sealed);
    assert!(serde_json::from_str::<SealedBytes>("\"not base64!\"").is_err());
}

#[test]
fn sealed_bytes_over_the_limit_are_refused_while_parsing() {
    let at_limit = STANDARD.encode(vec![0u8; MAX_SEALED_BYTES]);
    let over = STANDARD.encode(vec![0u8; MAX_SEALED_BYTES + 1]);
    assert_eq!(SealedBytes::try_from(at_limit).unwrap().len(), MAX_SEALED_BYTES);
    assert!(matches!(
        SealedBytes::try_from(over),
        Err(SealedBytesParseError::TooLarge { max: MAX_SEALED_BYTES, .. })
    ));
    // A text far over the limit is refused before it is decoded.
    let huge = "A".repeat(10 * MAX_SEALED_BYTES);
    assert!(matches!(SealedBytes::try_from(huge), Err(SealedBytesParseError::TooLarge { .. })));
}

#[test]
fn deposit_check_refuses_zero_and_too_long_ttl() {
    let mut deposit = valid_deposit();
    assert_eq!(deposit.check(7_200), Ok(()));
    deposit.ttl_secs = 0;
    assert_eq!(deposit.check(7_200), Err(DepositError::ZeroTtl));
    deposit.ttl_secs = 7_201;
    assert_eq!(
        deposit.check(7_200),
        Err(DepositError::TtlTooLong { ttl_secs: 7_201, max_secs: 7_200 })
    );
    deposit.ttl_secs = 7_200;
    assert_eq!(deposit.check(7_200), Ok(()));
}

#[test]
fn deposit_check_refuses_a_destination_that_is_not_a_did_key() {
    let mut deposit = valid_deposit();
    deposit.destination = "did:web:example.com".to_string();
    assert!(matches!(deposit.check(7_200), Err(DepositError::NotADidKey(_))));
}

#[test]
fn deposit_check_refuses_sealed_bytes_shorter_than_the_overhead() {
    let mut deposit = valid_deposit();
    deposit.sealed = SealedBytes::new(vec![0u8; SEAL_OVERHEAD - 1]);
    assert_eq!(
        deposit.check(7_200),
        Err(DepositError::TooSmall { bytes: SEAL_OVERHEAD - 1, min: SEAL_OVERHEAD })
    );
    deposit.sealed = SealedBytes::new(vec![0u8; MAX_SEALED_BYTES + 1]);
    assert!(matches!(deposit.check(7_200), Err(DepositError::TooLarge { .. })));
}

#[test]
fn a_deposit_with_an_unknown_field_is_accepted() {
    let deposit = valid_deposit();
    let mut value = serde_json::to_value(&deposit).unwrap();
    value["from_a_newer_version"] = json!({"x": 1});
    let parsed: Deposit = serde_json::from_value(value).unwrap();
    assert_eq!(parsed, deposit);
}

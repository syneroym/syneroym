use syneroym_core::mailbox::codes;
use syneroym_rpc::RpcError;

use super::*;

#[test]
fn each_refusal_maps_to_its_own_code_and_kind() {
    let cases: Vec<(MailboxError, i32, &str)> = vec![
        (MailboxError::Unauthenticated, codes::UNAUTHENTICATED, "unauthenticated"),
        (MailboxError::NotAnInstance("bad".into()), codes::NOT_AN_INSTANCE, "not_an_instance"),
        (
            MailboxError::UnknownDestination("dest".into()),
            codes::UNKNOWN_DESTINATION,
            "unknown_destination",
        ),
        (MailboxError::TooLarge { bytes: 200, max: 100 }, codes::TOO_LARGE, "too_large"),
        (MailboxError::QueueFull("dest".into()), codes::QUEUE_FULL, "queue_full"),
        (MailboxError::RateLimited, codes::RATE_LIMITED, "rate_limited"),
        (MailboxError::MailboxFull, codes::MAILBOX_FULL, "mailbox_full"),
        (MailboxError::StaleInstance("dest".into()), codes::STALE_INSTANCE, "stale_instance"),
        (MailboxError::BadRecord("bad".into()), codes::BAD_RECORD, "bad_record"),
        (
            MailboxError::DuplicateDeposit("id1".into()),
            codes::DUPLICATE_DEPOSIT,
            "duplicate_deposit",
        ),
        (
            MailboxError::TtlTooLong { ttl_secs: 500, max_secs: 100 },
            codes::TTL_TOO_LONG,
            "ttl_too_long",
        ),
        (
            MailboxError::PassportRefused { code: "expired", reason: "old".into() },
            codes::PASSPORT_REFUSED,
            "passport_refused",
        ),
    ];

    for (err, expected_code, expected_kind) in cases {
        assert_eq!(err.kind(), expected_kind);
        let rpc_err: RpcError = err.into();
        assert_eq!(rpc_err.code(), expected_code);
        let data = rpc_err.data().expect("custom errors must carry data");
        assert_eq!(data["kind"], expected_kind);
    }

    let invalid: RpcError = MailboxError::InvalidParams("bad param".into()).into();
    assert_eq!(invalid.code(), -32602);

    let internal: RpcError = MailboxError::Internal("db failed".into()).into();
    assert_eq!(internal.code(), -32603);
}

#[test]
fn ttl_too_long_carries_max_ttl_secs() {
    let err = MailboxError::TtlTooLong { ttl_secs: 10_000, max_secs: 1_000 };
    let rpc_err: RpcError = err.into();
    assert_eq!(rpc_err.code(), codes::TTL_TOO_LONG);
    let data = rpc_err.data().unwrap();
    assert_eq!(data["kind"], "ttl_too_long");
    assert_eq!(data["max_ttl_secs"], 1_000);
}

#[test]
fn code_of_returns_standard_codes_for_invalid_params_and_internal() {
    assert_eq!(code_of(&MailboxError::InvalidParams("bad".into())), -32602);
    assert_eq!(code_of(&MailboxError::Internal("fail".into())), -32603);
}

#[test]
fn a_refused_passport_carries_its_code() {
    let err: MailboxError = RegisterError::Denied.into();
    let rpc_err: RpcError = err.into();
    assert_eq!(rpc_err.code(), codes::PASSPORT_REFUSED);
    let data = rpc_err.data().unwrap();
    assert_eq!(data["kind"], "passport_refused");
    assert_eq!(data["code"], "denied");
}

#[test]
fn a_lease_store_failure_is_an_internal_error() {
    let err: MailboxError = RegisterError::Store("disk".into()).into();
    assert_eq!(err, MailboxError::Internal("disk".into()));
    let rpc_err: RpcError = err.into();
    assert_eq!(rpc_err.code(), -32603);
}

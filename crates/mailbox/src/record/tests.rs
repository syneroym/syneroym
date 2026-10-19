use syneroym_core::dht_registry::EndpointType;

use super::*;
use crate::test_support::{Member, record};

#[test]
fn valid_service_record_is_accepted() {
    let member = Member::new(3600);
    let owner = member.instance();
    let mailbox_did = "did:key:zMailboxNode";
    let now = member.cert.issued_at_secs + 10;
    let rec = record(&member, mailbox_did);

    let checked = CheckedRecord::check(&rec, &owner, mailbox_did, now).unwrap();
    assert_eq!(checked.not_after, rec.info.not_after);
    assert!(checked.signed_at > 0);
}

#[test]
fn record_for_another_service_is_refused() {
    let member = Member::new(3600);
    let other_member = Member::new(3600);
    let owner = other_member.instance();
    let mailbox_did = "did:key:zMailboxNode";
    let now = member.cert.issued_at_secs + 10;
    let rec = record(&member, mailbox_did);

    let err = CheckedRecord::check(&rec, &owner, mailbox_did, now).unwrap_err();
    assert!(matches!(err, MailboxError::BadRecord(_)));
}

#[test]
fn non_service_record_is_refused() {
    let member = Member::new(3600);
    let owner = member.instance();
    let mailbox_did = "did:key:zMailboxNode";
    let now = member.cert.issued_at_secs + 10;
    let mut rec = record(&member, mailbox_did);
    rec.info.endpoint_type = EndpointType::Substrate;
    // Re-sign with Substrate endpoint type
    let resigned = rec.info.sign(&member.master).unwrap();

    let err = CheckedRecord::check(&resigned, &owner, mailbox_did, now).unwrap_err();
    assert!(matches!(err, MailboxError::BadRecord(_)));
}

#[test]
fn private_record_is_refused() {
    let member = Member::new(3600);
    let owner = member.instance();
    let mailbox_did = "did:key:zMailboxNode";
    let now = member.cert.issued_at_secs + 10;
    let mut rec = record(&member, mailbox_did);
    rec.info.is_private = true;
    let resigned = rec.info.sign(&member.master).unwrap();

    let err = CheckedRecord::check(&resigned, &owner, mailbox_did, now).unwrap_err();
    assert!(matches!(err, MailboxError::BadRecord(_)));
}

#[test]
fn wrong_mailbox_named_is_refused() {
    let member = Member::new(3600);
    let owner = member.instance();
    let mailbox_did = "did:key:zMailboxNode";
    let other_mailbox_did = "did:key:zOtherMailboxNode";
    let now = member.cert.issued_at_secs + 10;

    // Record with other mailbox
    let rec_other = record(&member, other_mailbox_did);
    let err = CheckedRecord::check(&rec_other, &owner, mailbox_did, now).unwrap_err();
    assert!(matches!(err, MailboxError::BadRecord(_)));

    // Record with no mailbox
    let mut rec_none = record(&member, mailbox_did);
    rec_none.info.mailbox = None;
    let resigned = rec_none.info.sign(&member.master).unwrap();
    let err2 = CheckedRecord::check(&resigned, &owner, mailbox_did, now).unwrap_err();
    assert!(matches!(err2, MailboxError::BadRecord(_)));
}

#[test]
fn tampered_record_fails_verification() {
    let member = Member::new(3600);
    let owner = member.instance();
    let mailbox_did = "did:key:zMailboxNode";
    let now = member.cert.issued_at_secs + 10;
    let mut rec = record(&member, mailbox_did);
    // Alter info without re-signing
    rec.info.nickname = Some("tampered".to_string());

    let err = CheckedRecord::check(&rec, &owner, mailbox_did, now).unwrap_err();
    assert!(matches!(err, MailboxError::BadRecord(_)));
}

#[test]
fn expired_record_is_refused() {
    let member = Member::new(3600);
    let owner = member.instance();
    let mailbox_did = "did:key:zMailboxNode";
    let now = 1_000_000;
    let mut rec = record(&member, mailbox_did);
    rec.info.not_after = now - 10;
    let resigned = rec.info.sign(&member.master).unwrap();

    let err = CheckedRecord::check(&resigned, &owner, mailbox_did, now).unwrap_err();
    assert!(matches!(err, MailboxError::BadRecord(_)));
}

#[test]
fn record_too_large_is_refused() {
    let member = Member::new(3600);
    let owner = member.instance();
    let mailbox_did = "did:key:zMailboxNode";
    let now = member.cert.issued_at_secs + 10;
    let mut rec = record(&member, mailbox_did);
    rec.pkarr_packet_hex = "00".repeat(5000);

    let err = CheckedRecord::check(&rec, &owner, mailbox_did, now).unwrap_err();
    match err {
        MailboxError::BadRecord(msg) => assert!(msg.contains("too large")),
        other => panic!("expected BadRecord too large, got {other:?}"),
    }
}

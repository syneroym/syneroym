use syneroym_identity::{DelegationCertificate, delegation::SCOPE_ROUTING};

use super::*;
use crate::{
    DepositId, OpenPolicy, Sender,
    message::seal_unchecked,
    open,
    test_support::{CERT_LIFE_SECS, Member, TTL_SECS, common_now, content, seal_between},
};

#[test]
fn every_reject_code_round_trips_through_its_string() {
    let mut seen = std::collections::HashSet::new();
    for code in RejectCode::ALL {
        assert_eq!(RejectCode::parse(code.as_str()), Some(code));
        assert!(seen.insert(code.as_str()), "{} is used twice", code.as_str());
    }
    assert_eq!(RejectCode::parse("unknown_code"), None);
    assert_eq!(RejectCode::parse("TOO_OLD"), None);
    assert_eq!(RejectCode::parse(""), None);
}

#[test]
fn only_the_three_sender_caused_codes_are_sender_faults() {
    let faults: Vec<RejectCode> =
        RejectCode::ALL.into_iter().filter(|code| code.is_sender_fault()).collect();
    assert_eq!(faults, [RejectCode::FromFuture, RejectCode::Revoked, RejectCode::NotEligible]);
    assert!(!RejectCode::BadCertificate.is_sender_fault());
}

#[test]
fn every_open_error_has_its_own_reject_code() {
    assert_eq!(OpenError::TooLarge { bytes: 2, max: 1 }.reject_code(), RejectCode::TooLarge);
    assert_eq!(OpenError::Undecryptable.reject_code(), RejectCode::Undecryptable);
    assert_eq!(OpenError::Malformed(String::new()).reject_code(), RejectCode::Malformed);
    assert_eq!(OpenError::BadSignature.reject_code(), RejectCode::BadSignature);

    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let routing = DelegationCertificate::issue(
        &sender.master,
        sender.instance.public_key(),
        CERT_LIFE_SECS,
        SCOPE_ROUTING.to_string(),
    )
    .unwrap();
    let now = common_now(&[&sender, &receiver]);
    let id = receiver.service_id();
    let deposit_id = DepositId::random().unwrap();
    let sealed = seal_unchecked(
        &content(),
        receiver.recipient(&id),
        Sender { instance: &sender.instance, certificate: &routing },
        deposit_id,
        TTL_SECS,
        now,
    )
    .unwrap();
    let error = open(
        &receiver.mailbox,
        &id,
        &deposit_id,
        &sealed.deposit.sealed,
        OpenPolicy::default(),
        now,
    )
    .unwrap_err();
    assert_eq!(error.reject_code(), RejectCode::BadCertificate);
    assert!(error.to_string().contains("bad_certificate"));
}

#[test]
fn no_error_text_shows_message_content() {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, now) = seal_between(&sender, &receiver);
    let error = open(
        &receiver.mailbox,
        &receiver.service_id(),
        &DepositId::random().unwrap(),
        &sealed.deposit.sealed,
        OpenPolicy::default(),
        now,
    )
    .unwrap_err();
    let text = format!("{error} {error:?}");
    assert!(!text.contains("hello world"));
}

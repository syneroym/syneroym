use syneroym_identity::{Identity, delegation::SCOPE_ROUTING};

use super::*;
use crate::{
    DepositId, OpenError, OpenPolicy, RefusedMessage, Sealed, SentDeposit,
    message::{Recipient, Sender, seal_unchecked},
    open, seal,
    test_support::{CERT_LIFE_SECS, Member, TTL_SECS, common_now, content, seal_between},
};

const DAY: u64 = 24 * 3600;
const SKEW: u64 = DAY;

/// A sender, a receiver, the deposit, the message the receiver opened, and a
/// time inside both certificates.
struct Exchange {
    sender: Member,
    receiver: Member,
    sealed: Sealed,
    opened: OpenedMessage,
    now: u64,
}

fn exchange() -> Exchange {
    let (sender, receiver) = (Member::new(CERT_LIFE_SECS), Member::new(CERT_LIFE_SECS));
    let (sealed, now) = seal_between(&sender, &receiver);
    let opened = open_for(&receiver, &sealed, now).unwrap();
    Exchange { sender, receiver, sealed, opened, now }
}

fn open_for(receiver: &Member, sealed: &Sealed, now: u64) -> Result<OpenedMessage, OpenError> {
    open(
        &receiver.mailbox,
        &receiver.service_id(),
        &sealed.deposit.deposit_id,
        &sealed.deposit.sealed,
        OpenPolicy::default(),
        now,
    )
}

fn delivered(x: &Exchange) -> Receipt {
    x.opened.sign_delivered(&x.receiver.instance, &x.receiver.cert, x.now).unwrap()
}

/// A rejection signed by hand with any code, to test what `verify_signed`
/// refuses. `sign` itself would refuse some of these codes.
fn hand_signed_rejection(x: &Exchange, code: &str) -> Receipt {
    let statement = receipt_statement(
        &x.sealed.sent.deposit_id,
        &x.sealed.sent.sender_master_did,
        &x.sealed.sent.message_hash,
        "rejected",
        Some(code),
        x.now,
    );
    let signature = x.receiver.instance.sign_json(&statement).unwrap();
    let proof = ReceiverProof { receiver_certificate: x.receiver.cert.clone(), signature };
    Receipt::Rejected { at_secs: x.now, code: code.to_string(), proof: Some(proof) }
}

fn verify(receipt: &Receipt, sent: &SentDeposit, now: u64) -> Result<SignedOutcome, ReceiptError> {
    receipt.verify_signed(sent, SKEW, now)
}

#[test]
fn a_delivered_receipt_verifies_against_the_sent_deposit() {
    let x = exchange();
    let receipt = delivered(&x);
    assert_eq!(verify(&receipt, &x.sealed.sent, x.now), Ok(SignedOutcome::Delivered));
}

#[test]
fn a_delivered_receipt_for_another_deposit_fails() {
    let x = exchange();
    let id = x.receiver.service_id();
    let second =
        seal(&content(), x.receiver.recipient(&id), x.sender.sender(), TTL_SECS, x.now + 1)
            .unwrap();
    let receipt = delivered(&x);
    assert_eq!(verify(&receipt, &second.sent, x.now), Err(ReceiptError::BadSignature));
}

#[test]
fn a_delivered_receipt_from_another_service_fails() {
    let x = exchange();
    let other = Member::new(CERT_LIFE_SECS);
    let at = common_now(&[&x.receiver, &other]);
    let receipt = x.opened.sign_delivered(&other.instance, &other.cert, at).unwrap();
    assert_eq!(verify(&receipt, &x.sealed.sent, at), Err(ReceiptError::WrongReceiver));
}

#[test]
fn a_delivered_receipt_with_a_changed_time_fails() {
    let x = exchange();
    let Receipt::Delivered { at_secs, proof } = delivered(&x) else { panic!("not delivered") };
    let changed = Receipt::Delivered { at_secs: at_secs + 1, proof };
    assert_eq!(verify(&changed, &x.sealed.sent, x.now), Err(ReceiptError::BadSignature));
}

#[test]
fn a_delivered_receipt_signed_after_the_certificate_expired_fails() {
    let x = exchange();
    let expired_at = x.receiver.cert.expires_at_secs;
    let refused = x.opened.sign_delivered(&x.receiver.instance, &x.receiver.cert, expired_at);
    assert!(matches!(refused, Err(ReceiptError::BadCertificate(_))));

    let statement = receipt_statement(
        &x.sealed.sent.deposit_id,
        &x.sealed.sent.sender_master_did,
        &x.sealed.sent.message_hash,
        "delivered",
        None,
        expired_at,
    );
    let proof = ReceiverProof {
        receiver_certificate: x.receiver.cert.clone(),
        signature: x.receiver.instance.sign_json(&statement).unwrap(),
    };
    let by_hand = Receipt::Delivered { at_secs: expired_at, proof };
    assert!(matches!(
        verify(&by_hand, &x.sealed.sent, expired_at),
        Err(ReceiptError::BadCertificate(_))
    ));
}

#[test]
fn a_receipt_from_the_future_fails() {
    let x = exchange();
    let receipt = delivered(&x);
    let long_ago = x.now - SKEW - 1;
    assert_eq!(verify(&receipt, &x.sealed.sent, long_ago), Err(ReceiptError::FromFuture));
    assert!(verify(&receipt, &x.sealed.sent, x.now - SKEW).is_ok());
}

#[test]
fn a_delivery_of_a_mailbox_made_message_under_the_same_deposit_id_fails() {
    // An honest sender deposits; the mailbox drops it, seals its own message under
    // that sender's deposit id, lets the receiver deliver it, and shows the sender
    // the receipt.
    let x = exchange();
    let mailbox_member = Member::new(CERT_LIFE_SECS);
    let now = common_now(&[&x.receiver, &mailbox_member]);
    let id = x.receiver.service_id();
    let fake = seal_unchecked(
        &content(),
        x.receiver.recipient(&id),
        mailbox_member.sender(),
        x.sealed.sent.deposit_id,
        TTL_SECS,
        now,
    )
    .unwrap();
    let opened = open_for(&x.receiver, &fake, now).unwrap();
    let receipt = opened.sign_delivered(&x.receiver.instance, &x.receiver.cert, now).unwrap();
    assert_eq!(verify(&receipt, &x.sealed.sent, now), Err(ReceiptError::BadSignature));
}

#[test]
fn a_signed_rejection_for_a_sender_caused_code_verifies() {
    let x = exchange();
    for code in [RejectCode::Revoked, RejectCode::NotEligible] {
        let receipt =
            x.opened.sign_rejection(&x.receiver.instance, &x.receiver.cert, code, x.now).unwrap();
        assert_eq!(verify(&receipt, &x.sealed.sent, x.now), Ok(SignedOutcome::Rejected(code)));
    }
}

/// Builds a message with `sent_at`, opens it at `open_at` and returns the
/// refusal and the sender's record of the deposit.
fn refused_exchange(
    x: &Exchange,
    sender: Sender<'_>,
    sent_at: u64,
    open_at: u64,
) -> (RefusedMessage, SentDeposit) {
    let id = x.receiver.service_id();
    let recipient = Recipient { service_id: &id, mailbox_key: &x.receiver.mailbox_public };
    let sealed = seal_unchecked(
        &content(),
        recipient,
        sender,
        DepositId::random().unwrap(),
        TTL_SECS,
        sent_at,
    )
    .unwrap();
    match open_for(&x.receiver, &sealed, open_at) {
        Err(OpenError::Refused(refused)) => (*refused, sealed.sent),
        other => panic!("expected a refused message, got {other:?}"),
    }
}

#[test]
fn a_refused_message_gets_a_signed_or_an_unsigned_receipt_by_its_code() {
    let x = exchange();
    let sent_at = x.sender.at();
    let ack_at = x.now;

    let (future, sent) = refused_exchange(&x, x.sender.sender(), sent_at, sent_at - 2 * DAY);
    assert_eq!(future.code, RejectCode::FromFuture);
    let receipt = future.receipt(&x.receiver.instance, &x.receiver.cert, ack_at).unwrap();
    assert_eq!(
        verify(&receipt, &sent, ack_at),
        Ok(SignedOutcome::Rejected(RejectCode::FromFuture))
    );

    let (old, _) = refused_exchange(&x, x.sender.sender(), sent_at, sent_at + 9 * DAY);
    assert_eq!(old.code, RejectCode::TooOld);
    let receipt = old.receipt(&x.receiver.instance, &x.receiver.cert, ack_at).unwrap();
    assert_eq!(receipt, Receipt::unsigned_rejection(RejectCode::TooOld, ack_at));

    let routing = DelegationCertificate::issue(
        &x.sender.master,
        x.sender.instance.public_key(),
        CERT_LIFE_SECS,
        SCOPE_ROUTING.to_string(),
    )
    .unwrap();
    let by_routing = Sender { instance: &x.sender.instance, certificate: &routing };
    let (bad, _) = refused_exchange(&x, by_routing, sent_at, sent_at);
    assert_eq!(bad.code, RejectCode::BadCertificate);
    let receipt = bad.receipt(&x.receiver.instance, &x.receiver.cert, ack_at).unwrap();
    assert_eq!(receipt, Receipt::unsigned_rejection(RejectCode::BadCertificate, ack_at));
}

#[test]
fn no_code_the_mailbox_could_cause_can_be_signed() {
    let x = exchange();
    for code in [
        RejectCode::Malformed,
        RejectCode::Undecryptable,
        RejectCode::BadSignature,
        RejectCode::TooLarge,
        RejectCode::BadCertificate,
        RejectCode::TooOld,
        RejectCode::TooManyClaims,
    ] {
        let result = x.opened.sign_rejection(&x.receiver.instance, &x.receiver.cert, code, x.now);
        assert_eq!(result, Err(ReceiptError::NotSignable), "{code:?}");
    }
}

#[test]
fn a_signed_rejection_with_a_code_the_mailbox_could_cause_is_not_final() {
    let x = exchange();
    let receipt = hand_signed_rejection(&x, "too_old");
    assert_eq!(verify(&receipt, &x.sealed.sent, x.now), Err(ReceiptError::NotSignable));
}

#[test]
fn a_forged_certificate_cannot_get_a_signed_rejection() {
    // The mailbox works with a past host of the receiver, so it can read
    // the honest sender's message. It signs the same statement with its own key and
    // attaches a certificate that names the honest sender's master.
    let x = exchange();
    let sent_at = x.sender.cert.issued_at_secs + 60;
    let id = x.receiver.service_id();
    let recipient = Recipient { service_id: &id, mailbox_key: &x.receiver.mailbox_public };
    let deposit_id = DepositId::random().unwrap();
    let honest =
        seal_unchecked(&content(), recipient, x.sender.sender(), deposit_id, TTL_SECS, sent_at)
            .unwrap();

    let attacker = Member::new(CERT_LIFE_SECS);
    let mut forged = attacker.cert.clone();
    forged.master_did = x.sender.cert.master_did.clone();
    let by_attacker = Sender { instance: &attacker.instance, certificate: &forged };
    let fake =
        seal_unchecked(&content(), recipient, by_attacker, deposit_id, TTL_SECS, sent_at).unwrap();
    assert_eq!(fake.sent.message_hash, honest.sent.message_hash);

    let refused = match open_for(&x.receiver, &fake, sent_at) {
        Err(OpenError::Refused(refused)) => *refused,
        other => panic!("expected a refused message, got {other:?}"),
    };
    assert_eq!(refused.code, RejectCode::BadCertificate);
    let receipt = refused.receipt(&x.receiver.instance, &x.receiver.cert, x.now).unwrap();
    assert_eq!(receipt, Receipt::unsigned_rejection(RejectCode::BadCertificate, x.now));
    assert_eq!(verify(&receipt, &honest.sent, x.now), Err(ReceiptError::Unsigned));

    let statement = receipt_statement(
        &honest.sent.deposit_id,
        &honest.sent.sender_master_did,
        &honest.sent.message_hash,
        "rejected",
        Some("bad_certificate"),
        x.now,
    );
    let proof = ReceiverProof {
        receiver_certificate: x.receiver.cert.clone(),
        signature: x.receiver.instance.sign_json(&statement).unwrap(),
    };
    let signed = Receipt::Rejected {
        at_secs: x.now,
        code: "bad_certificate".to_string(),
        proof: Some(proof),
    };
    assert_eq!(verify(&signed, &honest.sent, x.now), Err(ReceiptError::NotSignable));
}

#[test]
fn a_rejection_with_a_changed_code_fails() {
    let x = exchange();
    let signed = x
        .opened
        .sign_rejection(&x.receiver.instance, &x.receiver.cert, RejectCode::Revoked, x.now)
        .unwrap();
    let Receipt::Rejected { at_secs, proof, .. } = signed else { panic!("not rejected") };
    let changed = Receipt::Rejected { at_secs, code: "not_eligible".to_string(), proof };
    assert_eq!(verify(&changed, &x.sealed.sent, x.now), Err(ReceiptError::BadSignature));
}

#[test]
fn an_unknown_reject_code_is_kept_and_is_never_final() {
    let x = exchange();
    let receipt = hand_signed_rejection(&x, "rate_limited");
    let json = serde_json::to_string(&receipt).unwrap();
    assert!(json.contains("\"code\":\"rate_limited\""));
    let parsed: Receipt = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, receipt);
    assert_eq!(verify(&parsed, &x.sealed.sent, x.now), Err(ReceiptError::NotSignable));
}

#[test]
fn stored_expired_unknown_and_unsigned_rejections_are_unsigned() {
    let x = exchange();
    let receipts = [
        Receipt::Stored { at_secs: 5 },
        Receipt::Expired { at_secs: 5 },
        Receipt::Unknown,
        Receipt::unsigned_rejection(RejectCode::Revoked, 5),
        Receipt::unsigned_rejection(RejectCode::TooManyClaims, 5),
    ];
    for receipt in receipts {
        assert_eq!(verify(&receipt, &x.sealed.sent, x.now), Err(ReceiptError::Unsigned));
    }
}

#[test]
fn an_unknown_receipt_state_reads_as_unknown() {
    let parsed: Receipt = serde_json::from_str(r#"{"state":"held","at_secs":5}"#).unwrap();
    assert_eq!(parsed, Receipt::Unknown);
}

#[test]
fn receipts_serialize_with_a_state_tag() {
    let json = |receipt: &Receipt| serde_json::to_string(receipt).unwrap();
    assert_eq!(json(&Receipt::Stored { at_secs: 5 }), r#"{"state":"stored","at_secs":5}"#);
    assert_eq!(json(&Receipt::Expired { at_secs: 5 }), r#"{"state":"expired","at_secs":5}"#);
    assert_eq!(
        json(&Receipt::unsigned_rejection(RejectCode::TooOld, 5)),
        r#"{"state":"rejected","at_secs":5,"code":"too_old"}"#
    );
    assert_eq!(json(&Receipt::Unknown), r#"{"state":"unknown"}"#);

    let x = exchange();
    let signed = serde_json::to_value(delivered(&x)).unwrap();
    assert_eq!(signed["state"], "delivered");
    assert!(signed["proof"]["signature"].is_string());
    assert!(signed["proof"]["receiver_certificate"]["master_did"].is_string());
}

#[test]
fn a_receipt_with_an_unknown_field_still_parses() {
    let x = exchange();
    let mut value = serde_json::to_value(delivered(&x)).unwrap();
    value["added_by_a_newer_version"] = serde_json::json!(1);
    value["proof"]["added_by_a_newer_version"] = serde_json::json!(1);
    let parsed: Receipt = serde_json::from_value(value).unwrap();
    assert_eq!(verify(&parsed, &x.sealed.sent, x.now), Ok(SignedOutcome::Delivered));
}

#[test]
fn signing_refuses_an_instance_key_that_does_not_match_the_certificate() {
    let x = exchange();
    let other = Identity::generate().unwrap();
    let result = x.opened.sign_delivered(&other, &x.receiver.cert, x.now);
    assert_eq!(result, Err(ReceiptError::SignerMismatch));
}

#[test]
fn a_receipt_signed_before_the_send_time_fails() {
    // A past host of the receiver still holds an old instance key whose
    // certificate ended before the message was sent.
    let sender = Member::new(3 * DAY);
    let old_host = Member::new(5);
    let sent_at = old_host.cert.expires_at_secs + DAY + 10;
    let id = old_host.service_id();
    let sealed = seal_unchecked(
        &content(),
        old_host.recipient(&id),
        sender.sender(),
        DepositId::random().unwrap(),
        TTL_SECS,
        sent_at,
    )
    .unwrap();
    let opened = open_for(&old_host, &sealed, sent_at).unwrap();
    let old_time = old_host.cert.issued_at_secs + 2;
    let receipt = opened.sign_delivered(&old_host.instance, &old_host.cert, old_time).unwrap();
    assert_eq!(verify(&receipt, &sealed.sent, sent_at), Err(ReceiptError::BeforeSend));
}

#[test]
fn a_known_state_with_a_missing_field_does_not_parse() {
    // The caller treats such an answer like `expired`; see the `Receipt`
    // doc comment.
    for text in [
        r#"{"state":"delivered","at_secs":5}"#,
        r#"{"state":"stored"}"#,
        r#"{"state":"rejected","at_secs":5}"#,
    ] {
        assert!(serde_json::from_str::<Receipt>(text).is_err(), "{text}");
    }
}

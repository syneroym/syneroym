use syneroym_core::mailbox::{AckEntry, AckOutcome, QueueLimits};
use syneroym_identity::DelegationCertificate;
use syneroym_mailbox_format::{DepositId, Receipt, ReceiverProof, RejectCode, SealedBytes, open};

use crate::{
    error::MailboxError,
    record::CheckedRecord,
    test_support::{Member, record, sealed_deposit, small_limits, store_with},
};

const NODE_DID: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

#[test]
fn claim_returns_messages_oldest_first_up_to_the_batch_and_byte_limit() {
    let mut limits = small_limits();
    limits.queue.max_messages = 10;
    limits.queue.max_bytes = 100_000;
    let store = store_with(limits);

    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let d1 = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    let d2 = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    let d3 = sealed_deposit(&sender, &receiver, 3600, now).deposit;

    store.deposit(&sender.service_id(), &d1, now).unwrap();
    store.deposit(&sender.service_id(), &d2, now + 1).unwrap();
    store.deposit(&sender.service_id(), &d3, now + 2).unwrap();

    let claimed = store.claim(&receiver.instance(), 2, now + 5).unwrap();
    assert_eq!(claimed.messages.len(), 2);
    assert_eq!(claimed.messages[0].deposit_id, d1.deposit_id);
    assert_eq!(claimed.messages[1].deposit_id, d2.deposit_id);
    assert_eq!(claimed.visible_again_at_secs, now + 5 + 10);
}

#[test]
fn a_claimed_message_is_hidden_until_its_visibility_time_ends() {
    let store = store_with(small_limits());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    store.deposit(&sender.service_id(), &dep, now).unwrap();

    let c1 = store.claim(&receiver.instance(), 10, now).unwrap();
    assert_eq!(c1.messages.len(), 1);

    let c2 = store.claim(&receiver.instance(), 10, now + 5).unwrap();
    assert_eq!(c2.messages.len(), 0);

    let c3 = store.claim(&receiver.instance(), 10, now + 11).unwrap();
    assert_eq!(c3.messages.len(), 1);
    assert_eq!(c3.messages[0].claim_count, 2);
}

#[test]
fn claim_without_a_queue_is_unknown_destination() {
    let store = store_with(small_limits());
    let receiver = Member::new(3600);
    let res = store.claim(&receiver.instance(), 10, receiver.cert.issued_at_secs + 10);

    assert!(matches!(res, Err(MailboxError::UnknownDestination(_))));
}

#[test]
fn five_claims_without_an_ack_close_the_message_unsigned() {
    let store = store_with(small_limits());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    let dep_id = dep.deposit_id;
    store.deposit(&sender.service_id(), &dep, now).unwrap();

    let mut t = now;
    for _ in 0..5 {
        let c = store.claim(&receiver.instance(), 10, t).unwrap();
        assert_eq!(c.messages.len(), 1);
        t += 11;
    }

    let c6 = store.claim(&receiver.instance(), 10, t).unwrap();
    assert_eq!(c6.messages.len(), 0);

    let st = store.status(&sender.service_id(), &[dep_id], t).unwrap();
    assert_eq!(st[0].receipt, Receipt::unsigned_rejection(RejectCode::TooManyClaims, t));
}

#[test]
fn an_ack_during_the_fifth_visibility_window_succeeds() {
    let store = store_with(small_limits());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let sealed = sealed_deposit(&sender, &receiver, 3600, now);
    let dep_id = sealed.deposit.deposit_id;
    store.deposit(&sender.service_id(), &sealed.deposit, now).unwrap();

    let mut t = now;
    // 4 previous claims after visibility expirations
    for _ in 0..4 {
        let c = store.claim(&receiver.instance(), 10, t).unwrap();
        assert_eq!(c.messages.len(), 1);
        t += 11;
    }
    // 5th claim at time t (visibility_secs = 10, so claimed_until = t + 10)
    let c5 = store.claim(&receiver.instance(), 10, t).unwrap();
    assert_eq!(c5.messages.len(), 1);

    // During the 5th visibility window (t + 5 < t + 10):
    // 1. A status check shows message is NOT closed with TooManyClaims
    let st_during = store.status(&sender.service_id(), &[dep_id], t + 5).unwrap();
    assert_eq!(st_during[0].receipt, Receipt::Stored { at_secs: now });

    // 2. An ack during this window succeeds
    let opened = open(
        &receiver.mailbox,
        &receiver.service_id(),
        &dep_id,
        &c5.messages[0].sealed,
        syneroym_mailbox_format::OpenPolicy { max_ttl_secs: 86400, max_clock_skew_secs: 3600 },
        t + 5,
    )
    .unwrap();
    let delivered = opened.sign_delivered(&receiver.instance, &receiver.cert, t + 5).unwrap();
    let ack_res = store
        .ack(
            &receiver.instance(),
            &[AckEntry {
                sender: sender.service_id(),
                deposit_id: dep_id,
                receipt: delivered.clone(),
            }],
            t + 5,
        )
        .unwrap();
    assert_eq!(ack_res, vec![AckOutcome::Stored]);

    // Status is Delivered, not rejected with TooManyClaims
    let st_after = store.status(&sender.service_id(), &[dep_id], t + 15).unwrap();
    assert_eq!(st_after[0].receipt, delivered);
}

#[test]
fn an_ack_stores_the_receipt_as_sent_and_removes_the_body() {
    let store = store_with(small_limits());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let sealed = sealed_deposit(&sender, &receiver, 3600, now);
    store.deposit(&sender.service_id(), &sealed.deposit, now).unwrap();

    let c = store.claim(&receiver.instance(), 10, now).unwrap();
    let opened = open(
        &receiver.mailbox,
        &receiver.service_id(),
        &c.messages[0].deposit_id,
        &c.messages[0].sealed,
        syneroym_mailbox_format::OpenPolicy { max_ttl_secs: 86400, max_clock_skew_secs: 3600 },
        now,
    )
    .unwrap();

    let delivered = opened.sign_delivered(&receiver.instance, &receiver.cert, now).unwrap();

    let ack_out = store
        .ack(
            &receiver.instance(),
            &[AckEntry {
                sender: sender.service_id(),
                deposit_id: sealed.deposit.deposit_id,
                receipt: delivered.clone(),
            }],
            now,
        )
        .unwrap();
    assert_eq!(ack_out, vec![AckOutcome::Stored]);

    let st = store.status(&sender.service_id(), &[sealed.deposit.deposit_id], now).unwrap();
    assert_eq!(st[0].receipt, delivered);
    assert!(delivered.verify_signed(&sealed.sent, 3600, now).is_ok());

    let c2 = store.claim(&receiver.instance(), 10, now + 15).unwrap();
    assert_eq!(c2.messages.len(), 0);
}

#[test]
fn an_ack_of_a_stored_or_unknown_receipt_is_refused() {
    let store = store_with(small_limits());
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let refused_ack = store
        .ack(
            &receiver.instance(),
            &[
                AckEntry {
                    sender: "did:key:dummy".to_string(),
                    deposit_id: DepositId::random().unwrap(),
                    receipt: Receipt::Stored { at_secs: now },
                },
                AckEntry {
                    sender: "did:key:dummy".to_string(),
                    deposit_id: DepositId::random().unwrap(),
                    receipt: Receipt::Unknown,
                },
            ],
            now,
        )
        .unwrap();
    assert_eq!(refused_ack, vec![AckOutcome::Refused, AckOutcome::Refused]);
}

#[test]
fn an_oversized_receipt_is_refused() {
    let store = store_with(small_limits());
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let cert = DelegationCertificate::issue(
        &receiver.master,
        receiver.instance.public_key(),
        3600,
        "x".repeat(5000),
    )
    .unwrap();
    let oversized_receipt = Receipt::Delivered {
        at_secs: now,
        proof: ReceiverProof { receiver_certificate: cert, signature: "sig".to_string() },
    };

    let outcomes = store
        .ack(
            &receiver.instance(),
            &[AckEntry {
                sender: "did:key:dummy".to_string(),
                deposit_id: DepositId::random().unwrap(),
                receipt: oversized_receipt,
            }],
            now,
        )
        .unwrap();
    assert_eq!(outcomes, vec![AckOutcome::Refused]);
}

#[test]
fn an_ack_for_another_queues_message_is_not_found() {
    let store = store_with(small_limits());
    let s = Member::new(3600);
    let r1 = Member::new(3600);
    let r2 = Member::new(3600);
    let now = r1.cert.issued_at_secs + 10;

    let rec1 = record(&r1, NODE_DID);
    let c1 = CheckedRecord::check(&rec1, &r1.instance(), NODE_DID, now).unwrap();
    store.register(&r1.instance(), &c1, now).unwrap();

    let rec2 = record(&r2, NODE_DID);
    let c2 = CheckedRecord::check(&rec2, &r2.instance(), NODE_DID, now).unwrap();
    store.register(&r2.instance(), &c2, now).unwrap();

    let dep = sealed_deposit(&s, &r1, 3600, now).deposit;
    store.deposit(&s.service_id(), &dep, now).unwrap();

    let outcomes = store
        .ack(
            &r2.instance(),
            &[AckEntry {
                sender: s.service_id(),
                deposit_id: dep.deposit_id,
                receipt: Receipt::unsigned_rejection(RejectCode::Malformed, now),
            }],
            now,
        )
        .unwrap();
    assert_eq!(outcomes, vec![AckOutcome::NotFound]);
}

#[test]
fn an_ack_after_eviction_is_not_found() {
    let mut limits = small_limits();
    limits.queue = QueueLimits { max_bytes: 10_000, max_messages: 5, max_ttl_secs: 86400 };
    let store = store_with(limits);

    let s1 = Member::new(3600);
    let s2 = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let d1 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    store.deposit(&s1.service_id(), &d1, now).unwrap();

    let mut d2 = sealed_deposit(&s2, &receiver, 3600, now).deposit;
    let padded = vec![42u8; 9500];
    d2.sealed = SealedBytes::try_from(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        padded,
    ))
    .unwrap();
    store.deposit(&s2.service_id(), &d2, now + 1).unwrap();

    let outcomes = store
        .ack(
            &receiver.instance(),
            &[AckEntry {
                sender: s1.service_id(),
                deposit_id: d1.deposit_id,
                receipt: Receipt::unsigned_rejection(RejectCode::Malformed, now),
            }],
            now + 2,
        )
        .unwrap();
    assert_eq!(outcomes, vec![AckOutcome::NotFound]);

    let st = store.status(&s1.service_id(), &[d1.deposit_id], now + 3).unwrap();
    assert_eq!(st[0].receipt, Receipt::Expired { at_secs: now + 1 });
}

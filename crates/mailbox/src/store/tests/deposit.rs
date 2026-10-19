use base64::{Engine, engine::general_purpose::STANDARD};
use syneroym_core::mailbox::{AckEntry, QueueLimits};
use syneroym_mailbox_format::{DepositId, Receipt, SealedBytes, open};

use crate::{
    error::MailboxError,
    record::CheckedRecord,
    test_support::{Member, record, sealed_deposit, small_limits, store_with},
};

const NODE_DID: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

#[test]
fn a_deposit_to_an_unknown_destination_is_refused() {
    let store = store_with(small_limits());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;
    let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;

    let res = store.deposit(&sender.service_id(), &dep, now);
    assert!(matches!(res, Err(MailboxError::UnknownDestination(_))));
}

#[test]
fn a_deposit_is_stored_and_its_receipt_is_stored() {
    let store = store_with(small_limits());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    let dep_id = dep.deposit_id;
    let outcome = store.deposit(&sender.service_id(), &dep, now).unwrap();
    assert_eq!(outcome.receipt, Receipt::Stored { at_secs: now });
    assert_eq!(outcome.evicted, 0);

    let statuses = store.status(&sender.service_id(), &[dep_id], now).unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].receipt, Receipt::Stored { at_secs: now });
}

#[test]
fn the_same_deposit_twice_answers_the_first_time() {
    let store = store_with(small_limits());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    let out1 = store.deposit(&sender.service_id(), &dep, now).unwrap();
    let out2 = store.deposit(&sender.service_id(), &dep, now + 10).unwrap();

    assert_eq!(out1.receipt, Receipt::Stored { at_secs: now });
    assert_eq!(out2.receipt, Receipt::Stored { at_secs: now });
}

#[test]
fn the_same_id_with_other_content_is_a_duplicate() {
    let store = store_with(small_limits());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let dep1 = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    store.deposit(&sender.service_id(), &dep1, now).unwrap();

    let mut dep2 = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    dep2.deposit_id = dep1.deposit_id;

    let res = store.deposit(&sender.service_id(), &dep2, now + 1);
    assert!(matches!(res, Err(MailboxError::DuplicateDeposit(_))));
}

#[test]
fn an_identical_retry_after_delivery_returns_delivered_receipt() {
    let store = store_with(small_limits());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let sealed = sealed_deposit(&sender, &receiver, 3600, now);
    store.deposit(&sender.service_id(), &sealed.deposit, now).unwrap();

    let claimed = store.claim(&receiver.instance(), 10, now).unwrap();
    assert_eq!(claimed.messages.len(), 1);

    let opened = open(
        &receiver.mailbox,
        &receiver.service_id(),
        &claimed.messages[0].deposit_id,
        &claimed.messages[0].sealed,
        syneroym_mailbox_format::OpenPolicy { max_ttl_secs: 86400, max_clock_skew_secs: 3600 },
        now,
    )
    .unwrap();

    let delivered = opened.sign_delivered(&receiver.instance, &receiver.cert, now).unwrap();

    store
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

    // An identical retry after delivery returns the current stored receipt
    // (Delivered)
    let retry = store.deposit(&sender.service_id(), &sealed.deposit, now + 10).unwrap();
    assert_eq!(retry.receipt, delivered);

    // But reusing the same id with a different deposit is refused as duplicate
    let mut different_deposit = sealed.deposit.clone();
    different_deposit.ttl_secs += 60;
    let res = store.deposit(&sender.service_id(), &different_deposit, now + 10);
    assert!(matches!(res, Err(MailboxError::DuplicateDeposit(_))));
}

#[test]
fn two_senders_may_use_the_same_deposit_id() {
    let store = store_with(small_limits());
    let s1 = Member::new(3600);
    let s2 = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let dep_id = DepositId::random().unwrap();
    let mut d1 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    d1.deposit_id = dep_id;
    let mut d2 = sealed_deposit(&s2, &receiver, 3600, now).deposit;
    d2.deposit_id = dep_id;

    let res1 = store.deposit(&s1.service_id(), &d1, now);
    let res2 = store.deposit(&s2.service_id(), &d2, now);
    assert!(res1.is_ok());
    assert!(res2.is_ok());
}

#[test]
fn a_ttl_above_the_queue_limit_is_ttl_too_long() {
    let mut limits = small_limits();
    limits.queue.max_ttl_secs = 600;
    let store = store_with(limits);

    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let dep = sealed_deposit(&sender, &receiver, 1200, now).deposit;
    let res = store.deposit(&sender.service_id(), &dep, now);
    assert_eq!(res, Err(MailboxError::TtlTooLong { ttl_secs: 1200, max_secs: 600 }));
}

#[test]
fn a_full_queue_evicts_the_largest_sender_first() {
    let mut limits = small_limits();
    limits.queue = QueueLimits { max_bytes: 10_000, max_messages: 5, max_ttl_secs: 86400 };
    let store = store_with(limits);

    let s1 = Member::new(3600);
    let s2 = Member::new(3600);
    let s3 = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let d1_1 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    let d1_2 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    let d2 = sealed_deposit(&s2, &receiver, 3600, now).deposit;

    store.deposit(&s1.service_id(), &d1_1, now).unwrap();
    store.deposit(&s1.service_id(), &d1_2, now).unwrap();
    store.deposit(&s2.service_id(), &d2, now).unwrap();

    let mut d3 = sealed_deposit(&s3, &receiver, 3600, now).deposit;
    let padded = vec![42u8; 8500];
    d3.sealed = SealedBytes::try_from(STANDARD.encode(padded)).unwrap();

    let out = store.deposit(&s3.service_id(), &d3, now + 10).unwrap();
    assert!(out.evicted >= 1);

    let st = store.status(&s1.service_id(), &[d1_1.deposit_id], now + 20).unwrap();
    assert_eq!(st[0].receipt, Receipt::Expired { at_secs: now + 10 });
}

#[test]
fn a_full_queue_refuses_the_largest_sender_itself() {
    let mut limits = small_limits();
    limits.queue = QueueLimits { max_bytes: 10_000, max_messages: 5, max_ttl_secs: 86400 };
    let store = store_with(limits);

    let s1 = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let mut d1 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    let padded = vec![42u8; 8000];
    d1.sealed = SealedBytes::try_from(STANDARD.encode(padded)).unwrap();
    store.deposit(&s1.service_id(), &d1, now).unwrap();

    let mut d2 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    let padded2 = vec![42u8; 3000];
    d2.sealed = SealedBytes::try_from(STANDARD.encode(padded2)).unwrap();

    let res = store.deposit(&s1.service_id(), &d2, now + 5);
    assert!(matches!(res, Err(MailboxError::QueueFull(_))));
}

#[test]
fn the_message_count_limit_also_evicts() {
    let mut limits = small_limits();
    limits.queue = QueueLimits { max_bytes: 1_000_000, max_messages: 2, max_ttl_secs: 86400 };
    let store = store_with(limits);

    let s1 = Member::new(3600);
    let s2 = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let d1 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    let d2 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    store.deposit(&s1.service_id(), &d1, now).unwrap();
    store.deposit(&s1.service_id(), &d2, now).unwrap();

    let d3 = sealed_deposit(&s2, &receiver, 3600, now).deposit;
    let out = store.deposit(&s2.service_id(), &d3, now + 10).unwrap();
    assert_eq!(out.evicted, 1);
}

#[test]
fn the_mailbox_cap_refuses_and_evicts_nothing() {
    let mut limits = small_limits();
    limits.max_total_bytes = 1000;
    limits.queue = QueueLimits { max_bytes: 2000, max_messages: 10, max_ttl_secs: 86400 };
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
    let padded = vec![42u8; 800];
    d2.sealed = SealedBytes::try_from(STANDARD.encode(padded)).unwrap();

    let res = store.deposit(&s2.service_id(), &d2, now + 10);
    assert_eq!(res, Err(MailboxError::MailboxFull));

    let st = store.status(&s1.service_id(), &[d1.deposit_id], now + 20).unwrap();
    assert_eq!(st[0].receipt, Receipt::Stored { at_secs: now });
}

#[test]
fn status_shows_only_the_callers_own_receipts() {
    let store = store_with(small_limits());
    let s1 = Member::new(3600);
    let s2 = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let d1 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    store.deposit(&s1.service_id(), &d1, now).unwrap();

    let st_s2 = store.status(&s2.service_id(), &[d1.deposit_id], now + 10).unwrap();
    assert_eq!(st_s2[0].receipt, Receipt::Unknown);

    let random_id = DepositId::random().unwrap();
    let st_unknown = store.status(&s1.service_id(), &[random_id], now + 10).unwrap();
    assert_eq!(st_unknown[0].receipt, Receipt::Unknown);
}

#[test]
fn unclaimed_messages_are_evicted_before_claimed_in_flight_messages() {
    let mut limits = small_limits();
    // Allow at most 2 messages in the queue
    limits.queue = QueueLimits { max_bytes: 10_000, max_messages: 2, max_ttl_secs: 86400 };
    let store = store_with(limits);

    let s1 = Member::new(3600);
    let s2 = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let d1 = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    let d2 = sealed_deposit(&s1, &receiver, 3600, now).deposit;

    // s1 deposits d1, then d2
    store.deposit(&s1.service_id(), &d1, now).unwrap();
    store.deposit(&s1.service_id(), &d2, now + 1).unwrap();

    // Receiver claims 1 message: d1 is claimed (in-flight with claimed_until > now)
    let claim_res = store.claim(&receiver.instance(), 1, now + 2).unwrap();
    assert_eq!(claim_res.messages.len(), 1);
    assert_eq!(claim_res.messages[0].deposit_id, d1.deposit_id);

    // s2 deposits d3, filling queue and forcing eviction of 1 message from largest
    // sender (s1)
    let d3 = sealed_deposit(&s2, &receiver, 3600, now).deposit;
    let out = store.deposit(&s2.service_id(), &d3, now + 3).unwrap();
    assert_eq!(out.evicted, 1);

    // Unclaimed message d2 was evicted, while claimed in-flight message d1 was
    // preserved!
    let st_d1 = store.status(&s1.service_id(), &[d1.deposit_id], now + 4).unwrap();
    assert_eq!(st_d1[0].receipt, Receipt::Stored { at_secs: now });

    let st_d2 = store.status(&s1.service_id(), &[d2.deposit_id], now + 4).unwrap();
    assert_eq!(st_d2[0].receipt, Receipt::Expired { at_secs: now + 3 });
}

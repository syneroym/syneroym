use syneroym_core::mailbox::{AckEntry, QueueLimits};
use syneroym_mailbox_format::{Receipt, SealedBytes, open};

use crate::{
    record::CheckedRecord,
    test_support::{Member, record, sealed_deposit, small_limits, store_with},
};

const NODE_DID: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

#[test]
fn an_expired_message_is_removed_and_its_receipt_says_expired() {
    let store = store_with(small_limits());
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let dep = sealed_deposit(&sender, &receiver, 100, now).deposit;
    let dep_id = dep.deposit_id;
    store.deposit(&sender.service_id(), &dep, now).unwrap();

    let rep1 = store.sweep(now + 50).unwrap();
    assert_eq!(rep1.expired, 0);

    let rep2 = store.sweep(now + 101).unwrap();
    assert_eq!(rep2.expired, 1);

    let st = store.status(&sender.service_id(), &[dep_id], now + 102).unwrap();
    assert_eq!(st[0].receipt, Receipt::Expired { at_secs: now + 101 });
}

#[test]
fn a_receipt_is_kept_for_the_keep_time_then_removed() {
    let mut limits = small_limits();
    limits.receipt_keep_secs = 50;
    let store = store_with(limits);

    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let dep = sealed_deposit(&sender, &receiver, 100, now).deposit;
    let dep_id = dep.deposit_id;
    store.deposit(&sender.service_id(), &dep, now).unwrap();

    store.sweep(now + 100).unwrap();

    let keep_until = now + 100 + 50;
    let st_before = store.status(&sender.service_id(), &[dep_id], keep_until - 1).unwrap();
    assert_eq!(st_before[0].receipt, Receipt::Expired { at_secs: now + 100 });

    store.sweep(keep_until).unwrap();

    let st_after = store.status(&sender.service_id(), &[dep_id], keep_until).unwrap();
    assert_eq!(st_after[0].receipt, Receipt::Unknown);
}

#[test]
fn an_idle_queue_is_removed_after_its_lifetime() {
    let mut limits = small_limits();
    limits.queue_lifetime_secs = 300;
    let store = store_with(limits);

    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let rep1 = store.sweep(now + 299).unwrap();
    assert_eq!(rep1.queues_removed, 0);

    let rep2 = store.sweep(now + 300).unwrap();
    assert_eq!(rep2.queues_removed, 1);

    let tot = store.totals().unwrap();
    assert_eq!(tot.queues, 0);
}

#[test]
fn a_claim_keeps_a_queue_alive() {
    let mut limits = small_limits();
    limits.queue_lifetime_secs = 300;
    let store = store_with(limits);

    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    store.claim(&receiver.instance(), 10, now + 200).unwrap();

    let rep = store.sweep(now + 350).unwrap();
    assert_eq!(rep.queues_removed, 0);

    let rep2 = store.sweep(now + 501).unwrap();
    assert_eq!(rep2.queues_removed, 1);
}

#[test]
fn a_record_is_due_after_register_and_not_after_it_was_published() {
    let store = store_with(small_limits());
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let due = store.records_due(now, 10).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].service_id, receiver.service_id());

    store.mark_published(&receiver.service_id(), now + 3600).unwrap();

    let due_after = store.records_due(now + 500, 10).unwrap();
    assert_eq!(due_after.len(), 0);
}

#[test]
fn a_record_past_not_after_is_never_due() {
    let store = store_with(small_limits());
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let mut checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID).unwrap();
    checked.not_after = now + 50;
    store.register(&receiver.instance(), &checked, now).unwrap();

    let due = store.records_due(now + 60, 10).unwrap();
    assert_eq!(due.len(), 0);
}

#[test]
fn the_counters_match_the_messages_after_every_kind_of_removal() {
    let mut limits = small_limits();
    limits.queue = QueueLimits { max_bytes: 100_000, max_messages: 10, max_ttl_secs: 86400 };
    let store = store_with(limits);

    let s1 = Member::new(3600);
    let s2 = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();

    let d_evict = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    store.deposit(&s1.service_id(), &d_evict, now).unwrap();

    let mut d_big = sealed_deposit(&s2, &receiver, 3600, now).deposit;
    let padded = vec![42u8; 50_000];

    d_big.sealed = SealedBytes::try_from(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        padded,
    ))
    .unwrap();
    store.deposit(&s2.service_id(), &d_big, now + 1).unwrap();

    let d_ack = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    store.deposit(&s1.service_id(), &d_ack, now + 2).unwrap();

    let cl = store.claim(&receiver.instance(), 10, now + 5).unwrap();
    let msg_to_ack = cl.messages.iter().find(|m| m.deposit_id == d_ack.deposit_id).unwrap();
    let opened = open(
        &receiver.mailbox,
        &receiver.service_id(),
        &msg_to_ack.deposit_id,
        &msg_to_ack.sealed,
        syneroym_mailbox_format::OpenPolicy { max_ttl_secs: 86400, max_clock_skew_secs: 3600 },
        now + 5,
    )
    .unwrap();

    let receipt = opened.sign_delivered(&receiver.instance, &receiver.cert, now + 5).unwrap();
    store
        .ack(
            &receiver.instance(),
            &[AckEntry { sender: s1.service_id(), deposit_id: d_ack.deposit_id, receipt }],
            now + 6,
        )
        .unwrap();

    let d_exp = sealed_deposit(&s1, &receiver, 10, now).deposit;
    store.deposit(&s1.service_id(), &d_exp, now + 7).unwrap();
    store.sweep(now + 20).unwrap();

    let conn = store.lock();
    let (real_count, real_bytes): (i64, i64) = conn
        .query_row("SELECT COUNT(*), COALESCE(SUM(size), 0) FROM messages", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    drop(conn);

    let totals = store.totals().unwrap();
    assert_eq!(totals.messages, real_count as u64);
    assert_eq!(totals.bytes, real_bytes as u64);
}

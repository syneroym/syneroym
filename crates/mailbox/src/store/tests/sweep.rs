use base64::{Engine, engine::general_purpose::STANDARD};
use syneroym_core::mailbox::{AckEntry, QueueLimits};
use syneroym_mailbox_format::{Receipt, SealedBytes, open};

use crate::{
    record::CheckedRecord,
    store::MailboxStore,
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
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
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
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
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
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
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
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
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
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
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
    let mut checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    checked.not_after = now + 50;
    store.register(&receiver.instance(), &checked, now).unwrap();

    let due = store.records_due(now + 60, 10).unwrap();
    assert_eq!(due.len(), 0);
}

fn assert_counters_match(store: &MailboxStore) {
    let conn = store.lock();
    let (real_count, real_bytes): (i64, i64) = conn
        .query_row("SELECT COUNT(*), COALESCE(SUM(size), 0) FROM messages", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    let (q_messages, q_bytes): (i64, i64) = conn
        .query_row(
            "SELECT COALESCE(SUM(messages), 0), COALESCE(SUM(bytes), 0) FROM queues",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let mut stmt = conn.prepare("SELECT messages, bytes FROM queues").unwrap();
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).unwrap();
    for row in rows {
        let (m, b) = row.unwrap();
        assert!(m >= 0, "messages underflowed: {m}");
        assert!(b >= 0, "bytes underflowed: {b}");
    }
    let (stats_bytes,): (i64,) = conn
        .query_row("SELECT total_bytes FROM mailbox_stats WHERE id = 1", [], |r| Ok((r.get(0)?,)))
        .unwrap();
    drop(stmt);
    drop(conn);

    let totals = store.totals().unwrap();
    assert_eq!(totals.messages, real_count as u64);
    assert_eq!(totals.bytes, real_bytes as u64);
    assert_eq!(totals.messages, q_messages as u64);
    assert_eq!(totals.bytes, q_bytes as u64);
    assert_eq!(totals.bytes, stats_bytes as u64);
}

#[test]
fn the_counters_match_the_messages_after_every_kind_of_removal() {
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
    assert_counters_match(&store);

    // 1. Ack removal: deposit, claim, ack
    let d_ack = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    store.deposit(&s1.service_id(), &d_ack, now).unwrap();
    assert_counters_match(&store);

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
    assert_counters_match(&store);

    // 2. Eviction removal: deposit d_evict, then s2 deposits oversized message to
    //    evict d_evict
    let d_evict = sealed_deposit(&s1, &receiver, 3600, now).deposit;
    store.deposit(&s1.service_id(), &d_evict, now + 7).unwrap();
    assert_counters_match(&store);

    let mut d_big = sealed_deposit(&s2, &receiver, 3600, now).deposit;
    let padded = vec![42u8; 9500];
    d_big.sealed = SealedBytes::try_from(STANDARD.encode(padded)).unwrap();
    let outcome = store.deposit(&s2.service_id(), &d_big, now + 8).unwrap();
    assert!(outcome.evicted >= 1);
    assert_counters_match(&store);

    // 3. Expiry removal: deposit short-lived message and sweep
    let d_exp = sealed_deposit(&s1, &receiver, 10, now).deposit;
    store.deposit(&s1.service_id(), &d_exp, now + 9).unwrap();
    assert_counters_match(&store);
    store.sweep(now + 30).unwrap();
    assert_counters_match(&store);

    // 4. 5-claim close: claim d_big 5 times without an ack so it closes unsigned
    for i in 0..5 {
        let t = now + 40 + i * 400;
        let _ = store.claim(&receiver.instance(), 10, t).unwrap();
    }
    assert_counters_match(&store);

    // 5. Unregister removal: unregister the queue
    store.unregister(&receiver.instance(), now + 3000).unwrap();
    assert_counters_match(&store);
    let final_totals = store.totals().unwrap();
    assert_eq!(final_totals.queues, 0);
    assert_eq!(final_totals.messages, 0);
    assert_eq!(final_totals.bytes, 0);
}

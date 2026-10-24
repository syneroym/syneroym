//! The settled count against the receipts that really are in the table, for
//! every way a message can settle, and the fairness of the cap.

use syneroym_mailbox_format::Receipt;

use super::settled_receipts::{Fixture, fixture, fixture_on, limits_with_cap};
use crate::{
    limits::Limits,
    store::{MailboxStore, settled::fair_cuts},
    test_support::{Member, small_limits},
};

fn assert_count_matches_rows(f: &Fixture, at_least: u64) {
    assert!(f.settled() >= at_least, "expected at least {at_least} settled receipts");
    assert_eq!(f.settled(), f.settled_rows(), "the stored count must match the table");
}

#[test]
fn a_sender_cannot_push_the_receipts_of_others_out() {
    let f = fixture(limits_with_cap(4));
    let honest = Member::new(3600);
    let first = f.deposit_from(&honest, f.now).deposit.deposit_id;
    f.settle_from(&honest, first, f.now + 1);

    for i in 0..8 {
        let id = f.deposit(f.now + 2 + i).deposit.deposit_id;
        f.settle(id, f.now + 2 + i);
    }

    assert_eq!(f.settled(), 4);
    assert!(matches!(
        f.store.status(&honest.service_id(), &[first], f.now + 20).unwrap()[0].receipt,
        Receipt::Rejected { .. }
    ));
    assert_count_matches_rows(&f, 4);
}

#[test]
fn the_cuts_come_from_the_senders_that_hold_the_most() {
    let counts = vec![("a".to_string(), 10), ("b".to_string(), 2), ("c".to_string(), 1)];

    assert_eq!(fair_cuts(counts.clone(), 5), vec![("a".to_string(), 5)]);
    assert_eq!(fair_cuts(counts.clone(), 0), Vec::new());
    for want in 0..=13 {
        let cuts = fair_cuts(counts.clone(), want);
        assert_eq!(cuts.iter().map(|(_, n)| n).sum::<u64>(), want, "want {want}: {cuts:?}");
        for (sender, n) in &cuts {
            let held = counts.iter().find(|(s, _)| s == sender).map(|(_, c)| *c).unwrap();
            assert!(*n <= held, "{sender} holds {held} and loses {n}");
        }
    }
}

#[test]
fn lowering_the_cap_trims_the_receipts_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mailbox.db");
    let f = fixture_on(MailboxStore::open(&path, limits_with_cap(10)).unwrap());
    for i in 0..6 {
        let id = f.deposit(f.now).deposit.deposit_id;
        f.settle(id, f.now + 1 + i);
    }
    assert_eq!(f.settled(), 6);
    drop(f);

    let reopened = MailboxStore::open(&path, limits_with_cap(3)).unwrap();

    assert_eq!(reopened.totals().unwrap().settled_receipts, 3);
    let rows: i64 = reopened
        .lock()
        .query_row("SELECT COUNT(*) FROM receipts WHERE settled = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 3);
}

#[test]
fn the_count_matches_the_table_after_a_message_expires() {
    let f = fixture(small_limits());
    f.deposit(f.now);

    f.store.sweep(f.now + 3601).unwrap();

    assert_count_matches_rows(&f, 1);
}

#[test]
fn the_count_matches_the_table_after_a_message_is_evicted() {
    let mut limits = limits_with_cap(100);
    limits.queue.max_messages = 2;
    let f = fixture(limits);
    f.deposit(f.now);
    f.deposit(f.now);
    let other = Member::new(3600);

    f.deposit_from(&other, f.now + 1);

    assert_count_matches_rows(&f, 1);
}

#[test]
fn the_count_matches_the_table_after_a_message_is_closed_unacked() {
    let f = fixture(small_limits());
    f.deposit(f.now);
    let visibility = f.store.limits().visibility_secs;
    for round in 0..6 {
        f.store.claim(&f.receiver.instance(), 10, f.now + round * (visibility + 1)).unwrap();
    }

    assert_count_matches_rows(&f, 1);
}

#[test]
fn the_count_matches_the_table_after_the_queue_is_unregistered() {
    let f = fixture(small_limits());
    f.deposit(f.now);
    f.deposit(f.now);

    f.store.unregister(&f.receiver.instance(), f.now + 1).unwrap();

    assert_count_matches_rows(&f, 2);
}

#[test]
fn the_count_matches_the_table_after_a_dead_queue_is_swept() {
    let limits: Limits = small_limits();
    let lifetime = limits.queue_lifetime_secs;
    let f = fixture(limits);
    f.deposit(f.now);

    f.store.sweep(f.now + lifetime + 1).unwrap();

    assert_count_matches_rows(&f, 1);
}

use syneroym_core::mailbox::AckEntry;
use syneroym_mailbox_format::{DepositId, Receipt, RejectCode, Sealed};

use crate::{
    limits::Limits,
    record::CheckedRecord,
    store::MailboxStore,
    test_support::{Member, record, sealed_deposit, small_limits, store_with},
};

const NODE_DID: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

struct Fixture {
    store: MailboxStore,
    sender: Member,
    receiver: Member,
    now: u64,
}

fn limits_with_cap(max_settled: u64) -> Limits {
    let mut limits = small_limits();
    limits.queue.max_messages = 10;
    limits.max_settled_receipts = max_settled;
    limits
}

fn fixture(limits: Limits) -> Fixture {
    fixture_on(store_with(limits))
}

fn fixture_on(store: MailboxStore) -> Fixture {
    let sender = Member::new(3600);
    let receiver = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;
    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    store.register(&receiver.instance(), &checked, now).unwrap();
    Fixture { store, sender, receiver, now }
}

impl Fixture {
    fn deposit(&self, at: u64) -> Sealed {
        let sealed = sealed_deposit(&self.sender, &self.receiver, 3600, at);
        self.store.deposit(&self.sender.service_id(), &sealed.deposit, at).unwrap();
        sealed
    }

    /// Settles the held message with a rejection the receiver acknowledged at
    /// `at`.
    fn settle(&self, id: DepositId, at: u64) {
        let ack = AckEntry {
            sender: self.sender.service_id(),
            deposit_id: id,
            receipt: Receipt::unsigned_rejection(RejectCode::Malformed, at),
        };
        self.store.ack(&self.receiver.instance(), &[ack], at).unwrap();
    }

    fn receipt(&self, id: DepositId, at: u64) -> Receipt {
        self.store.status(&self.sender.service_id(), &[id], at).unwrap().remove(0).receipt
    }

    fn settled(&self) -> u64 {
        self.store.totals().unwrap().settled_receipts
    }
}

#[test]
fn settling_past_the_cap_drops_the_settled_receipt_that_expires_first() {
    let f = fixture(limits_with_cap(2));
    let ids: Vec<_> = (0..3).map(|_| f.deposit(f.now).deposit.deposit_id).collect();

    f.settle(ids[0], f.now + 1);
    f.settle(ids[1], f.now + 2);
    assert_eq!(f.settled(), 2);
    f.settle(ids[2], f.now + 3);

    assert_eq!(f.settled(), 2);
    let at = f.now + 4;
    assert_eq!(f.receipt(ids[0], at), Receipt::Unknown);
    assert!(matches!(f.receipt(ids[1], at), Receipt::Rejected { .. }));
    assert!(matches!(f.receipt(ids[2], at), Receipt::Rejected { .. }));
}

#[test]
fn a_dropped_receipt_reads_unknown_in_status() {
    let f = fixture(limits_with_cap(1));
    let first = f.deposit(f.now).deposit.deposit_id;
    let second = f.deposit(f.now).deposit.deposit_id;

    f.settle(first, f.now + 1);
    assert!(matches!(f.receipt(first, f.now + 2), Receipt::Rejected { .. }));
    f.settle(second, f.now + 2);

    assert_eq!(f.receipt(first, f.now + 3), Receipt::Unknown);
}

#[test]
fn a_receipt_of_a_held_message_is_never_dropped() {
    let f = fixture(limits_with_cap(1));
    let ids: Vec<_> = (0..3).map(|_| f.deposit(f.now).deposit.deposit_id).collect();

    f.settle(ids[0], f.now + 1);
    f.settle(ids[1], f.now + 2);

    let at = f.now + 3;
    assert_eq!(f.receipt(ids[0], at), Receipt::Unknown);
    assert!(matches!(f.receipt(ids[1], at), Receipt::Rejected { .. }));
    assert_eq!(f.receipt(ids[2], at), Receipt::Stored { at_secs: f.now });
}

#[test]
fn deposits_are_accepted_when_the_settled_receipts_are_at_the_cap() {
    let f = fixture(limits_with_cap(1));
    let first = f.deposit(f.now).deposit.deposit_id;
    f.settle(first, f.now + 1);
    let second = f.deposit(f.now + 2).deposit.deposit_id;
    f.settle(second, f.now + 3);
    assert_eq!(f.settled(), 1);

    let third = sealed_deposit(&f.sender, &f.receiver, 3600, f.now + 4);
    let out = f.store.deposit(&f.sender.service_id(), &third.deposit, f.now + 4).unwrap();

    assert_eq!(out.receipt, Receipt::Stored { at_secs: f.now + 4 });
    assert_eq!(f.store.totals().unwrap().messages, 1);
}

#[test]
fn an_identical_retry_after_its_receipt_was_dropped_is_stored_again() {
    let f = fixture(limits_with_cap(1));
    let first = f.deposit(f.now);
    let second = f.deposit(f.now);
    f.settle(first.deposit.deposit_id, f.now + 1);
    f.settle(second.deposit.deposit_id, f.now + 2);
    assert_eq!(f.receipt(first.deposit.deposit_id, f.now + 3), Receipt::Unknown);
    assert_eq!(f.store.totals().unwrap().messages, 0);

    let retry = f.store.deposit(&f.sender.service_id(), &first.deposit, f.now + 3).unwrap();

    assert_eq!(retry.receipt, Receipt::Stored { at_secs: f.now + 3 });
    assert_eq!(f.store.totals().unwrap().messages, 1);
}

#[test]
fn the_settled_count_follows_settles_drops_and_sweeps() {
    let mut limits = limits_with_cap(2);
    limits.receipt_keep_secs = 50;
    let f = fixture(limits);
    let ids: Vec<_> = (0..3).map(|_| f.deposit(f.now).deposit.deposit_id).collect();

    assert_eq!(f.settled(), 0);
    f.settle(ids[0], f.now + 1);
    f.settle(ids[1], f.now + 10);
    assert_eq!(f.settled(), 2);
    f.settle(ids[2], f.now + 20);
    assert_eq!(f.settled(), 2, "the oldest one was dropped");

    // The receipt of ids[1] ends at now + 60 and the one of ids[2] at now + 70.
    let report = f.store.sweep(f.now + 60).unwrap();
    assert_eq!(report.receipts_removed, 1);
    assert_eq!(f.settled(), 1);
    f.store.sweep(f.now + 70).unwrap();
    assert_eq!(f.settled(), 0);
}

#[test]
fn the_settled_count_is_counted_again_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mailbox.db");
    let f = fixture_on(MailboxStore::open(&path, limits_with_cap(10)).unwrap());
    let ids: Vec<_> = (0..2).map(|_| f.deposit(f.now).deposit.deposit_id).collect();
    f.settle(ids[0], f.now + 1);
    f.settle(ids[1], f.now + 2);
    assert_eq!(f.settled(), 2);
    drop(f);

    let reopened = MailboxStore::open(&path, limits_with_cap(10)).unwrap();

    assert_eq!(reopened.totals().unwrap().settled_receipts, 2);
}

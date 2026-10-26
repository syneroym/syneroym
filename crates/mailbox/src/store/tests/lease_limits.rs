//! The limits of a queue come from the open limits and the running lease.

use syneroym_core::mailbox::QueueLimits;
use syneroym_mailbox_format::{DEDICATED_MAX_TTL_SECS, OPEN_MAX_TTL_SECS};
use syneroym_passport::facts;

use crate::{
    error::MailboxError,
    record::CheckedRecord,
    store::MailboxStore,
    test_support::{LeaseFixture, Member, record, sealed_deposit, small_limits},
};

const NODE_DID: &str = "did:key:h7wy36tabek9ik75co38yrxwzh5zry8r1tpih7t7sqzbc74brtckeaea";

const LIFE: u64 = 1_000;

fn store_with_leases() -> (MailboxStore, LeaseFixture) {
    let fixture = LeaseFixture::new();
    let store =
        MailboxStore::open_in_memory_with_leases(small_limits(), Some(fixture.leases())).unwrap();
    (store, fixture)
}

fn now_of(member: &Member) -> u64 {
    member.cert.issued_at_secs + 10
}

#[test]
fn a_queue_without_a_lease_has_the_open_limits() {
    let (store, _fixture) = store_with_leases();
    let member = Member::new(3600);
    assert_eq!(store.queue_limits(&member.service_id(), now_of(&member)), small_limits().queue);
}

#[test]
fn a_lease_raises_each_limit_to_its_fact() {
    let (store, fixture) = store_with_leases();
    let member = Member::new(3600);
    let now = now_of(&member);
    let raised =
        [(facts::MAX_BYTES, 900_000), (facts::MAX_MESSAGES, 50), (facts::MAX_TTL_SECS, 864_000)];
    fixture.grant(&member, &raised, LIFE, now);
    assert_eq!(
        store.queue_limits(&member.service_id(), now),
        QueueLimits { max_bytes: 900_000, max_messages: 50, max_ttl_secs: 864_000 }
    );
}

#[test]
fn a_fact_below_the_open_limit_is_ignored() {
    let (store, fixture) = store_with_leases();
    let member = Member::new(3600);
    let now = now_of(&member);
    let low = [(facts::MAX_BYTES, 1), (facts::MAX_MESSAGES, 1), (facts::MAX_TTL_SECS, 1)];
    fixture.grant(&member, &low, LIFE, now);
    assert_eq!(store.queue_limits(&member.service_id(), now), small_limits().queue);
}

#[test]
fn the_ttl_fact_is_cut_to_the_dedicated_cap() {
    let (store, fixture) = store_with_leases();
    let member = Member::new(3600);
    let now = now_of(&member);
    fixture.grant(&member, &[(facts::MAX_TTL_SECS, DEDICATED_MAX_TTL_SECS * 4)], LIFE, now);
    let limits = store.queue_limits(&member.service_id(), now);
    assert_eq!(limits.max_ttl_secs, DEDICATED_MAX_TTL_SECS);
    assert!(limits.max_ttl_secs > OPEN_MAX_TTL_SECS);
}

#[test]
fn the_open_limits_return_when_the_lease_ends() {
    let (store, fixture) = store_with_leases();
    let member = Member::new(3600);
    let now = now_of(&member);
    let lease = fixture.grant(&member, &[(facts::MAX_MESSAGES, 50)], LIFE, now);
    assert_eq!(store.queue_limits(&member.service_id(), lease.lease_ends_at - 1).max_messages, 50);
    assert_eq!(store.queue_limits(&member.service_id(), lease.lease_ends_at), small_limits().queue);
}

#[test]
fn a_deposit_beyond_the_open_message_limit_fits_under_a_lease() {
    let (store, fixture) = store_with_leases();
    let (sender, receiver) = (Member::new(3600), Member::new(3600));
    let now = now_of(&receiver);
    let lease = fixture.grant(&receiver, &[(facts::MAX_MESSAGES, 6)], LIFE, now);

    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();
    let answer = store.register(&receiver.instance(), &checked, now).unwrap().answer;
    assert_eq!(answer.limits.max_messages, 6);

    for _ in 0..small_limits().queue.max_messages + 2 {
        let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;
        store.deposit(&sender.service_id(), &dep, now).unwrap();
    }
    let dep = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    let beyond = store.deposit(&sender.service_id(), &dep, lease.lease_ends_at);
    assert!(matches!(beyond, Err(MailboxError::QueueFull(_))), "the lease ended: {beyond:?}");
}

#[test]
fn a_lease_lifts_the_queue_count_for_its_master_only() {
    let (store, fixture) = store_with_leases();
    let stranger = Member::new(3600);
    let now = now_of(&stranger);
    for _ in 0..small_limits().max_queues {
        let m = Member::new(3600);
        let checked = CheckedRecord::check(&record(&m, NODE_DID), &m.instance(), NODE_DID, now);
        store.register(&m.instance(), &checked.unwrap(), now).unwrap();
    }

    let rec =
        CheckedRecord::check(&record(&stranger, NODE_DID), &stranger.instance(), NODE_DID, now)
            .unwrap();
    assert!(matches!(
        store.register(&stranger.instance(), &rec, now),
        Err(MailboxError::MailboxFull)
    ));

    fixture.grant(&stranger, &[], LIFE, now);
    assert!(store.register(&stranger.instance(), &rec, now).is_ok());
}

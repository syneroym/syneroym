use syneroym_mailbox_format::Receipt;

use crate::{
    caller::Instance,
    error::MailboxError,
    record::CheckedRecord,
    test_support::{Member, record, sealed_deposit, small_limits, store_with},
};

const NODE_DID: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

#[test]
fn register_creates_a_queue_and_reports_its_limits_and_end() {
    let store = store_with(small_limits());
    let member = Member::new(3600);
    let rec = record(&member, NODE_DID);
    let now = member.cert.issued_at_secs + 10;
    let checked = CheckedRecord::check(&rec, &member.instance(), NODE_DID, now).unwrap();

    let registered = store.register(&member.instance(), &checked, now).unwrap();

    assert!(registered.publish_now);
    assert_eq!(registered.answer.limits, store.limits().queue);
    assert_eq!(registered.answer.queue_ends_at_secs, now + store.limits().queue_lifetime_secs);
}

#[test]
fn a_newer_record_replaces_the_stored_one_and_an_older_one_is_kept() {
    let store = store_with(small_limits());
    let member = Member::new(3600);
    let rec1 = record(&member, NODE_DID);
    let now = member.cert.issued_at_secs + 10;
    let checked1 = CheckedRecord::check(&rec1, &member.instance(), NODE_DID, now).unwrap();

    let r1 = store.register(&member.instance(), &checked1, now).unwrap();
    assert!(r1.publish_now);

    let checked_older = CheckedRecord {
        json: checked1.json.clone(),
        signed_at: checked1.signed_at.saturating_sub(10),
        not_after: checked1.not_after,
    };
    let r_older = store.register(&member.instance(), &checked_older, now + 5).unwrap();
    assert!(!r_older.publish_now);

    let checked_newer = CheckedRecord {
        json: checked1.json.clone(),
        signed_at: checked1.signed_at + 100,
        not_after: checked1.not_after + 100,
    };
    let r_newer = store.register(&member.instance(), &checked_newer, now + 10).unwrap();
    assert!(r_newer.publish_now);
}

#[test]
fn a_newer_instance_takes_the_queue_and_the_old_one_is_stale() {
    let store = store_with(small_limits());
    let member1 = Member::new(3600);
    let rec1 = record(&member1, NODE_DID);
    let now = member1.cert.issued_at_secs + 10;
    let checked1 = CheckedRecord::check(&rec1, &member1.instance(), NODE_DID, now).unwrap();

    let now = member1.cert.issued_at_secs + 10;
    store.register(&member1.instance(), &checked1, now).unwrap();

    let member2_instance = syneroym_identity::Identity::generate().unwrap();
    let cert2 = syneroym_identity::DelegationCertificate::issue(
        &member1.master,
        member2_instance.public_key(),
        3600,
        syneroym_identity::delegation::SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let instance2 = Instance {
        master_did: member1.service_id(),
        instance_did: cert2.temporary_did.clone(),
        issued_at_secs: member1.cert.issued_at_secs + 20,
    };

    let r2 = store.register(&instance2, &checked1, now + 30).unwrap();
    assert!(!r2.publish_now);

    let claim_old = store.claim(&member1.instance(), 10, now + 40);
    assert!(matches!(claim_old, Err(MailboxError::StaleInstance(_))));

    let ack_old = store.ack(&member1.instance(), &[], now + 40);
    assert!(matches!(ack_old, Err(MailboxError::StaleInstance(_))));

    let unreg_old = store.unregister(&member1.instance(), now + 40);
    assert!(matches!(unreg_old, Err(MailboxError::StaleInstance(_))));

    let claim_new = store.claim(&instance2, 10, now + 40);
    assert!(claim_new.is_ok());
}

#[test]
fn the_same_instance_with_an_older_certificate_is_still_admitted() {
    let store = store_with(small_limits());
    let member = Member::new(3600);
    let now = member.cert.issued_at_secs + 10;
    let rec = record(&member, NODE_DID);
    let checked = CheckedRecord::check(&rec, &member.instance(), NODE_DID, now).unwrap();

    store.register(&member.instance(), &checked, now).unwrap();

    let mut old_instance = member.instance();
    old_instance.issued_at_secs = old_instance.issued_at_secs.saturating_sub(10);

    let claim = store.claim(&old_instance, 10, now + 20);
    assert!(claim.is_ok());
}

#[test]
fn two_instances_with_the_same_issue_time_keep_the_first() {
    let store = store_with(small_limits());
    let member1 = Member::new(3600);
    let now = member1.cert.issued_at_secs + 10;
    let rec1 = record(&member1, NODE_DID);
    let checked1 = CheckedRecord::check(&rec1, &member1.instance(), NODE_DID, now).unwrap();

    store.register(&member1.instance(), &checked1, now).unwrap();

    let instance2 = Instance {
        master_did: member1.service_id(),
        instance_did: "did:key:z6MkrOtherInstanceKeyHere".to_string(),
        issued_at_secs: member1.cert.issued_at_secs,
    };

    let claim_first = store.claim(&member1.instance(), 10, now + 20);
    assert!(claim_first.is_ok());

    let claim_second = store.claim(&instance2, 10, now + 20);
    assert!(matches!(claim_second, Err(MailboxError::StaleInstance(_))));
}

#[test]
fn a_new_queue_is_refused_when_the_mailbox_holds_its_maximum() {
    let mut limits = small_limits();
    limits.max_queues = 2;
    let store = store_with(limits);

    let m1 = Member::new(3600);
    let r1 = record(&m1, NODE_DID);
    let c1 = CheckedRecord::check(&r1, &m1.instance(), NODE_DID, 100).unwrap();
    store.register(&m1.instance(), &c1, 100).unwrap();

    let m2 = Member::new(3600);
    let r2 = record(&m2, NODE_DID);
    let c2 = CheckedRecord::check(&r2, &m2.instance(), NODE_DID, 100).unwrap();
    store.register(&m2.instance(), &c2, 100).unwrap();

    let m3 = Member::new(3600);
    let r3 = record(&m3, NODE_DID);
    let c3 = CheckedRecord::check(&r3, &m3.instance(), NODE_DID, 100).unwrap();
    let res = store.register(&m3.instance(), &c3, 100);
    assert!(matches!(res, Err(MailboxError::MailboxFull)));
}

#[test]
fn unregister_removes_the_queue_and_expires_its_messages() {
    let store = store_with(small_limits());
    let receiver = Member::new(3600);
    let sender = Member::new(3600);
    let now = receiver.cert.issued_at_secs + 10;
    let rec = record(&receiver, NODE_DID);
    let checked = CheckedRecord::check(&rec, &receiver.instance(), NODE_DID, now).unwrap();

    let now = receiver.cert.issued_at_secs + 10;
    store.register(&receiver.instance(), &checked, now).unwrap();

    let deposit = sealed_deposit(&sender, &receiver, 3600, now).deposit;
    let deposit_id = deposit.deposit_id;
    store.deposit(&sender.service_id(), &deposit, now).unwrap();

    let removed = store.unregister(&receiver.instance(), now + 50).unwrap();
    assert!(removed);

    let removed_again = store.unregister(&receiver.instance(), now + 60).unwrap();
    assert!(!removed_again);

    let statuses = store.status(&sender.service_id(), &[deposit_id], now + 70).unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].receipt, Receipt::Expired { at_secs: now + 50 });
}

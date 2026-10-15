use serde_json::{Map, json};
use syneroym_identity::Identity;

use crate::{
    PassportId, SignerBinding, child_token,
    test_support::{Fixture, PASSPORT_LIFE_SECS, did, limits},
};

#[test]
fn relay_registration_writes_a_lease() {
    let f = Fixture::relay();
    let facts = limits(&[("max_nodes", json!(3))]);
    let passport = f.passport(facts.clone(), PASSPORT_LIFE_SECS);
    let token = f.child(&passport, &f.subject);
    let lease = f.register_relay(&token, &f.subject, f.now).unwrap();

    let subject = did(&f.subject);
    assert_eq!(lease.subject, subject);
    assert_eq!(lease.owner_did, did(&f.owner));
    assert_eq!(lease.passport_id, PassportId::of(&passport).unwrap());
    assert_eq!(lease.limits, facts);
    assert_eq!(lease.registered_at, f.now);
    assert!(f.registrar.leases().is_admitted(&subject, f.now));
    assert_eq!(f.registrar.leases().lease_for(&subject, f.now), Some(lease));
}

/// A self-hosted relay: the controller is the only root, and it is also the
/// owner that receives the passport and signs the child token.
#[test]
fn a_self_hosted_relay_where_the_owner_is_a_root_registers() {
    let mut f = Fixture::relay();
    f.owner = Identity::from_bytes(&f.operator.to_bytes());
    let roots = vec![did(&f.operator)];
    let f = f.with_roots(roots);
    f.register(&f.token(Map::new()), f.now).unwrap();
}

#[test]
fn mailbox_registration_writes_a_lease_for_the_master() {
    let f = Fixture::mailbox();
    let facts = limits(&[("max_services", json!(2)), ("max_bytes", json!(1_000_000))]);
    let token = f.child(&f.passport(facts.clone(), PASSPORT_LIFE_SECS), &f.subject);
    let lease = f.register_mailbox(&token, &f.instance, &f.subject, f.now).unwrap();
    assert_eq!(lease.subject, did(&f.subject));
    assert_ne!(lease.subject, did(&f.instance));
    assert_eq!(lease.limits, facts);
    assert!(f.registrar.leases().is_admitted(&did(&f.subject), f.now));
    assert!(!f.registrar.leases().is_admitted(&did(&f.instance), f.now));
}

#[test]
fn challenge_form_works_with_an_instance_certificate() {
    let f = Fixture::mailbox();
    let token = f.token(Map::new());
    let proof = f.challenge_proof(&f.instance, &token, f.now);
    let binding = Fixture::instance_binding(&f.subject, &f.instance);
    let lease = f.register_with(&token, &proof, &binding, f.now).unwrap();
    assert_eq!(lease.subject, did(&f.subject));
}

#[test]
fn transport_form_works_with_the_same_key() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let proof = Fixture::transport_proof(&did(&f.subject));
    f.register_with(&token, &proof, &SignerBinding::SameKey, f.now).unwrap();
}

#[test]
fn a_passport_rooted_at_either_accepted_root_is_accepted() {
    let node = Identity::generate().unwrap();
    let f = Fixture::relay();
    let roots = vec![did(&f.operator), did(&node)];
    let f = f.with_roots(roots);
    for root in [&f.operator, &node] {
        let passport = f.passport_from(root, Map::new(), PASSPORT_LIFE_SECS);
        f.register(&f.child(&passport, &f.subject), f.now).unwrap();
    }
    assert_eq!(f.registrar.leases().running_leases(f.now).len(), 2);
}

#[test]
fn the_lease_ends_at_the_earliest_limit() {
    let f = Fixture::relay();
    let passport = f.passport(Map::new(), PASSPORT_LIFE_SECS);

    let short_child = child_token(&f.owner, &passport, &did(&f.subject), 100, f.now).unwrap();
    assert_eq!(f.register(&short_child, f.now).unwrap().lease_ends_at, f.now + 100);

    let token = f.child(&passport, &f.subject);
    assert_eq!(f.register(&token, f.now).unwrap().lease_ends_at, f.now + PASSPORT_LIFE_SECS);

    let long = f.passport(Map::new(), f.registrar.passport_use().max_lease_secs * 2);
    let lease = f.register(&f.child(&long, &f.subject), f.now).unwrap();
    assert_eq!(lease.lease_ends_at, f.now + f.registrar.passport_use().max_lease_secs);
}

#[test]
fn renewing_moves_the_lease_end() {
    let f = Fixture::relay();
    let long = f.passport(Map::new(), f.registrar.passport_use().max_lease_secs * 2);
    let token = f.child(&long, &f.subject);
    let first = f.register(&token, f.now).unwrap();
    let second = f.register(&token, f.now + 60).unwrap();
    assert_eq!(second.lease_ends_at, first.lease_ends_at + 60);
    assert_eq!(second.registered_at, f.now + 60);
    let running = f.registrar.leases().running_leases(f.now + 60);
    assert_eq!(running, vec![second]);
}

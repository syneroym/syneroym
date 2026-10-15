use serde_json::{Map, json};
use syneroym_identity::Identity;

use crate::{
    LeaseStore, MAILBOX_USE, PassportUse, RELAY_HOME, RegisterError, Registrar, SignerBinding,
    child_token,
    test_support::{Fixture, PASSPORT_LIFE_SECS, did, limits},
};

const _: fn() = || {
    fn is_send_and_sync<T: Send + Sync>() {}
    is_send_and_sync::<LeaseStore>();
    is_send_and_sync::<Registrar>();
};

#[test]
fn a_registrar_refuses_a_lease_store_of_another_use() {
    let f = Fixture::relay();
    let error = Registrar::new(
        MAILBOX_USE,
        f.host_node_did.clone(),
        vec![did(&f.operator)],
        LeaseStore::open_in_memory(&RELAY_HOME).unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("lease store"), "{error}");
}

#[test]
fn two_uses_on_one_host_keep_their_leases_apart() {
    let relay = Fixture::relay();
    let mailbox = Registrar::new(
        MAILBOX_USE,
        relay.host_node_did.clone(),
        vec![did(&relay.operator)],
        LeaseStore::open_in_memory(&MAILBOX_USE).unwrap(),
    )
    .unwrap();
    let token = relay.token(Map::new());
    relay.register_relay(&token, &relay.subject, relay.now).unwrap();
    assert!(relay.registrar.leases().is_admitted(&did(&relay.subject), relay.now));
    assert!(!mailbox.leases().is_admitted(&did(&relay.subject), relay.now));
}

#[test]
fn a_registrar_refuses_a_name_that_is_not_a_canonical_did() {
    let f = Fixture::relay();
    let new = |host: &str, roots: Vec<String>| {
        Registrar::new(
            RELAY_HOME,
            host.to_string(),
            roots,
            LeaseStore::open_in_memory(&RELAY_HOME).unwrap(),
        )
    };
    new(&f.host_node_did, vec![did(&f.operator)]).unwrap();
    assert!(new("not-a-did", vec![did(&f.operator)]).is_err());
    assert!(new(&f.host_node_did, vec![did(&f.operator), "did:key:zbad".to_string()]).is_err());
    assert!(new(&f.host_node_did, vec![f.host_node_did.to_uppercase()]).is_err());
}

#[test]
fn a_registration_is_granted_only_by_the_registrar_that_checked_it() {
    let f = Fixture::relay();
    let other = Registrar::new(
        RELAY_HOME,
        f.host_node_did.clone(),
        vec![did(&f.operator)],
        LeaseStore::open_in_memory(&RELAY_HOME).unwrap(),
    )
    .unwrap();
    let token = f.token(Map::new());
    let proof = f.challenge_proof(&f.subject, &token, f.now);
    let checked = f.registrar.check(&token, &proof, &SignerBinding::SameKey, f.now).unwrap();
    assert!(matches!(other.grant(checked.clone(), f.now), Err(RegisterError::Store(_))));
    assert!(!other.leases().is_admitted(&did(&f.subject), f.now));
    f.registrar.grant(checked, f.now).unwrap();
}

#[test]
fn an_open_count_limit_applies_when_the_passport_has_no_count_key() {
    let capped = PassportUse { open_count_limit: Some(2), ..RELAY_HOME };
    let f = Fixture::new(capped);
    let passport = f.passport(Map::new(), PASSPORT_LIFE_SECS);
    let nodes: Vec<_> = (0..3).map(|_| identity()).collect();
    for node in &nodes[..2] {
        f.register_relay(&f.child(&passport, node), node, f.now).unwrap();
    }
    assert_eq!(
        f.register_relay(&f.child(&passport, &nodes[2]), &nodes[2], f.now).unwrap_err(),
        RegisterError::LimitReached { limit: 2 }
    );
}

#[test]
fn the_passport_count_key_wins_over_the_open_count_limit() {
    let capped = PassportUse { open_count_limit: Some(1), ..RELAY_HOME };
    let f = Fixture::new(capped);
    let passport = f.passport(limits(&[("max_nodes", json!(2))]), PASSPORT_LIFE_SECS);
    for node in (0..2).map(|_| identity()) {
        f.register_relay(&f.child(&passport, &node), &node, f.now).unwrap();
    }
}

#[test]
fn a_child_token_a_few_seconds_ahead_of_the_service_clock_registers() {
    let f = Fixture::relay();
    let passport = f.passport(Map::new(), PASSPORT_LIFE_SECS);
    let token = child_token(&f.owner, &passport, &did(&f.subject), 600, f.now + 60).unwrap();
    assert_eq!(token.not_before_secs, f.now + 60);
    f.register_relay(&token, &f.subject, f.now).unwrap();
}

#[test]
fn a_child_token_far_ahead_of_the_service_clock_is_refused() {
    let f = Fixture::relay();
    let passport = f.passport(Map::new(), PASSPORT_LIFE_SECS);
    let token = child_token(&f.owner, &passport, &did(&f.subject), 600, f.now + 1_000).unwrap();
    assert!(matches!(
        f.register_relay(&token, &f.subject, f.now),
        Err(RegisterError::InvalidChain(_))
    ));
}

fn identity() -> Identity {
    Identity::generate().unwrap()
}

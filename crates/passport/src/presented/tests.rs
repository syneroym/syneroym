use serde_json::Map;
use syneroym_ucan::{Ability, Capability, ResourceUri};

use super::*;
use crate::{
    RELAY_HOME,
    issue::PassportId,
    test_support::{Fixture, did, hand_child},
};

#[test]
fn a_good_child_token_gives_the_right_fields() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let subject_did = did(&f.subject);
    let presented = inspect_child_token(&token, &RELAY_HOME, &subject_did, f.now).unwrap();

    let expected_passport_id = PassportId::of(&token.proofs[0]).unwrap();
    assert_eq!(presented.passport_id, expected_passport_id);
    assert_eq!(presented.host_node_did, f.host_node_did);
    assert_eq!(presented.owner_did, did(&f.owner));
    assert_eq!(presented.subject, subject_did);
    assert_eq!(
        presented.access_ends_at,
        token.expires_at_secs.min(token.proofs[0].expires_at_secs)
    );
}

#[test]
fn wrong_subject_is_not_bound() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let other_did = did(&f.owner);
    let err = inspect_child_token(&token, &RELAY_HOME, &other_did, f.now).unwrap_err();
    assert!(matches!(err, RegisterError::NotBound));
}

#[test]
fn bare_substrate_resource_is_malformed() {
    let f = Fixture::relay();
    let mut passport = f.passport(Map::new(), 3600);
    passport.capabilities[0] = Capability {
        with: ResourceUri(format!("substrate:{}", f.host_node_did)),
        can: Ability(RELAY_HOME.ability.to_string()),
        caveats: None,
    };
    passport.sign(&f.operator).unwrap();
    let child = hand_child(&f.owner, &passport, &did(&f.subject), Map::new());

    let err = inspect_child_token(&child, &RELAY_HOME, &did(&f.subject), f.now).unwrap_err();
    assert!(matches!(err, RegisterError::Malformed(_)));
}

#[test]
fn mailbox_passport_checked_as_relay_is_malformed() {
    let f = Fixture::mailbox();
    let token = f.token(Map::new());
    let err = inspect_child_token(&token, &RELAY_HOME, &did(&f.subject), f.now).unwrap_err();
    assert!(matches!(err, RegisterError::Malformed(_)));
}

#[test]
fn expired_child_is_expired() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let err = inspect_child_token(&token, &RELAY_HOME, &did(&f.subject), token.expires_at_secs + 1)
        .unwrap_err();
    assert!(matches!(err, RegisterError::Expired));
}

#[test]
fn tampered_passport_fact_is_invalid_chain() {
    let f = Fixture::relay();
    let mut passport = f.passport(Map::new(), 3600);
    // Tamper with facts after signature
    passport.facts.insert("max_nodes".to_string(), serde_json::json!(10));
    let child = hand_child(&f.owner, &passport, &did(&f.subject), Map::new());

    let err = inspect_child_token(&child, &RELAY_HOME, &did(&f.subject), f.now).unwrap_err();
    assert!(matches!(err, RegisterError::InvalidChain(_)));
}

#[test]
fn child_with_two_proofs_is_malformed() {
    let f = Fixture::relay();
    let passport = f.passport(Map::new(), 3600);
    let mut child = hand_child(&f.owner, &passport, &did(&f.subject), Map::new());
    child.proofs.push(passport);
    child.sign(&f.owner).unwrap();

    let err = inspect_child_token(&child, &RELAY_HOME, &did(&f.subject), f.now).unwrap_err();
    assert!(matches!(err, RegisterError::Malformed(_)));
}

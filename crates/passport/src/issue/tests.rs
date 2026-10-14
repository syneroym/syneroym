use serde_json::json;
use syneroym_identity::Identity;

use super::*;
use crate::{
    passport_use::{MAILBOX_USE, RELAY_HOME},
    test_support::{did, limits, new_did, other_spelling},
};

/// Far from the wall clock, to show the functions never read it.
const NOW: u64 = 1_000_000;
const LIFE: u64 = 3_600;

struct Parties {
    operator: Identity,
    owner: Identity,
    host: String,
}

fn parties() -> Parties {
    Parties {
        operator: Identity::generate().unwrap(),
        owner: Identity::generate().unwrap(),
        host: new_did(),
    }
}

fn passport(p: &Parties, limits: Map<String, Value>) -> Result<CapabilityToken> {
    issue_passport(&p.operator, &p.host, &RELAY_HOME, &did(&p.owner), limits, LIFE, NOW)
}

#[test]
fn issue_refuses_a_limit_key_of_another_use() {
    let p = parties();
    let err = passport(&p, limits(&[("max_bytes", json!(5))])).unwrap_err();
    assert!(err.to_string().contains("max_bytes is not a use limit"), "{err}");
    issue_passport(
        &p.operator,
        &p.host,
        &MAILBOX_USE,
        &did(&p.owner),
        limits(&[("max_bytes", json!(5))]),
        LIFE,
        NOW,
    )
    .unwrap();
}

#[test]
fn issue_refuses_a_limit_that_is_not_a_positive_whole_number() {
    let p = parties();
    for bad in [json!(0), json!(-1), json!(1.5), json!("3")] {
        let err = passport(&p, limits(&[("max_nodes", bad.clone())])).unwrap_err();
        assert!(err.to_string().contains("whole number of at least 1"), "{bad}: {err}");
    }
}

#[test]
fn issue_refuses_a_zero_life() {
    let p = parties();
    let err = issue_passport(&p.operator, &p.host, &RELAY_HOME, &did(&p.owner), Map::new(), 0, NOW)
        .unwrap_err();
    assert!(err.to_string().contains("at least one second"), "{err}");
}

#[test]
fn issued_passport_names_the_selector_resource() {
    let p = parties();
    let token = passport(&p, limits(&[("max_nodes", json!(3))])).unwrap();
    assert_eq!(token.capabilities, vec![RELAY_HOME.capability(&p.host)]);
    assert_eq!(token.capabilities[0].with.0, format!("substrate:{}/relay", p.host));
    assert_eq!(token.issuer_did, did(&p.operator));
    assert_eq!(token.audience_did, did(&p.owner));
    assert_eq!(token.facts, limits(&[("max_nodes", json!(3))]));
    assert!(token.proofs.is_empty());
}

#[test]
fn issued_passport_uses_the_given_clock() {
    let p = parties();
    let token = passport(&p, Map::new()).unwrap();
    assert_eq!(token.not_before_secs, NOW);
    assert_eq!(token.expires_at_secs, NOW + LIFE);
}

#[test]
fn child_token_never_outlives_the_passport() {
    let p = parties();
    let token = passport(&p, Map::new()).unwrap();
    let child = child_token(&p.owner, &token, &new_did(), LIFE * 10, NOW).unwrap();
    assert_eq!(child.expires_at_secs, token.expires_at_secs);
    assert_eq!(child.not_before_secs, NOW);
    assert_eq!(child.proofs, vec![token]);
}

#[test]
fn child_token_refuses_a_passport_for_another_owner() {
    let p = parties();
    let token = passport(&p, Map::new()).unwrap();
    let stranger = Identity::generate().unwrap();
    let err = child_token(&stranger, &token, &new_did(), LIFE, NOW).unwrap_err();
    assert!(err.to_string().contains("not to the owner identity"), "{err}");
}

#[test]
fn child_token_refuses_an_expired_passport() {
    let p = parties();
    let token = passport(&p, Map::new()).unwrap();
    let err = child_token(&p.owner, &token, &new_did(), LIFE, NOW + LIFE).unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");
}

#[test]
fn child_token_refuses_a_token_that_is_not_a_passport() {
    let p = parties();
    let token = passport(&p, Map::new()).unwrap();
    let mut child = child_token(&p.owner, &token, &did(&p.owner), LIFE, NOW).unwrap();
    child.audience_did = did(&p.owner);
    let err = child_token(&p.owner, &child, &new_did(), LIFE, NOW).unwrap_err();
    assert!(err.to_string().contains("not a passport"), "{err}");
}

#[test]
fn child_token_has_no_facts() {
    let p = parties();
    let token = passport(&p, limits(&[("max_nodes", json!(2))])).unwrap();
    let child = child_token(&p.owner, &token, &new_did(), LIFE, NOW).unwrap();
    assert!(child.facts.is_empty());
}

/// The z-base-32 decoder may accept another spelling of the same key in the
/// spare bits of the last character. If it does, both builders refuse it.
#[test]
fn a_non_canonical_did_is_refused() {
    let p = parties();
    let owner_did = did(&p.owner);
    let Some(other_owner) = other_spelling(&owner_did) else {
        assert!(
            other_spelling(&new_did()).is_none(),
            "the decoder has no other spelling; the canonical check stays a cheap guard"
        );
        return;
    };
    let err =
        issue_passport(&p.operator, &p.host, &RELAY_HOME, &other_owner, Map::new(), LIFE, NOW)
            .unwrap_err();
    assert!(format!("{err:#}").contains("canonical"), "{err:#}");

    let token = passport(&p, Map::new()).unwrap();
    let subject = new_did();
    let other_subject = other_spelling(&subject).unwrap();
    let err = child_token(&p.owner, &token, &other_subject, LIFE, NOW).unwrap_err();
    assert!(format!("{err:#}").contains("canonical"), "{err:#}");
}

#[test]
fn passport_id_round_trips_and_rejects_bad_hex() {
    let p = parties();
    let id = PassportId::of(&passport(&p, Map::new()).unwrap()).unwrap();
    assert_eq!(id.as_str().len(), 64);
    assert_eq!(id.to_string().parse::<PassportId>().unwrap(), id);
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(serde_json::from_str::<PassportId>(&json).unwrap(), id);

    let hex = id.as_str();
    for bad in [&hex[..63], &hex.to_uppercase(), &format!("{}g", &hex[..63])] {
        assert!(bad.parse::<PassportId>().is_err(), "{bad} must be refused");
        assert!(serde_json::from_str::<PassportId>(&format!("\"{bad}\"")).is_err());
    }
}

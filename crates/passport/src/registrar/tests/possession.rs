use serde_json::{Map, json};
use syneroym_identity::{Identity, delegation};
use syneroym_ucan::CapabilityToken;

use crate::{
    CHALLENGE_LIFE_SECS, MAILBOX_USE, Proof, RegisterError, SignerBinding, child_token,
    sign_challenge,
    test_support::{CERT_LIFE_SECS, Fixture, PASSPORT_LIFE_SECS, did, limits, new_did},
};

/// A challenge-form proof whose signature is made by `signer` but which
/// names `claimed_did` as the signer.
fn claimed_proof(
    f: &Fixture,
    signer: &Identity,
    claimed_did: &str,
    token: &CapabilityToken,
) -> Proof {
    match f.challenge_proof(signer, token, f.now) {
        Proof::Challenge { challenge, signature, .. } => {
            Proof::Challenge { challenge, signer_did: claimed_did.to_string(), signature }
        }
        Proof::Transport { .. } => unreachable!(),
    }
}

#[test]
fn a_leaked_passport_and_child_token_are_not_enough() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let attacker = Identity::generate().unwrap();
    let same_key = SignerBinding::SameKey;
    let check = |proof: &Proof, binding: &SignerBinding| {
        f.registrar.check(&token, proof, binding, f.now).map(|_| ())
    };

    let as_subject = claimed_proof(&f, &attacker, &did(&f.subject), &token);
    assert_eq!(check(&as_subject, &same_key), Err(RegisterError::BadSignature));

    let as_attacker = f.challenge_proof(&attacker, &token, f.now);
    assert_eq!(check(&as_attacker, &same_key), Err(RegisterError::NotBound));

    let transport = Fixture::transport_proof(&did(&attacker));
    assert_eq!(check(&transport, &same_key), Err(RegisterError::NotBound));

    let attacker_master = Identity::generate().unwrap();
    let own_certificate = Fixture::instance_binding(&attacker_master, &attacker);
    assert_eq!(check(&transport, &own_certificate), Err(RegisterError::NotBound));
}

#[test]
fn an_instance_certificate_must_match_master_key_and_scope() {
    let f = Fixture::mailbox();
    let token = f.token(Map::new());
    let proof = Fixture::transport_proof(&did(&f.instance));
    let check = |binding: &SignerBinding, at: u64| {
        f.registrar.check(&token, &proof, binding, at).map(|_| ())
    };
    let other = Identity::generate().unwrap();

    let wrong_master = Fixture::instance_binding(&other, &f.instance);
    assert_eq!(check(&wrong_master, f.now), Err(RegisterError::NotBound));

    let wrong_key = Fixture::instance_binding(&f.subject, &other);
    assert_eq!(check(&wrong_key, f.now), Err(RegisterError::NotBound));

    let routing = Fixture::certificate(&f.subject, &f.instance, delegation::SCOPE_ROUTING);
    assert_eq!(check(&routing, f.now), Err(RegisterError::NotBound));

    // The certificate reads the wall clock, so test well past its end.
    let valid = Fixture::instance_binding(&f.subject, &f.instance);
    assert_eq!(check(&valid, f.now), Ok(()));
    const { assert!(PASSPORT_LIFE_SECS > CERT_LIFE_SECS + 5) };
    assert_eq!(check(&valid, f.now + CERT_LIFE_SECS + 5), Err(RegisterError::NotBound));
}

#[test]
fn a_challenge_signature_for_another_service_is_refused() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let challenge = f.registrar.issue_challenge(f.now).unwrap();
    let signature = sign_challenge(&f.subject, &new_did(), &f.use_(), &challenge, &token).unwrap();
    let proof = Proof::Challenge { challenge, signer_did: did(&f.subject), signature };
    assert_eq!(
        f.register_with(&token, &proof, &SignerBinding::SameKey, f.now),
        Err(RegisterError::BadSignature)
    );
}

#[test]
fn a_challenge_signature_for_another_token_is_refused() {
    let f = Fixture::relay();
    let passport = f.passport(Map::new(), PASSPORT_LIFE_SECS);
    let child_a = f.child(&passport, &f.subject);
    let child_b = child_token(&f.owner, &passport, &did(&f.subject), 100, f.now).unwrap();
    let proof = f.challenge_proof(&f.subject, &child_a, f.now);
    assert_eq!(
        f.register_with(&child_b, &proof, &SignerBinding::SameKey, f.now),
        Err(RegisterError::BadSignature)
    );
}

#[test]
fn a_challenge_signature_for_the_other_use_is_refused() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let challenge = f.registrar.issue_challenge(f.now).unwrap();
    let signature =
        sign_challenge(&f.subject, &f.host_node_did, &MAILBOX_USE, &challenge, &token).unwrap();
    let proof = Proof::Challenge { challenge, signer_did: did(&f.subject), signature };
    assert_eq!(
        f.register_with(&token, &proof, &SignerBinding::SameKey, f.now),
        Err(RegisterError::BadSignature)
    );
}

#[test]
fn a_challenge_is_single_use_even_after_a_bad_signature() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let good = f.challenge_proof(&f.subject, &token, f.now);
    let Proof::Challenge { challenge, signer_did, signature } = good else { unreachable!() };
    let bad = Proof::Challenge {
        challenge,
        signer_did: signer_did.clone(),
        signature: f.subject.sign_json(&json!({"other": 1})).unwrap(),
    };
    let same_key = SignerBinding::SameKey;
    assert_eq!(f.register_with(&token, &bad, &same_key, f.now), Err(RegisterError::BadSignature));
    let good = Proof::Challenge { challenge, signer_did, signature };
    assert_eq!(
        f.register_with(&token, &good, &same_key, f.now),
        Err(RegisterError::UnknownChallenge)
    );
}

#[test]
fn an_old_challenge_is_refused() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let proof = f.challenge_proof(&f.subject, &token, f.now);
    let at = f.now + CHALLENGE_LIFE_SECS;
    assert_eq!(
        f.register_with(&token, &proof, &SignerBinding::SameKey, at),
        Err(RegisterError::ChallengeExpired)
    );
}

/// A service that refuses with `Revoked` after `check` leaves no lease.
#[test]
fn check_writes_nothing() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let proof = f.challenge_proof(&f.subject, &token, f.now);
    let checked = f.registrar.check(&token, &proof, &SignerBinding::SameKey, f.now).unwrap();
    assert_eq!(checked.token(), &token);
    assert_eq!(checked.lease().subject, did(&f.subject));
    assert!(!f.registrar.leases().is_admitted(&did(&f.subject), f.now));
    assert!(f.registrar.leases().running_leases(f.now).is_empty());
}

#[test]
fn grant_refuses_a_lease_that_ended_during_the_lookup() {
    let f = Fixture::relay();
    let passport = f.passport(Map::new(), PASSPORT_LIFE_SECS);
    let token = child_token(&f.owner, &passport, &did(&f.subject), 10, f.now).unwrap();
    let proof = f.challenge_proof(&f.subject, &token, f.now);
    let checked = f.registrar.check(&token, &proof, &SignerBinding::SameKey, f.now).unwrap();
    assert_eq!(f.registrar.grant(checked, f.now + 10), Err(RegisterError::Expired));
}

#[test]
fn a_deny_between_check_and_grant_wins() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let proof = f.challenge_proof(&f.subject, &token, f.now);
    let checked = f.registrar.check(&token, &proof, &SignerBinding::SameKey, f.now).unwrap();
    f.registrar.leases().deny(&checked.lease().passport_id, f.now).unwrap();
    assert_eq!(f.registrar.grant(checked, f.now), Err(RegisterError::Denied));
}

#[test]
fn an_unknown_challenge_is_refused() {
    let f = Fixture::relay();
    let token = f.token(Map::new());
    let other = Fixture::relay();
    let foreign = other.challenge_proof(&f.subject, &token, f.now);
    assert_eq!(
        f.register_with(&token, &foreign, &SignerBinding::SameKey, f.now),
        Err(RegisterError::UnknownChallenge)
    );
}

/// The reason the challenge signature covers the whole presented token:
/// the signed body of the child token leaves out its proofs.
#[test]
fn a_swapped_passport_under_the_same_child_is_refused() {
    let f = Fixture::relay();
    let first = f.passport(Map::new(), PASSPORT_LIFE_SECS);
    let second = f.passport(limits(&[("max_nodes", json!(9))]), PASSPORT_LIFE_SECS);
    let child = f.child(&first, &f.subject);
    let mut swapped = child.clone();
    swapped.proofs = vec![second];
    let proof = f.challenge_proof(&f.subject, &child, f.now);
    assert_eq!(
        f.register_with(&swapped, &proof, &SignerBinding::SameKey, f.now),
        Err(RegisterError::BadSignature)
    );
}

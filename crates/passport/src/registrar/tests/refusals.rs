use serde_json::{Map, json};
use syneroym_identity::Identity;
use syneroym_ucan::{Ability, Capability, CapabilityToken, ResourceUri};

use crate::{
    Lease, PassportId, RegisterError, child_token,
    test_support::{
        Fixture, PASSPORT_LIFE_SECS, both, did, hand_child, limits, new_did, other_spelling,
    },
};

fn malformed(result: Result<Lease, RegisterError>) -> bool {
    matches!(result, Err(RegisterError::Malformed(_)))
}

/// `passport` changed by `edit`, then signed again by the operator, and
/// delegated to the fixture subject by hand (so the child copies the
/// changed capability).
fn edited(f: &Fixture, edit: impl FnOnce(&mut CapabilityToken)) -> CapabilityToken {
    let mut passport = f.passport(Map::new(), PASSPORT_LIFE_SECS);
    edit(&mut passport);
    passport.sign(&f.operator).unwrap();
    hand_child(&f.owner, &passport, &did(&f.subject), Map::new())
}

#[test]
fn malformed_chain_shapes_are_refused() {
    for f in both() {
        let token = f.token(Map::new());
        let passport = token.proofs[0].clone();

        let mut no_proof = token.clone();
        no_proof.proofs.clear();
        assert!(malformed(f.register(&no_proof, f.now)), "no proof");

        let mut two_proofs = token.clone();
        two_proofs.proofs.push(passport.clone());
        assert!(malformed(f.register(&two_proofs, f.now)), "two proofs");

        let mut deep = passport.clone();
        deep.proofs.push(passport.clone());
        let three_tokens = hand_child(&f.owner, &deep, &did(&f.subject), Map::new());
        assert!(malformed(f.register(&three_tokens, f.now)), "three tokens");

        let mut two_caps = token.clone();
        two_caps.capabilities.push(two_caps.capabilities[0].clone());
        two_caps.sign(&f.owner).unwrap();
        assert!(malformed(f.register(&two_caps, f.now)), "two capabilities in the child");

        let two_caps_passport = edited(&f, |p| p.capabilities.push(p.capabilities[0].clone()));
        assert!(malformed(f.register(&two_caps_passport, f.now)), "two in the passport");

        let count_key = f.use_().count_key;
        let text_limit = edited(&f, |p| {
            p.facts.insert(count_key.to_string(), json!("3"));
        });
        assert!(malformed(f.register(&text_limit, f.now)), "a count that is text");
    }
}

#[test]
fn a_passport_from_an_untrusted_root_is_refused() {
    for f in both() {
        let stranger = Identity::generate().unwrap();
        let passport = f.passport_from(&stranger, Map::new(), PASSPORT_LIFE_SECS);
        let token = f.child(&passport, &f.subject);
        assert_eq!(f.register(&token, f.now), Err(RegisterError::UntrustedRoot));
    }
}

#[test]
fn a_child_from_someone_other_than_the_passport_audience_is_refused() {
    for f in both() {
        let passport = f.passport(Map::new(), PASSPORT_LIFE_SECS);
        let stranger = Identity::generate().unwrap();
        let token = hand_child(&stranger, &passport, &did(&f.subject), Map::new());
        assert_eq!(f.register(&token, f.now), Err(RegisterError::BrokenChain));
    }
}

#[test]
fn a_wrong_ability_or_resource_is_refused() {
    for f in both() {
        let use_ = f.use_();
        let other_ability = if use_.ability == Ability::RELAY_HOME {
            Ability::MAILBOX_USE
        } else {
            Ability::RELAY_HOME
        };
        let host = &f.host_node_did;
        let wrong = [
            (use_.resource(host).0, other_ability),
            (format!("substrate:{host}"), use_.ability),
            (format!("substrate:{}/{}", new_did(), use_.selector), use_.ability),
            (format!("substrate:{host}"), Ability::SUBSTRATE_ADMIN),
        ];
        for (with, can) in wrong {
            let cap = Capability {
                with: ResourceUri(with.clone()),
                can: Ability(can.to_string()),
                caveats: None,
            };
            let token = edited(&f, |p| p.capabilities = vec![cap]);
            assert!(
                matches!(f.register(&token, f.now), Err(RegisterError::WrongCapability { .. })),
                "{can} on {with}"
            );
        }
    }
}

/// Every token time comes from `Fixture::now` and every helper issues its
/// challenge at `at`, so the boundary is exact. The life is above the
/// challenge life (a helper that issued its challenge at `now` would fail
/// with `ChallengeExpired`) and below the certificate life of the mailbox
/// fixture.
#[test]
fn an_expired_passport_or_child_is_refused() {
    const LIFE: u64 = 300;
    for make in [Fixture::relay, Fixture::mailbox] {
        let f = make();
        let short_passport = f.child(&f.passport(Map::new(), LIFE), &f.subject);
        let long_passport = f.passport(Map::new(), PASSPORT_LIFE_SECS);
        let short_child =
            child_token(&f.owner, &long_passport, &did(&f.subject), LIFE, f.now).unwrap();
        for token in [short_passport, short_child] {
            assert_eq!(f.register(&token, f.now + LIFE), Err(RegisterError::Expired));
            f.register(&token, f.now + LIFE - 1).unwrap();
        }
    }
}

/// When the owner is also a root, `verify_chain` alone would accept the
/// child token on its own and never read the passport's caveat.
#[test]
fn a_passport_that_forbids_delegation_is_refused() {
    for mut f in both() {
        let token =
            edited(&f, |p| p.capabilities[0].caveats = Some(json!({"can_delegate": false})));
        let root_sets = [vec![did(&f.operator)], vec![did(&f.operator), did(&f.owner)]];
        for roots in root_sets {
            f = f.with_roots(roots.clone());
            assert!(
                matches!(f.register(&token, f.now), Err(RegisterError::InvalidChain(_))),
                "roots: {roots:?}"
            );
        }
    }
}

/// `verify_chain` checks the passport signature even when the child token
/// is rooted on its own.
#[test]
fn a_tampered_passport_is_refused_when_the_owner_is_a_root() {
    for f in both() {
        let roots = vec![did(&f.operator), did(&f.owner)];
        let f = f.with_roots(roots);
        let mut token = f.token(limits(&[(f.use_().count_key, json!(1))]));
        token.proofs[0].facts.insert(f.use_().count_key.to_string(), json!(1_000));
        assert!(matches!(f.register(&token, f.now), Err(RegisterError::InvalidChain(_))));
    }
}

#[test]
fn a_non_canonical_subject_is_refused() {
    for f in both() {
        let Some(other) = other_spelling(&did(&f.subject)) else {
            assert!(
                other_spelling(&new_did()).is_none(),
                "the decoder has no other spelling; the canonical check stays a cheap guard"
            );
            return;
        };
        let passport = f.passport(Map::new(), PASSPORT_LIFE_SECS);
        let token = hand_child(&f.owner, &passport, &other, Map::new());
        assert!(malformed(f.register(&token, f.now)));
    }
}

#[test]
fn a_denied_passport_is_refused() {
    for f in both() {
        let token = f.token(Map::new());
        let id = PassportId::of(&token.proofs[0]).unwrap();
        f.register(&token, f.now).unwrap();
        f.registrar.leases().deny(&id, f.now).unwrap();
        assert!(!f.registrar.leases().is_admitted(&did(&f.subject), f.now));
        assert_eq!(f.register(&token, f.now), Err(RegisterError::Denied));
    }
}

#[test]
fn the_use_limit_is_enforced() {
    for f in both() {
        let passport = f.passport(limits(&[(f.use_().count_key, json!(1))]), PASSPORT_LIFE_SECS);
        f.register(&f.child(&passport, &f.subject), f.now).unwrap();
        let second = Identity::generate().unwrap();
        assert_eq!(
            f.register_as(&f.child(&passport, &second), &second, f.now),
            Err(RegisterError::LimitReached { limit: 1 })
        );
        f.register(&f.child(&passport, &f.subject), f.now + 1).unwrap();
    }
}

#[test]
fn limits_in_the_child_token_are_ignored() {
    for f in both() {
        let count_key = f.use_().count_key;
        let passport = f.passport(limits(&[(count_key, json!(1))]), PASSPORT_LIFE_SECS);
        f.register(&f.child(&passport, &f.subject), f.now).unwrap();
        let second = Identity::generate().unwrap();
        let wide =
            hand_child(&f.owner, &passport, &did(&second), limits(&[(count_key, json!(1_000))]));
        assert_eq!(
            f.register_as(&wide, &second, f.now),
            Err(RegisterError::LimitReached { limit: 1 })
        );
    }
}

use syneroym_identity::substrate::derive_did_key;

use super::*;
use crate::capability::{Ability, ResourceUri};

fn cap(resource: ResourceUri, ability: &str) -> Capability {
    Capability { with: resource, can: Ability(ability.to_string()), caveats: None }
}

fn no_root(_iss: &str, _cap: &Capability) -> bool {
    false
}

#[test]
fn happy_path_direct_root() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let alice_did = derive_did_key(&alice.public_key());

    let resource = ResourceUri::service("app1", "s1");
    let token = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let opts = ChainVerifyOpts {
        expected_audience_did: &alice_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };

    let granted = verify_chain(&token, &opts).unwrap();
    assert_eq!(granted, vec![cap(resource, Ability::DATA_LAYER_READ)]);
}

#[test]
fn happy_path_one_hop_attenuation() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let alice_did = derive_did_key(&alice.public_key());
    let bob_did = derive_did_key(&bob.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let owner_to_alice = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_ADMIN)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let alice_to_bob = CapabilityToken::issue(
        &alice,
        &bob_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_WRITE)],
        Map::new(),
        3600,
        vec![owner_to_alice],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let opts = ChainVerifyOpts {
        expected_audience_did: &bob_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    let granted = verify_chain(&alice_to_bob, &opts).unwrap();
    assert_eq!(granted, vec![cap(resource, Ability::DATA_LAYER_WRITE)]);
}

#[test]
fn escalation_blocked() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let alice_did = derive_did_key(&alice.public_key());
    let bob_did = derive_did_key(&bob.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let owner_to_alice = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let alice_to_bob = CapabilityToken::issue(
        &alice,
        &bob_did,
        vec![cap(resource, Ability::DATA_LAYER_WRITE)],
        Map::new(),
        3600,
        vec![owner_to_alice],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let opts = ChainVerifyOpts {
        expected_audience_did: &bob_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    let granted = verify_chain(&alice_to_bob, &opts).unwrap();
    assert!(granted.is_empty());
}

#[test]
fn untrusted_root_dropped() {
    let alice = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let bob_did = derive_did_key(&bob.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let token = CapabilityToken::issue(
        &alice,
        &bob_did,
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let opts = ChainVerifyOpts {
        expected_audience_did: &bob_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap(),
    };
    let granted = verify_chain(&token, &opts).unwrap();
    assert!(granted.is_empty());
}

#[test]
fn audience_mismatch_is_error() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let alice_did = derive_did_key(&alice.public_key());
    let bob_did = derive_did_key(&bob.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let token = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let opts = ChainVerifyOpts {
        expected_audience_did: &bob_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap(),
    };
    assert!(verify_chain(&token, &opts).is_err());
}

#[test]
fn expired_leaf_is_error() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let alice_did = derive_did_key(&alice.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let token = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        0,
        vec![],
    )
    .unwrap();

    let opts = ChainVerifyOpts {
        expected_audience_did: &alice_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap() + 1,
    };
    assert!(verify_chain(&token, &opts).is_err());
}

#[test]
fn expired_proof_is_error() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let alice_did = derive_did_key(&alice.public_key());
    let bob_did = derive_did_key(&bob.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let owner_to_alice = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_ADMIN)],
        Map::new(),
        0,
        vec![],
    )
    .unwrap();
    let alice_to_bob = CapabilityToken::issue(
        &alice,
        &bob_did,
        vec![cap(resource, Ability::DATA_LAYER_WRITE)],
        Map::new(),
        3600,
        vec![owner_to_alice],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let opts = ChainVerifyOpts {
        expected_audience_did: &bob_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap() + 1,
    };
    assert!(verify_chain(&alice_to_bob, &opts).is_err());
}

#[test]
fn tampered_signature_is_error() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let alice_did = derive_did_key(&alice.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let mut token = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    token.signature = "a".repeat(token.signature.len());

    let opts = ChainVerifyOpts {
        expected_audience_did: &alice_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap(),
    };
    assert!(verify_chain(&token, &opts).is_err());
}

#[test]
fn tampered_capability_after_signing_is_error() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let alice_did = derive_did_key(&alice.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let mut token = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    token.capabilities = vec![cap(resource, Ability::DATA_LAYER_ADMIN)];

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let opts = ChainVerifyOpts {
        expected_audience_did: &alice_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    assert!(verify_chain(&token, &opts).is_err());
}

#[test]
fn continuity_break_drops_the_capability() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let mallory = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let mallory_did = derive_did_key(&mallory.public_key());
    let bob_did = derive_did_key(&bob.public_key());
    let resource = ResourceUri::service("app1", "s1");

    // owner issues a valid proof addressed to mallory, not alice.
    let owner_to_mallory = CapabilityToken::issue(
        &owner,
        &mallory_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_ADMIN)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    // alice re-presents that proof as if it delegated to her (continuity
    // break: proof.audience_did == mallory_did != alice's issuer_did).
    let alice_to_bob = CapabilityToken::issue(
        &alice,
        &bob_did,
        vec![cap(resource, Ability::DATA_LAYER_WRITE)],
        Map::new(),
        3600,
        vec![owner_to_mallory],
    )
    .unwrap();

    let opts = ChainVerifyOpts {
        expected_audience_did: &bob_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap(),
    };
    let granted = verify_chain(&alice_to_bob, &opts).unwrap();
    assert!(granted.is_empty());
}

#[test]
fn substrate_scope_covers_any_resource_via_has_capability() {
    use crate::session::SessionContext;

    let owner = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let bob_did = derive_did_key(&bob.public_key());

    let token = CapabilityToken::issue(
        &owner,
        &bob_did,
        vec![cap(ResourceUri::substrate(&admin_root), Ability::SUBSTRATE_ADMIN)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let opts = ChainVerifyOpts {
        expected_audience_did: &bob_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    let session = SessionContext::from_verified_chain(&token, &opts).unwrap();
    let arbitrary_resource = ResourceUri::service("app-anything", "svc-anything");
    assert!(
        session
            .has_capability(&arbitrary_resource, &Ability(Ability::DATA_LAYER_ADMIN.to_string()))
    );
}

#[test]
fn facts_from_a_self_issued_leaf_are_dropped_even_with_a_backed_capability() {
    use crate::session::SessionContext;

    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let alice_did = derive_did_key(&alice.public_key());
    let resource = ResourceUri::service("app1", "s1");

    // owner grants alice a real, attenuable capability.
    let owner_to_alice = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_ADMIN)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    // alice self-issues the *leaf* she presents -- addressed to
    // herself, backed by the real proof above -- and stuffs it with
    // fabricated facts. Nothing about the capability-attenuation logic
    // touches `facts`, so without a trust check they would ride along
    // for free.
    let mut fabricated_facts = Map::new();
    fabricated_facts.insert("role".to_string(), Value::String("admin".to_string()));
    let self_issued_leaf = CapabilityToken::issue(
        &alice,
        &alice_did,
        vec![cap(resource, Ability::DATA_LAYER_WRITE)],
        fabricated_facts,
        3600,
        vec![owner_to_alice],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let opts = ChainVerifyOpts {
        expected_audience_did: &alice_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    let session = SessionContext::from_verified_chain(&self_issued_leaf, &opts).unwrap();

    // The capability legitimately attenuates through the real proof...
    assert!(!session.capabilities.is_empty());
    // ...but the self-issued leaf's fabricated facts must never be
    // trusted, since alice (the leaf's issuer) is not the root.
    assert!(session.claims.is_empty());
}

#[test]
fn chain_exceeding_max_nodes_is_rejected() {
    let root = Identity::generate().unwrap();
    let root_did = derive_did_key(&root.public_key());
    let resource = ResourceUri::service("app1", "s1");

    // A linear chain of MAX_CHAIN_NODES + 1 tokens (root -> h0 -> h1 ->
    // ... -> leaf) -- one over the cap.
    let identities: Vec<Identity> =
        (0..=MAX_CHAIN_NODES).map(|_| Identity::generate().unwrap()).collect();
    let mut chain = CapabilityToken::issue(
        &root,
        &derive_did_key(&identities[0].public_key()),
        vec![cap(resource.clone(), Ability::DATA_LAYER_ADMIN)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    for i in 0..identities.len() - 1 {
        chain = CapabilityToken::issue(
            &identities[i],
            &derive_did_key(&identities[i + 1].public_key()),
            vec![cap(resource.clone(), Ability::DATA_LAYER_ADMIN)],
            Map::new(),
            3600,
            vec![chain],
        )
        .unwrap();
    }
    let leaf_did = derive_did_key(&identities.last().unwrap().public_key());

    let is_root = |iss: &str, _cap: &Capability| iss == root_did;
    let opts = ChainVerifyOpts {
        expected_audience_did: &leaf_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };

    let err = verify_chain(&chain, &opts).unwrap_err();
    assert!(err.to_string().contains("more than"));
}

/// ADR-0015 A3: a `can_delegate: false` parent capability does not back
/// a child's attenuated capability -- the delegation attempt is dropped
/// (fail-closed), not an error, matching every other "not backed"
/// outcome in this module.
#[test]
fn can_delegate_false_blocks_further_delegation() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let alice_did = derive_did_key(&alice.public_key());
    let bob_did = derive_did_key(&bob.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let non_delegable = Capability {
        with: resource.clone(),
        can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
        caveats: Some(serde_json::json!({"can_delegate": false})),
    };
    let owner_to_alice =
        CapabilityToken::issue(&owner, &alice_did, vec![non_delegable], Map::new(), 3600, vec![])
            .unwrap();
    let alice_to_bob = CapabilityToken::issue(
        &alice,
        &bob_did,
        vec![cap(resource, Ability::DATA_LAYER_WRITE)],
        Map::new(),
        3600,
        vec![owner_to_alice],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let opts = ChainVerifyOpts {
        expected_audience_did: &bob_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    let granted = verify_chain(&alice_to_bob, &opts).unwrap();
    assert!(granted.is_empty(), "a can_delegate: false capability must not back a child's");
}

/// The block is terminal across hops, not just at the first one: a
/// grandchild attenuated through an intermediate that itself received
/// nothing (because its own parent was `can_delegate: false`) also gets
/// nothing -- there is no capability to re-derive from downstream.
#[test]
fn can_delegate_false_is_terminal_across_two_hops() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let bob = Identity::generate().unwrap();
    let carol = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let alice_did = derive_did_key(&alice.public_key());
    let bob_did = derive_did_key(&bob.public_key());
    let carol_did = derive_did_key(&carol.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let non_delegable = Capability {
        with: resource.clone(),
        can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
        caveats: Some(serde_json::json!({"can_delegate": false})),
    };
    let owner_to_alice =
        CapabilityToken::issue(&owner, &alice_did, vec![non_delegable], Map::new(), 3600, vec![])
            .unwrap();
    let alice_to_bob = CapabilityToken::issue(
        &alice,
        &bob_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_WRITE)],
        Map::new(),
        3600,
        vec![owner_to_alice],
    )
    .unwrap();
    let bob_to_carol = CapabilityToken::issue(
        &bob,
        &carol_did,
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![alice_to_bob],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let opts = ChainVerifyOpts {
        expected_audience_did: &carol_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    let granted = verify_chain(&bob_to_carol, &opts).unwrap();
    assert!(granted.is_empty(), "a can_delegate: false block must not be re-derivable downstream");
}

#[test]
fn from_verified_chain_populates_fields() {
    use crate::session::SessionContext;

    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let alice_did = derive_did_key(&alice.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let mut facts = Map::new();
    facts.insert("tenant".to_string(), Value::String("acme".to_string()));

    let now = now_secs().unwrap();
    let token = CapabilityToken::issue(
        &owner,
        &alice_did,
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        facts.clone(),
        3600,
        vec![],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let opts = ChainVerifyOpts {
        expected_audience_did: &alice_did,
        is_trusted_root: &is_root,
        now_secs: now,
    };
    let session = SessionContext::from_verified_chain(&token, &opts).unwrap();

    assert_eq!(session.subject_did, alice_did);
    assert_eq!(session.capabilities.len(), 1);
    assert_eq!(session.claims, facts);
    assert_eq!(session.verified_at_secs, now);
}

// -- ADR-0015 A5 (amended): anchor-stamp propagation invariant, over a
// -- table of chain shapes plus the attack cases.

#[test]
fn owner_rooted_anchor_propagates_through_two_service_hops() {
    use crate::session::SessionContext;

    let user_a = Identity::generate().unwrap();
    let svc_1 = Identity::generate().unwrap();
    let user_a_did = derive_did_key(&user_a.public_key());
    let svc_1_did = derive_did_key(&svc_1.public_key());
    let svc_2_did = "did:key:zSvc2Placeholder";
    let resource = ResourceUri::service("app1", "s1");

    let user_a_to_svc1 = CapabilityToken::issue_with_anchor(
        &user_a,
        &svc_1_did,
        Some(user_a_did.clone()),
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let svc1_to_svc2 = CapabilityToken::issue_with_anchor(
        &svc_1,
        svc_2_did,
        Some(user_a_did.clone()),
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![user_a_to_svc1],
    )
    .unwrap();

    let opts = ChainVerifyOpts {
        expected_audience_did: svc_2_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap(),
    };
    assert!(verify_chain(&svc1_to_svc2, &opts).is_ok());
    let session = SessionContext::from_verified_chain(&svc1_to_svc2, &opts).unwrap();
    assert_eq!(session.anchor_did, Some(user_a_did));
}

#[test]
fn admin_rooted_anchor_self_stamps_at_first_service_delegation() {
    use crate::session::SessionContext;

    let root_admin = Identity::generate().unwrap();
    let user_a = Identity::generate().unwrap();
    let svc_1 = Identity::generate().unwrap();
    let user_a_did = derive_did_key(&user_a.public_key());
    let svc_1_did = derive_did_key(&svc_1.public_key());
    let svc_2_did = "did:key:zSvc2Placeholder";
    let resource = ResourceUri::service("app1", "s1");

    // The admin->user_A grant carries no anchor -- user_A self-stamps
    // only when it first delegates to a service.
    let root_admin_to_user_a = CapabilityToken::issue(
        &root_admin,
        &user_a_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_ADMIN)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let user_a_to_svc1 = CapabilityToken::issue_with_anchor(
        &user_a,
        &svc_1_did,
        Some(user_a_did.clone()),
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![root_admin_to_user_a],
    )
    .unwrap();
    let svc1_to_svc2 = CapabilityToken::issue_with_anchor(
        &svc_1,
        svc_2_did,
        Some(user_a_did.clone()),
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![user_a_to_svc1],
    )
    .unwrap();

    let opts = ChainVerifyOpts {
        expected_audience_did: svc_2_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap(),
    };
    assert!(verify_chain(&svc1_to_svc2, &opts).is_ok());
    let session = SessionContext::from_verified_chain(&svc1_to_svc2, &opts).unwrap();
    assert_eq!(session.anchor_did, Some(user_a_did));
}

#[test]
fn three_hop_pass_through_anchor_survives_every_hop() {
    use crate::session::SessionContext;

    let user_a = Identity::generate().unwrap();
    let svc_1 = Identity::generate().unwrap();
    let svc_2 = Identity::generate().unwrap();
    let user_a_did = derive_did_key(&user_a.public_key());
    let svc_1_did = derive_did_key(&svc_1.public_key());
    let svc_2_did = derive_did_key(&svc_2.public_key());
    let svc_3_did = "did:key:zSvc3Placeholder";
    let resource = ResourceUri::service("app1", "s1");

    let hop1 = CapabilityToken::issue_with_anchor(
        &user_a,
        &svc_1_did,
        Some(user_a_did.clone()),
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let hop2 = CapabilityToken::issue_with_anchor(
        &svc_1,
        &svc_2_did,
        Some(user_a_did.clone()),
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![hop1],
    )
    .unwrap();
    let hop3 = CapabilityToken::issue_with_anchor(
        &svc_2,
        svc_3_did,
        Some(user_a_did.clone()),
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![hop2],
    )
    .unwrap();

    let opts = ChainVerifyOpts {
        expected_audience_did: svc_3_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap(),
    };
    assert!(verify_chain(&hop3, &opts).is_ok());
    let session = SessionContext::from_verified_chain(&hop3, &opts).unwrap();
    assert_eq!(session.anchor_did, Some(user_a_did));
}

#[test]
fn direct_grant_with_no_anchor_leaves_session_anchor_did_none() {
    use crate::session::SessionContext;

    let owner = Identity::generate().unwrap();
    let alice_did = "did:key:zAlicePlaceholder";
    let resource = ResourceUri::service("app1", "s1");

    let token = CapabilityToken::issue(
        &owner,
        alice_did,
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let opts = ChainVerifyOpts {
        expected_audience_did: alice_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap(),
    };
    let session = SessionContext::from_verified_chain(&token, &opts).unwrap();
    assert_eq!(session.anchor_did, None);
}

/// The confused-deputy defense's crux: a middle service cannot upgrade
/// the anchor to a principal it was never delegated from. `svc_1`
/// receives no anchor grant for `mallory` yet asserts
/// `anchor_did = mallory` when delegating to `svc_2` -- neither
/// self-declared (`mallory != svc_1`) nor inherited (its proof carries
/// no `mallory` anchor), so the whole chain rejects with a hard `Err`.
#[test]
fn middle_service_rewriting_anchor_to_an_undelegated_principal_is_rejected() {
    let user_a = Identity::generate().unwrap();
    let svc_1 = Identity::generate().unwrap();
    let mallory = Identity::generate().unwrap();
    let user_a_did = derive_did_key(&user_a.public_key());
    let svc_1_did = derive_did_key(&svc_1.public_key());
    let mallory_did = derive_did_key(&mallory.public_key());
    let svc_2_did = "did:key:zSvc2Placeholder";
    let resource = ResourceUri::service("app1", "s1");

    let user_a_to_svc1 = CapabilityToken::issue_with_anchor(
        &user_a,
        &svc_1_did,
        Some(user_a_did.clone()),
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    // svc_1 asserts an anchor it was never delegated -- an escalation
    // attempt, not a legitimate self-declared downgrade or inheritance.
    let svc1_to_svc2 = CapabilityToken::issue_with_anchor(
        &svc_1,
        svc_2_did,
        Some(mallory_did),
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![user_a_to_svc1],
    )
    .unwrap();

    // user_a roots its own grant so the capability is actually admitted
    // (and the anchor check, which only runs against admitted
    // capabilities, gets a chance to fire).
    let is_root = |iss: &str, _cap: &Capability| iss == user_a_did;
    let opts = ChainVerifyOpts {
        expected_audience_did: svc_2_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    let err = verify_chain(&svc1_to_svc2, &opts).unwrap_err();
    assert!(err.to_string().contains("anchor_did"));
}

/// Anchor inheritance must be substantiated by the *same* proof that
/// backs the capability being exercised, not by an unrelated sibling
/// proof. `svc_1` genuinely holds two independent grants -- `medical`
/// from the admin root (no anchor involved) and `calendar` from
/// `user_a` (anchor = `user_a`) -- and combines them into one
/// self-issued leaf asserting `medical` under `anchor = user_a`. The
/// `user_a` proof never actually authorized anything about `medical`,
/// so this must reject even though every individual proof in the
/// presentation is genuine and validly signed.
#[test]
fn anchor_inherited_from_an_unrelated_capabilitys_proof_is_rejected() {
    let admin_root = Identity::generate().unwrap();
    let user_a = Identity::generate().unwrap();
    let svc_1 = Identity::generate().unwrap();
    let admin_root_did = derive_did_key(&admin_root.public_key());
    let user_a_did = derive_did_key(&user_a.public_key());
    let svc_1_did = derive_did_key(&svc_1.public_key());
    let svc_2_did = "did:key:zSvc2Placeholder";
    let medical = ResourceUri::service("app1", "medical");
    let calendar = ResourceUri::service("app1", "calendar");

    let admin_root_to_svc1 = CapabilityToken::issue(
        &admin_root,
        &svc_1_did,
        vec![cap(medical.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let user_a_to_svc1 = CapabilityToken::issue_with_anchor(
        &user_a,
        &svc_1_did,
        Some(user_a_did.clone()),
        vec![cap(calendar, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let svc1_leaf = CapabilityToken::issue_with_anchor(
        &svc_1,
        svc_2_did,
        Some(user_a_did),
        vec![cap(medical, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![admin_root_to_svc1, user_a_to_svc1],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == admin_root_did;
    let opts = ChainVerifyOpts {
        expected_audience_did: svc_2_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    let err = verify_chain(&svc1_leaf, &opts).unwrap_err();
    assert!(err.to_string().contains("anchor_did"));
}

/// A service *can* self-declare `anchor = itself`, discarding an
/// inherited anchor -- "acting as myself" is harmless (never an
/// escalation), unlike asserting someone else's principal.
#[test]
fn self_declared_downgrade_to_acting_as_self_is_accepted() {
    use crate::session::SessionContext;

    let user_a = Identity::generate().unwrap();
    let svc_1 = Identity::generate().unwrap();
    let user_a_did = derive_did_key(&user_a.public_key());
    let svc_1_did = derive_did_key(&svc_1.public_key());
    let svc_2_did = "did:key:zSvc2Placeholder";
    let resource = ResourceUri::service("app1", "s1");

    let user_a_to_svc1 = CapabilityToken::issue_with_anchor(
        &user_a,
        &svc_1_did,
        Some(user_a_did),
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let svc1_to_svc2 = CapabilityToken::issue_with_anchor(
        &svc_1,
        svc_2_did,
        Some(svc_1_did.clone()),
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![user_a_to_svc1],
    )
    .unwrap();

    let opts = ChainVerifyOpts {
        expected_audience_did: svc_2_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap(),
    };
    assert!(verify_chain(&svc1_to_svc2, &opts).is_ok());
    let session = SessionContext::from_verified_chain(&svc1_to_svc2, &opts).unwrap();
    assert_eq!(session.anchor_did, Some(svc_1_did));
}

/// `anchor_did` is part of `signing_value()`, so tampering it after
/// issuance breaks the signature exactly like tampering any other field.
#[test]
fn anchor_did_tamper_after_signing_fails_signature_verification() {
    let owner = Identity::generate().unwrap();
    let mallory = Identity::generate().unwrap();
    let alice_did = "did:key:zAlicePlaceholder";
    let mallory_did = derive_did_key(&mallory.public_key());
    let resource = ResourceUri::service("app1", "s1");

    let mut token = CapabilityToken::issue_with_anchor(
        &owner,
        alice_did,
        Some(derive_did_key(&owner.public_key())),
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    token.anchor_did = Some(mallory_did);

    let opts = ChainVerifyOpts {
        expected_audience_did: alice_did,
        is_trusted_root: &no_root,
        now_secs: now_secs().unwrap(),
    };
    assert!(verify_chain(&token, &opts).is_err());
}

/// Anchor validation runs on *every* node of the chain, not only the
/// presented leaf -- a mid-chain unsubstantiated-anchor rewrite must
/// abort the whole verification even when the leaf being verified is
/// two hops downstream of the violation, not the violator itself.
#[test]
fn mid_chain_anchor_rewrite_aborts_the_whole_chain_not_just_the_leaf() {
    let user_a = Identity::generate().unwrap();
    let svc_1 = Identity::generate().unwrap();
    let svc_2 = Identity::generate().unwrap();
    let mallory = Identity::generate().unwrap();
    let user_a_did = derive_did_key(&user_a.public_key());
    let svc_1_did = derive_did_key(&svc_1.public_key());
    let svc_2_did = derive_did_key(&svc_2.public_key());
    let mallory_did = derive_did_key(&mallory.public_key());
    let svc_3_did = "did:key:zSvc3Placeholder";
    let resource = ResourceUri::service("app1", "s1");

    let user_a_to_svc1 = CapabilityToken::issue_with_anchor(
        &user_a,
        &svc_1_did,
        Some(user_a_did.clone()),
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    // Mid-chain violation: svc_1 asserts an anchor it was never
    // delegated. Not the leaf being verified below.
    let svc1_to_svc2 = CapabilityToken::issue_with_anchor(
        &svc_1,
        &svc_2_did,
        Some(mallory_did),
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![user_a_to_svc1],
    )
    .unwrap();
    // The leaf itself inherits consistently from its own immediate
    // proof (svc1_to_svc2) -- its own local check would pass in
    // isolation; only recursing into svc1_to_svc2's own violation
    // catches this.
    let svc2_to_svc3 = CapabilityToken::issue_with_anchor(
        &svc_2,
        svc_3_did,
        Some(svc_1_did.clone()),
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![svc1_to_svc2],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == user_a_did || iss == svc_1_did;
    let opts = ChainVerifyOpts {
        expected_audience_did: svc_3_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    let err = verify_chain(&svc2_to_svc3, &opts).unwrap_err();
    assert!(err.to_string().contains("anchor_did"));
}

/// A continuity-broken proof cannot substantiate an inherited anchor no
/// matter what value it carries -- it is addressed to a third party
/// (`mallory`, not this token's issuer), so it never actually delegated
/// anything to the issuer asserting the anchor.
#[test]
fn continuity_broken_proof_cannot_substantiate_an_inherited_anchor() {
    let user_a = Identity::generate().unwrap();
    let svc_1 = Identity::generate().unwrap();
    let mallory = Identity::generate().unwrap();
    let user_a_did = derive_did_key(&user_a.public_key());
    let svc_1_did = derive_did_key(&svc_1.public_key());
    let mallory_did = derive_did_key(&mallory.public_key());
    let svc_2_did = "did:key:zSvc2Placeholder";
    let resource = ResourceUri::service("app1", "s1");

    // Legitimate backing for the capability -- carries no anchor.
    let user_a_to_svc1 = CapabilityToken::issue(
        &user_a,
        &svc_1_did,
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    // A real, validly-signed, correctly self-declared anchored token --
    // but addressed to mallory, not svc_1. Continuity-broken relative
    // to svc_1: it cannot be evidence for svc_1's anchor claim no
    // matter what value it carries.
    let user_a_to_mallory = CapabilityToken::issue_with_anchor(
        &user_a,
        &mallory_did,
        Some(user_a_did.clone()),
        vec![cap(resource.clone(), Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![],
    )
    .unwrap();
    let svc1_leaf = CapabilityToken::issue_with_anchor(
        &svc_1,
        svc_2_did,
        Some(user_a_did.clone()),
        vec![cap(resource, Ability::DATA_LAYER_READ)],
        Map::new(),
        3600,
        vec![user_a_to_svc1, user_a_to_mallory],
    )
    .unwrap();

    let is_root = |iss: &str, _cap: &Capability| iss == user_a_did;
    let opts = ChainVerifyOpts {
        expected_audience_did: svc_2_did,
        is_trusted_root: &is_root,
        now_secs: now_secs().unwrap(),
    };
    let err = verify_chain(&svc1_leaf, &opts).unwrap_err();
    assert!(err.to_string().contains("anchor_did"));
}

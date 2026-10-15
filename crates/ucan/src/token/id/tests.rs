use serde_json::{Map, json};
use sha2::{Digest, Sha256};
use syneroym_identity::{Identity, substrate};

use crate::{
    capability::{Ability, Capability, ResourceUri},
    token::CapabilityToken,
};

fn token() -> CapabilityToken {
    let issuer = Identity::generate().unwrap();
    let audience = Identity::generate().unwrap();
    let mut facts = Map::new();
    facts.insert("max_nodes".to_string(), json!(3));
    CapabilityToken::issue(
        &issuer,
        &substrate::derive_did_key(&audience.public_key()),
        vec![Capability {
            with: ResourceUri("substrate:did:key:hnode/relay".to_string()),
            can: Ability(Ability::RELAY_HOME.to_string()),
            caveats: None,
        }],
        facts,
        3600,
        vec![],
    )
    .unwrap()
}

#[test]
fn token_id_is_stable_for_the_same_token() {
    let token = token();
    assert_eq!(token.token_id().unwrap(), token.token_id().unwrap());
    let round_trip: CapabilityToken =
        serde_json::from_str(&serde_json::to_string(&token).unwrap()).unwrap();
    assert_eq!(round_trip.token_id().unwrap(), token.token_id().unwrap());
}

#[test]
fn token_id_changes_when_a_signed_field_changes() {
    let token = token();
    let id = token.token_id().unwrap();

    let mut other_facts = token.clone();
    other_facts.facts.insert("max_nodes".to_string(), json!(4));
    assert_ne!(other_facts.token_id().unwrap(), id);

    let mut other_expiry = token.clone();
    other_expiry.expires_at_secs += 1;
    assert_ne!(other_expiry.token_id().unwrap(), id);
}

#[test]
fn token_id_ignores_proofs_and_signature() {
    let token = token();
    let mut other = token.clone();
    other.signature = "another signature".to_string();
    other.proofs.push(token.clone());
    assert_eq!(other.token_id().unwrap(), token.token_id().unwrap());
}

/// `Identity::sign_json` signs exactly these bytes, so the id is the hash of
/// what the signature covers.
#[test]
fn token_id_is_the_hash_of_the_signed_bytes() {
    let token = token();
    let canonical = substrate::canonicalize_json_value(&token.signing_value());
    let expected: [u8; 32] =
        Sha256::digest(serde_json::to_string(&canonical).unwrap().as_bytes()).into();
    assert_eq!(token.token_id().unwrap(), expected);
}

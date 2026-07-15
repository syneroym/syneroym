//! Signed UCAN capability tokens and delegation-chain verification
//! (ADR-0015 §2).

use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use syneroym_identity::{Identity, substrate};

use crate::capability::{Capability, ResourceUri};

/// Clock-skew tolerance for `not_before` (mirrors `DelegationCertificate`'s
/// 300 s future-issue tolerance).
const CLOCK_SKEW_SECS: u64 = 300;

fn now_secs() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX epoch")
        .map(|d| d.as_secs())
}

/// A signed UCAN capability token (ADR-0015 §2). `proofs` are the parent
/// tokens forming the delegation chain. The Ed25519 signature covers the
/// RFC-8785 canonicalization of every field **except `signature` and
/// `proofs`** — each proof is independently signed by its own issuer, and
/// chain continuity (`proof.audience_did == child.issuer_did`) binds them, so
/// a valid proof cannot be repackaged under a child the proof's issuer never
/// signed for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityToken {
    pub issuer_did: String,
    pub audience_did: String,
    pub capabilities: Vec<Capability>,
    /// Proven claims surfaced into `SessionContext.claims` (co-design seam
    /// #1: M04B binds these as SQL `?` params). Empty by default.
    #[serde(default)]
    pub facts: Map<String, Value>,
    pub not_before_secs: u64,
    pub expires_at_secs: u64,
    #[serde(default)]
    pub proofs: Vec<CapabilityToken>,
    /// z-base-32 Ed25519 signature over the canonical body (sans this field
    /// and `proofs`).
    pub signature: String,
}

impl CapabilityToken {
    fn signing_value(&self) -> Value {
        // Serialize self, then drop the two non-signed fields. Reusing the
        // serde form keeps field names in exact sync with the struct.
        #[allow(clippy::expect_used)]
        let mut v = serde_json::to_value(self).expect("CapabilityToken is serializable");
        if let Some(obj) = v.as_object_mut() {
            obj.remove("signature");
            obj.remove("proofs");
        }
        v // canonicalization happens inside sign_json / verify_json_signature
    }

    /// Issue a new signed `CapabilityToken`.
    pub fn issue(
        issuer: &Identity,
        audience_did: &str,
        capabilities: Vec<Capability>,
        facts: Map<String, Value>,
        expires_in_secs: u64,
        proofs: Vec<CapabilityToken>,
    ) -> Result<Self> {
        let issuer_did = substrate::derive_did_key(&issuer.public_key());
        let now = now_secs()?;
        let mut token = Self {
            issuer_did,
            audience_did: audience_did.to_string(),
            capabilities,
            facts,
            not_before_secs: now,
            expires_at_secs: now + expires_in_secs,
            proofs,
            signature: String::new(),
        };
        token.signature = issuer.sign_json(&token.signing_value())?;
        Ok(token)
    }

    /// Per-token structural verification (signature + time bounds); does not
    /// walk the proof chain.
    fn verify_self(&self, now_secs: u64) -> Result<()> {
        if self.not_before_secs >= self.expires_at_secs {
            return Err(anyhow!("token has non-positive validity window"));
        }
        if self.not_before_secs > now_secs + CLOCK_SKEW_SECS {
            return Err(anyhow!("token not_before is in the future"));
        }
        if now_secs >= self.expires_at_secs {
            return Err(anyhow!("token expired"));
        }
        substrate::verify_json_signature(&self.issuer_did, &self.signing_value(), &self.signature)
            .context("token signature verification failed")
    }

    /// Pre-order walk of this token and every proof in its chain, yielding
    /// each `(issuer_did, audience_did)` edge. Used by the router to check
    /// each edge's audience against the issuer's revocation anchor.
    #[must_use]
    pub fn chain_edges(&self) -> Vec<(&str, &str)> {
        let mut edges = vec![(self.issuer_did.as_str(), self.audience_did.as_str())];
        for proof in &self.proofs {
            edges.extend(proof.chain_edges());
        }
        edges
    }
}

/// Options for verifying a presented token chain.
pub struct ChainVerifyOpts<'a> {
    /// The DID this token must be addressed to (the verified connection
    /// identity). The leaf's `audience_did` must equal this — binds the
    /// token to the presenter, preventing replay of a token issued to
    /// someone else.
    pub expected_audience_did: &'a str,
    /// Returns whether `issuer_did` is a trusted root of authority for
    /// `resource`. At B1 the router passes `|iss, _res| iss == admin_root`.
    /// `Send + Sync` so `ChainVerifyOpts` (and futures holding it across an
    /// `.await`) stay usable from `tokio::spawn`ed connection handlers.
    pub is_trusted_root: &'a (dyn Fn(&str, &ResourceUri) -> bool + Send + Sync),
    pub now_secs: u64,
}

impl fmt::Debug for ChainVerifyOpts<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChainVerifyOpts")
            .field("expected_audience_did", &self.expected_audience_did)
            .field("is_trusted_root", &"<fn>")
            .field("now_secs", &self.now_secs)
            .finish()
    }
}

/// Verify the chain and return the set of capabilities the leaf provably
/// holds — i.e. each is either rooted directly at a trusted issuer or
/// attenuated from a valid, trusted proof. Capabilities that cannot be
/// traced to a trusted root are **dropped** (fail-closed at the capability
/// granularity); an entirely unbacked leaf yields an empty set, not an
/// error. Returns `Err` only on a structural failure (bad signature,
/// expiry, audience mismatch).
pub fn verify_chain(leaf: &CapabilityToken, opts: &ChainVerifyOpts<'_>) -> Result<Vec<Capability>> {
    if leaf.audience_did != opts.expected_audience_did {
        return Err(anyhow!(
            "token audience {} does not match presenter {}",
            leaf.audience_did,
            opts.expected_audience_did
        ));
    }
    granted_capabilities(leaf, opts)
}

fn granted_capabilities(
    token: &CapabilityToken,
    opts: &ChainVerifyOpts<'_>,
) -> Result<Vec<Capability>> {
    token.verify_self(opts.now_secs)?; // fail-closed: bad link aborts
    let mut effective = Vec::new();
    // Verify proofs once; a proof that fails structurally makes the whole
    // presentation invalid (do not silently ignore a tampered proof).
    let parent_grants: Vec<Vec<Capability>> = token
        .proofs
        .iter()
        .map(|p| {
            if p.audience_did != token.issuer_did {
                // continuity break: this proof does not delegate to this issuer
                return Ok(Vec::new());
            }
            granted_capabilities(p, opts)
        })
        .collect::<Result<_>>()?;
    for cap in &token.capabilities {
        let rooted = (opts.is_trusted_root)(&token.issuer_did, &cap.with);
        let backed = parent_grants.iter().flatten().any(|pc| pc.covers(cap));
        if rooted || backed {
            effective.push(cap.clone());
        }
        // else: dropped (fail-closed) — issuer is neither a trusted root nor
        // holds a proof entailing this capability.
    }
    Ok(effective)
}

#[cfg(test)]
mod tests {
    use syneroym_identity::substrate::derive_did_key;

    use super::*;
    use crate::capability::Ability;

    fn cap(resource: ResourceUri, ability: &str) -> Capability {
        Capability { with: resource, can: Ability(ability.to_string()), caveats: None }
    }

    fn no_root(_iss: &str, _res: &ResourceUri) -> bool {
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

        let is_root = |iss: &str, _res: &ResourceUri| iss == admin_root;
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

        let is_root = |iss: &str, _res: &ResourceUri| iss == admin_root;
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

        let is_root = |iss: &str, _res: &ResourceUri| iss == admin_root;
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

        let is_root = |iss: &str, _res: &ResourceUri| iss == admin_root;
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

        let is_root = |iss: &str, _res: &ResourceUri| iss == admin_root;
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

        let is_root = |iss: &str, _res: &ResourceUri| iss == admin_root;
        let opts = ChainVerifyOpts {
            expected_audience_did: &bob_did,
            is_trusted_root: &is_root,
            now_secs: now_secs().unwrap(),
        };
        let session = SessionContext::from_verified_chain(&token, &opts).unwrap();
        let arbitrary_resource = ResourceUri::service("app-anything", "svc-anything");
        assert!(
            session.has_capability(
                &arbitrary_resource,
                &Ability(Ability::DATA_LAYER_ADMIN.to_string())
            )
        );
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

        let is_root = |iss: &str, _res: &ResourceUri| iss == admin_root;
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
}

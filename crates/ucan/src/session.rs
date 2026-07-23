//! The verified, in-memory result of capability resolution (ADR-0015 §3).

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    capability::{Ability, Capability, ResourceUri},
    token::{CapabilityToken, ChainVerifyOpts, verify_chain},
};

/// The *verified, in-memory* result of resolving a caller's capabilities —
/// never deserialized-and-trusted from the wire. At B0 `capabilities` is
/// populated by the interim admin-root path, not a real UCAN chain; B1
/// replaces that with `SessionContext::from_verified_chain`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContext {
    pub subject_did: String,
    /// The original principal the chain acts for, when it differs from
    /// `subject_did` (ADR-0015 A5, amended). `None` for a direct call --
    /// callers that need "who is this really for" should read
    /// `anchor_did.unwrap_or(subject_did)`, not this field alone (a direct
    /// caller *is* its own anchor, not an absent one).
    pub anchor_did: Option<String>,
    pub capabilities: Vec<Capability>,
    pub claims: serde_json::Map<String, serde_json::Value>,
    pub verified_at_secs: u64,
}

impl SessionContext {
    #[must_use]
    pub fn has_capability(&self, resource: &ResourceUri, ability: &Ability) -> bool {
        self.capabilities.iter().any(|c| c.grants(resource, ability))
    }

    /// Verify a presented UCAN chain and normalize it into a `SessionContext`
    /// (ADR-0015 §3, Slice B1). The leaf's `facts` become `claims` -- but
    /// only when the leaf's own issuer is *itself* a trusted root.
    ///
    /// Unlike `capabilities`, `facts` are not attenuated through the proof
    /// chain -- they are simply whatever the presented leaf declares. Any
    /// caller can act as the issuer of the leaf it presents (the leaf's
    /// issuer need not be a trusted party at all; only its *proofs* need to
    /// chain back to one for a capability to attenuate). So a caller holding
    /// a legitimately root-issued proof can still wrap it in a self-authored
    /// leaf carrying arbitrary fabricated `facts` -- the capability
    /// attenuation logic would correctly admit the *capability* from the
    /// proof, but would just as happily carry the fabricated `facts` along
    /// for free if they were copied unconditionally. Since `facts` are the
    /// co-design seam M04B binds as SQL `?` parameters, that would be a
    /// claims-injection path with no attenuation check at all. Guard against
    /// it by only trusting `facts` when the leaf was signed directly by a
    /// trusted root -- i.e. the root asserted them itself, not a delegate.
    pub fn from_verified_chain(leaf: &CapabilityToken, opts: &ChainVerifyOpts<'_>) -> Result<Self> {
        let capabilities = verify_chain(leaf, opts)?;
        // M04A Slice B7b (ADR-0015 A6): resolved the former TODO(B7). Once
        // `is_trusted_root` became resource-scoped (owner-rooted trust per
        // service, not just the node-wide admin root), the old synthetic
        // `substrate(leaf.issuer_did)` probe stopped making sense -- it
        // asked "is this issuer a root for a resource named after itself?",
        // which is not a question about any resource the leaf actually
        // targets. Trust the leaf's `facts` only if its issuer is a trusted
        // root for *every* resource its own capabilities name -- a root for
        // *something* is not a root for *anything*, and a leaf naming zero
        // capabilities has no resource to attest the issuer against, so it
        // gets no facts either (fail-closed).
        let leaf_issuer_is_trusted_root = !leaf.capabilities.is_empty()
            && leaf.capabilities.iter().all(|c| (opts.is_trusted_root)(&leaf.issuer_did, c));
        let claims =
            if leaf_issuer_is_trusted_root { leaf.facts.clone() } else { serde_json::Map::new() };
        Ok(Self {
            subject_did: leaf.audience_did.clone(),
            anchor_did: leaf.anchor_did.clone(),
            capabilities,
            claims,
            verified_at_secs: opts.now_secs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_capability_hits_on_granting_capability() {
        let resource = ResourceUri::service("app-1", "svc-a");
        let session = SessionContext {
            subject_did: "did:key:z6MkCaller".to_string(),
            capabilities: vec![Capability {
                with: resource.clone(),
                can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
                caveats: None,
            }],
            ..Default::default()
        };
        assert!(session.has_capability(&resource, &Ability(Ability::DATA_LAYER_WRITE.to_string())));
    }

    #[test]
    fn has_capability_misses_without_a_granting_capability() {
        let resource = ResourceUri::service("app-1", "svc-a");
        let session =
            SessionContext { subject_did: "did:key:z6MkCaller".to_string(), ..Default::default() };
        assert!(!session.has_capability(&resource, &Ability(Ability::DATA_LAYER_READ.to_string())));
    }

    // -- M04A Slice B7b (ADR-0015 A6): from_verified_chain's trusted-root
    // -- facts gate, formerly the session.rs TODO(B7).

    use crate::token::CapabilityToken;

    fn cap(resource: ResourceUri, ability: &str) -> Capability {
        Capability { with: resource, can: Ability(ability.to_string()), caveats: None }
    }

    fn now() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
    }

    /// Fail-closed per the doc comment: a leaf naming *zero* capabilities has
    /// no resource to attest the issuer against, so it must get no facts
    /// even when issued directly by a root the predicate would otherwise
    /// trust unconditionally (e.g. a node-wide admin-root predicate that
    /// ignores its resource argument). This is the exact bug the old
    /// synthetic-resource probe could have masked: it would have asked the
    /// predicate about a resource named after the issuer itself, and a
    /// resource-agnostic predicate would answer "yes" regardless of whether
    /// the leaf named any real resource at all.
    #[test]
    fn empty_capabilities_leaf_never_gets_trusted_facts() {
        use syneroym_identity::{Identity, substrate::derive_did_key};

        use crate::token::ChainVerifyOpts;

        let root = Identity::generate().unwrap();
        let root_did = derive_did_key(&root.public_key());
        let audience = "did:key:zAudience";

        let mut facts = serde_json::Map::new();
        facts.insert("role".to_string(), serde_json::Value::String("admin".to_string()));

        let leaf = CapabilityToken::issue(&root, audience, vec![], facts, 3600, vec![]).unwrap();

        let is_root = |iss: &str, _cap: &Capability| iss == root_did;
        let opts = ChainVerifyOpts {
            expected_audience_did: audience,
            is_trusted_root: &is_root,
            now_secs: now(),
        };

        let session = SessionContext::from_verified_chain(&leaf, &opts).unwrap();
        assert!(
            session.claims.is_empty(),
            "a leaf with zero capabilities must never carry trusted facts, even from a root"
        );
    }

    /// Facts are trusted only when the issuer is a root for *every*
    /// capability's resource, not just some of them -- a leaf mixing one
    /// resource the issuer legitimately roots with one it does not must
    /// yield no facts, even though the first capability still attenuates
    /// (it is separately, individually rooted).
    #[test]
    fn mixing_a_rooted_and_an_unrooted_capability_yields_no_facts() {
        use syneroym_identity::{Identity, substrate::derive_did_key};

        use crate::token::ChainVerifyOpts;

        let owner = Identity::generate().unwrap();
        let owner_did = derive_did_key(&owner.public_key());
        let audience = "did:key:zAudience";
        let resource_a = ResourceUri::service("app-a", "svc-a");
        let resource_b = ResourceUri::service("app-b", "svc-b");

        let mut facts = serde_json::Map::new();
        facts.insert("tenant".to_string(), serde_json::Value::String("acme".to_string()));

        let leaf = CapabilityToken::issue(
            &owner,
            audience,
            vec![
                cap(resource_a.clone(), Ability::DATA_LAYER_READ),
                cap(resource_b, Ability::DATA_LAYER_READ),
            ],
            facts,
            3600,
            vec![],
        )
        .unwrap();

        // The issuer is a trusted root for resource_a only.
        let is_root = move |iss: &str, cap: &Capability| iss == owner_did && cap.with == resource_a;
        let opts = ChainVerifyOpts {
            expected_audience_did: audience,
            is_trusted_root: &is_root,
            now_secs: now(),
        };

        let session = SessionContext::from_verified_chain(&leaf, &opts).unwrap();
        assert_eq!(session.capabilities.len(), 1, "only the rooted capability attenuates");
        assert!(
            session.claims.is_empty(),
            "mixing one rooted and one unrooted capability must yield no facts at all"
        );
    }
}

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
        // The resource passed here is a synthetic probe: B1's only
        // `is_trusted_root` predicate (`|iss, _res| iss == admin_root`)
        // ignores it, since node-admin trust isn't resource-scoped. It
        // exists so this check can reuse `ChainVerifyOpts`'s existing
        // signature rather than adding a second, resource-free variant.
        let leaf_issuer_is_trusted_root =
            (opts.is_trusted_root)(&leaf.issuer_did, &ResourceUri::substrate(&leaf.issuer_did));
        let claims =
            if leaf_issuer_is_trusted_root { leaf.facts.clone() } else { serde_json::Map::new() };
        Ok(Self {
            subject_did: leaf.audience_did.clone(),
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
}

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
    /// (ADR-0015 §3, Slice B1). The leaf's `facts` become `claims`.
    pub fn from_verified_chain(leaf: &CapabilityToken, opts: &ChainVerifyOpts<'_>) -> Result<Self> {
        let capabilities = verify_chain(leaf, opts)?;
        Ok(Self {
            subject_did: leaf.audience_did.clone(),
            capabilities,
            claims: leaf.facts.clone(),
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

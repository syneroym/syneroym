//! The identity of the substrate that runs an infrastructure service (a
//! relay, a mailbox), and the passport root it accepts (ADR-0015 A10).

use syneroym_identity::substrate::{SubstrateIdentityState, SubstrateIdentityStatus};
#[cfg(any(test, feature = "test-utils"))]
use syneroym_identity::{Identity, substrate};

use crate::coordinator_info::IdentityInfo;

/// The host substrate of a service, as read at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostIdentity {
    /// The node DID of the host substrate.
    pub node_did: String,
    /// The state of its controller agreement.
    pub controller_status: SubstrateIdentityStatus,
    /// The controller of a verified agreement. `None` when there is no
    /// agreement or it is not verified.
    pub verified_controller: Option<String>,
}

impl HostIdentity {
    /// From the boot identity state. A controller counts only when its
    /// agreement is verified.
    #[must_use]
    pub fn new(state: &SubstrateIdentityState) -> Self {
        let verified_controller = (state.status == SubstrateIdentityStatus::Verified)
            .then(|| state.controller.clone())
            .flatten();
        Self {
            node_did: state.did.clone(),
            controller_status: state.status.clone(),
            verified_controller,
        }
    }

    /// A host with no controller.
    #[must_use]
    pub fn unowned(node_did: String) -> Self {
        Self {
            node_did,
            controller_status: SubstrateIdentityStatus::None,
            verified_controller: None,
        }
    }

    /// The one DID a passport for a service of this host may be issued by
    /// (ADR-0015 A10 point 4): the verified controller, or the node DID
    /// when there is none. Never the node-wide `[iam].admin_ucan_root`.
    #[must_use]
    pub fn passport_root(&self) -> &str {
        self.verified_controller.as_deref().unwrap_or(&self.node_did)
    }

    /// What `/v1/info` reports as `identity`.
    #[must_use]
    pub fn identity_info(&self) -> IdentityInfo {
        let status = match self.controller_status {
            SubstrateIdentityStatus::Verified => "verified",
            SubstrateIdentityStatus::Unverified => "unverified",
            SubstrateIdentityStatus::None => "none",
        };
        IdentityInfo { did: self.node_did.clone(), controller_status: status.to_string() }
    }
}

/// A host with a fresh random node key and no controller, for tests.
#[cfg(any(test, feature = "test-utils"))]
impl HostIdentity {
    #[expect(clippy::expect_used, reason = "test helper generates an in-memory key")]
    #[must_use]
    pub fn random_unowned() -> Self {
        let id = Identity::generate().expect("random identity");
        Self::unowned(substrate::derive_did_key(&id.public_key()))
    }
}

#[cfg(test)]
mod tests;

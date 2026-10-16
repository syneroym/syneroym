//! Checks on a child token that the holder can run before registration.

use syneroym_ucan::CapabilityToken;

use crate::{
    chain::{ChainRules, PassportChain},
    error::RegisterError,
    issue::{PassportId, ensure_canonical_did},
    passport_use::PassportUse,
};

/// What the holder of a child token can check before it registers. It does
/// not check that the passport root is accepted: only the service knows its
/// root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedToken {
    pub passport_id: PassportId,
    /// From the passport resource. The challenge signature names this DID,
    /// never a DID the service reports about itself.
    pub host_node_did: String,
    pub owner_did: String,
    pub subject: String,
    /// `min(child expiry, passport expiry)`.
    pub access_ends_at: u64,
}

/// The shape, link, capability, delegation, expiry and signature checks of
/// `Registrar::check`, with the passport issuer taken as the root, for the
/// child token `token` held by `subject`.
///
/// # Errors
/// `Malformed` for a bad shape or a resource that is not this use's
/// selector form, `NotBound` when the audience is not `subject`, and the
/// errors of the chain check.
pub fn inspect_child_token(
    token: &CapabilityToken,
    use_: &PassportUse,
    subject: &str,
    now_secs: u64,
) -> Result<PresentedToken, RegisterError> {
    let chain = PassportChain::parse(token)?;
    if chain.subject() != subject {
        return Err(RegisterError::NotBound);
    }
    let host = use_.host_of(&chain.passport.capabilities[0].with).ok_or_else(|| {
        RegisterError::Malformed("the passport resource is not substrate:<did>/<selector>".into())
    })?;
    ensure_canonical_did(host)
        .map_err(|e| RegisterError::Malformed(format!("the host node DID: {e:#}")))?;
    let root = chain.passport.issuer_did.as_str();
    let is_root = |i: &str| i == root;
    ChainRules { use_, host_node_did: host, is_root: &is_root }.check(&chain, now_secs)?;
    Ok(PresentedToken {
        passport_id: PassportId::of(chain.passport)
            .map_err(|e| RegisterError::Malformed(e.to_string()))?,
        host_node_did: host.to_owned(),
        owner_did: chain.passport.audience_did.clone(),
        subject: subject.to_owned(),
        access_ends_at: chain.leaf.expires_at_secs.min(chain.passport.expires_at_secs),
    })
}

#[cfg(test)]
mod tests;

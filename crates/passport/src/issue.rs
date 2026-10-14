//! Issuing a passport (operator) and its child token (owner).

use std::{fmt, str::FromStr};

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use syneroym_identity::{Identity, substrate};
use syneroym_ucan::CapabilityToken;

use crate::passport_use::PassportUse;

/// The id of a passport: hex of `CapabilityToken::token_id`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PassportId(String);

impl PassportId {
    /// The id of `passport`.
    pub fn of(passport: &CapabilityToken) -> Result<Self> {
        Ok(Self(hex::encode(passport.token_id()?)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PassportId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let valid =
            s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        ensure!(valid, "a passport id is 64 lowercase hex characters");
        Ok(Self(s.to_string()))
    }
}

impl TryFrom<String> for PassportId {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<PassportId> for String {
    fn from(id: PassportId) -> Self {
        id.0
    }
}

impl fmt::Display for PassportId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A DID is canonical when it is exactly what `derive_did_key` makes for its
/// key. Another spelling of the same key is refused, because leases and
/// lookups compare DIDs as strings.
pub(crate) fn ensure_canonical_did(did: &str) -> Result<()> {
    let key = substrate::resolve_did_key(did)?;
    ensure!(substrate::derive_did_key(&key) == did, "{did} is not in canonical form");
    Ok(())
}

/// Issue a passport: the operator root grants `use_` on `host_node_did` to
/// `audience_did` (the user's owner DID), with signed use limits.
pub fn issue_passport(
    operator: &Identity,
    host_node_did: &str,
    use_: &PassportUse,
    audience_did: &str,
    limits: Map<String, Value>,
    expires_in_secs: u64,
    now_secs: u64,
) -> Result<CapabilityToken> {
    for (key, value) in &limits {
        ensure!(
            use_.limit_keys.contains(&key.as_str()),
            "{key} is not a use limit of {}; allowed: {}",
            use_.ability,
            use_.limit_keys.join(", ")
        );
        ensure!(
            value.as_u64().is_some_and(|n| n >= 1),
            "{key} must be a whole number of at least 1"
        );
    }
    ensure!(expires_in_secs > 0, "a passport must live at least one second");
    ensure_canonical_did(audience_did).context("the passport audience")?;
    ensure_canonical_did(host_node_did).context("the host node DID")?;
    let expires_at_secs = now_secs
        .checked_add(expires_in_secs)
        .ok_or_else(|| anyhow!("the passport expiry is out of range"))?;
    let mut token = CapabilityToken {
        issuer_did: substrate::derive_did_key(&operator.public_key()),
        audience_did: audience_did.to_string(),
        anchor_did: None,
        capabilities: vec![use_.capability(host_node_did)],
        facts: limits,
        not_before_secs: now_secs,
        expires_at_secs,
        proofs: vec![],
        signature: String::new(),
    };
    token.sign(operator)?;
    Ok(token)
}

/// The owner re-delegates `passport` to `subject_did` (the node DID for the
/// relay, the member master DID for the mailbox). The child carries the same
/// capability, no facts (the service reads limits from the passport only),
/// and never outlives the passport.
pub fn child_token(
    owner: &Identity,
    passport: &CapabilityToken,
    subject_did: &str,
    expires_in_secs: u64,
    now_secs: u64,
) -> Result<CapabilityToken> {
    let owner_did = substrate::derive_did_key(&owner.public_key());
    ensure!(
        passport.audience_did == owner_did,
        "this passport was issued to {}, not to the owner identity {owner_did}",
        passport.audience_did
    );
    ensure!(
        passport.proofs.is_empty() && passport.capabilities.len() == 1,
        "this token is not a passport"
    );
    ensure!(now_secs < passport.expires_at_secs, "the passport has expired");
    ensure_canonical_did(subject_did).context("the subject")?;
    let mut token = CapabilityToken {
        issuer_did: owner_did,
        audience_did: subject_did.to_string(),
        anchor_did: None,
        capabilities: passport.capabilities.clone(),
        facts: Map::new(),
        not_before_secs: now_secs,
        expires_at_secs: now_secs.saturating_add(expires_in_secs).min(passport.expires_at_secs),
        proofs: vec![passport.clone()],
        signature: String::new(),
    };
    token.sign(owner)?;
    Ok(token)
}

#[cfg(test)]
mod tests;

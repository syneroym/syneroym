//! The id of a token: the SHA-256 of the canonical body its signature covers
//! (ADR-0015 A10). Deny lists and use counts key on it.

use anyhow::Result;
use sha2::{Digest, Sha256};
use syneroym_identity::substrate;

use super::CapabilityToken;

impl CapabilityToken {
    /// SHA-256 of the RFC 8785 canonical JSON of the signed body: every
    /// field except `signature` and `proofs`. Two tokens with the same id
    /// carry the same signed claims. The id does not change when the proofs
    /// change.
    pub fn token_id(&self) -> Result<[u8; 32]> {
        let canonical = substrate::canonicalize_json_value(&self.signing_value());
        let bytes = serde_json::to_vec(&canonical)?;
        Ok(Sha256::digest(&bytes).into())
    }
}

#[cfg(test)]
mod tests;

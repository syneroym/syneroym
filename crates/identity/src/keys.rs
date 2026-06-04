//! Cryptographic identity and keypair management
//!
//! Defines the primary `Identity` struct utilizing Ed25519 dalek for key generation,
//! secure storage, signing, and DID document generation.

use std::{
    fmt::{Debug, Formatter},
    fs,
    path::Path,
};

use anyhow::Context;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::{IdentityDoc, substrate};

/// Represents the cryptographic identity of a Syneroym node.
pub struct Identity {
    signing_key: SigningKey,
}

impl Debug for Identity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity").field("public_key", &self.public_key()).finish()
    }
}

impl Identity {
    /// Generate a new random Ed25519 identity keypair.
    ///
    /// # Errors
    /// Returns an error if the system's random number generator fails (e.g., in sandboxed environments).
    pub fn generate() -> anyhow::Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)
            .context("Failed to generate random bytes for Ed25519 keypair")?;
        let signing_key = SigningKey::from_bytes(&bytes);
        Ok(Self { signing_key })
    }

    /// Load an identity from a 32-byte secret key slice.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(bytes);
        Self { signing_key }
    }

    /// Load an identity from a file path.
    /// Expects a 32-byte secret key file.
    pub fn load_from_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path)
            .with_context(|| format!("Failed to read identity file at {}", path.display()))?;
        let len = bytes.len();
        let bytes_array: [u8; 32] = bytes.try_into().map_err(|_| {
            anyhow::anyhow!("Invalid key file size ({}) at {}", len, path.display())
        })?;
        Ok(Self::from_bytes(&bytes_array))
    }

    /// Save the identity to a file path.
    /// Writes the 32-byte secret key.
    pub fn save_to_path(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create parent directories for {}", path.display())
            })?;
        }
        let bytes = self.to_bytes();
        fs::write(path, bytes)
            .with_context(|| format!("Failed to write identity file to {}", path.display()))?;
        Ok(())
    }

    /// Export the secret key as a 32-byte array.
    /// WARNING: This must be kept highly secure.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Get the public verifying key associated with this identity.
    #[must_use]
    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Sign a message payload using this identity.
    #[must_use]
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    /// Sign a JSON value using RFC 8785 (JSON Canonicalization Scheme).
    /// Returns a z-base-32 encoded signature.
    pub fn sign_json(&self, value: &serde_json::Value) -> anyhow::Result<String> {
        let canonical_value = substrate::canonicalize_json_value(value);
        let canonical_string = serde_json::to_string(&canonical_value)?;
        let signature = self.sign(canonical_string.as_bytes());
        Ok(z32::encode(&signature.to_bytes()))
    }

    /// Generate a public `IdentityDoc` for this node.
    #[must_use]
    pub fn to_doc(&self, created_at: u64) -> IdentityDoc {
        let pubkey_bytes = self.public_key().to_bytes();
        let pubkey_hex = hex::encode(pubkey_bytes);
        let id = format!("did:syn:{pubkey_hex}");

        IdentityDoc { id, pubkey_hex, created_at }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_sign_json_deterministic() {
        let identity = Identity::generate().unwrap();
        let v1 = json!({"a": 1, "b": 2});
        let v2 = json!({"b": 2, "a": 1}); // Different key order

        let s1 = identity.sign_json(&v1).unwrap();
        let s2 = identity.sign_json(&v2).unwrap();

        assert_eq!(s1, s2, "Signatures should be identical due to canonicalization");
    }

    #[test]
    fn test_sign_json_nested() {
        let identity = Identity::generate().unwrap();
        let v1 = json!({"x": {"b": 2, "a": 1}, "y": [3, 2, 1]});
        let s1 = identity.sign_json(&v1).unwrap();
        assert!(!s1.is_empty());
    }
}

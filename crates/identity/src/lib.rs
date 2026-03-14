use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Represents the cryptographic identity of a Syneroym node.
pub struct Identity {
    signing_key: SigningKey,
}

impl Identity {
    /// Generate a new random Ed25519 identity keypair.
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("Failed to generate random bytes");
        let signing_key = SigningKey::from_bytes(&bytes);
        Self { signing_key }
    }

    /// Load an identity from a 32-byte secret key slice.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(bytes);
        Self { signing_key }
    }

    /// Export the secret key as a 32-byte array.
    /// WARNING: This must be kept highly secure.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Get the public verifying key associated with this identity.
    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    /// Sign a message payload using this identity.
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }
}

/// A serialized representation of the node's public identity document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityDoc {
    pub id: String,
    pub pubkey_hex: String,
    pub created_at: u64,
}

impl Identity {
    /// Generate a public `IdentityDoc` for this node.
    pub fn to_doc(&self, created_at: u64) -> IdentityDoc {
        let pubkey_bytes = self.public_key().to_bytes();
        let pubkey_hex = hex::encode(pubkey_bytes);
        let id = format!("did:syn:{}", pubkey_hex);

        IdentityDoc { id, pubkey_hex, created_at }
    }
}

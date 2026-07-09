//! Delegation Certificates for temporary keys signed by a Master Identity

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{Identity, substrate};

/// A cryptographic certificate that binds a temporary identity key to a master
/// DID for a specific duration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationCertificate {
    pub master_did: String,
    pub temporary_did: String,
    pub issued_at_secs: u64,
    pub expires_at_secs: u64,
    pub scope: String,     // e.g., "routing"
    pub signature: String, // z-base-32 Ed25519 signature over canonical JSON of the 5 fields above
}

impl DelegationCertificate {
    fn canonical_payload_bytes(
        master_did: &str,
        temporary_did: &str,
        issued_at_secs: u64,
        expires_at_secs: u64,
        scope: &str,
    ) -> Result<Vec<u8>> {
        let payload = serde_json::json!({
            "master_did": master_did,
            "temporary_did": temporary_did,
            "issued_at_secs": issued_at_secs,
            "expires_at_secs": expires_at_secs,
            "scope": scope,
        });
        let canonical_payload = substrate::canonicalize_json_value(&payload);
        serde_json::to_vec(&canonical_payload).context("Failed to serialize canonical payload")
    }

    /// Issue a new DelegationCertificate.
    /// Signs canonical JSON of the 5 fields using the master's private identity
    /// key.
    pub fn issue(
        master: &Identity,
        temp_pubkey: VerifyingKey,
        expires_in_secs: u64,
        scope: String,
    ) -> Result<Self> {
        let master_pubkey = master.public_key();
        let master_did = substrate::derive_did_key(&master_pubkey);
        let temporary_did = substrate::derive_did_key(&temp_pubkey);

        let issued_at_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time is before UNIX epoch")?
            .as_secs();
        let expires_at_secs = issued_at_secs + expires_in_secs;

        let payload_bytes = Self::canonical_payload_bytes(
            &master_did,
            &temporary_did,
            issued_at_secs,
            expires_at_secs,
            &scope,
        )?;

        let signature = z32::encode(&master.sign(&payload_bytes).to_bytes());

        Ok(Self { master_did, temporary_did, issued_at_secs, expires_at_secs, scope, signature })
    }

    /// Serializes the delegation certificate to JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).context("Failed to serialize DelegationCertificate to JSON")
    }

    /// Deserializes the delegation certificate from JSON.
    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).context("Failed to deserialize DelegationCertificate from JSON")
    }

    /// Verifies the signature of the DelegationCertificate and checks if it's
    /// expired.
    pub fn verify(&self, expected_master_did: &str) -> Result<()> {
        if self.master_did != expected_master_did {
            return Err(anyhow!(
                "Confused deputy prevention: expected master DID {}, but certificate is for {}",
                expected_master_did,
                self.master_did
            ));
        }

        if self.issued_at_secs >= self.expires_at_secs {
            return Err(anyhow!("Delegation certificate has non-positive validity window"));
        }

        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("System time is before UNIX epoch")?
            .as_secs();

        // Reject certs issued more than 300 seconds in the future (clock skew
        // tolerance)
        if self.issued_at_secs > now_secs + 300 {
            return Err(anyhow!("Delegation certificate issued_at is in the future"));
        }

        if now_secs >= self.expires_at_secs {
            return Err(anyhow!(
                "Delegation certificate has expired (expired at {}, now {})",
                self.expires_at_secs,
                now_secs
            ));
        }

        // 1. Resolve master public key
        let master_pubkey = substrate::resolve_did_key(&self.master_did)
            .context("Failed to resolve master DID in delegation certificate")?;

        // 2. Re-create canonical payload of the 5 fields
        let payload_bytes = Self::canonical_payload_bytes(
            &self.master_did,
            &self.temporary_did,
            self.issued_at_secs,
            self.expires_at_secs,
            &self.scope,
        )?;

        // 3. Decode signature
        let sig_bytes = z32::decode(self.signature.as_bytes())
            .map_err(|_| anyhow!("Invalid signature format in delegation certificate"))?;
        let signature =
            Signature::from_slice(&sig_bytes).context("Invalid Ed25519 signature bytes")?;

        // 4. Verify signature
        master_pubkey
            .verify(&payload_bytes, &signature)
            .context("Delegation certificate signature verification failed")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delegation_cert_valid() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();
        let temp_pubkey = temp.public_key();

        let cert = DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string())
            .unwrap();
        cert.verify(&cert.master_did).expect("Valid certificate verification failed");
    }

    #[test]
    fn test_delegation_cert_expired() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();
        let temp_pubkey = temp.public_key();

        let cert =
            DelegationCertificate::issue(&master, temp_pubkey, 0, "routing".to_string()).unwrap();
        // Since expires_in is 0, duration_since(UNIX_EPOCH) will be >= expires_at_secs
        // immediately or very soon.
        assert!(cert.verify(&cert.master_did).is_err());
    }

    #[test]
    fn test_delegation_cert_wrong_sig() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();
        let temp_pubkey = temp.public_key();

        let mut cert =
            DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string())
                .unwrap();
        // Tamper with the signature bytes slightly
        cert.signature = "a".repeat(cert.signature.len());
        assert!(cert.verify(&cert.master_did).is_err());
    }

    #[test]
    fn test_delegation_cert_wrong_master_did() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();
        let temp_pubkey = temp.public_key();

        let cert = DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string())
            .unwrap();
        let wrong_master_did = "did:key:z6Mku5U2Lg5r5UqVbZq8aA7t5N4h4C9b1d7d8e9f0g1h2i3j";
        assert!(cert.verify(wrong_master_did).is_err());
    }

    #[test]
    fn test_json_serialization() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();
        let temp_pubkey = temp.public_key();

        let cert = DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string())
            .unwrap();
        let json_str = cert.to_json().unwrap();
        let deserialized = DelegationCertificate::from_json(&json_str).unwrap();
        assert_eq!(cert, deserialized);
    }

    #[test]
    fn test_delegation_cert_invalid_validity_window() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();
        let mut cert =
            DelegationCertificate::issue(&master, temp.public_key(), 3600, "routing".to_string())
                .unwrap();

        // Non-positive validity window (issued_at >= expires_at)
        cert.issued_at_secs = cert.expires_at_secs;
        assert!(cert.verify(&cert.master_did).is_err());

        cert.issued_at_secs = cert.expires_at_secs + 10;
        assert!(cert.verify(&cert.master_did).is_err());
    }

    #[test]
    fn test_delegation_cert_issued_in_future() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();
        let mut cert =
            DelegationCertificate::issue(&master, temp.public_key(), 3600, "routing".to_string())
                .unwrap();

        // Issued in the future (skew more than 300s)
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        cert.issued_at_secs = now + 400;
        cert.expires_at_secs = now + 4000;
        assert!(cert.verify(&cert.master_did).is_err());
    }
}

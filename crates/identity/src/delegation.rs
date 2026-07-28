//! Delegation Certificates for temporary keys signed by a Master Identity

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::{Identity, substrate};

/// A key delegated to route connections under its master's identity -- an
/// operator's device key, a client session key.
pub const SCOPE_ROUTING: &str = "routing";
/// A substrate-derived instance key certified by a member master, so the
/// instance speaks as that member (ADR-0020 §1).
pub const SCOPE_SERVICE_INSTANCE: &str = "service-instance";
/// Scopes admissible as *transport* identity on an inbound connection: a
/// human operator's delegated device key and a service instance both route a
/// connection under a master's identity, and the router cannot tell from the
/// target which one legitimately belongs.
pub const TRANSPORT_SCOPES: [&str; 2] = [SCOPE_ROUTING, SCOPE_SERVICE_INSTANCE];

/// A cryptographic certificate that binds a temporary identity key to a master
/// DID for a specific duration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegationCertificate {
    pub master_did: String,
    pub temporary_did: String,
    pub issued_at_secs: u64,
    pub expires_at_secs: u64,
    /// What the delegated key is authorized to do as the master: `"routing"`
    /// for a routing/session key, `"service-instance"` for a substrate-derived
    /// instance key (see the `SCOPE_*` constants above). Signed as part of the
    /// certificate, and `verify` enforces it against a caller-supplied
    /// accepted set -- a certificate minted for one purpose is not replayable
    /// onto a connection that requires another.
    pub scope: String,
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

    /// Whether this certificate is already past `expires_at_secs`. A cheap,
    /// infallible pre-check for a caller deciding whether to *present* a
    /// certificate at all -- unlike `verify`, never rejects on signature,
    /// scope, or clock skew, so it must not be used as a substitute for
    /// `verify` at an actual trust boundary.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let now_secs =
            SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        now_secs >= self.expires_at_secs
    }

    /// Verifies the signature of the DelegationCertificate, that its `scope`
    /// is one the caller accepts, and that it's not expired.
    ///
    /// `expected_master_did` is a confused-deputy check against whatever the
    /// caller already believes the master to be. On the router's only
    /// production call site the caller reads that value from the certificate
    /// itself before calling `verify`, which makes the check a tautology
    /// there -- not a hole, since the connection's claim is "I am delegated
    /// by M" and M is whatever the certificate says, with binding to a
    /// *target* resolved downstream on `master_did`. Do not "fix" this by
    /// tightening the comparison; there is nothing independent to compare
    /// against on that path. `accepted_scopes` is the check that actually
    /// bites: an unlisted scope is rejected before the validity window is
    /// even examined, so a certificate minted for one purpose can't be
    /// replayed where a different one is required.
    pub fn verify(&self, expected_master_did: &str, accepted_scopes: &[&str]) -> Result<()> {
        if self.master_did != expected_master_did {
            return Err(anyhow!(
                "Confused deputy prevention: expected master DID {}, but certificate is for {}",
                expected_master_did,
                self.master_did
            ));
        }

        if !accepted_scopes.contains(&self.scope.as_str()) {
            return Err(anyhow!(
                "certificate scope '{}' is not accepted here (accepted: {:?})",
                self.scope,
                accepted_scopes
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
        cert.verify(&cert.master_did, &TRANSPORT_SCOPES)
            .expect("Valid certificate verification failed");
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
        assert!(cert.verify(&cert.master_did, &TRANSPORT_SCOPES).is_err());
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
        assert!(cert.verify(&cert.master_did, &TRANSPORT_SCOPES).is_err());
    }

    #[test]
    fn test_delegation_cert_wrong_master_did() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();
        let temp_pubkey = temp.public_key();

        let cert = DelegationCertificate::issue(&master, temp_pubkey, 3600, "routing".to_string())
            .unwrap();
        let wrong_master_did = "did:key:z6Mku5U2Lg5r5UqVbZq8aA7t5N4h4C9b1d7d8e9f0g1h2i3j";
        assert!(cert.verify(wrong_master_did, &TRANSPORT_SCOPES).is_err());
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
        assert!(cert.verify(&cert.master_did, &TRANSPORT_SCOPES).is_err());

        cert.issued_at_secs = cert.expires_at_secs + 10;
        assert!(cert.verify(&cert.master_did, &TRANSPORT_SCOPES).is_err());
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
        assert!(cert.verify(&cert.master_did, &TRANSPORT_SCOPES).is_err());
    }

    #[test]
    fn a_certificate_verifies_against_an_accepted_scope() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();

        let cert = DelegationCertificate::issue(
            &master,
            temp.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();

        assert!(cert.verify(&cert.master_did, &[SCOPE_SERVICE_INSTANCE]).is_ok());
    }

    #[test]
    fn a_certificate_is_rejected_when_its_scope_is_not_accepted() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();

        let cert = DelegationCertificate::issue(
            &master,
            temp.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();

        let err = cert
            .verify(&cert.master_did, &[SCOPE_SERVICE_INSTANCE])
            .expect_err("routing-scoped certificate must not verify against service-instance");
        let message = err.to_string();
        assert!(message.contains(SCOPE_ROUTING), "message should name the presented scope");
        assert!(message.contains(SCOPE_SERVICE_INSTANCE), "message should name the accepted scope");
    }

    #[test]
    fn an_unknown_scope_is_rejected_even_with_a_valid_signature() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();

        let cert = DelegationCertificate::issue(
            &master,
            temp.public_key(),
            3600,
            "vault-unseal".to_string(),
        )
        .unwrap();

        assert!(cert.verify(&cert.master_did, &TRANSPORT_SCOPES).is_err());
    }

    #[test]
    fn any_listed_scope_is_admitted() {
        let master = Identity::generate().unwrap();

        for scope in TRANSPORT_SCOPES {
            let temp = Identity::generate().unwrap();
            let cert =
                DelegationCertificate::issue(&master, temp.public_key(), 3600, scope.to_string())
                    .unwrap();
            assert!(
                cert.verify(&cert.master_did, &TRANSPORT_SCOPES).is_ok(),
                "scope {scope} should be admitted by TRANSPORT_SCOPES"
            );
        }
    }

    #[test]
    fn the_scope_cannot_be_edited_after_issue() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();

        let mut cert = DelegationCertificate::issue(
            &master,
            temp.public_key(),
            3600,
            SCOPE_ROUTING.to_string(),
        )
        .unwrap();

        // Rewriting the field to an accepted scope must not let a
        // routing-issued certificate pass as a service-instance one -- the
        // signature covers `scope`, so mutating it invalidates the signature
        // rather than changing what the certificate is admitted for.
        cert.scope = SCOPE_SERVICE_INSTANCE.to_string();
        let err = cert
            .verify(&cert.master_did, &[SCOPE_SERVICE_INSTANCE])
            .expect_err("mutated scope must fail signature verification, not pass as valid");
        assert!(
            err.to_string().contains("signature"),
            "failure should be a signature error, not a scope error: {err}"
        );
    }

    #[test]
    fn a_scope_mismatch_is_reported_before_expiry() {
        let master = Identity::generate().unwrap();
        let temp = Identity::generate().unwrap();

        // Expired and wrong-scope: the scope error must win, since a
        // categorical rejection should not be masked by a timing-derived one.
        let cert =
            DelegationCertificate::issue(&master, temp.public_key(), 0, SCOPE_ROUTING.to_string())
                .unwrap();

        let err = cert
            .verify(&cert.master_did, &[SCOPE_SERVICE_INSTANCE])
            .expect_err("expired + wrong-scope certificate must fail verification");
        assert!(
            err.to_string().contains("scope"),
            "failure should be the scope error, not an expiry error: {err}"
        );
    }
}

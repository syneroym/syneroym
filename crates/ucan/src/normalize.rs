//! External-auth normalization seam (ADR-0015 §5).

use anyhow::Result;
use syneroym_identity::substrate;

/// Normalizes an external authentication assertion into an internal
/// `did:key`. Forward-looking seam for OIDC/WebAuthn (M6+); only
/// `DidKeyNormalizer` ships for M4.
pub trait AuthNormalizer {
    /// # Errors
    /// Returns an error if `external` cannot be normalized into a resolvable
    /// `did:key`.
    fn normalize(&self, external: &str) -> Result<String>;
}

/// The only normalizer that ships in M4: validates that the input already is
/// a resolvable `did:key` and returns it unchanged (identity normalization).
/// The router's ingress already yields a `did:key` from the handshake, so
/// this is a no-op today (Flag F4) — it exists to satisfy the ADR-mandated
/// trait seam for M6's OIDC/WebAuthn normalizers.
#[derive(Debug)]
pub struct DidKeyNormalizer;

impl AuthNormalizer for DidKeyNormalizer {
    fn normalize(&self, external: &str) -> Result<String> {
        substrate::resolve_did_key(external)?;
        Ok(external.to_string())
    }
}

#[cfg(test)]
mod tests {
    use syneroym_identity::Identity;

    use super::*;

    #[test]
    fn accepts_a_resolvable_did_key() {
        let identity = Identity::generate().unwrap();
        let did = substrate::derive_did_key(&identity.public_key());

        let normalized = DidKeyNormalizer.normalize(&did).unwrap();
        assert_eq!(normalized, did);
    }

    #[test]
    fn rejects_a_non_did_key() {
        assert!(DidKeyNormalizer.normalize("did:web:example.com").is_err());
    }
}

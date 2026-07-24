//! `RelationshipProof`: the signed, TTL'd wire record answering "which rows
//! does `principal` reach via `relation`" (Slice B3 pipeline stage 2,
//! ADR-0017 §6). Shared by the receiving side (`syneroym-control-plane`'s
//! `resolve_relation`, which signs one via [`RelationshipProof::sign`]) and
//! the requesting side (`syneroym-rpc`'s `fdae_fetch::resolve_fetches`,
//! which verifies one via [`RelationshipProof::verify`]) so both sides agree
//! on one wire shape.

use serde::{Deserialize, Serialize};
use syneroym_identity::{
    Identity,
    substrate::{derive_did_key, verify_json_signature},
};

/// How long a `resolve-relation` answer is valid for (ADR-0017 §6's own
/// worked example: "valid 60s"). A fixed constant, not policy-configurable,
/// in this phase -- the fetch is used immediately by the same request that
/// triggered it; a cache honoring this TTL is a pure future addition
/// (D-B3-6), not something this phase relies on.
pub const RELATIONSHIP_PROOF_TTL_SECS: u64 = 60;

/// B3-09: returns an error rather than a bogus timestamp on failure. The
/// only way `duration_since(UNIX_EPOCH)` fails is a system clock set before
/// 1970 -- vanishingly unlikely, but this value feeds a cryptographically
/// **signed** artifact, unlike an ordinary internal timestamp field:
/// silently minting `valid_until_secs = 60` (Unix epoch + the TTL) would
/// leave the node's own signature attesting to a claim it never intended.
pub fn now_secs() -> anyhow::Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| anyhow::anyhow!("system clock is before the Unix epoch: {e}"))
}

/// A signed, TTL'd assertion answering "which rows does `principal` reach
/// via `relation`" (Slice B3, ADR-0017 §6): the wire response of
/// `resolve-relation`. Signing (not just transport authentication) is what
/// makes the id-set self-authenticating for the `DecisionTrace` provenance a
/// *successful* fetch records and for any future cache (D-B3-6, deferred) --
/// both need to know *which node* asserted this, independent of the
/// connection it arrived over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipProof {
    pub asserter_did: String,
    pub relation: String,
    pub principal: String,
    pub ids: Vec<String>,
    pub valid_until_secs: u64,
    /// z-base-32-encoded signature over the JSON-canonicalized form of this
    /// struct with `signature` itself set to `""` -- mirrors
    /// `Identity::sign_json`'s existing use elsewhere (e.g.
    /// `EndpointInfo::sign`), never re-deriving a bespoke signing scheme.
    pub signature: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RelationshipProofError {
    /// D-B3-8 (load-bearing): the proof's own `asserter_did` field is
    /// self-referential -- any signer can embed its own DID and sign under
    /// the matching key. This must be checked against a policy-declared
    /// `expected_asserter_did` the fetching side already trusted *before*
    /// issuing the fetch, never derived from the response.
    #[error("relationship proof asserter mismatch: expected '{expected}', got '{actual}'")]
    AsserterMismatch { expected: String, actual: String },
    #[error("relationship proof expired at {valid_until_secs} (now {now})")]
    Expired { now: u64, valid_until_secs: u64 },
    #[error("relationship proof signature is invalid: {0}")]
    BadSignature(String),
    #[error("internal error verifying relationship proof: {0}")]
    Internal(String),
}

impl RelationshipProof {
    /// Signs `ids` as a [`RelationshipProof`] asserted by `identity`. The
    /// signature covers every field except itself (set to `""` for
    /// signing, matching the canonicalize-then-sign convention
    /// `Identity::sign_json`'s other callers use).
    pub fn sign(
        identity: &Identity,
        relation: &str,
        principal: &str,
        ids: Vec<String>,
    ) -> anyhow::Result<Self> {
        let mut proof = Self {
            asserter_did: derive_did_key(&identity.public_key()),
            relation: relation.to_string(),
            principal: principal.to_string(),
            ids,
            valid_until_secs: now_secs()? + RELATIONSHIP_PROOF_TTL_SECS,
            signature: String::new(),
        };
        let unsigned = serde_json::to_value(&proof)?;
        proof.signature = identity.sign_json(&unsigned)?;
        Ok(proof)
    }

    /// Verifies this proof against a policy-declared `expected_asserter_did`
    /// (D-B3-8) -- **not** the proof's own `asserter_did` field alone, which
    /// is self-referential and would verify for any signer self-declaring
    /// its own DID. Also enforces the TTL: an expired proof is rejected even
    /// if the signature is otherwise valid.
    pub fn verify(&self, expected_asserter_did: &str) -> Result<(), RelationshipProofError> {
        if self.asserter_did != expected_asserter_did {
            return Err(RelationshipProofError::AsserterMismatch {
                expected: expected_asserter_did.to_string(),
                actual: self.asserter_did.clone(),
            });
        }
        let now = now_secs().map_err(|e| RelationshipProofError::Internal(e.to_string()))?;
        if now > self.valid_until_secs {
            return Err(RelationshipProofError::Expired {
                now,
                valid_until_secs: self.valid_until_secs,
            });
        }
        let mut unsigned = self.clone();
        unsigned.signature = String::new();
        let unsigned_value = serde_json::to_value(&unsigned)
            .map_err(|e| RelationshipProofError::Internal(e.to_string()))?;
        verify_json_signature(&self.asserter_did, &unsigned_value, &self.signature)
            .map_err(|e| RelationshipProofError::BadSignature(e.to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity::generate().unwrap()
    }

    #[test]
    fn sign_then_verify_round_trips() {
        let id = identity();
        let asserter_did = derive_did_key(&id.public_key());
        let proof =
            RelationshipProof::sign(&id, "employee", "did:key:alice", vec!["emp-1".to_string()])
                .unwrap();
        proof.verify(&asserter_did).unwrap();
    }

    #[test]
    fn verify_rejects_a_mismatched_expected_asserter() {
        let id = identity();
        let proof =
            RelationshipProof::sign(&id, "employee", "did:key:alice", vec!["emp-1".to_string()])
                .unwrap();
        let err = proof.verify("did:key:zSomeoneElse").unwrap_err();
        assert!(matches!(err, RelationshipProofError::AsserterMismatch { .. }));
    }

    #[test]
    fn verify_rejects_a_tampered_field() {
        let id = identity();
        let asserter_did = derive_did_key(&id.public_key());
        let mut proof =
            RelationshipProof::sign(&id, "employee", "did:key:alice", vec!["emp-1".to_string()])
                .unwrap();
        proof.ids.push("emp-2".to_string());
        let err = proof.verify(&asserter_did).unwrap_err();
        assert!(matches!(err, RelationshipProofError::BadSignature(_)));
    }

    #[test]
    fn verify_rejects_an_expired_proof() {
        let id = identity();
        let asserter_did = derive_did_key(&id.public_key());
        let mut proof =
            RelationshipProof::sign(&id, "employee", "did:key:alice", vec!["emp-1".to_string()])
                .unwrap();
        // Re-sign with a `valid_until_secs` already in the past so the TTL
        // check fails while the signature itself stays valid.
        proof.valid_until_secs = 1;
        let unsigned = {
            let mut cleared = proof.clone();
            cleared.signature = String::new();
            serde_json::to_value(&cleared).unwrap()
        };
        proof.signature = id.sign_json(&unsigned).unwrap();
        let err = proof.verify(&asserter_did).unwrap_err();
        assert!(matches!(err, RelationshipProofError::Expired { .. }));
    }
}

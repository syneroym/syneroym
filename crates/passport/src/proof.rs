//! Proof of possession (challenge form or transport form) and the signer
//! binding: may the proven key act for the subject?

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use syneroym_identity::{DelegationCertificate, Identity, delegation, substrate};
use syneroym_ucan::CapabilityToken;

use crate::{challenge_store::ChallengeStore, error::RegisterError, passport_use::PassportUse};

/// The domain tag of the challenge signature. A signature made for another
/// purpose never verifies as a challenge signature.
pub const CHALLENGE_DOMAIN: &str = "syneroym-passport-challenge-v1";

/// A fresh random value the caller must sign (challenge form).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Challenge([u8; 32]);

impl Challenge {
    /// 32 random bytes. Returns an error instead of a panic.
    pub(crate) fn random() -> Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| anyhow!("no random bytes for a challenge: {e}"))?;
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Parse the 64 hex characters of a challenge.
    pub fn from_hex(s: &str) -> Result<Self, RegisterError> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(s, &mut bytes).map_err(|e| {
            RegisterError::Malformed(format!("a challenge is 64 hex characters: {e}"))
        })?;
        Ok(Self(bytes))
    }
}

/// How the caller proves it holds a key.
#[derive(Debug, Clone)]
pub enum Proof {
    /// Plain HTTPS (relay): `signer_did` signed the challenge message.
    Challenge { challenge: Challenge, signer_did: String, signature: String },
    /// Iroh (mailbox): the transport already verified this key.
    Transport { verified_did: String },
}

/// May the proven key act for the subject?
#[derive(Debug, Clone)]
pub enum SignerBinding {
    /// The proven key is the subject (relay: the node key).
    SameKey,
    /// The proven key is an instance key, bound to the subject (a member
    /// master DID) by its service-instance certificate (ADR-0016). The
    /// certificate comes from the verified caller proof
    /// (`CallerContext.proof.delegation_json`), never from the request
    /// body. This check is the only one that requires the
    /// `service-instance` scope; the transport also accepts `routing`.
    InstanceCertificate(DelegationCertificate),
}

impl SignerBinding {
    pub(crate) fn check(
        &self,
        proven_did: &str,
        subject: &str,
        now_secs: u64,
    ) -> Result<(), RegisterError> {
        let bound = match self {
            Self::SameKey => proven_did == subject,
            Self::InstanceCertificate(cert) => {
                cert.temporary_did == proven_did
                    && cert.master_did == subject
                    && cert
                        .verify_at(subject, &[delegation::SCOPE_SERVICE_INSTANCE], now_secs)
                        .is_ok()
            }
        };
        if bound { Ok(()) } else { Err(RegisterError::NotBound) }
    }
}

impl Proof {
    /// Check the proof and return the proven DID. In the challenge form the
    /// challenge is used up before the signature check, so one challenge
    /// allows one guess.
    pub(crate) fn verify<'a>(
        &'a self,
        challenges: &ChallengeStore,
        host_node_did: &str,
        use_: &PassportUse,
        token: &CapabilityToken,
        now_secs: u64,
    ) -> Result<&'a str, RegisterError> {
        match self {
            Self::Challenge { challenge, signer_did, signature } => {
                challenges.take(challenge, now_secs)?;
                let hash = presentation_hash(token)
                    .map_err(|e| RegisterError::Malformed(e.to_string()))?;
                let message = challenge_message(host_node_did, use_.ability, challenge, &hash);
                substrate::verify_json_signature(signer_did, &message, signature)
                    .map_err(|_| RegisterError::BadSignature)?;
                Ok(signer_did)
            }
            Self::Transport { verified_did } => Ok(verified_did),
        }
    }
}

/// SHA-256 of the canonical JSON of the whole presented token, proofs and
/// signatures included. The signed body of a token leaves out its proofs,
/// so a hash of the body alone would let someone place another passport
/// under the same child token.
pub fn presentation_hash(token: &CapabilityToken) -> Result<[u8; 32]> {
    let canonical = substrate::canonicalize_json_value(&serde_json::to_value(token)?);
    Ok(Sha256::digest(serde_json::to_vec(&canonical)?).into())
}

/// The object the challenge signature covers.
pub(crate) fn challenge_message(
    host_node_did: &str,
    ability: &str,
    challenge: &Challenge,
    token_hash: &[u8; 32],
) -> Value {
    json!({
        "domain": CHALLENGE_DOMAIN,
        "service_did": host_node_did,
        "ability": ability,
        "challenge": challenge.to_hex(),
        "token_hash": hex::encode(token_hash),
    })
}

/// Sign a challenge for registration of `token` at `host_node_did`.
pub fn sign_challenge(
    signer: &Identity,
    host_node_did: &str,
    use_: &PassportUse,
    challenge: &Challenge,
    token: &CapabilityToken,
) -> Result<String> {
    signer.sign_json(&challenge_message(
        host_node_did,
        use_.ability,
        challenge,
        &presentation_hash(token)?,
    ))
}

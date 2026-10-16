//! The bodies of the two registration endpoints of the challenge form,
//! `POST /v1/home-relay/challenge` and `POST /v1/home-relay/register`.
//!
//! A request refuses unknown fields, so a typo is an error. A response
//! ignores unknown fields: a substrate reads responses from coordinators of
//! other operators, and a newer coordinator may add a field.
//!
//! Whether a challenge that is too old gets `challenge_expired` or
//! `unknown_challenge` depends on other traffic: the store drops an ended
//! challenge when the next one is issued. A client must treat both codes the
//! same way: ask for a new challenge and try again.
//!
//! It also holds `LeaseInfo`, the lease list the operator reads through the
//! running service.

use serde::{Deserialize, Serialize};
use syneroym_ucan::CapabilityToken;

use crate::{
    error::RegisterError,
    issue::PassportId,
    lease_store::Lease,
    proof::{Challenge, Proof},
};

/// Largest accepted registration body.
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;
/// The path of the challenge endpoint on the coordinator's info server.
pub const CHALLENGE_PATH: &str = "/v1/home-relay/challenge";
/// The path of the register endpoint on the coordinator's info server.
pub const REGISTER_PATH: &str = "/v1/home-relay/register";

/// One running lease, as the operator lists it. Mirrors the WIT record
/// `relay-lease` of the control-plane `security` interface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeaseInfo {
    pub passport_id: PassportId,
    pub owner_did: String,
    pub subject: String,
    pub lease_ends_at: u64,
    pub registered_at: u64,
}

impl From<&Lease> for LeaseInfo {
    fn from(lease: &Lease) -> Self {
        Self {
            passport_id: lease.passport_id.clone(),
            owner_did: lease.owner_did.clone(),
            subject: lease.subject.clone(),
            lease_ends_at: lease.lease_ends_at,
            registered_at: lease.registered_at,
        }
    }
}

/// The body of a challenge response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChallengeResponse {
    /// 64 hex characters.
    pub challenge: String,
    pub expires_in_secs: u64,
}

/// The body of a register request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    /// The child token, with the passport as its one proof.
    pub token: CapabilityToken,
    /// 64 hex characters.
    pub challenge: String,
    pub signer_did: String,
    pub signature: String,
}

/// The body of a successful register response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisterResponse {
    pub passport_id: PassportId,
    pub lease_ends_at: u64,
}

/// The body of a refusal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorBody {
    /// `RegisterError::code`.
    pub code: String,
    pub message: String,
}

impl RegisterRequest {
    /// Size first, then JSON shape; no signature is checked here.
    pub fn from_slice(body: &[u8]) -> Result<Self, RegisterError> {
        if body.len() > MAX_REQUEST_BYTES {
            return Err(RegisterError::RequestTooLarge { max: MAX_REQUEST_BYTES });
        }
        serde_json::from_slice(body).map_err(|e| RegisterError::Malformed(e.to_string()))
    }

    /// The token and the challenge-form proof of this request.
    pub fn into_parts(self) -> Result<(CapabilityToken, Proof), RegisterError> {
        let challenge = Challenge::from_hex(&self.challenge)?;
        let proof =
            Proof::Challenge { challenge, signer_did: self.signer_did, signature: self.signature };
        Ok((self.token, proof))
    }
}

impl From<&RegisterError> for ErrorBody {
    /// A store error carries the text of a database error. That text stays
    /// inside the service: the body has a fixed message, and the service
    /// must log the error itself before it converts it.
    fn from(e: &RegisterError) -> Self {
        let message = match e {
            RegisterError::Store(_) => "internal error".to_string(),
            other => other.to_string(),
        };
        Self { code: e.code().to_string(), message }
    }
}

/// The `code` of a client-side failure in `HomeRelayStatus::last_error`:
/// no answer, or an answer that is not a refusal body.
pub const UNREACHABLE_CODE: &str = "unreachable";

/// How a registration client treats a refusal code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryClass {
    /// `unknown_challenge`, `challenge_expired`: ask for a new challenge now.
    NewChallenge,
    /// `rate_limited`, `internal`, `unreachable`, and any code this build
    /// does not know: try again with backoff.
    Backoff,
    /// Every other known code: trying again soon gives the same answer.
    Settled,
}

impl RetryClass {
    #[must_use]
    pub fn of(code: &str) -> Self {
        match code {
            "unknown_challenge" | "challenge_expired" => Self::NewChallenge,
            "rate_limited" | "internal" | UNREACHABLE_CODE => Self::Backoff,
            "not_accepted" | "request_too_large" | "malformed" | "bad_signature" | "not_bound"
            | "untrusted_root" | "broken_chain" | "wrong_capability" | "expired"
            | "invalid_chain" | "denied" | "revoked" | "limit_reached" => Self::Settled,
            _ => Self::Backoff,
        }
    }
}

/// The params of the control-plane method `security.home-relay-set`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomeRelaySetRequest {
    /// The child token for the node key, with the passport as its one proof.
    pub token: CapabilityToken,
    /// Base URL of the coordinator info server. `https` only.
    pub info_url: String,
    /// The relay the passport is for. `None`: the node's
    /// `parent_coordinator.iroh.url`.
    #[serde(default)]
    pub relay_url: Option<String>,
    /// An extra root certificate (PEM) to trust for `info_url`.
    #[serde(default)]
    pub ca_cert_pem: Option<String>,
}

/// The stored home-relay passport of a node, and its last registration.
/// Accepts unknown fields, like every response type here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HomeRelayStatus {
    pub relay_url: String,
    pub info_url: String,
    /// False when `relay_url` is not the node's `parent_coordinator.iroh.url`:
    /// the passport is kept but not used.
    pub active: bool,
    pub passport_id: PassportId,
    pub host_node_did: String,
    pub owner_did: String,
    pub access_ends_at: u64,
    #[serde(default)]
    pub lease_ends_at: Option<u64>,
    #[serde(default)]
    pub last_attempt_at: Option<u64>,
    /// A refusal code, or `UNREACHABLE_CODE`.
    #[serde(default)]
    pub last_error: Option<ErrorBody>,
    #[serde(default)]
    pub next_attempt_at: Option<u64>,
}

#[cfg(test)]
mod tests;

//! The bodies of the two registration endpoints of the challenge form,
//! `POST /v1/home-relay/challenge` and `POST /v1/home-relay/register`.

use serde::{Deserialize, Serialize};
use syneroym_ucan::CapabilityToken;

use crate::{
    error::RegisterError,
    issue::PassportId,
    proof::{Challenge, Proof},
};

/// Largest accepted registration body.
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// The body of a challenge response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct RegisterResponse {
    pub passport_id: PassportId,
    pub lease_ends_at: u64,
}

/// The body of a refusal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
    fn from(e: &RegisterError) -> Self {
        Self { code: e.code().to_string(), message: e.to_string() }
    }
}

#[cfg(test)]
mod tests;

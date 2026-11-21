//! Signed commitment-failure records.
//!
//! The giver of one commitment signs it when its node cannot keep the
//! reservation it took for that commitment. The writer of the agreement
//! files it, marks that commitment `failed`, and cancels the others.
//!
//! The record names no party DID. Who may sign it is a rule of the writer: the
//! signer must be the giver of that commitment, which the agreement it
//! already holds shows.

use serde::{Deserialize, Serialize};
use syneroym_signed_record::VerifyOptions;
use thiserror::Error;

use crate::{
    listing,
    record::{self, RECORD_COMMITMENT_FAILURE},
    verdict::RecordVerdict,
};

pub const COMMITMENT_FAILURE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommitmentFailureError {
    #[error("agreement '{0}' must start with 'rec_'")]
    InvalidAgreement(String),
    #[error("conversation id is empty")]
    ConversationEmpty,
    #[error("commitment_id is empty")]
    CommitmentIdEmpty,
}

/// Why a commitment failed. One value is known today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureReason {
    /// The giver's reservation was taken by another booking before the giver
    /// confirmed it, or the giver had already released it.
    ReservationLost,
}

impl FailureReason {
    /// The reason as it is written in a snapshot's commitment line.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReservationLost => "reservation-lost",
        }
    }
}

/// The signed payload of a `commitment-failure` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitmentFailurePayload {
    /// The quote version's record id. It is also the envelope's subject.
    pub agreement: String,
    pub conversation: String,
    pub commitment_id: String,
    pub reason: FailureReason,
}

impl CommitmentFailurePayload {
    pub fn validate(&self) -> Result<(), CommitmentFailureError> {
        if !self.agreement.starts_with("rec_") {
            return Err(CommitmentFailureError::InvalidAgreement(self.agreement.clone()));
        }
        if self.conversation.trim().is_empty() {
            return Err(CommitmentFailureError::ConversationEmpty);
        }
        if self.commitment_id.trim().is_empty() {
            return Err(CommitmentFailureError::CommitmentIdEmpty);
        }
        Ok(())
    }
}

/// Verifies a signed `commitment-failure` record envelope.
pub fn verify_commitment_failure(
    envelope: &str,
    now_secs: u64,
) -> RecordVerdict<CommitmentFailurePayload> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_COMMITMENT_FAILURE
        || verified.version != COMMITMENT_FAILURE_VERSION
    {
        return RecordVerdict::refused("not a commitment-failure record this build understands");
    }
    let payload: CommitmentFailurePayload = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return RecordVerdict::refused(format!("payload: {e}")),
    };
    if let Err(e) = payload.validate() {
        return RecordVerdict::refused(e.to_string());
    }
    if verified.subject != payload.agreement {
        return RecordVerdict::refused("envelope subject does not match agreement");
    }
    if verified.expires_at_secs.is_some() {
        return RecordVerdict::refused("a commitment failure may not declare an expiry");
    }
    RecordVerdict {
        verified: true,
        expired: false,
        reason: None,
        revocation_status: Some(listing::revocation_status_word(verified.revocation_status)),
        record_id: Some(verified.record_id),
        issuer: Some(verified.issuer),
        signer_did: Some(verified.signer_did),
        issued_at_secs: Some(verified.issued_at_secs),
        expires_at_secs: None,
        supersedes: verified.supersedes,
        payload: Some(payload),
    }
}

#[cfg(test)]
mod tests;

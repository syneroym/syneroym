//! Fulfilment receipt records: attestations by quote author or counterparty
//! that work for a commitment was done.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    deal::{LocalText, SettlementRole},
    lang::TranslationError,
    listing, person,
    record::{self, RECORD_FULFILMENT_RECEIPT, VerifyOptions},
    verdict::RecordVerdict,
};

pub const FULFILMENT_RECEIPT_VERSION: u32 = 1;
pub const MAX_SETTLEMENT_NOTE_LEN: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FulfilmentError {
    #[error("agreement '{0}' must start with 'rec_'")]
    InvalidAgreement(String),
    #[error("counterparty_did '{0}' is not a valid did:key")]
    InvalidCounterpartyDid(String),
    #[error("quote_author_did '{0}' is not a valid did:key")]
    InvalidQuoteAuthorDid(String),
    #[error("counterparty and quote author cannot be the same DID")]
    SamePartyDid,
    #[error("conversation is empty")]
    ConversationEmpty,
    #[error("commitment_id is empty")]
    CommitmentIdEmpty,
    #[error("settlement_note: {0}")]
    SettlementNote(#[from] TranslationError),
}

/// One party's attestation regarding fulfilment of a commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FulfilmentReceiptPayload {
    pub agreement: String,
    pub conversation: String,
    pub commitment_id: String,
    pub counterparty_did: String,
    pub quote_author_did: String,
    pub role: SettlementRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement_note: Option<LocalText>,
}

impl FulfilmentReceiptPayload {
    pub fn validate(&self) -> Result<(), FulfilmentError> {
        if !self.agreement.starts_with("rec_") {
            return Err(FulfilmentError::InvalidAgreement(self.agreement.clone()));
        }
        if !person::is_did_key(&self.counterparty_did) {
            return Err(FulfilmentError::InvalidCounterpartyDid(self.counterparty_did.clone()));
        }
        if !person::is_did_key(&self.quote_author_did) {
            return Err(FulfilmentError::InvalidQuoteAuthorDid(self.quote_author_did.clone()));
        }
        if self.counterparty_did == self.quote_author_did {
            return Err(FulfilmentError::SamePartyDid);
        }
        if self.conversation.is_empty() {
            return Err(FulfilmentError::ConversationEmpty);
        }
        if self.commitment_id.is_empty() {
            return Err(FulfilmentError::CommitmentIdEmpty);
        }
        if let Some(note) = &self.settlement_note {
            note.validate("settlement_note", MAX_SETTLEMENT_NOTE_LEN)?;
        }
        Ok(())
    }
}

/// The hidden fields of a fulfilment receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FulfilmentHidden {
    pub settlement_note: Option<LocalText>,
}

impl FulfilmentHidden {
    pub fn validate(&self) -> Result<(), FulfilmentError> {
        if let Some(note) = &self.settlement_note {
            note.validate("settlement_note", MAX_SETTLEMENT_NOTE_LEN)?;
        }
        Ok(())
    }
}

/// Both halves must agree on agreement, conversation, parties, and
/// commitment_id, and come from opposite roles.
#[must_use]
pub fn fulfilment_halves_agree(a: &FulfilmentReceiptPayload, b: &FulfilmentReceiptPayload) -> bool {
    a.agreement == b.agreement
        && a.conversation == b.conversation
        && a.counterparty_did == b.counterparty_did
        && a.quote_author_did == b.quote_author_did
        && a.commitment_id == b.commitment_id
        && a.role != b.role
}

pub fn verify_fulfilment_receipt(
    envelope: &str,
    now_secs: u64,
) -> RecordVerdict<FulfilmentReceiptPayload> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_FULFILMENT_RECEIPT
        || verified.version != FULFILMENT_RECEIPT_VERSION
    {
        return RecordVerdict::refused("not a fulfilment-receipt record this build understands");
    }
    let payload: FulfilmentReceiptPayload = match serde_json::from_value(verified.payload.clone()) {
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
        return RecordVerdict::refused("a fulfilment receipt may not declare an expiry");
    }
    if verified.issuer != payload.quote_author_did && verified.issuer != payload.counterparty_did {
        return RecordVerdict::refused("fulfilment receipt issuer is not a party to the agreement");
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

//! Fulfilment receipt records: attestations by quote author or counterparty
//! that work for a commitment was done.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    deal::{LocalText, SettlementRole},
    disclosure::check_sd_shape,
    lang::TranslationError,
    person,
    record::RECORD_FULFILMENT_RECEIPT,
    verdict::{self, PartyRecordPayload, RecordVerdict},
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
    #[error("selective disclosure shape invalid: {0}")]
    SdShape(String),
}

impl FulfilmentError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidAgreement(_) => "invalid-agreement",
            Self::InvalidCounterpartyDid(_) => "invalid-counterparty-did",
            Self::InvalidQuoteAuthorDid(_) => "invalid-quote-author-did",
            Self::SamePartyDid => "same-party-did",
            Self::ConversationEmpty => "conversation-empty",
            Self::CommitmentIdEmpty => "commitment-id-empty",
            Self::SettlementNote(_) => "settlement-note",
            Self::SdShape(_) => "sd-shape",
        }
    }
}

/// One party's attestation regarding fulfilment of a commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FulfilmentReceiptPayload {
    pub agreement: String,
    pub conversation: String,
    pub commitment_id: String,
    pub counterparty_did: String,
    pub quote_author_did: String,
    pub role: SettlementRole,
    #[serde(rename = "_sd")]
    pub sd: Vec<String>,
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
        check_sd_shape(&self.sd, 1, "fulfilment-receipt").map_err(FulfilmentError::SdShape)?;
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

impl PartyRecordPayload for FulfilmentReceiptPayload {
    type Error = FulfilmentError;

    fn validate(&self) -> Result<(), Self::Error> {
        self.validate()
    }

    fn agreement(&self) -> &str {
        &self.agreement
    }

    fn quote_author_did(&self) -> &str {
        &self.quote_author_did
    }

    fn counterparty_did(&self) -> &str {
        &self.counterparty_did
    }
}

pub fn verify_fulfilment_receipt(
    envelope: &str,
    now_secs: u64,
) -> RecordVerdict<FulfilmentReceiptPayload> {
    verdict::verify_party_record(
        envelope,
        now_secs,
        RECORD_FULFILMENT_RECEIPT,
        FULFILMENT_RECEIPT_VERSION,
        "fulfilment receipt",
        "fulfilment receipt",
    )
}

#[cfg(test)]
mod tests;

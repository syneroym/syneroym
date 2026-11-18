//! Payment records: payment requests from receivers, and payment
//! acknowledgements from either party per commitment.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    deal::{Commitment, LocalText, SettlementRole},
    disclosure::check_sd_shape,
    lang::TranslationError,
    listing::{self, MAX_PAYMENT_METHOD_LEN},
    person,
    record::{self, RECORD_PAYMENT_ACKNOWLEDGEMENT, RECORD_PAYMENT_REQUEST, VerifyOptions},
    verdict::RecordVerdict,
};

pub const PAYMENT_REQUEST_VERSION: u32 = 1;
pub const PAYMENT_ACKNOWLEDGEMENT_VERSION: u32 = 1;
pub const MAX_NOTE_LEN: usize = 512;
pub const MAX_SETTLEMENT_NOTE_LEN: usize = 1024;
pub const MAX_REFERENCE_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PaymentError {
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
    #[error("observed_at_secs must be greater than zero")]
    ZeroObservedTime,
    #[error("payment method '{0}' is not 1..={MAX_PAYMENT_METHOD_LEN} chars of [a-z0-9-]")]
    MethodShape(String),
    #[error("reference is over {MAX_REFERENCE_LEN} characters")]
    ReferenceTooLong,
    #[error("note: {0}")]
    Note(#[from] TranslationError),
    #[error("selective disclosure shape invalid: {0}")]
    SdShape(String),
}

/// A request to be paid for a currency commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentRequestPayload {
    pub agreement: String,
    pub conversation: String,
    pub commitment_id: String,
    pub quote_author_did: String,
    pub counterparty_did: String,
    #[serde(rename = "_sd")]
    pub sd: Vec<String>,
}

impl PaymentRequestPayload {
    pub fn validate(&self) -> Result<(), PaymentError> {
        if !self.agreement.starts_with("rec_") {
            return Err(PaymentError::InvalidAgreement(self.agreement.clone()));
        }
        if !person::is_did_key(&self.counterparty_did) {
            return Err(PaymentError::InvalidCounterpartyDid(self.counterparty_did.clone()));
        }
        if !person::is_did_key(&self.quote_author_did) {
            return Err(PaymentError::InvalidQuoteAuthorDid(self.quote_author_did.clone()));
        }
        if self.counterparty_did == self.quote_author_did {
            return Err(PaymentError::SamePartyDid);
        }
        if self.conversation.is_empty() {
            return Err(PaymentError::ConversationEmpty);
        }
        if self.commitment_id.is_empty() {
            return Err(PaymentError::CommitmentIdEmpty);
        }
        check_sd_shape(&self.sd, 1, "payment-request").map_err(PaymentError::SdShape)?;
        Ok(())
    }
}

/// The hidden fields of a payment request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentRequestHidden {
    pub note: Option<LocalText>,
}

impl PaymentRequestHidden {
    pub fn validate(&self) -> Result<(), PaymentError> {
        if let Some(note) = &self.note {
            note.validate("note", MAX_NOTE_LEN)?;
        }
        Ok(())
    }
}

/// One party's statement about a payment for a specific commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentAcknowledgementPayload {
    pub agreement: String,
    pub conversation: String,
    pub commitment_id: String,
    pub quote_author_did: String,
    pub counterparty_did: String,
    pub role: SettlementRole,
    pub observed_at_secs: u64,
    #[serde(rename = "_sd")]
    pub sd: Vec<String>,
}

impl PaymentAcknowledgementPayload {
    pub fn validate(&self) -> Result<(), PaymentError> {
        if !self.agreement.starts_with("rec_") {
            return Err(PaymentError::InvalidAgreement(self.agreement.clone()));
        }
        if !person::is_did_key(&self.counterparty_did) {
            return Err(PaymentError::InvalidCounterpartyDid(self.counterparty_did.clone()));
        }
        if !person::is_did_key(&self.quote_author_did) {
            return Err(PaymentError::InvalidQuoteAuthorDid(self.quote_author_did.clone()));
        }
        if self.counterparty_did == self.quote_author_did {
            return Err(PaymentError::SamePartyDid);
        }
        if self.conversation.is_empty() {
            return Err(PaymentError::ConversationEmpty);
        }
        if self.commitment_id.is_empty() {
            return Err(PaymentError::CommitmentIdEmpty);
        }
        if self.observed_at_secs == 0 {
            return Err(PaymentError::ZeroObservedTime);
        }
        check_sd_shape(&self.sd, 3, "payment-acknowledgement").map_err(PaymentError::SdShape)?;
        Ok(())
    }
}

/// The hidden fields of a payment acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentAckHidden {
    pub method: Option<String>,
    pub reference: Option<String>,
    pub settlement_note: Option<LocalText>,
}

impl PaymentAckHidden {
    pub fn validate(&self) -> Result<(), PaymentError> {
        if let Some(m) = &self.method
            && !listing::valid_token(m, MAX_PAYMENT_METHOD_LEN)
        {
            return Err(PaymentError::MethodShape(m.clone()));
        }
        if self.reference.as_ref().is_some_and(|r| r.len() > MAX_REFERENCE_LEN) {
            return Err(PaymentError::ReferenceTooLong);
        }
        if let Some(note) = &self.settlement_note {
            note.validate("settlement_note", MAX_SETTLEMENT_NOTE_LEN)?;
        }
        Ok(())
    }
}

/// Checks whether a commitment accepts a given payment method.
#[must_use]
pub fn commitment_accepts(commitment: &Commitment, method: &str) -> bool {
    commitment
        .payment_methods
        .as_ref()
        .is_none_or(|methods| methods.is_empty() || methods.iter().any(|m| m == method))
}

/// The relaxed pair equality allowed for payment halves:
/// agreement, parties, commitment agree; roles differ.
#[must_use]
pub fn acknowledgements_agree(
    a: &PaymentAcknowledgementPayload,
    b: &PaymentAcknowledgementPayload,
) -> bool {
    a.agreement == b.agreement
        && a.conversation == b.conversation
        && a.counterparty_did == b.counterparty_did
        && a.quote_author_did == b.quote_author_did
        && a.commitment_id == b.commitment_id
        && a.role != b.role
}

/// A correction may change only the issuer's own observations.
#[must_use]
pub fn is_valid_correction(
    old: &PaymentAcknowledgementPayload,
    new: &PaymentAcknowledgementPayload,
) -> bool {
    old.agreement == new.agreement
        && old.conversation == new.conversation
        && old.counterparty_did == new.counterparty_did
        && old.quote_author_did == new.quote_author_did
        && old.commitment_id == new.commitment_id
        && old.role == new.role
}

pub fn verify_payment_request(
    envelope: &str,
    now_secs: u64,
) -> RecordVerdict<PaymentRequestPayload> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_PAYMENT_REQUEST || verified.version != PAYMENT_REQUEST_VERSION
    {
        return RecordVerdict::refused("not a payment-request record this build understands");
    }
    let payload: PaymentRequestPayload = match serde_json::from_value(verified.payload.clone()) {
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
        return RecordVerdict::refused("a payment request may not declare an expiry");
    }
    if verified.supersedes.is_some() {
        return RecordVerdict::refused("a payment request cannot be corrected");
    }
    if verified.issuer != payload.quote_author_did && verified.issuer != payload.counterparty_did {
        return RecordVerdict::refused("payment request issuer is not a party to the agreement");
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

pub fn verify_payment_acknowledgement(
    envelope: &str,
    now_secs: u64,
) -> RecordVerdict<PaymentAcknowledgementPayload> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_PAYMENT_ACKNOWLEDGEMENT
        || verified.version != PAYMENT_ACKNOWLEDGEMENT_VERSION
    {
        return RecordVerdict::refused(
            "not a payment-acknowledgement record this build understands",
        );
    }
    let payload: PaymentAcknowledgementPayload =
        match serde_json::from_value(verified.payload.clone()) {
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
        return RecordVerdict::refused("a payment acknowledgement may not declare an expiry");
    }
    if verified.issuer != payload.quote_author_did && verified.issuer != payload.counterparty_did {
        return RecordVerdict::refused("acknowledgement issuer is not a party to the agreement");
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

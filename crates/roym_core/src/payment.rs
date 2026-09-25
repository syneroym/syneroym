//! Payment records: payment requests from providers, and payment
//! acknowledgements from either party.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    listing::{self, MAX_PAYMENT_METHOD_LEN},
    money, person,
    record::{self, RECORD_PAYMENT_ACKNOWLEDGEMENT, RECORD_PAYMENT_REQUEST, VerifyOptions},
    transaction::{AgreedTerms, Role},
    verdict::RecordVerdict,
};

pub const PAYMENT_REQUEST_VERSION: u32 = 1;
pub const PAYMENT_ACKNOWLEDGEMENT_VERSION: u32 = 1;
pub const MAX_NOTE_LEN: usize = 512;
pub const MAX_REFERENCE_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PaymentError {
    #[error("agreement '{0}' must start with 'rec_'")]
    InvalidAgreement(String),
    #[error("consumer_did '{0}' is not a valid did:key")]
    InvalidConsumerDid(String),
    #[error("provider_did '{0}' is not a valid did:key")]
    InvalidProviderDid(String),
    #[error("consumer and provider cannot be the same DID")]
    SamePartyDid,
    #[error("conversation is empty")]
    ConversationEmpty,
    #[error("currency '{0}' is not a valid ISO currency code")]
    UnknownCurrency(String),
    #[error("amount_minor ({0}) cannot be negative")]
    NegativeAmount(i64),
    #[error("observed_at_secs must be greater than zero")]
    ZeroObservedTime,
    #[error("payment method is empty or over {MAX_PAYMENT_METHOD_LEN} characters")]
    InvalidPaymentMethod,
    #[error("reference is over {MAX_REFERENCE_LEN} characters")]
    ReferenceTooLong,
    #[error("note is over {MAX_NOTE_LEN} characters")]
    NoteTooLong,
}

/// The provider asking to be paid. No payee: the payee is the one the
/// signed agreement binds, and nothing else may name one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentRequestPayload {
    pub agreement: String,
    pub conversation: String,
    pub consumer_did: String,
    pub provider_did: String,
    pub currency: String,
    pub amount_minor: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl PaymentRequestPayload {
    pub fn validate(&self) -> Result<(), PaymentError> {
        if !self.agreement.starts_with("rec_") {
            return Err(PaymentError::InvalidAgreement(self.agreement.clone()));
        }
        if !person::is_did_key(&self.consumer_did) {
            return Err(PaymentError::InvalidConsumerDid(self.consumer_did.clone()));
        }
        if !person::is_did_key(&self.provider_did) {
            return Err(PaymentError::InvalidProviderDid(self.provider_did.clone()));
        }
        if self.consumer_did == self.provider_did {
            return Err(PaymentError::SamePartyDid);
        }
        if self.conversation.is_empty() {
            return Err(PaymentError::ConversationEmpty);
        }
        if money::currency_minor_exponent(&self.currency).is_none() {
            return Err(PaymentError::UnknownCurrency(self.currency.clone()));
        }
        if self.amount_minor < 0 {
            return Err(PaymentError::NegativeAmount(self.amount_minor));
        }
        if self.note.as_ref().is_some_and(|note| note.len() > MAX_NOTE_LEN) {
            return Err(PaymentError::NoteTooLong);
        }
        Ok(())
    }
}

/// One party's statement about a payment. The provider's half is the
/// payee confirming receipt; the consumer's half is the payer saying
/// they paid. Neither proves money moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentAcknowledgementPayload {
    pub agreement: String,
    pub conversation: String,
    pub consumer_did: String,
    pub provider_did: String,
    pub role: Role,
    pub currency: String,
    pub amount_minor: i64,
    /// When the issuer says the payment happened. The issuer's own word.
    pub observed_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Whatever proof the issuer has, as text. Never fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

impl PaymentAcknowledgementPayload {
    pub fn validate(&self) -> Result<(), PaymentError> {
        if !self.agreement.starts_with("rec_") {
            return Err(PaymentError::InvalidAgreement(self.agreement.clone()));
        }
        if !person::is_did_key(&self.consumer_did) {
            return Err(PaymentError::InvalidConsumerDid(self.consumer_did.clone()));
        }
        if !person::is_did_key(&self.provider_did) {
            return Err(PaymentError::InvalidProviderDid(self.provider_did.clone()));
        }
        if self.consumer_did == self.provider_did {
            return Err(PaymentError::SamePartyDid);
        }
        if self.conversation.is_empty() {
            return Err(PaymentError::ConversationEmpty);
        }
        if money::currency_minor_exponent(&self.currency).is_none() {
            return Err(PaymentError::UnknownCurrency(self.currency.clone()));
        }
        if self.amount_minor < 0 {
            return Err(PaymentError::NegativeAmount(self.amount_minor));
        }
        if self.observed_at_secs == 0 {
            return Err(PaymentError::ZeroObservedTime);
        }
        if self.method.as_ref().is_some_and(|m| m.is_empty() || m.len() > MAX_PAYMENT_METHOD_LEN) {
            return Err(PaymentError::InvalidPaymentMethod);
        }
        if self.reference.as_ref().is_some_and(|r| r.len() > MAX_REFERENCE_LEN) {
            return Err(PaymentError::ReferenceTooLong);
        }
        Ok(())
    }
}

/// One payment per agreement: the amount and currency are the agreed ones,
/// and a named method is one the terms list (when they list any).
#[must_use]
pub fn matches_terms(
    currency: &str,
    amount_minor: i64,
    method: Option<&str>,
    terms: &AgreedTerms,
) -> bool {
    if terms.currency != currency || terms.amount_minor != amount_minor {
        return false;
    }
    if let Some(m) = method
        && !terms.payment_methods.is_empty()
        && !terms.payment_methods.iter().any(|tm| tm == m)
    {
        return false;
    }
    true
}

/// The relaxed pair equality allowed for payment halves:
/// agreement, parties, amount and currency agree; roles differ.
#[must_use]
pub fn acknowledgements_agree(
    a: &PaymentAcknowledgementPayload,
    b: &PaymentAcknowledgementPayload,
) -> bool {
    a.agreement == b.agreement
        && a.conversation == b.conversation
        && a.consumer_did == b.consumer_did
        && a.provider_did == b.provider_did
        && a.currency == b.currency
        && a.amount_minor == b.amount_minor
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
        && old.consumer_did == new.consumer_did
        && old.provider_did == new.provider_did
        && old.role == new.role
        && old.currency == new.currency
        && old.amount_minor == new.amount_minor
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
    if verified.issuer != payload.provider_did {
        return RecordVerdict::refused("payment request issuer does not match provider_did");
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
    let expected_issuer = match payload.role {
        Role::Consumer => &payload.consumer_did,
        Role::Provider => &payload.provider_did,
    };
    if verified.issuer != *expected_issuer {
        return RecordVerdict::refused("acknowledgement issuer does not match role's DID");
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

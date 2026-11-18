//! The three signed records of an offer: what the counterparty asked for,
//! the exact terms the quote author offered, and each party's attestation
//! that they accepted those terms.
//!
//! Every number here is an integer, for the reason `listing` states:
//! a signed payload may hold no number that is not an integer, so money
//! is minor units with an explicit currency and geography is
//! micro-degrees.
//!
//! An attestation has one issuer and one signature. "Signed by both" is a
//! completeness rule over a pair of independent attestations of the same
//! record type, one from each party the quote names; neither half
//! references the other, and no signature carries a condition.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_signed_record::{EnvelopeError, VerifyOptions, content_digest};

use crate::{
    area::Area,
    deal::LocalText,
    lang::LangTag,
    listing::{self, ServiceLocation},
    person, record,
};
pub use crate::{
    deal::{Answers, DealTerms},
    record::{RECORD_AGREEMENT_RECEIPT, RECORD_DEAL_DECLINE, RECORD_ENQUIRY, RECORD_QUOTE},
};

pub mod decline;
pub use decline::{DEAL_DECLINE_VERSION, DealDeclinePayload, verify_deal_decline};

pub const ENQUIRY_VERSION: u32 = 1;
pub const QUOTE_VERSION: u32 = 1;
pub const AGREEMENT_RECEIPT_VERSION: u32 = 1;

pub const EVENT_CARD: &str = "transaction.card";
pub const EVENT_PROGRESS: &str = "transaction.progress";

/// The English text of the notice a counterparty is shown before an enquiry
/// is signed, and which the enquiry then carries under their own signature.
/// `roymctl` uses it as the default. The Hub signs the notice in its own
/// language: `request-data-use-notice` in the string catalog is this text in
/// `en.ftl` (a test reads that file and compares), and the Hindi catalog
/// holds the Hindi text. One constant for English, so `roymctl`, the English
/// Hub and the record cannot drift apart, and so the notice is never an
/// empty string nobody noticed.
pub const DEFAULT_DATA_USE_NOTICE: &str = "This request is signed by you and sent to the person \
                                           or business you chose. They keep a copy. It carries \
                                           the area you gave, not your exact address; an address \
                                           is disclosed only inside a quote you accept.";

/// Shown above the address field whenever a quote states one. The Hub holds
/// it as `quote-address-disclosure-notice` in `en.ftl`; a test reads that
/// file and compares, and the browser suite pins it character for character.
pub const ADDRESS_DISCLOSURE_NOTICE: &str = "This address becomes part of a signed record that \
                                             both parties keep and can export. It cannot be \
                                             removed from a record already signed.";

pub const ENQUIRY_ID_PREFIX: &str = "enq_";
pub const QUOTE_ID_PREFIX: &str = "quo_";

pub const MAX_DESCRIPTION_LEN: usize = 4096;
pub const MAX_SCOPE_LEN: usize = 4096;
pub const MAX_TERMS_TEXT_LEN: usize = 2048; // cancellation, refund, dispute
pub const MAX_NOTICE_LEN: usize = 2048;
pub const MAX_ADDRESS_LEN: usize = 512;
pub const MAX_CATEGORIES: usize = 8;
pub const MAX_CATEGORY_LEN: usize = 64;
pub const MAX_PAYEE_LEN: usize = 256;
pub const MAX_PAYMENT_METHODS: usize = 16;
pub const MAX_PAYMENT_METHOD_LEN: usize = 32;

/// A quote may not be offered open-endedly, so both a floor and a ceiling
/// are enforced on how long one can stay valid.
pub const MIN_QUOTE_LIFETIME_SECS: u64 = 300;
pub const MAX_QUOTE_LIFETIME_SECS: u64 = 90 * 24 * 3600;

/// How many messages one `sync` reads, in **one** `conversation.history`
/// call. Not a page size: `sync` never loops.
pub const SYNC_WINDOW: u32 = 500;
/// Cards one conversation keeps. A conversation past this stops filing
/// new ones rather than growing without bound.
pub const MAX_CARDS_PER_CONVERSATION: usize = 2_000;

/// One of the two parties of a deal: the quote author or the counterparty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DealRole {
    QuoteAuthor,
    Counterparty,
}

impl DealRole {
    /// The role's name on the wire, the same text serde writes. Ids that
    /// are built from a role use this, so they cannot drift from the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QuoteAuthor => "quote-author",
            Self::Counterparty => "counterparty",
        }
    }
}

/// A window in unix seconds. `latest_secs` is inclusive and must not be
/// before `earliest_secs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeWindow {
    pub earliest_secs: u64,
    pub latest_secs: u64,
}

/// Where the work happens, as a quote states it. `address` is the one
/// field the spec's disclosure rule is about: it is present only when the
/// work is at the receiver's place, it is part of a signed record both parties
/// keep, and the product says so before it is filled in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteLocation {
    #[serde(rename = "where")]
    pub where_: ServiceLocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<Area>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnquiryPayload {
    /// `content_digest("enq_", {conversation, issuer, sequence})`.
    pub enquiry_id: String,
    /// The conversation this enquiry was made in. Both parties compute
    /// the same value, so it is the one thing a receiver can check the
    /// id against.
    pub conversation: String,
    /// The nth enquiry this issuer made in this conversation, from 1.
    pub sequence: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listing_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    pub description: LocalText,
    /// Approximate, deliberately. An exact address is disclosed inside a
    /// quote, when the work needs one, and never here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<Area>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<TimeWindow>,
    /// What the counterparty was told about how this data is used, recorded
    /// under their own signature so the notice is part of the record.
    pub data_use_notice: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotePayload {
    /// `content_digest("quo_", {conversation, issuer, sequence})`.
    pub quote_id: String,
    pub conversation: String,
    pub sequence: u32,
    pub answers: Answers,
    pub counterparty_did: String,
    pub terms: DealTerms,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgreementReceiptPayload {
    /// The quote's own `record_id`. A content hash of the whole quote
    /// envelope, so it pins the terms exactly.
    pub quote_record_id: String,
    pub counterparty_did: String,
    pub quote_author_did: String,
    /// The only field the two halves of a complete pair differ in.
    pub role: DealRole,
    pub terms: DealTerms,
    /// The language of the text this party read before it signed. It is
    /// outside `terms`, so the two halves may differ in it.
    pub viewed_lang: LangTag,
    /// True when that text was a machine translation.
    pub viewed_via_mt: bool,
}

/// One party's attestation, as it is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptHalf {
    pub envelope: String,
    pub record_id: String,
    pub issuer: String,
    pub issued_at_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum PairState {
    /// Neither party has attested.
    None,
    /// Exactly one half exists.
    Half { role: DealRole },
    /// One attestation from each of the two parties the quote names, with
    /// identical terms and each inside its own validity window.
    Complete,
}

/// Computes the pair state for two halves.
///
/// The stronger checks (identical terms, issuer matches role) are made
/// when each half is filed, not when the pair is read -- a half that
/// failed them is never stored as a half.
#[must_use]
pub fn pair_state(
    counterparty: Option<&ReceiptHalf>,
    quote_author: Option<&ReceiptHalf>,
) -> PairState {
    match (counterparty, quote_author) {
        (Some(_), Some(_)) => PairState::Complete,
        (Some(_), None) => PairState::Half { role: DealRole::Counterparty },
        (None, Some(_)) => PairState::Half { role: DealRole::QuoteAuthor },
        (None, None) => PairState::None,
    }
}

mod error;
pub use error::TransactionError;

impl TimeWindow {
    pub fn validate(&self) -> Result<(), TransactionError> {
        if self.latest_secs < self.earliest_secs {
            return Err(TransactionError::InvalidTimeWindow {
                earliest_secs: self.earliest_secs,
                latest_secs: self.latest_secs,
            });
        }
        Ok(())
    }
}

impl QuoteLocation {
    pub fn validate(&self) -> Result<(), TransactionError> {
        if self.where_ != ServiceLocation::AtReceiver && self.address.is_some() {
            return Err(TransactionError::AddressNotApplicable);
        }
        if self.address.as_deref().is_some_and(|addr| addr.len() > MAX_ADDRESS_LEN) {
            return Err(TransactionError::AddressTooLong);
        }
        if let Some(ref a) = self.area {
            a.validate()?;
        }
        Ok(())
    }
}

impl EnquiryPayload {
    pub fn validate(&self) -> Result<(), TransactionError> {
        if self.enquiry_id.is_empty() {
            return Err(TransactionError::EnquiryIdEmpty);
        }
        if self.conversation.is_empty() {
            return Err(TransactionError::ConversationEmpty);
        }
        if self.sequence < 1 {
            return Err(TransactionError::InvalidSequence(self.sequence));
        }
        self.description.validate("description", MAX_DESCRIPTION_LEN)?;
        if self.categories.len() > MAX_CATEGORIES {
            return Err(TransactionError::TooManyCategories);
        }
        for cat in &self.categories {
            if !listing::valid_token(cat, MAX_CATEGORY_LEN) {
                return Err(TransactionError::InvalidCategory(cat.clone()));
            }
        }
        if let Some(ref a) = self.area {
            a.validate()?;
        }
        if let Some(ref w) = self.window {
            w.validate()?;
        }
        if self.data_use_notice.trim().is_empty() {
            return Err(TransactionError::DataUseNoticeEmpty);
        }
        if self.data_use_notice.len() > MAX_NOTICE_LEN {
            return Err(TransactionError::DataUseNoticeTooLong);
        }
        Ok(())
    }
}

impl QuotePayload {
    pub fn validate(&self) -> Result<(), TransactionError> {
        if self.quote_id.is_empty() {
            return Err(TransactionError::QuoteIdEmpty);
        }
        if self.conversation.is_empty() {
            return Err(TransactionError::ConversationEmpty);
        }
        if self.sequence < 1 {
            return Err(TransactionError::InvalidSequence(self.sequence));
        }
        match &self.answers {
            Answers::Enquiry { record_id } => {
                if !record_id.starts_with("rec_") {
                    return Err(TransactionError::InvalidRecordId(record_id.clone()));
                }
            }
            Answers::Want { .. } => {
                return Err(TransactionError::AnswersKindNotSupported);
            }
        }
        if !person::is_did_key(&self.counterparty_did) {
            return Err(TransactionError::InvalidDid(self.counterparty_did.clone()));
        }
        self.terms.validate()?;
        Ok(())
    }
}

impl AgreementReceiptPayload {
    pub fn validate(&self) -> Result<(), TransactionError> {
        if !self.quote_record_id.starts_with("rec_") {
            return Err(TransactionError::InvalidRecordId(self.quote_record_id.clone()));
        }
        if !person::is_did_key(&self.counterparty_did) {
            return Err(TransactionError::InvalidDid(self.counterparty_did.clone()));
        }
        if !person::is_did_key(&self.quote_author_did) {
            return Err(TransactionError::InvalidDid(self.quote_author_did.clone()));
        }
        if self.counterparty_did == self.quote_author_did {
            return Err(TransactionError::SamePartyDid);
        }
        self.terms.validate()?;
        if self.viewed_via_mt && self.viewed_lang == self.terms.scope.lang {
            return Err(TransactionError::ViewedMtOfOriginal);
        }
        Ok(())
    }
}

pub fn derive_enquiry_id(
    conversation: &str,
    issuer: &str,
    sequence: u32,
) -> Result<String, EnvelopeError> {
    content_digest(
        ENQUIRY_ID_PREFIX,
        &json!({
            "conversation": conversation,
            "issuer": issuer,
            "sequence": sequence,
        }),
    )
}

pub fn derive_quote_id(
    conversation: &str,
    issuer: &str,
    sequence: u32,
) -> Result<String, EnvelopeError> {
    content_digest(
        QUOTE_ID_PREFIX,
        &json!({
            "conversation": conversation,
            "issuer": issuer,
            "sequence": sequence,
        }),
    )
}

/// True when two halves of a pair agree on everything a pair must agree
/// on: every other field matches and they come from opposite roles.
#[must_use]
pub fn halves_agree(a: &AgreementReceiptPayload, b: &AgreementReceiptPayload) -> bool {
    a.quote_record_id == b.quote_record_id
        && a.counterparty_did == b.counterparty_did
        && a.quote_author_did == b.quote_author_did
        && a.terms == b.terms
        && a.role != b.role
}

pub use crate::verdict::RecordVerdict;

pub fn verify_enquiry(envelope: &str, now_secs: u64) -> RecordVerdict<EnquiryPayload> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_ENQUIRY || verified.version != ENQUIRY_VERSION {
        return RecordVerdict::refused("not an enquiry record this build understands");
    }
    let payload: EnquiryPayload = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return RecordVerdict::refused(format!("payload: {e}")),
    };
    if let Err(e) = payload.validate() {
        return RecordVerdict::refused(e.to_string());
    }
    let expected_id =
        match derive_enquiry_id(&payload.conversation, &verified.issuer, payload.sequence) {
            Ok(id) => id,
            Err(e) => return RecordVerdict::refused(e.to_string()),
        };
    if payload.enquiry_id != expected_id {
        return RecordVerdict::refused(
            "enquiry_id is not derivable from the signature's own issuer",
        );
    }
    if verified.expires_at_secs.is_some() {
        return RecordVerdict::refused("an enquiry may not declare an expiry");
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

pub fn verify_quote(envelope: &str, now_secs: u64) -> RecordVerdict<QuotePayload> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_QUOTE || verified.version != QUOTE_VERSION {
        return RecordVerdict::refused("not a quote record this build understands");
    }
    let payload: QuotePayload = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return RecordVerdict::refused(format!("payload: {e}")),
    };
    if let Err(e) = payload.validate() {
        return RecordVerdict::refused(e.to_string());
    }
    if verified.expires_at_secs != Some(payload.terms.quote_expires_at_secs) {
        return RecordVerdict::refused(
            "envelope expires_at_secs does not match quote_expires_at_secs in terms",
        );
    }
    let lifetime = verified.expires_at_secs.unwrap_or(0).saturating_sub(verified.issued_at_secs);
    if !(MIN_QUOTE_LIFETIME_SECS..=MAX_QUOTE_LIFETIME_SECS).contains(&lifetime) {
        return RecordVerdict::refused("quote lifetime outside permitted bounds");
    }
    if payload.counterparty_did == verified.issuer {
        return RecordVerdict::refused("counterparty_did cannot be the quote issuer");
    }
    let expected_id =
        match derive_quote_id(&payload.conversation, &verified.issuer, payload.sequence) {
            Ok(id) => id,
            Err(e) => return RecordVerdict::refused(e.to_string()),
        };
    if payload.quote_id != expected_id {
        return RecordVerdict::refused("quote_id is not derivable from the signature's own issuer");
    }
    let expired = verified.expires_at_secs.is_some_and(|e| now_secs >= e);
    RecordVerdict {
        verified: true,
        expired,
        reason: None,
        revocation_status: Some(listing::revocation_status_word(verified.revocation_status)),
        record_id: Some(verified.record_id),
        issuer: Some(verified.issuer),
        signer_did: Some(verified.signer_did),
        issued_at_secs: Some(verified.issued_at_secs),
        expires_at_secs: verified.expires_at_secs,
        supersedes: verified.supersedes,
        payload: Some(payload),
    }
}

pub fn verify_agreement_receipt(
    envelope: &str,
    now_secs: u64,
) -> RecordVerdict<AgreementReceiptPayload> {
    let opts = VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_AGREEMENT_RECEIPT
        || verified.version != AGREEMENT_RECEIPT_VERSION
    {
        return RecordVerdict::refused("not an agreement-receipt record this build understands");
    }
    let payload: AgreementReceiptPayload = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return RecordVerdict::refused(format!("payload: {e}")),
    };
    if let Err(e) = payload.validate() {
        return RecordVerdict::refused(e.to_string());
    }
    if verified.subject != payload.quote_record_id {
        return RecordVerdict::refused("envelope subject does not match quote_record_id");
    }
    if verified.expires_at_secs.is_some() {
        return RecordVerdict::refused("an agreement receipt may not declare an expiry");
    }
    match payload.role {
        DealRole::Counterparty => {
            if verified.issuer != payload.counterparty_did {
                return RecordVerdict::refused(
                    "counterparty receipt issuer does not match counterparty_did",
                );
            }
        }
        DealRole::QuoteAuthor => {
            if verified.issuer != payload.quote_author_did {
                return RecordVerdict::refused(
                    "quote author receipt issuer does not match quote_author_did",
                );
            }
        }
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
        expires_at_secs: verified.expires_at_secs,
        supersedes: verified.supersedes,
        payload: Some(payload),
    }
}

#[cfg(test)]
mod tests;

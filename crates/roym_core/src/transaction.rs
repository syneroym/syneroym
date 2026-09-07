//! The three signed records of an offer: what the consumer asked for,
//! the exact terms the provider offered, and each party's attestation
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
use syneroym_signed_record::{EnvelopeError, content_digest};
use thiserror::Error;

use crate::{
    area::{Area, AreaError},
    listing::{self, ServiceLocation},
    money, person,
    record::{self, RECORD_AGREEMENT_RECEIPT, RECORD_QUOTE, RECORD_REQUEST},
};

pub const REQUEST_VERSION: u32 = 1;
pub const QUOTE_VERSION: u32 = 1;
pub const AGREEMENT_RECEIPT_VERSION: u32 = 1;

/// The notice a consumer is shown before a request is signed, and which
/// the request then carries under their own signature. One constant, so
/// the Hub, `roymctl` and the record cannot drift apart, and so the
/// notice is never an empty string nobody noticed.
pub const DEFAULT_DATA_USE_NOTICE: &str = "This request is signed by you and sent to the provider \
                                           you chose. They keep a copy. It carries the area you \
                                           gave, not your exact address; an address is disclosed \
                                           only inside a quote you accept.";

/// Shown above the address field whenever a quote states one, and pinned
/// character-for-character by the browser suite -- the same discipline
/// `messages.ts` already applies to the deletion notes.
pub const ADDRESS_DISCLOSURE_NOTICE: &str = "This address becomes part of a signed record that \
                                             both parties keep and can export. It cannot be \
                                             removed from a record already signed.";

pub const REQUEST_ID_PREFIX: &str = "req_";
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

/// A quote may not be offered open-endedly; failure-matrix row 9 is the
/// reason there is a ceiling as well as a floor.
pub const MIN_QUOTE_LIFETIME_SECS: u64 = 300;
pub const MAX_QUOTE_LIFETIME_SECS: u64 = 90 * 24 * 3600;

/// How many messages one `sync` reads, in **one** `conversation.history`
/// call. Not a page size: `sync` never loops.
pub const SYNC_WINDOW: u32 = 500;
/// How far behind its own watermark a `sync` re-reads. `history` orders by
/// sender timestamp, so a message that arrives late inserts before the
/// watermark; this is the window in which that is invisible.
pub const SYNC_OVERLAP: u64 = 50;
/// Cards one conversation keeps. A conversation past this stops filing
/// new ones rather than growing without bound.
pub const MAX_CARDS_PER_CONVERSATION: usize = 2_000;

/// When the quote says money changes hands. A signed term, used to drive
/// what the product asks next, never to gate a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaymentTiming {
    BeforeWork,
    AfterWork,
}

/// Which of the two parties a receipt half is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Consumer,
    Provider,
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
/// work is at the customer, it is part of a signed record both parties
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

/// The one struct the quote states and both halves of the agreement
/// receipt carry verbatim. Every field the Records table names for an
/// `agreement-receipt` is here: payee, expiry, cancellation and refund
/// terms, and dispute path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgreedTerms {
    pub scope: String,
    /// ISO-4217, and a code `money::currency_minor_exponent` knows.
    pub currency: String,
    /// The total the consumer owes, inclusive, in minor units.
    pub amount_minor: i64,
    /// Informational breakdowns of `amount_minor`, never additions to it.
    pub tax_minor: i64,
    pub fees_minor: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub payment_methods: Vec<String>,
    /// Bound here. A later chat message naming another payee changes
    /// nothing, and the product shows this one.
    pub payee: String,
    pub payment_timing: PaymentTiming,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<TimeWindow>,
    pub location: QuoteLocation,
    pub cancellation_terms: String,
    pub refund_terms: String,
    pub dispute_path: String,
    /// What the quote's own envelope expiry was. A record of the window
    /// acceptance had to fall inside; the receipt does not expire.
    pub quote_expires_at_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPayload {
    /// `content_digest("req_", {conversation, issuer, sequence})`.
    pub request_id: String,
    /// The conversation this request was made in. Both parties compute
    /// the same value, so it is the one thing a receiver can check the
    /// id against.
    pub conversation: String,
    /// The nth request this issuer made in this conversation, from 1.
    pub sequence: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listing_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    pub description: String,
    /// Approximate, deliberately. An exact address is disclosed inside a
    /// quote, when the work needs one, and never here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<Area>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<TimeWindow>,
    /// What the consumer was told about how this data is used, recorded
    /// under their own signature so the notice is part of the record.
    pub data_use_notice: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotePayload {
    /// `content_digest("quo_", {conversation, issuer, sequence})`.
    pub quote_id: String,
    pub conversation: String,
    pub sequence: u32,
    /// The `record_id` of the request version this answers.
    pub request_record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listing_id: Option<String>,
    /// The request's own issuer. This is what names the second party, and
    /// it is what makes the agreement pair checkable.
    pub consumer_did: String,
    pub terms: AgreedTerms,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgreementReceiptPayload {
    /// The quote's own `record_id`. A content hash of the whole quote
    /// envelope, so it pins the terms exactly.
    pub quote_record_id: String,
    pub consumer_did: String,
    pub provider_did: String,
    /// The only field the two halves of a complete pair differ in.
    pub role: Role,
    pub terms: AgreedTerms,
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
    Half { role: Role },
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
pub fn pair_state(consumer: Option<&ReceiptHalf>, provider: Option<&ReceiptHalf>) -> PairState {
    match (consumer, provider) {
        (Some(_), Some(_)) => PairState::Complete,
        (Some(_), None) => PairState::Half { role: Role::Consumer },
        (None, Some(_)) => PairState::Half { role: Role::Provider },
        (None, None) => PairState::None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransactionError {
    #[error("latest_secs ({latest_secs}) is before earliest_secs ({earliest_secs})")]
    InvalidTimeWindow { earliest_secs: u64, latest_secs: u64 },

    #[error("address is only applicable when location is at-customer")]
    AddressNotApplicable,

    #[error("address is over {MAX_ADDRESS_LEN} characters")]
    AddressTooLong,

    #[error("area: {0}")]
    Area(#[from] AreaError),

    #[error("scope is empty")]
    ScopeEmpty,

    #[error("scope is over {MAX_SCOPE_LEN} characters")]
    ScopeTooLong,

    #[error("currency '{0}' is not a currency code this build knows")]
    CurrencyUnknown(String),

    #[error("amount_minor ({0}) cannot be negative")]
    NegativeAmount(i64),

    #[error("tax_minor ({0}) cannot be negative")]
    NegativeTax(i64),

    #[error("fees_minor ({0}) cannot be negative")]
    NegativeFees(i64),

    #[error(
        "tax_minor ({tax_minor}) + fees_minor ({fees_minor}) exceeds amount_minor ({amount_minor})"
    )]
    BreakdownExceedsTotal { tax_minor: i64, fees_minor: i64, amount_minor: i64 },

    #[error("payee is empty")]
    PayeeEmpty,

    #[error("payee is over {MAX_PAYEE_LEN} characters")]
    PayeeTooLong,

    #[error("more than {MAX_PAYMENT_METHODS} payment methods")]
    TooManyPaymentMethods,

    #[error("payment method is empty or over {MAX_PAYMENT_METHOD_LEN} characters")]
    PaymentMethodTooLong,

    #[error("cancellation_terms is empty")]
    CancellationTermsEmpty,

    #[error("cancellation_terms is over {MAX_TERMS_TEXT_LEN} characters")]
    CancellationTermsTooLong,

    #[error("refund_terms is empty")]
    RefundTermsEmpty,

    #[error("refund_terms is over {MAX_TERMS_TEXT_LEN} characters")]
    RefundTermsTooLong,

    #[error("dispute_path is empty")]
    DisputePathEmpty,

    #[error("dispute_path is over {MAX_TERMS_TEXT_LEN} characters")]
    DisputePathTooLong,

    #[error("quote_expires_at_secs must be non-zero")]
    QuoteExpiryZero,

    #[error("quote_id is empty")]
    QuoteIdEmpty,

    #[error("conversation is empty")]
    ConversationEmpty,

    #[error("sequence must be >= 1, got {0}")]
    InvalidSequence(u32),

    #[error("record_id '{0}' must start with 'rec_'")]
    InvalidRecordId(String),

    #[error("did '{0}' is not a valid did:key")]
    InvalidDid(String),

    #[error("request_id is empty")]
    RequestIdEmpty,

    #[error("description is empty")]
    DescriptionEmpty,

    #[error("description is over {MAX_DESCRIPTION_LEN} characters")]
    DescriptionTooLong,

    #[error("more than {MAX_CATEGORIES} categories")]
    TooManyCategories,

    #[error("category '{0}' is not 1..={MAX_CATEGORY_LEN} chars of [a-z0-9-]")]
    InvalidCategory(String),

    #[error("data_use_notice is empty")]
    DataUseNoticeEmpty,

    #[error("data_use_notice is over {MAX_NOTICE_LEN} characters")]
    DataUseNoticeTooLong,
}

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
        if self.where_ != ServiceLocation::AtCustomer && self.address.is_some() {
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

impl AgreedTerms {
    pub fn validate(&self) -> Result<(), TransactionError> {
        if self.scope.trim().is_empty() {
            return Err(TransactionError::ScopeEmpty);
        }
        if self.scope.len() > MAX_SCOPE_LEN {
            return Err(TransactionError::ScopeTooLong);
        }
        if money::currency_minor_exponent(&self.currency).is_none() {
            return Err(TransactionError::CurrencyUnknown(self.currency.clone()));
        }
        if self.amount_minor < 0 {
            return Err(TransactionError::NegativeAmount(self.amount_minor));
        }
        if self.tax_minor < 0 {
            return Err(TransactionError::NegativeTax(self.tax_minor));
        }
        if self.fees_minor < 0 {
            return Err(TransactionError::NegativeFees(self.fees_minor));
        }
        let breakdown = self.tax_minor.checked_add(self.fees_minor);
        match breakdown {
            Some(b) if b <= self.amount_minor => {}
            _ => {
                return Err(TransactionError::BreakdownExceedsTotal {
                    tax_minor: self.tax_minor,
                    fees_minor: self.fees_minor,
                    amount_minor: self.amount_minor,
                });
            }
        }
        if self.payee.trim().is_empty() {
            return Err(TransactionError::PayeeEmpty);
        }
        if self.payee.len() > MAX_PAYEE_LEN {
            return Err(TransactionError::PayeeTooLong);
        }
        if self.payment_methods.len() > MAX_PAYMENT_METHODS {
            return Err(TransactionError::TooManyPaymentMethods);
        }
        for m in &self.payment_methods {
            if m.trim().is_empty() || m.len() > MAX_PAYMENT_METHOD_LEN {
                return Err(TransactionError::PaymentMethodTooLong);
            }
        }
        if let Some(ref sched) = self.schedule {
            sched.validate()?;
        }
        self.location.validate()?;
        if self.cancellation_terms.trim().is_empty() {
            return Err(TransactionError::CancellationTermsEmpty);
        }
        if self.cancellation_terms.len() > MAX_TERMS_TEXT_LEN {
            return Err(TransactionError::CancellationTermsTooLong);
        }
        if self.refund_terms.trim().is_empty() {
            return Err(TransactionError::RefundTermsEmpty);
        }
        if self.refund_terms.len() > MAX_TERMS_TEXT_LEN {
            return Err(TransactionError::RefundTermsTooLong);
        }
        if self.dispute_path.trim().is_empty() {
            return Err(TransactionError::DisputePathEmpty);
        }
        if self.dispute_path.len() > MAX_TERMS_TEXT_LEN {
            return Err(TransactionError::DisputePathTooLong);
        }
        if self.quote_expires_at_secs == 0 {
            return Err(TransactionError::QuoteExpiryZero);
        }
        Ok(())
    }
}

impl RequestPayload {
    pub fn validate(&self) -> Result<(), TransactionError> {
        if self.request_id.is_empty() {
            return Err(TransactionError::RequestIdEmpty);
        }
        if self.conversation.is_empty() {
            return Err(TransactionError::ConversationEmpty);
        }
        if self.sequence < 1 {
            return Err(TransactionError::InvalidSequence(self.sequence));
        }
        if self.description.trim().is_empty() {
            return Err(TransactionError::DescriptionEmpty);
        }
        if self.description.len() > MAX_DESCRIPTION_LEN {
            return Err(TransactionError::DescriptionTooLong);
        }
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
        if !self.request_record_id.starts_with("rec_") {
            return Err(TransactionError::InvalidRecordId(self.request_record_id.clone()));
        }
        if !person::is_did_key(&self.consumer_did) {
            return Err(TransactionError::InvalidDid(self.consumer_did.clone()));
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
        if !person::is_did_key(&self.consumer_did) {
            return Err(TransactionError::InvalidDid(self.consumer_did.clone()));
        }
        if !person::is_did_key(&self.provider_did) {
            return Err(TransactionError::InvalidDid(self.provider_did.clone()));
        }
        self.terms.validate()?;
        Ok(())
    }
}

pub fn derive_request_id(
    conversation: &str,
    issuer: &str,
    sequence: u32,
) -> Result<String, EnvelopeError> {
    content_digest(
        REQUEST_ID_PREFIX,
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
/// on: every field except `role`.
#[must_use]
pub fn halves_agree(a: &AgreementReceiptPayload, b: &AgreementReceiptPayload) -> bool {
    a.quote_record_id == b.quote_record_id
        && a.consumer_did == b.consumer_did
        && a.provider_did == b.provider_did
        && a.terms == b.terms
        && a.role != b.role
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordVerdict<P> {
    pub verified: bool,
    /// **Expiry is not a refusal here.** A record past its own
    /// `expires_at_secs` still verifies -- the signature, the issuer and
    /// the delegation window are all still good -- and this says the
    /// window has passed. A caller that must decide whether to *act*
    /// (`agreement.accept`) refuses on this; a caller that must *show
    /// what was offered* (the card filer, the Hub) does not. Always
    /// false for a record type that carries no expiry.
    pub expired: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_at_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<P>,
}

impl<P> RecordVerdict<P> {
    fn refused(reason: impl Into<String>) -> Self {
        Self {
            verified: false,
            expired: false,
            reason: Some(reason.into()),
            revocation_status: None,
            record_id: None,
            issuer: None,
            issued_at_secs: None,
            expires_at_secs: None,
            supersedes: None,
            payload: None,
        }
    }
}

pub fn verify_request(envelope: &str, now_secs: u64) -> RecordVerdict<RequestPayload> {
    let opts = record::VerifyOptions::new(now_secs).allowing_expired();
    let verified = match record::verify_json(envelope, &opts) {
        Ok(v) => v,
        Err(e) => return RecordVerdict::refused(e.to_string()),
    };
    if verified.record_type != RECORD_REQUEST || verified.version != REQUEST_VERSION {
        return RecordVerdict::refused("not a request record this build understands");
    }
    let payload: RequestPayload = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return RecordVerdict::refused(format!("payload: {e}")),
    };
    if let Err(e) = payload.validate() {
        return RecordVerdict::refused(e.to_string());
    }
    let expected_id =
        match derive_request_id(&payload.conversation, &verified.issuer, payload.sequence) {
            Ok(id) => id,
            Err(e) => return RecordVerdict::refused(e.to_string()),
        };
    if payload.request_id != expected_id {
        return RecordVerdict::refused(
            "request_id is not derivable from the signature's own issuer",
        );
    }
    let expired = verified.expires_at_secs.is_some_and(|e| now_secs >= e);
    RecordVerdict {
        verified: true,
        expired,
        reason: None,
        revocation_status: Some(listing::revocation_status_word(verified.revocation_status)),
        record_id: Some(verified.record_id),
        issuer: Some(verified.issuer),
        issued_at_secs: Some(verified.issued_at_secs),
        expires_at_secs: verified.expires_at_secs,
        supersedes: verified.supersedes,
        payload: Some(payload),
    }
}

pub fn verify_quote(envelope: &str, now_secs: u64) -> RecordVerdict<QuotePayload> {
    let opts = record::VerifyOptions::new(now_secs).allowing_expired();
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
    let opts = record::VerifyOptions::new(now_secs).allowing_expired();
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
    match payload.role {
        Role::Consumer => {
            if verified.issuer != payload.consumer_did {
                return RecordVerdict::refused(
                    "consumer receipt issuer does not match consumer_did",
                );
            }
        }
        Role::Provider => {
            if verified.issuer != payload.provider_did {
                return RecordVerdict::refused(
                    "provider receipt issuer does not match provider_did",
                );
            }
        }
    }
    let expired = verified.expires_at_secs.is_some_and(|e| now_secs >= e);
    RecordVerdict {
        verified: true,
        expired,
        reason: None,
        revocation_status: Some(listing::revocation_status_word(verified.revocation_status)),
        record_id: Some(verified.record_id),
        issuer: Some(verified.issuer),
        issued_at_secs: Some(verified.issued_at_secs),
        expires_at_secs: verified.expires_at_secs,
        supersedes: verified.supersedes,
        payload: Some(payload),
    }
}

#[cfg(test)]
mod tests {
    use syneroym_identity::{Identity, substrate};
    use syneroym_signed_record::{Envelope, RecordDraft};

    use super::*;

    fn sample_terms() -> AgreedTerms {
        AgreedTerms {
            scope: "Install garden fence".to_string(),
            currency: "EUR".to_string(),
            amount_minor: 50000,
            tax_minor: 5000,
            fees_minor: 2000,
            payment_methods: vec!["card".to_string(), "cash".to_string()],
            payee: "Garden Services Ltd".to_string(),
            payment_timing: PaymentTiming::AfterWork,
            schedule: Some(TimeWindow { earliest_secs: 10000, latest_secs: 20000 }),
            location: QuoteLocation {
                where_: ServiceLocation::AtCustomer,
                area: None,
                address: Some("123 Flower St".to_string()),
            },
            cancellation_terms: "Cancel 24h prior for full refund".to_string(),
            refund_terms: "Full refund if unsatisfactory".to_string(),
            dispute_path: "Contact disputes@example.com".to_string(),
            quote_expires_at_secs: 15000,
        }
    }

    fn sample_request(issuer: &str) -> RequestPayload {
        let conversation = "conv_123456";
        let sequence = 1;
        let request_id = derive_request_id(conversation, issuer, sequence).unwrap();
        RequestPayload {
            request_id,
            conversation: conversation.to_string(),
            sequence,
            listing_id: Some("lst_abc123".to_string()),
            categories: vec!["gardening".to_string()],
            description: "Need fence installed".to_string(),
            area: None,
            window: Some(TimeWindow { earliest_secs: 10000, latest_secs: 20000 }),
            data_use_notice: DEFAULT_DATA_USE_NOTICE.to_string(),
        }
    }

    fn sample_quote(issuer: &str, consumer_did: &str) -> QuotePayload {
        let conversation = "conv_123456";
        let sequence = 1;
        let quote_id = derive_quote_id(conversation, issuer, sequence).unwrap();
        QuotePayload {
            quote_id,
            conversation: conversation.to_string(),
            sequence,
            request_record_id: "rec_req123".to_string(),
            listing_id: Some("lst_abc123".to_string()),
            consumer_did: consumer_did.to_string(),
            terms: sample_terms(),
        }
    }

    fn sign_envelope(
        key: &Identity,
        record_type: &str,
        version: u32,
        subject: &str,
        payload: serde_json::Value,
        expires_at_secs: Option<u64>,
        now_secs: u64,
    ) -> String {
        let issuer = substrate::derive_did_key(&key.public_key());
        let draft = RecordDraft {
            version,
            record_type: record_type.to_string(),
            subject: subject.to_string(),
            payload,
            expires_at_secs,
            supersedes: None,
        };
        let (mut env, bytes) = Envelope::unsigned(draft, issuer, None, now_secs).unwrap();
        let sig = z32::encode(&key.sign(&bytes).to_bytes());
        env.attach_signature(sig).unwrap();
        env.to_json().unwrap()
    }

    #[test]
    fn time_window_validate() {
        let valid = TimeWindow { earliest_secs: 100, latest_secs: 200 };
        assert!(valid.validate().is_ok());
        let equal = TimeWindow { earliest_secs: 100, latest_secs: 100 };
        assert!(equal.validate().is_ok());
        let invalid = TimeWindow { earliest_secs: 200, latest_secs: 100 };
        assert!(matches!(invalid.validate(), Err(TransactionError::InvalidTimeWindow { .. })));
    }

    #[test]
    fn quote_location_validate() {
        let at_cust = QuoteLocation {
            where_: ServiceLocation::AtCustomer,
            area: None,
            address: Some("123 Main St".to_string()),
        };
        assert!(at_cust.validate().is_ok());

        let remote_with_addr = QuoteLocation {
            where_: ServiceLocation::Remote,
            area: None,
            address: Some("123 Main St".to_string()),
        };
        assert_eq!(remote_with_addr.validate(), Err(TransactionError::AddressNotApplicable));

        let long_addr = QuoteLocation {
            where_: ServiceLocation::AtCustomer,
            area: None,
            address: Some("a".repeat(MAX_ADDRESS_LEN + 1)),
        };
        assert_eq!(long_addr.validate(), Err(TransactionError::AddressTooLong));
    }

    #[test]
    fn agreed_terms_validate_rules() {
        let mut t = sample_terms();
        assert!(t.validate().is_ok());

        // Scope empty / too long
        t.scope = "   ".to_string();
        assert_eq!(t.validate(), Err(TransactionError::ScopeEmpty));
        t.scope = "a".repeat(MAX_SCOPE_LEN + 1);
        assert_eq!(t.validate(), Err(TransactionError::ScopeTooLong));
        t.scope = "Valid scope".to_string();

        // Currency unknown
        t.currency = "XYZ".to_string();
        assert!(matches!(t.validate(), Err(TransactionError::CurrencyUnknown(_))));
        t.currency = "EUR".to_string();

        // Negative amounts
        t.amount_minor = -1;
        assert!(matches!(t.validate(), Err(TransactionError::NegativeAmount(-1))));
        t.amount_minor = 50000;

        t.tax_minor = -1;
        assert!(matches!(t.validate(), Err(TransactionError::NegativeTax(-1))));
        t.tax_minor = 5000;

        t.fees_minor = -1;
        assert!(matches!(t.validate(), Err(TransactionError::NegativeFees(-1))));
        t.fees_minor = 2000;

        // Breakdown exceeds total
        t.tax_minor = 40000;
        t.fees_minor = 20000; // 40000 + 20000 = 60000 > 50000
        assert!(matches!(t.validate(), Err(TransactionError::BreakdownExceedsTotal { .. })));
        t.tax_minor = 5000;
        t.fees_minor = 2000;

        // Payee
        t.payee = "".to_string();
        assert_eq!(t.validate(), Err(TransactionError::PayeeEmpty));
        t.payee = "p".repeat(MAX_PAYEE_LEN + 1);
        assert_eq!(t.validate(), Err(TransactionError::PayeeTooLong));
        t.payee = "Valid Payee".to_string();

        // Payment methods
        t.payment_methods = (0..MAX_PAYMENT_METHODS + 1).map(|i| format!("m{i}")).collect();
        assert_eq!(t.validate(), Err(TransactionError::TooManyPaymentMethods));
        t.payment_methods = vec!["x".repeat(MAX_PAYMENT_METHOD_LEN + 1)];
        assert_eq!(t.validate(), Err(TransactionError::PaymentMethodTooLong));
        t.payment_methods = vec!["card".to_string()];

        // Cancellation terms
        t.cancellation_terms = "".to_string();
        assert_eq!(t.validate(), Err(TransactionError::CancellationTermsEmpty));
        t.cancellation_terms = "x".repeat(MAX_TERMS_TEXT_LEN + 1);
        assert_eq!(t.validate(), Err(TransactionError::CancellationTermsTooLong));
        t.cancellation_terms = "Valid cancel".to_string();

        // Refund terms
        t.refund_terms = "".to_string();
        assert_eq!(t.validate(), Err(TransactionError::RefundTermsEmpty));
        t.refund_terms = "x".repeat(MAX_TERMS_TEXT_LEN + 1);
        assert_eq!(t.validate(), Err(TransactionError::RefundTermsTooLong));
        t.refund_terms = "Valid refund".to_string();

        // Dispute path
        t.dispute_path = "".to_string();
        assert_eq!(t.validate(), Err(TransactionError::DisputePathEmpty));
        t.dispute_path = "x".repeat(MAX_TERMS_TEXT_LEN + 1);
        assert_eq!(t.validate(), Err(TransactionError::DisputePathTooLong));
        t.dispute_path = "Valid dispute".to_string();

        // Quote expiry zero
        t.quote_expires_at_secs = 0;
        assert_eq!(t.validate(), Err(TransactionError::QuoteExpiryZero));
        t.quote_expires_at_secs = 1000;
        assert!(t.validate().is_ok());
    }

    #[test]
    fn request_payload_validate_rules() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let mut r = sample_request(&issuer);
        assert!(r.validate().is_ok());

        r.request_id = "".to_string();
        assert_eq!(r.validate(), Err(TransactionError::RequestIdEmpty));
        r.request_id = "req_123".to_string();

        r.conversation = "".to_string();
        assert_eq!(r.validate(), Err(TransactionError::ConversationEmpty));
        r.conversation = "conv_123".to_string();

        r.sequence = 0;
        assert_eq!(r.validate(), Err(TransactionError::InvalidSequence(0)));
        r.sequence = 1;

        r.description = "   ".to_string();
        assert_eq!(r.validate(), Err(TransactionError::DescriptionEmpty));
        r.description = "d".repeat(MAX_DESCRIPTION_LEN + 1);
        assert_eq!(r.validate(), Err(TransactionError::DescriptionTooLong));
        r.description = "Valid description".to_string();

        r.categories = (0..MAX_CATEGORIES + 1).map(|i| format!("c{i}")).collect();
        assert_eq!(r.validate(), Err(TransactionError::TooManyCategories));
        r.categories = vec!["UPPERCASE".to_string()];
        assert!(matches!(r.validate(), Err(TransactionError::InvalidCategory(_))));
        r.categories = vec!["valid-cat".to_string()];

        r.data_use_notice = "".to_string();
        assert_eq!(r.validate(), Err(TransactionError::DataUseNoticeEmpty));
        r.data_use_notice = "n".repeat(MAX_NOTICE_LEN + 1);
        assert_eq!(r.validate(), Err(TransactionError::DataUseNoticeTooLong));
        r.data_use_notice = DEFAULT_DATA_USE_NOTICE.to_string();
        assert!(r.validate().is_ok());
    }

    #[test]
    fn quote_payload_validate_rules() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let consumer_key = Identity::generate().unwrap();
        let consumer_did = substrate::derive_did_key(&consumer_key.public_key());

        let mut q = sample_quote(&issuer, &consumer_did);
        assert!(q.validate().is_ok());

        q.quote_id = "".to_string();
        assert_eq!(q.validate(), Err(TransactionError::QuoteIdEmpty));
        q.quote_id = "quo_123".to_string();

        q.conversation = "".to_string();
        assert_eq!(q.validate(), Err(TransactionError::ConversationEmpty));
        q.conversation = "conv_123".to_string();

        q.sequence = 0;
        assert_eq!(q.validate(), Err(TransactionError::InvalidSequence(0)));
        q.sequence = 1;

        q.request_record_id = "bad_prefix".to_string();
        assert!(matches!(q.validate(), Err(TransactionError::InvalidRecordId(_))));
        q.request_record_id = "rec_req123".to_string();

        q.consumer_did = "not-a-did".to_string();
        assert!(matches!(q.validate(), Err(TransactionError::InvalidDid(_))));
        q.consumer_did = consumer_did;
        assert!(q.validate().is_ok());
    }

    #[test]
    fn agreement_receipt_payload_validate_rules() {
        let consumer_key = Identity::generate().unwrap();
        let consumer_did = substrate::derive_did_key(&consumer_key.public_key());
        let provider_key = Identity::generate().unwrap();
        let provider_did = substrate::derive_did_key(&provider_key.public_key());

        let mut a = AgreementReceiptPayload {
            quote_record_id: "rec_quote123".to_string(),
            consumer_did: consumer_did.clone(),
            provider_did: provider_did.clone(),
            role: Role::Consumer,
            terms: sample_terms(),
        };
        assert!(a.validate().is_ok());

        a.quote_record_id = "bad_prefix".to_string();
        assert!(matches!(a.validate(), Err(TransactionError::InvalidRecordId(_))));
        a.quote_record_id = "rec_quote123".to_string();

        a.consumer_did = "invalid".to_string();
        assert!(matches!(a.validate(), Err(TransactionError::InvalidDid(_))));
        a.consumer_did = consumer_did;

        a.provider_did = "invalid".to_string();
        assert!(matches!(a.validate(), Err(TransactionError::InvalidDid(_))));
        a.provider_did = provider_did;
        assert!(a.validate().is_ok());
    }

    #[test]
    fn derive_request_id_and_quote_id_are_stable_and_differ_on_any_input_change() {
        let id1 = derive_request_id("c1", "did:key:1", 1).unwrap();
        let id2 = derive_request_id("c1", "did:key:1", 1).unwrap();
        assert_eq!(id1, id2);
        assert!(id1.starts_with(REQUEST_ID_PREFIX));

        // Changing conversation
        assert_ne!(id1, derive_request_id("c2", "did:key:1", 1).unwrap());
        // Changing issuer
        assert_ne!(id1, derive_request_id("c1", "did:key:2", 1).unwrap());
        // Changing sequence
        assert_ne!(id1, derive_request_id("c1", "did:key:1", 2).unwrap());

        // Quote id
        let q1 = derive_quote_id("c1", "did:key:1", 1).unwrap();
        let q2 = derive_quote_id("c1", "did:key:1", 1).unwrap();
        assert_eq!(q1, q2);
        assert!(q1.starts_with(QUOTE_ID_PREFIX));
        assert_ne!(id1, q1);
        assert_ne!(q1, derive_quote_id("c2", "did:key:1", 1).unwrap());
    }

    #[test]
    fn verify_quote_refuses_a_payload_whose_quote_id_was_derived_from_a_different_issuer() {
        let real_signer = Identity::generate().unwrap();
        let wrong_key = Identity::generate().unwrap();
        let wrong_issuer = substrate::derive_did_key(&wrong_key.public_key());

        let consumer_key = Identity::generate().unwrap();
        let consumer_did = substrate::derive_did_key(&consumer_key.public_key());

        // Derive quote_id using wrong_issuer
        let quote = sample_quote(&wrong_issuer, &consumer_did);
        let env = sign_envelope(
            &real_signer,
            RECORD_QUOTE,
            QUOTE_VERSION,
            &quote.quote_id,
            serde_json::to_value(&quote).unwrap(),
            Some(quote.terms.quote_expires_at_secs),
            1000,
        );

        let verdict = verify_quote(&env, 1000);
        assert!(!verdict.verified);
        assert_eq!(
            verdict.reason.as_deref(),
            Some("quote_id is not derivable from the signature's own issuer")
        );
    }

    #[test]
    fn verify_agreement_receipt_refuses_a_half_whose_role_does_not_match_its_issuer() {
        let consumer_key = Identity::generate().unwrap();
        let consumer_did = substrate::derive_did_key(&consumer_key.public_key());
        let provider_key = Identity::generate().unwrap();
        let provider_did = substrate::derive_did_key(&provider_key.public_key());

        // A receipt saying role is Consumer, but signed by provider_key!
        let payload = AgreementReceiptPayload {
            quote_record_id: "rec_quote123".to_string(),
            consumer_did: consumer_did.clone(),
            provider_did: provider_did.clone(),
            role: Role::Consumer,
            terms: sample_terms(),
        };
        let env_provider_signing_consumer_half = sign_envelope(
            &provider_key,
            RECORD_AGREEMENT_RECEIPT,
            AGREEMENT_RECEIPT_VERSION,
            &payload.quote_record_id,
            serde_json::to_value(&payload).unwrap(),
            None,
            1000,
        );
        let verdict = verify_agreement_receipt(&env_provider_signing_consumer_half, 1000);
        assert!(!verdict.verified);
        assert_eq!(
            verdict.reason.as_deref(),
            Some("consumer receipt issuer does not match consumer_did")
        );

        // Subject mismatch
        let env_subject_mismatch = sign_envelope(
            &consumer_key,
            RECORD_AGREEMENT_RECEIPT,
            AGREEMENT_RECEIPT_VERSION,
            "rec_wrong_subject",
            serde_json::to_value(&payload).unwrap(),
            None,
            1000,
        );
        let verdict_subj = verify_agreement_receipt(&env_subject_mismatch, 1000);
        assert!(!verdict_subj.verified);
        assert_eq!(
            verdict_subj.reason.as_deref(),
            Some("envelope subject does not match quote_record_id")
        );

        // Valid consumer half
        let env_valid = sign_envelope(
            &consumer_key,
            RECORD_AGREEMENT_RECEIPT,
            AGREEMENT_RECEIPT_VERSION,
            &payload.quote_record_id,
            serde_json::to_value(&payload).unwrap(),
            None,
            1000,
        );
        let verdict_ok = verify_agreement_receipt(&env_valid, 1000);
        assert!(verdict_ok.verified);
    }

    #[test]
    fn halves_agree_table_test() {
        let consumer_did = "did:key:zConsumer".to_string();
        let provider_did = "did:key:zProvider".to_string();
        let terms = sample_terms();

        let base_consumer = AgreementReceiptPayload {
            quote_record_id: "rec_quo123".to_string(),
            consumer_did: consumer_did.clone(),
            provider_did: provider_did.clone(),
            role: Role::Consumer,
            terms: terms.clone(),
        };
        let base_provider = AgreementReceiptPayload {
            quote_record_id: "rec_quo123".to_string(),
            consumer_did: consumer_did.clone(),
            provider_did: provider_did.clone(),
            role: Role::Provider,
            terms: terms.clone(),
        };

        // Identical except role -> true
        assert!(halves_agree(&base_consumer, &base_provider));
        assert!(halves_agree(&base_provider, &base_consumer));

        // Same role -> false
        assert!(!halves_agree(&base_consumer, &base_consumer));
        assert!(!halves_agree(&base_provider, &base_provider));

        // Differ in quote_record_id -> false
        let mut diff_quote = base_provider.clone();
        diff_quote.quote_record_id = "rec_diff".to_string();
        assert!(!halves_agree(&base_consumer, &diff_quote));

        // Differ in consumer_did -> false
        let mut diff_cdid = base_provider.clone();
        diff_cdid.consumer_did = "did:key:zOther".to_string();
        assert!(!halves_agree(&base_consumer, &diff_cdid));

        // Differ in provider_did -> false
        let mut diff_pdid = base_provider.clone();
        diff_pdid.provider_did = "did:key:zOther".to_string();
        assert!(!halves_agree(&base_consumer, &diff_pdid));

        // Differ in terms (e.g. amount) -> false
        let mut diff_terms = base_provider.clone();
        diff_terms.terms.amount_minor = 99999;
        assert!(!halves_agree(&base_consumer, &diff_terms));
    }

    #[test]
    fn expired_quote_envelope_verifies_with_expired_flag() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let consumer_key = Identity::generate().unwrap();
        let consumer_did = substrate::derive_did_key(&consumer_key.public_key());

        let quote = sample_quote(&issuer, &consumer_did);
        let quote_expiry = 1500;
        let env = sign_envelope(
            &key,
            RECORD_QUOTE,
            QUOTE_VERSION,
            &quote.quote_id,
            serde_json::to_value(&quote).unwrap(),
            Some(quote_expiry),
            1000,
        );

        // Before expiry: verified = true, expired = false
        let v_before = verify_quote(&env, 1400);
        assert!(v_before.verified);
        assert!(!v_before.expired);
        assert!(v_before.payload.is_some());

        // Exactly at or after expiry: verified = true, expired = true
        let v_after = verify_quote(&env, 2000);
        assert!(v_after.verified);
        assert!(v_after.expired);
        assert!(v_after.payload.is_some());
        assert_eq!(v_after.payload.unwrap().quote_id, quote.quote_id);
    }

    #[test]
    fn agreed_terms_validate_refuses_xyz_as_currency() {
        let mut t = sample_terms();
        t.currency = "XYZ".to_string();
        assert!(matches!(t.validate(), Err(TransactionError::CurrencyUnknown(c)) if c == "XYZ"));
    }

    #[test]
    fn pair_state_transitions() {
        let half_c = ReceiptHalf {
            envelope: "{}".to_string(),
            record_id: "rec_c".to_string(),
            issuer: "did:key:c".to_string(),
            issued_at_secs: 1000,
        };
        let half_p = ReceiptHalf {
            envelope: "{}".to_string(),
            record_id: "rec_p".to_string(),
            issuer: "did:key:p".to_string(),
            issued_at_secs: 1000,
        };

        assert_eq!(pair_state(None, None), PairState::None);
        assert_eq!(pair_state(Some(&half_c), None), PairState::Half { role: Role::Consumer });
        assert_eq!(pair_state(None, Some(&half_p)), PairState::Half { role: Role::Provider });
        assert_eq!(pair_state(Some(&half_c), Some(&half_p)), PairState::Complete);
    }

    #[test]
    fn verify_request_happy_and_mismatch() {
        let key = Identity::generate().unwrap();
        let issuer = substrate::derive_did_key(&key.public_key());
        let req = sample_request(&issuer);

        let env = sign_envelope(
            &key,
            RECORD_REQUEST,
            REQUEST_VERSION,
            &req.request_id,
            serde_json::to_value(&req).unwrap(),
            None,
            1000,
        );

        let v = verify_request(&env, 1000);
        assert!(v.verified);
        assert!(!v.expired);
        assert_eq!(v.payload.as_ref().unwrap().request_id, req.request_id);

        // Sign with mismatching issuer
        let wrong_key = Identity::generate().unwrap();
        let env_mismatch = sign_envelope(
            &wrong_key,
            RECORD_REQUEST,
            REQUEST_VERSION,
            &req.request_id,
            serde_json::to_value(&req).unwrap(),
            None,
            1000,
        );
        let v_bad = verify_request(&env_mismatch, 1000);
        assert!(!v_bad.verified);
        assert_eq!(
            v_bad.reason.as_deref(),
            Some("request_id is not derivable from the signature's own issuer")
        );
    }
}

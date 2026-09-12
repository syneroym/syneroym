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
use syneroym_signed_record::{EnvelopeError, VerifyOptions, content_digest};
use thiserror::Error;

pub use crate::record::{RECORD_AGREEMENT_RECEIPT, RECORD_QUOTE, RECORD_REQUEST};
use crate::{
    area::{Area, AreaError},
    listing::{self, ServiceLocation},
    money, person, record,
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

    #[error("consumer and provider cannot be the same DID")]
    SamePartyDid,

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
        if self.consumer_did == self.provider_did {
            return Err(TransactionError::SamePartyDid);
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
/// on: every other field matches and they come from opposite roles.
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
    let opts = VerifyOptions::new(now_secs).allowing_expired();
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
    if verified.expires_at_secs.is_some() {
        return RecordVerdict::refused("a request may not declare an expiry");
    }
    RecordVerdict {
        verified: true,
        expired: false,
        reason: None,
        revocation_status: Some(listing::revocation_status_word(verified.revocation_status)),
        record_id: Some(verified.record_id),
        issuer: Some(verified.issuer),
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
    if payload.consumer_did == verified.issuer {
        return RecordVerdict::refused("consumer_did cannot be the quote issuer");
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
    RecordVerdict {
        verified: true,
        expired: false,
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
mod tests;

//! The provider's offer: one signed record type, a small required core
//! plus seven optional named blocks. An edit is a new envelope carrying
//! `supersedes`; nothing is rewritten.
//!
//! Every number here is an integer. A signed payload may hold no number
//! that is not an integer -- the canonical encoding is only reproducible
//! for integers -- so money is minor units with an explicit currency and
//! geography is micro-degrees.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_signed_record::{EnvelopeError, content_digest};

use crate::{
    area::{self, Area, AreaError},
    money,
};

pub const LISTING_VERSION: u32 = 1;
pub const MAX_TITLE_LEN: usize = 160;
pub const MAX_SUMMARY_LEN: usize = 2048;
pub const MAX_CATEGORIES: usize = 8;
pub const MAX_SLUG_LEN: usize = 64;
pub const MAX_CATEGORY_LEN: usize = 64;
pub use crate::area::MAX_AREAS;

/// N-2: fields that were unbounded while their neighbours were capped.
/// From this slice a stranger's bytes sit on a SynOrg owner's disk, so
/// every field a payload carries gets a bound.
pub const MAX_PAYEE_LEN: usize = 256;
pub const MAX_PAYMENT_METHODS: usize = 16;
pub const MAX_PAYMENT_METHOD_LEN: usize = 32;
pub const MAX_UNIT_LEN: usize = 32;
pub const MAX_SKU_LEN: usize = 64;
pub const MAX_SERVICE_LIST_ITEMS: usize = 32;
pub const MAX_SERVICE_LIST_ITEM_LEN: usize = 128;

/// The id prefix, so a listing id can never be mistaken for a record id or
/// a report id.
const LISTING_ID_PREFIX: &str = "lst_";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ListingStatus {
    Draft,
    Active,
    Withdrawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BookingMode {
    Slots,
    Order,
    Enquiry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingTerms {
    pub mode: BookingMode,
    pub lead_time_secs: u64,
    pub cancellation_window_secs: u64,
    pub max_per_booking: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaymentModel {
    Fixed,
    PerHour,
    PerUnit,
    QuoteOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentTerms {
    /// ISO-4217, three uppercase letters.
    pub currency: String,
    pub model: PaymentModel,
    /// Absent for `quote-only`. Minor units, never a decimal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_minor: Option<i64>,
    pub tax_included: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fees_minor: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
    /// A free string here; it becomes binding only inside an
    /// `agreement-receipt`. The UI must never present it as agreed terms.
    pub payee: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProductCondition {
    New,
    Used,
    Refurbished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductDetail {
    pub unit: String,
    pub pack_size: u32,
    pub condition: ProductCondition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sku: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceDetail {
    pub duration_secs: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excludes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prerequisites: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceLocation {
    AtProvider,
    AtCustomer,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AddressDisclosure {
    /// Carries the disclosure rule as data rather than as UI convention:
    /// the exact address is revealed only once an agreement needs it.
    OnAgreement,
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocationTerms {
    #[serde(rename = "where")]
    pub where_: ServiceLocation,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_area: Vec<Area>,
    pub address_disclosure: AddressDisclosure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenTo {
    Anyone,
    Members,
    Referral,
    ExistingCustomers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipTerms {
    pub open_to: OpenTo,
    /// The group's DID, required when `open_to == members`. Stating the
    /// rule, not enforcing it -- enforcement needs the membership
    /// credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_of: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRecordTerms {
    pub issues_fulfilment_receipt: bool,
    /// Seconds; `0` means no stated warranty.
    pub warranty_secs: u64,
    /// Seconds the provider states it retains the record for.
    pub retention_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingPayload {
    /// Stable across every version of this listing, content-derived from
    /// `(issuer, slug)`. A new version supersedes the previous envelope
    /// and keeps this value.
    pub listing_id: String,
    pub slug: String,
    pub title: String,
    pub summary: String,
    /// Free-form category tokens, lowercase `[a-z0-9-]`. What a search
    /// filters on; the vocabulary is the group's, not the product's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// Under the provider's own signature, so a stranger who verifies this
    /// listing can start a conversation with no directory and no prior
    /// contact entry.
    pub conversation_address: String,
    pub status: ListingStatus,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub booking: Option<BookingTerms>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<PaymentTerms>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<ServiceDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<LocationTerms>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<RelationshipTerms>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_record: Option<ServiceRecordTerms>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ListingError {
    #[error("title is empty")]
    TitleEmpty,
    #[error("title is longer than {MAX_TITLE_LEN} bytes")]
    TitleTooLong,
    #[error("summary is longer than {MAX_SUMMARY_LEN} bytes")]
    SummaryTooLong,
    #[error("slug '{0}' is not 1..={MAX_SLUG_LEN} chars of [a-z0-9-]")]
    SlugShape(String),
    #[error("more than {MAX_CATEGORIES} categories")]
    TooManyCategories,
    #[error("category '{0}' is not 1..={MAX_CATEGORY_LEN} chars of [a-z0-9-]")]
    CategoryShape(String),
    #[error("conversation_address is empty")]
    ConversationAddressEmpty,
    #[error("listing_id is empty")]
    ListingIdEmpty,
    #[error("currency '{0}' is not a currency code this build knows")]
    CurrencyUnknown(String),
    #[error("payment terms are required unless booking.mode is enquiry")]
    PaymentRequired,
    #[error("amount_minor is required for a non-quote-only payment model")]
    AmountMinorRequired,
    #[error("amount_minor must be absent for a quote-only payment model")]
    AmountMinorForbidden,
    #[error("more than {MAX_AREAS} service areas")]
    TooManyAreas,
    #[error("service area: {0}")]
    Area(#[from] AreaError),
    #[error("member_of is required when open_to is members")]
    MemberOfRequired,
    #[error("member_of '{0}' is not a did:key")]
    MemberOfNotDid(String),
    #[error("payee is longer than {MAX_PAYEE_LEN} bytes")]
    PayeeTooLong,
    #[error("more than {MAX_PAYMENT_METHODS} payment methods")]
    TooManyPaymentMethods,
    #[error("a payment method name is longer than {MAX_PAYMENT_METHOD_LEN} bytes")]
    PaymentMethodTooLong,
    #[error("unit is longer than {MAX_UNIT_LEN} bytes")]
    UnitTooLong,
    #[error("sku is longer than {MAX_SKU_LEN} bytes")]
    SkuTooLong,
    #[error("{0} has more than {MAX_SERVICE_LIST_ITEMS} entries")]
    ServiceListTooLong(&'static str),
    #[error("an entry in {0} is longer than {MAX_SERVICE_LIST_ITEM_LEN} bytes")]
    ServiceListItemTooLong(&'static str),
}

fn is_slug_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

pub(crate) fn valid_token(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.chars().all(is_slug_char)
}

impl ListingPayload {
    pub fn validate(&self) -> Result<(), ListingError> {
        if self.listing_id.is_empty() {
            return Err(ListingError::ListingIdEmpty);
        }
        if self.title.trim().is_empty() {
            return Err(ListingError::TitleEmpty);
        }
        if self.title.len() > MAX_TITLE_LEN {
            return Err(ListingError::TitleTooLong);
        }
        if self.summary.len() > MAX_SUMMARY_LEN {
            return Err(ListingError::SummaryTooLong);
        }
        if !valid_token(&self.slug, MAX_SLUG_LEN) {
            return Err(ListingError::SlugShape(self.slug.clone()));
        }
        if self.conversation_address.trim().is_empty() {
            return Err(ListingError::ConversationAddressEmpty);
        }
        if self.categories.len() > MAX_CATEGORIES {
            return Err(ListingError::TooManyCategories);
        }
        for c in &self.categories {
            if !valid_token(c, MAX_CATEGORY_LEN) {
                return Err(ListingError::CategoryShape(c.clone()));
            }
        }

        let booking_is_enquiry =
            matches!(self.booking.as_ref().map(|b| b.mode), Some(BookingMode::Enquiry));
        match &self.payment {
            None if !booking_is_enquiry => return Err(ListingError::PaymentRequired),
            None => {}
            Some(p) => {
                if money::currency_minor_exponent(&p.currency).is_none() {
                    return Err(ListingError::CurrencyUnknown(p.currency.clone()));
                }
                match p.model {
                    PaymentModel::QuoteOnly if p.amount_minor.is_some() => {
                        return Err(ListingError::AmountMinorForbidden);
                    }
                    PaymentModel::QuoteOnly => {}
                    _ if p.amount_minor.is_none() => {
                        return Err(ListingError::AmountMinorRequired);
                    }
                    _ => {}
                }
                if p.payee.len() > MAX_PAYEE_LEN {
                    return Err(ListingError::PayeeTooLong);
                }
                if p.methods.len() > MAX_PAYMENT_METHODS {
                    return Err(ListingError::TooManyPaymentMethods);
                }
                for m in &p.methods {
                    if m.len() > MAX_PAYMENT_METHOD_LEN {
                        return Err(ListingError::PaymentMethodTooLong);
                    }
                }
            }
        }

        if let Some(prod) = &self.product {
            if prod.unit.len() > MAX_UNIT_LEN {
                return Err(ListingError::UnitTooLong);
            }
            if prod.sku.as_ref().is_some_and(|s| s.len() > MAX_SKU_LEN) {
                return Err(ListingError::SkuTooLong);
            }
        }

        if let Some(svc) = &self.service {
            for (name, list) in [
                ("includes", &svc.includes),
                ("excludes", &svc.excludes),
                ("prerequisites", &svc.prerequisites),
            ] {
                if list.len() > MAX_SERVICE_LIST_ITEMS {
                    return Err(ListingError::ServiceListTooLong(name));
                }
                for item in list.iter() {
                    if item.len() > MAX_SERVICE_LIST_ITEM_LEN {
                        return Err(ListingError::ServiceListItemTooLong(name));
                    }
                }
            }
        }

        if let Some(loc) = &self.location {
            if loc.service_area.len() > MAX_AREAS {
                return Err(ListingError::TooManyAreas);
            }
            for a in &loc.service_area {
                a.validate()?;
            }
        }

        if let Some(rel) = &self.relationship {
            match (&rel.open_to, &rel.member_of) {
                (OpenTo::Members, None) => return Err(ListingError::MemberOfRequired),
                (_, Some(did)) if !crate::person::is_did_key(did) => {
                    return Err(ListingError::MemberOfNotDid(did.clone()));
                }
                _ => {}
            }
        }

        Ok(())
    }
}

/// `content_digest("lst_", {"issuer": issuer, "slug": slug})`. A clock-free
/// identifier: the same content produces the same id on every call and on
/// both builds, and a second `listing.create` for the same slug is the
/// edit path, not a duplicate.
pub fn derive_listing_id(issuer: &str, slug: &str) -> Result<String, EnvelopeError> {
    content_digest(LISTING_ID_PREFIX, &json!({ "issuer": issuer, "slug": slug }))
}

/// Lowercases, keeps `[a-z0-9]`, collapses runs of anything else to `-`,
/// trims leading/trailing `-`, and truncates to [`MAX_SLUG_LEN`]. `None`
/// when nothing usable is left -- the caller answers `-32602` rather than
/// minting an identifier nobody can type.
#[must_use]
pub fn slug_from_title(title: &str) -> Option<String> {
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in title.chars() {
        if out.len() >= MAX_SLUG_LEN {
            break;
        }
        let lc = ch.to_ascii_lowercase();
        if lc.is_ascii_lowercase() || lc.is_ascii_digit() {
            if pending_dash && !out.is_empty() && out.len() < MAX_SLUG_LEN {
                out.push('-');
            }
            pending_dash = false;
            if out.len() < MAX_SLUG_LEN {
                out.push(lc);
            }
        } else {
            pending_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Re-export so a caller writing the box does not have to reach into
/// `area` for it.
pub use area::bounding_box;

/// One verification body, shared by `catalog.listing.verify` and the
/// directory client -- so a stranger's listing is never verified twice by
/// two copies of the same logic that could quietly disagree. Pure: no
/// host, no storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingVerdict {
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// `"good"` or `"unknown"` -- `crate::record::RevocationStatus` carries
    /// no serde impl of its own, and the wire shape is the word, not the
    /// enum. `Some` only when `verified`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listing_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ListingStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_at_secs: Option<u64>,
    /// The envelope's own `supersedes` -- the prior version's `record_id`,
    /// when this one is an edit. `None` for a listing's first version, or
    /// when `verified` is `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<ListingPayload>,
}

impl ListingVerdict {
    fn refused(reason: impl Into<String>) -> Self {
        Self {
            verified: false,
            reason: Some(reason.into()),
            revocation_status: None,
            listing_id: None,
            record_id: None,
            issuer: None,
            conversation_address: None,
            status: None,
            issued_at_secs: None,
            supersedes: None,
            payload: None,
        }
    }
}

/// The word a stranger's evidence renders as. `crate::record::
/// RevocationStatus` carries no `Display`; this is the one place that
/// spells it out, so every caller renders the same word.
#[must_use]
pub fn revocation_status_word(status: crate::record::RevocationStatus) -> String {
    match status {
        crate::record::RevocationStatus::Good => "good",
        crate::record::RevocationStatus::Unknown => "unknown",
    }
    .to_string()
}

/// Checks a signed `listing` envelope on the caller's own node: signature,
/// record shape, `listing_id` derivability, and payload validity. Never
/// consults a directory's own claim about any of this -- the consumer's
/// own node is the only party whose verdict counts.
#[must_use]
pub fn verify_envelope(envelope: &str, now_secs: u64) -> ListingVerdict {
    let verified =
        match crate::record::verify_json(envelope, &crate::record::VerifyOptions::new(now_secs)) {
            Ok(v) => v,
            Err(e) => return ListingVerdict::refused(e.to_string()),
        };
    if verified.record_type != crate::record::RECORD_LISTING || verified.version != LISTING_VERSION
    {
        return ListingVerdict::refused("not a listing record this build understands");
    }
    let payload: ListingPayload = match serde_json::from_value(verified.payload.clone()) {
        Ok(p) => p,
        Err(e) => return ListingVerdict::refused(format!("payload: {e}")),
    };
    if let Err(e) = payload.validate() {
        return ListingVerdict::refused(e.to_string());
    }
    let expected_id = match derive_listing_id(&verified.issuer, &payload.slug) {
        Ok(id) => id,
        Err(e) => return ListingVerdict::refused(e.to_string()),
    };
    if payload.listing_id != expected_id {
        return ListingVerdict::refused(
            "listing_id is not derivable from the signature's own issuer",
        );
    }
    ListingVerdict {
        verified: true,
        reason: None,
        revocation_status: Some(revocation_status_word(verified.revocation_status)),
        listing_id: Some(payload.listing_id.clone()),
        record_id: Some(verified.record_id.clone()),
        issuer: Some(verified.issuer.clone()),
        conversation_address: Some(payload.conversation_address.clone()),
        status: Some(payload.status),
        issued_at_secs: Some(verified.issued_at_secs),
        supersedes: verified.supersedes.clone(),
        payload: Some(payload),
    }
}

#[cfg(test)]
mod tests;

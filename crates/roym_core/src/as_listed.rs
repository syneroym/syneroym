//! "Agree as listed": the enquiry's consent to one offering version, and the
//! one pure function that turns a listing into the terms of a quote.
//!
//! Both nodes call [`terms_from_listing`] on their own verified payload, so
//! the owner's quote and the counterparty's check cannot drift apart. A
//! refusal names the first thing the listing lacks, so the code a person
//! sees is stable.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    deal::{
        Commitment, CommitmentKind, DealTerms, Details, LocalText, ResourceKind, TextTranslation,
        commitment_id,
    },
    disclosure::{MAX_DETAILS_DISCLOSURE_BYTES, MAX_RECORD_DISCLOSURE_BYTES},
    extension::{self, Manifest, SCHEDULING_EXTENSION, SCHEDULING_SLOT_SCHEMA},
    listing::{
        self, ListingDealTerms, ListingPayload, ListingStatus, PaymentModel, PaymentTerms,
        PaymentTiming, ServiceLocation,
    },
    money,
    transaction::{DealRole, QuoteLocation, TimeWindow, TransactionError},
};

#[cfg(test)]
mod tests;

/// How long a tap may hold. Seven days: a consent older than a week cannot
/// rest on terms the person still remembers reading.
pub const MAX_CONSENT_SECS: u64 = 7 * 86_400;

/// The counterparty's consent to one offering version, carried in the
/// enquiry under their own signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsListed {
    /// The offering version the person agreed to.
    pub record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<SlotChoice>,
    /// Reserved. Always absent in this build; [`AsListed::validate`] refuses
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// The consent holds until then.
    pub valid_until_secs: u64,
}

/// The slot the person picked, with its window. The window travels with the
/// id because a slot id is a digest, so the time cannot be read back from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotChoice {
    pub slot_id: String,
    pub window: TimeWindow,
}

impl AsListed {
    /// The checks that need no clock. `listing_id` is the enquiry's, which
    /// must be set for an as-listed enquiry.
    pub fn validate(&self, listing_id: Option<&str>) -> Result<(), TransactionError> {
        let Some(listing_id) = listing_id else {
            return Err(TransactionError::AsListedNeedsListing);
        };
        if self.variant.is_some() {
            return Err(TransactionError::VariantNotSupported);
        }
        if let Some(slot) = &self.slot {
            slot.window.validate()?;
            let expected = listing::derive_slot_id(
                listing_id,
                slot.window.earliest_secs,
                slot.window.latest_secs,
            )
            .map_err(|_| TransactionError::SlotDiffers)?;
            if slot.slot_id != expected {
                return Err(TransactionError::SlotDiffers);
            }
        }
        Ok(())
    }

    /// The consent must last at least one second past the signature and no
    /// more than [`MAX_CONSENT_SECS`].
    pub fn validate_consent(&self, issued_at_secs: u64) -> Result<(), TransactionError> {
        if self.valid_until_secs <= issued_at_secs
            || self.valid_until_secs > issued_at_secs.saturating_add(MAX_CONSENT_SECS)
        {
            return Err(TransactionError::ConsentWindowInvalid {
                issued_at_secs,
                valid_until_secs: self.valid_until_secs,
            });
        }
        Ok(())
    }
}

/// Why a listing cannot become the terms of a quote without a person filling
/// the gaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotComplete {
    PriceNotFixed,
    TaxNotIncluded,
    AtReceiver,
    SlotRequired,
    SlotNotOffered,
    NoDealTerms,
    /// The listing names an extension this build does not know, so it cannot
    /// say what the owner's commitment would carry.
    ExtensionUnknown,
    ListingNotActive,
    QuoteTooLarge,
    /// The build mapped a field wrongly. It carries the `DealError` code, so a
    /// person sees a normal refusal instead of a quote that `quote.set` would
    /// refuse.
    Invalid(&'static str),
}

impl NotComplete {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::PriceNotFixed => "price-not-fixed",
            Self::TaxNotIncluded => "tax-not-included",
            Self::AtReceiver => "at-receiver",
            Self::SlotRequired => "slot-required",
            Self::SlotNotOffered => "slot-not-offered",
            Self::NoDealTerms => "no-deal-terms",
            Self::ExtensionUnknown => "extension-unknown",
            Self::ListingNotActive => "listing-not-active",
            Self::QuoteTooLarge => "quote-too-large",
            Self::Invalid(code) => code,
        }
    }
}

/// The listing-only part: what a directory can decide without a slot or a
/// clock. It runs the same first refusals as [`terms_from_listing`].
pub fn as_listed_ready(listing: &ListingPayload) -> Result<(), NotComplete> {
    ready(listing).map(|_| ())
}

/// The refusals that do not depend on the tap's slot.
fn ready(listing: &ListingPayload) -> Result<(&PaymentTerms, &ListingDealTerms), NotComplete> {
    if listing.status != ListingStatus::Active {
        return Err(NotComplete::ListingNotActive);
    }
    let payment = fixed_payment(listing)?;
    if !payment.tax_included {
        return Err(NotComplete::TaxNotIncluded);
    }
    if listing.location.as_ref().is_some_and(|l| l.where_ == ServiceLocation::AtReceiver) {
        return Err(NotComplete::AtReceiver);
    }
    let deal_terms = listing.deal_terms.as_ref().ok_or(NotComplete::NoDealTerms)?;
    if let Some(ext) = &listing.extension
        && extension::registry().workflow(ext).is_none()
    {
        return Err(NotComplete::ExtensionUnknown);
    }
    Ok((payment, deal_terms))
}

fn fixed_payment(listing: &ListingPayload) -> Result<&PaymentTerms, NotComplete> {
    let payment = listing.payment.as_ref().ok_or(NotComplete::PriceNotFixed)?;
    if payment.model != PaymentModel::Fixed
        || payment.amount_minor.is_none()
        || payment.payee.trim().is_empty()
    {
        return Err(NotComplete::PriceNotFixed);
    }
    Ok(payment)
}

fn has_reserve_hook(ext: &crate::deal::ExtensionRef) -> bool {
    extension::registry().workflow(ext).is_some_and(Manifest::has_reserve_hook)
}

/// Builds the terms both nodes compare. `owner_did` is checked against the
/// payload's own `listing_id`, never copied.
pub fn terms_from_listing(
    listing: &ListingPayload,
    owner_did: &str,
    as_listed: &AsListed,
    now_secs: u64,
) -> Result<DealTerms, NotComplete> {
    let (payment, deal_terms) = ready(listing)?;
    let reservation = listing.extension.as_ref().is_some_and(has_reserve_hook);
    let slot = match (as_listed.slot.as_ref(), reservation) {
        (None, true) => return Err(NotComplete::SlotRequired),
        (Some(_), false) => return Err(NotComplete::SlotNotOffered),
        (slot, _) => slot,
    };
    let derived = listing::derive_listing_id(owner_did, &listing.slug)
        .map_err(|_| NotComplete::Invalid("listing-id-mismatch"))?;
    if derived != listing.listing_id {
        return Err(NotComplete::Invalid("listing-id-mismatch"));
    }

    let amount_minor = payment.amount_minor.ok_or(NotComplete::Invalid("amount-minor-missing"))?;
    let plain = money::plain_amount(amount_minor, &payment.currency)
        .ok_or(NotComplete::Invalid("currency-unknown"))?;
    let amount_text = format!("{} {plain}", payment.currency);

    let timing = deal_terms.payment_timing;
    let commitments = vec![
        owner_commitment(listing, slot, timing, amount_minor),
        counterparty_commitment(listing, payment, timing, amount_minor, &amount_text),
    ];

    let terms = DealTerms {
        currency: payment.currency.clone(),
        quote_expires_at_secs: now_secs.saturating_add(deal_terms.quote_valid_secs),
        home_exchange: None,
        commitments,
        listing_id: Some(listing.listing_id.clone()),
        scope: LocalText {
            lang: listing.lang.clone(),
            text: listing.title.clone(),
            translations: listing
                .translations
                .iter()
                .map(|t| TextTranslation { lang: t.lang.clone(), text: t.title.clone() })
                .collect(),
        },
        cancellation_terms: deal_terms.cancellation_terms.clone(),
        refund_terms: deal_terms.refund_terms.clone(),
        dispute_path: deal_terms.dispute_path.clone(),
    };
    if let Err(e) = terms.validate() {
        return Err(NotComplete::Invalid(e.code()));
    }
    match terms.disclosure_size() {
        Ok(size)
            if size.total_bytes <= MAX_RECORD_DISCLOSURE_BYTES
                && size.details.iter().all(|(_, bytes)| *bytes <= MAX_DETAILS_DISCLOSURE_BYTES) =>
        {
            Ok(terms)
        }
        _ => Err(NotComplete::QuoteTooLarge),
    }
}

/// The owner's side: the offering itself, the slot when there is one, and the
/// time the owner hands it over.
fn owner_commitment(
    listing: &ListingPayload,
    slot: Option<&SlotChoice>,
    timing: Option<PaymentTiming>,
    amount_minor: i64,
) -> Commitment {
    Commitment {
        commitment_id: commitment_id(0),
        giver: DealRole::QuoteAuthor,
        resource_kind: if slot.is_some() {
            ResourceKind::ServiceInSlot
        } else {
            ResourceKind::GoodsOrService
        },
        kind: CommitmentKind::Fulfilment,
        extension: listing.extension.clone(),
        after: if timing == Some(PaymentTiming::BeforeWork) {
            vec![commitment_id(1)]
        } else {
            Vec::new()
        },
        description: description_from_listing(listing),
        declared_value_minor: amount_minor,
        details: extension_details(listing, slot),
        window: slot.map(|s| s.window),
        location: listing.location.as_ref().map(|l| QuoteLocation {
            where_: l.where_,
            area: None,
            address: None,
        }),
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }
}

/// The counterparty's side: the money, the payee and the methods the listing
/// states.
fn counterparty_commitment(
    listing: &ListingPayload,
    payment: &PaymentTerms,
    timing: Option<PaymentTiming>,
    amount_minor: i64,
    amount_text: &str,
) -> Commitment {
    Commitment {
        commitment_id: commitment_id(1),
        giver: DealRole::Counterparty,
        resource_kind: ResourceKind::Currency,
        kind: CommitmentKind::Currency,
        extension: None,
        after: if timing == Some(PaymentTiming::AfterWork) {
            vec![commitment_id(0)]
        } else {
            Vec::new()
        },
        description: LocalText {
            lang: listing.lang.clone(),
            text: amount_text.to_string(),
            translations: listing
                .translations
                .iter()
                .map(|t| TextTranslation { lang: t.lang.clone(), text: amount_text.to_string() })
                .collect(),
        },
        declared_value_minor: amount_minor,
        details: None,
        window: None,
        location: None,
        payee: Some(payment.payee.clone()),
        payment_methods: (!payment.methods.is_empty()).then(|| payment.methods.clone()),
        tax_minor: None,
        fees_minor: payment.fees_minor,
        eep: None,
    }
}

/// The owner's `details` block. The scheduling extension carries the slot the
/// person picked. Any other workflow extension carries its first schema with
/// empty data: `DealTerms::validate` refuses a known extension with no block,
/// and a listing that names one must build a quote that `quote.set` accepts.
fn extension_details(listing: &ListingPayload, slot: Option<&SlotChoice>) -> Option<Details> {
    let ext = listing.extension.as_ref()?;
    if ext.id == SCHEDULING_EXTENSION {
        let slot = slot?;
        return Some(Details {
            extension: ext.id.clone(),
            schema: SCHEDULING_SLOT_SCHEMA.to_string(),
            data: serde_json::json!({
                "listing_id": listing.listing_id,
                "slot_id": slot.slot_id,
            }),
            extra: Vec::new(),
        });
    }
    let schema = extension::registry().workflow(ext)?.details.first()?;
    Some(Details {
        extension: ext.id.clone(),
        schema: schema.schema.clone(),
        data: serde_json::json!({}),
        extra: Vec::new(),
    })
}

/// The offering's own words as the quote's scope: the text and each
/// translation, with the title standing in for an empty summary.
fn description_from_listing(listing: &ListingPayload) -> LocalText {
    LocalText {
        lang: listing.lang.clone(),
        text: non_empty_or(&listing.summary, &listing.title).to_string(),
        translations: listing
            .translations
            .iter()
            .map(|t| TextTranslation {
                lang: t.lang.clone(),
                text: non_empty_or(&t.summary, &t.title).to_string(),
            })
            .collect(),
    }
}

fn non_empty_or<'a>(text: &'a str, fallback: &'a str) -> &'a str {
    if text.trim().is_empty() { fallback } else { text }
}

/// The set of a `LocalText`'s translation languages, for comparison with the
/// listing's own set.
#[must_use]
pub fn translation_langs(text: &LocalText) -> HashSet<String> {
    text.translations.iter().map(|t| t.lang.to_string()).collect()
}

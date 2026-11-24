//! The listing fields that make "agree as listed" possible: how the owner
//! answers, how many instant agreements the offering may hold, and the texts
//! a quote built from the listing carries.
//!
//! Each field is left out of the signed payload at its default, so a listing
//! that sets none of them signs the same bytes it did before.

use serde::{Deserialize, Serialize};

use crate::{
    deal::{LocalText, MAX_TERMS_TEXT_LEN},
    extension::{self, Manifest},
    listing::{ListingError, ListingPayload, OpenTo},
    transaction::MIN_QUOTE_LIFETIME_SECS,
};

#[cfg(test)]
mod tests;

/// The most open instant agreements one offering may hold.
pub const MAX_INSTANT_CAP: u16 = 500;
/// The cap a listing gets when it names none.
pub const DEFAULT_INSTANT_CAP: u16 = 20;
/// The longest a quote built from a listing may stay valid.
pub const MAX_LISTING_QUOTE_VALID_SECS: u64 = 7 * 86_400;

/// Whether the owner answers a tap at once or confirms each one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Acceptance {
    Instant,
    #[default]
    ConfirmFirst,
}

/// When the money moves, relative to the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaymentTiming {
    BeforeWork,
    AfterWork,
}

/// The texts and the validity a quote built from this listing carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingDealTerms {
    pub cancellation_terms: LocalText,
    pub refund_terms: LocalText,
    pub dispute_path: LocalText,
    pub quote_valid_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_timing: Option<PaymentTiming>,
}

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

pub(crate) const fn default_instant_cap() -> u16 {
    DEFAULT_INSTANT_CAP
}

pub(crate) fn is_default_cap(value: &u16) -> bool {
    *value == DEFAULT_INSTANT_CAP
}

impl ListingPayload {
    /// The rules of the "agree as listed" section. Runs inside
    /// [`ListingPayload::validate`], so `listing.set` and every verifier
    /// apply them.
    pub(crate) fn validate_as_listed(&self) -> Result<(), ListingError> {
        if self.acceptance == Acceptance::Instant {
            if self.relationship.as_ref().map(|r| r.open_to) != Some(OpenTo::Anyone) {
                return Err(ListingError::InstantNotAllowed { reason: "not-open-to-anyone" });
            }
            let slot_offering = self.extension.as_ref().is_some_and(|ext| {
                extension::registry().workflow(ext).is_some_and(Manifest::has_reserve_hook)
            });
            if !slot_offering && !self.not_limited {
                return Err(ListingError::InstantNotAllowed { reason: "not-limited-or-slot" });
            }
        }
        if !(1..=MAX_INSTANT_CAP).contains(&self.instant_cap) {
            return Err(ListingError::InstantCapRange(self.instant_cap));
        }
        let Some(terms) = &self.deal_terms else {
            return Ok(());
        };
        let listing_langs: std::collections::HashSet<String> =
            self.translations.iter().map(|t| t.lang.to_string()).collect();
        for text in [&terms.cancellation_terms, &terms.refund_terms, &terms.dispute_path] {
            text.validate("deal_terms", MAX_TERMS_TEXT_LEN)?;
            if text.lang != self.lang || crate::as_listed::translation_langs(text) != listing_langs
            {
                return Err(ListingError::DealTermsLanguages);
            }
        }
        if !(MIN_QUOTE_LIFETIME_SECS..=MAX_LISTING_QUOTE_VALID_SECS)
            .contains(&terms.quote_valid_secs)
        {
            return Err(ListingError::QuoteValidRange(terms.quote_valid_secs));
        }
        Ok(())
    }
}

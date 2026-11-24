//! Tests for the listing's agree-as-listed fields.

use serde_json::json;

use super::*;
use crate::{
    deal::LocalText,
    lang::LangTag,
    listing::{
        self, Acceptance, AddressDisclosure, DEFAULT_INSTANT_CAP, ListingDealTerms, ListingError,
        ListingPayload, ListingStatus, LocationTerms, OpenTo, PaymentModel, PaymentTerms,
        PaymentTiming, RelationshipTerms, ServiceLocation,
    },
};

fn lang(code: &str) -> LangTag {
    LangTag::parse(code).expect("valid tag")
}

fn text(body: &str) -> LocalText {
    LocalText::plain(lang("en"), body)
}

fn deal_terms() -> ListingDealTerms {
    ListingDealTerms {
        cancellation_terms: text("Free until 24 hours before"),
        refund_terms: text("Full refund"),
        dispute_path: text("Write to us first"),
        quote_valid_secs: 3_600,
        payment_timing: Some(PaymentTiming::AfterWork),
    }
}

fn base() -> ListingPayload {
    ListingPayload {
        listing_id: listing::derive_listing_id("did:key:zIssuer", "hedge-trimming")
            .expect("listing id"),
        slug: "hedge-trimming".to_string(),
        title: "Hedge trimming".to_string(),
        summary: "Neat hedges.".to_string(),
        lang: lang("en"),
        translations: Vec::new(),
        categories: vec!["gardening".to_string()],
        conversation_address: "did:key:zListingOwnerConv".to_string(),
        status: ListingStatus::Active,
        extension: None,
        booking: None,
        payment: Some(PaymentTerms {
            currency: "EUR".to_string(),
            model: PaymentModel::Fixed,
            amount_minor: Some(3_500),
            tax_included: true,
            fees_minor: None,
            methods: Vec::new(),
            payee: "A. Gardener".to_string(),
        }),
        product: None,
        service: None,
        location: Some(LocationTerms {
            where_: ServiceLocation::AtGiver,
            service_area: Vec::new(),
            address_disclosure: AddressDisclosure::OnAgreement,
        }),
        relationship: Some(RelationshipTerms { open_to: OpenTo::Anyone, member_of: None }),
        acceptance: Acceptance::ConfirmFirst,
        not_limited: false,
        instant_cap: DEFAULT_INSTANT_CAP,
        deal_terms: Some(deal_terms()),
        service_record: None,
    }
}

fn error(listing: &ListingPayload) -> ListingError {
    listing.validate().expect_err("refused")
}

#[test]
fn instant_needs_an_offering_open_to_anyone() {
    let mut listing = base();
    listing.acceptance = Acceptance::Instant;
    listing.not_limited = true;
    assert!(listing.validate().is_ok());

    listing.relationship = Some(RelationshipTerms {
        open_to: OpenTo::Members,
        member_of: Some("did:key:z6MkGroup".to_string()),
    });
    let refused = error(&listing);
    assert_eq!(refused.code(), Some("instant-not-allowed"));
    assert_eq!(refused.reason(), Some("not-open-to-anyone"));

    listing.relationship = None;
    let refused = error(&listing);
    assert_eq!(refused.reason(), Some("not-open-to-anyone"));
}

#[test]
fn instant_needs_a_slot_offering_or_not_limited() {
    let mut listing = base();
    listing.acceptance = Acceptance::Instant;
    listing.not_limited = false;
    let refused = error(&listing);
    assert_eq!(refused.code(), Some("instant-not-allowed"));
    assert_eq!(refused.reason(), Some("not-limited-or-slot"));

    listing.not_limited = true;
    assert!(listing.validate().is_ok());
}

#[test]
fn instant_is_allowed_on_a_slot_offering_that_is_not_limited() {
    let mut listing = base();
    listing.acceptance = Acceptance::Instant;
    listing.not_limited = false;
    listing.extension =
        Some(crate::deal::ExtensionRef { id: "scheduling".to_string(), version: 1 });
    listing.booking = Some(listing::BookingTerms {
        lead_time_secs: 3_600,
        cancellation_window_secs: 86_400,
        max_per_booking: 2,
    });
    assert!(listing.validate().is_ok());
}

#[test]
fn instant_cap_is_one_to_the_maximum() {
    for cap in [0, MAX_INSTANT_CAP + 1] {
        let mut listing = base();
        listing.instant_cap = cap;
        let refused = error(&listing);
        assert_eq!(refused.code(), Some("instant-cap-range"));
    }
    for cap in [1, MAX_INSTANT_CAP] {
        let mut listing = base();
        listing.instant_cap = cap;
        assert!(listing.validate().is_ok());
    }
}

#[test]
fn deal_terms_texts_keep_the_listings_language_set() {
    let mut listing = base();
    listing.translations = vec![listing::ListingTranslation {
        lang: lang("hi"),
        title: "झाड़ी काटना".to_string(),
        summary: String::new(),
    }];

    // A text with no translation while the listing has one.
    assert_eq!(error(&listing).code(), Some("deal-terms-languages"));

    // A text in another language.
    let mut wrong_lang = listing.clone();
    wrong_lang.deal_terms.as_mut().expect("terms").refund_terms.lang = lang("fr");
    assert_eq!(error(&wrong_lang).code(), Some("deal-terms-languages"));

    // Every text with the listing's translation set passes.
    let translated = LocalText {
        lang: lang("en"),
        text: "Full refund".to_string(),
        translations: vec![crate::deal::TextTranslation {
            lang: lang("hi"),
            text: "पूरा रिफंड".to_string(),
        }],
    };
    let terms = listing.deal_terms.as_mut().expect("terms");
    terms.cancellation_terms = translated.clone();
    terms.refund_terms = translated.clone();
    terms.dispute_path = translated;
    assert!(listing.validate().is_ok());
}

#[test]
fn quote_valid_secs_is_the_quote_floor_to_seven_days() {
    for secs in
        [0, crate::transaction::MIN_QUOTE_LIFETIME_SECS - 1, MAX_LISTING_QUOTE_VALID_SECS + 1]
    {
        let mut listing = base();
        listing.deal_terms.as_mut().expect("terms").quote_valid_secs = secs;
        assert_eq!(error(&listing).code(), Some("quote-valid-range"));
    }
    for secs in [crate::transaction::MIN_QUOTE_LIFETIME_SECS, MAX_LISTING_QUOTE_VALID_SECS] {
        let mut listing = base();
        listing.deal_terms.as_mut().expect("terms").quote_valid_secs = secs;
        assert!(listing.validate().is_ok());
    }
}

#[test]
fn a_listing_without_the_new_fields_signs_the_same_bytes() {
    let before = json!({
        "listing_id": "lst_old",
        "slug": "hedge-trimming",
        "title": "Hedge trimming",
        "summary": "Neat hedges.",
        "lang": "en",
        "categories": ["gardening"],
        "conversation_address": "did:key:zListingOwnerConv",
        "status": "active",
        "payment": {
            "currency": "EUR",
            "model": "fixed",
            "amount_minor": 3500,
            "tax_included": true,
            "payee": "A. Gardener",
        },
        "relationship": { "open_to": "anyone" }
    });
    let payload: ListingPayload = serde_json::from_value(before.clone()).expect("payload");
    assert_eq!(payload.acceptance, Acceptance::ConfirmFirst);
    assert_eq!(payload.instant_cap, DEFAULT_INSTANT_CAP);
    assert!(!payload.not_limited);
    assert!(payload.deal_terms.is_none());
    assert_eq!(serde_json::to_value(&payload).expect("json"), before);
}

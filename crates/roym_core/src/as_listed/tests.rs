//! Tests for the pure agree-as-listed core.

use super::*;
use crate::{
    deal::{CommitmentKind, LocalText, ResourceKind, TextTranslation, commitment_id},
    disclosure::{MAX_DETAILS_DISCLOSURE_BYTES, MAX_RECORD_DISCLOSURE_BYTES},
    extension::{self, SCHEDULING_EXTENSION, SCHEDULING_SLOT_SCHEMA},
    lang::LangTag,
    listing::{
        self, Acceptance, AddressDisclosure, DEFAULT_INSTANT_CAP, ListingDealTerms, ListingPayload,
        ListingStatus, ListingTranslation, LocationTerms, OpenTo, PaymentModel, PaymentTerms,
        PaymentTiming, RelationshipTerms, ServiceLocation,
    },
    money,
    transaction::{DealRole, QuoteLocation, TimeWindow},
};

const OWNER: &str = "did:key:zAsListedOwner";
const NOW: u64 = 1_700_000_000;

fn lang(code: &str) -> LangTag {
    LangTag::parse(code).expect("valid tag")
}

fn fixed_payment(currency: &str, amount_minor: i64) -> PaymentTerms {
    PaymentTerms {
        currency: currency.to_string(),
        model: PaymentModel::Fixed,
        amount_minor: Some(amount_minor),
        tax_included: true,
        fees_minor: Some(100),
        methods: vec!["cash".to_string()],
        payee: "A. Gardener".to_string(),
    }
}

/// A text in the listing's own language with a translation for each of its
/// translations, as `DealTerms::validate` and the listing's own check require.
fn translated(body: &str) -> LocalText {
    LocalText {
        lang: lang("en"),
        text: body.to_string(),
        translations: vec![TextTranslation { lang: lang("hi"), text: body.to_string() }],
    }
}

fn deal_terms() -> ListingDealTerms {
    ListingDealTerms {
        cancellation_terms: translated("Free until 24 hours before"),
        refund_terms: translated("Full refund"),
        dispute_path: translated("Write to us first"),
        quote_valid_secs: 3_600,
        payment_timing: None,
    }
}

fn base() -> ListingPayload {
    ListingPayload {
        listing_id: listing::derive_listing_id(OWNER, "hedge-trimming").expect("listing id"),
        slug: "hedge-trimming".to_string(),
        title: "Hedge trimming".to_string(),
        summary: "Neat hedges, fortnightly.".to_string(),
        lang: lang("en"),
        translations: vec![ListingTranslation {
            lang: lang("hi"),
            title: "झाड़ी काटना".to_string(),
            summary: "साफ झाड़ियाँ".to_string(),
        }],
        categories: vec!["gardening".to_string()],
        conversation_address: "did:key:zListingOwnerConv".to_string(),
        status: ListingStatus::Active,
        extension: None,
        booking: None,
        payment: Some(fixed_payment("EUR", 3_500)),
        product: None,
        service: None,
        location: Some(LocationTerms {
            where_: ServiceLocation::AtGiver,
            service_area: Vec::new(),
            address_disclosure: AddressDisclosure::OnAgreement,
        }),
        relationship: Some(RelationshipTerms { open_to: OpenTo::Anyone, member_of: None }),
        acceptance: Acceptance::Instant,
        not_limited: true,
        instant_cap: DEFAULT_INSTANT_CAP,
        deal_terms: Some(deal_terms()),
        service_record: None,
    }
}

fn consent() -> AsListed {
    AsListed {
        record_id: "rec_listing".to_string(),
        slot: None,
        variant: None,
        valid_until_secs: NOW + 3_600,
    }
}

fn built(listing: &ListingPayload) -> DealTerms {
    terms_from_listing(listing, OWNER, &consent(), NOW).expect("terms")
}

fn refusal(listing: &ListingPayload) -> &'static str {
    terms_from_listing(listing, OWNER, &consent(), NOW).expect_err("refused").code()
}

#[test]
fn each_mapping_row_builds_its_field() {
    let listing = base();
    let terms = built(&listing);

    assert_eq!(terms.currency, "EUR");
    assert_eq!(terms.listing_id.as_deref(), Some(listing.listing_id.as_str()));
    assert_eq!(terms.scope.lang, lang("en"));
    assert_eq!(terms.scope.text, "Hedge trimming");
    assert_eq!(terms.scope.translations[0].lang, lang("hi"));
    assert_eq!(terms.scope.translations[0].text, "झाड़ी काटना");
    assert_eq!(terms.cancellation_terms.text, "Free until 24 hours before");
    assert_eq!(terms.refund_terms.text, "Full refund");
    assert_eq!(terms.dispute_path.text, "Write to us first");
    assert_eq!(terms.quote_expires_at_secs, NOW + 3_600);
    assert!(terms.home_exchange.is_none());

    let [owner, payer] = terms.commitments.as_slice() else {
        panic!("two commitments");
    };
    assert_eq!(owner.commitment_id, commitment_id(0));
    assert_eq!(owner.giver, DealRole::QuoteAuthor);
    assert_eq!(owner.resource_kind, ResourceKind::GoodsOrService);
    assert_eq!(owner.kind, CommitmentKind::Fulfilment);
    assert!(owner.extension.is_none());
    assert!(owner.after.is_empty());
    assert!(owner.details.is_none());
    assert!(owner.window.is_none());
    assert_eq!(owner.description.text, "Neat hedges, fortnightly.");
    assert_eq!(owner.description.translations[0].text, "साफ झाड़ियाँ");
    assert_eq!(owner.declared_value_minor, 3_500);
    assert_eq!(
        owner.location,
        Some(QuoteLocation { where_: ServiceLocation::AtGiver, area: None, address: None })
    );
    assert!(owner.payee.is_none());

    assert_eq!(payer.commitment_id, commitment_id(1));
    assert_eq!(payer.giver, DealRole::Counterparty);
    assert_eq!(payer.resource_kind, ResourceKind::Currency);
    assert_eq!(payer.kind, CommitmentKind::Currency);
    assert!(payer.extension.is_none());
    assert!(payer.after.is_empty());
    assert_eq!(payer.description.text, "EUR 35.00");
    assert_eq!(payer.description.translations[0].text, "EUR 35.00");
    assert_eq!(payer.declared_value_minor, 3_500);
    assert_eq!(payer.payee.as_deref(), Some("A. Gardener"));
    assert_eq!(payer.payment_methods, Some(vec!["cash".to_string()]));
    assert_eq!(payer.fees_minor, Some(100));
    assert!(payer.tax_minor.is_none());
}

#[test]
fn payment_timing_orders_the_commitments() {
    let mut before = base();
    before.deal_terms.as_mut().expect("terms").payment_timing = Some(PaymentTiming::BeforeWork);
    let terms = built(&before);
    assert_eq!(terms.commitments[0].after, vec![commitment_id(1)]);
    assert!(terms.commitments[1].after.is_empty());

    let mut after = base();
    after.deal_terms.as_mut().expect("terms").payment_timing = Some(PaymentTiming::AfterWork);
    let terms = built(&after);
    assert!(terms.commitments[0].after.is_empty());
    assert_eq!(terms.commitments[1].after, vec![commitment_id(0)]);
}

#[test]
fn a_slot_listing_fills_the_scheduling_details_and_window() {
    let mut listing = base();
    listing.extension =
        Some(crate::deal::ExtensionRef { id: SCHEDULING_EXTENSION.to_string(), version: 1 });
    listing.booking = Some(listing::BookingTerms {
        lead_time_secs: 3_600,
        cancellation_window_secs: 86_400,
        max_per_booking: 2,
    });
    let window = TimeWindow { earliest_secs: NOW + 86_400, latest_secs: NOW + 90_000 };
    let slot_id =
        listing::derive_slot_id(&listing.listing_id, window.earliest_secs, window.latest_secs)
            .expect("slot id");
    let mut as_listed = consent();
    as_listed.slot = Some(SlotChoice { slot_id: slot_id.clone(), window });

    let terms = terms_from_listing(&listing, OWNER, &as_listed, NOW).expect("terms");
    let owner = &terms.commitments[0];
    assert_eq!(owner.resource_kind, ResourceKind::ServiceInSlot);
    assert_eq!(owner.window, Some(window));
    let details = owner.details.as_ref().expect("details");
    assert_eq!(details.extension, SCHEDULING_EXTENSION);
    assert_eq!(details.schema, SCHEDULING_SLOT_SCHEMA);
    assert_eq!(details.data["listing_id"], listing.listing_id);
    assert_eq!(details.data["slot_id"], slot_id);
}

#[test]
fn a_listing_with_another_extension_fills_its_details_block() {
    let mut listing = base();
    listing.extension = Some(crate::deal::ExtensionRef { id: "one-shot".to_string(), version: 1 });
    assert!(
        !extension::registry()
            .workflow(listing.extension.as_ref().unwrap())
            .expect("manifest")
            .has_reserve_hook()
    );

    let terms = built(&listing);
    let owner = &terms.commitments[0];
    assert_eq!(owner.extension.as_ref().map(|e| e.id.as_str()), Some("one-shot"));
    let details = owner.details.as_ref().expect("details");
    assert_eq!(details.schema, "one-shot/order@1");
    assert_eq!(details.data, serde_json::json!({}));
    assert!(owner.window.is_none());
}

#[test]
fn each_refusal_has_its_code() {
    let mut draft = base();
    draft.status = ListingStatus::Draft;
    assert_eq!(refusal(&draft), "listing-not-active");

    let mut per_hour = base();
    per_hour.payment.as_mut().expect("payment").model = PaymentModel::PerHour;
    assert_eq!(refusal(&per_hour), "price-not-fixed");

    let mut no_payment = base();
    no_payment.payment = None;
    assert_eq!(refusal(&no_payment), "price-not-fixed");

    let mut no_amount = base();
    no_amount.payment.as_mut().expect("payment").amount_minor = None;
    assert_eq!(refusal(&no_amount), "price-not-fixed");

    let mut tax_extra = base();
    tax_extra.payment.as_mut().expect("payment").tax_included = false;
    assert_eq!(refusal(&tax_extra), "tax-not-included");

    let mut at_receiver = base();
    at_receiver.location.as_mut().expect("location").where_ = ServiceLocation::AtReceiver;
    assert_eq!(refusal(&at_receiver), "at-receiver");

    let mut no_terms = base();
    no_terms.deal_terms = None;
    assert_eq!(refusal(&no_terms), "no-deal-terms");

    let mut slot_listing = base();
    slot_listing.extension =
        Some(crate::deal::ExtensionRef { id: SCHEDULING_EXTENSION.to_string(), version: 1 });
    slot_listing.booking = Some(listing::BookingTerms {
        lead_time_secs: 3_600,
        cancellation_window_secs: 86_400,
        max_per_booking: 2,
    });
    assert_eq!(refusal(&slot_listing), "slot-required");

    let mut unknown = base();
    unknown.extension = Some(crate::deal::ExtensionRef { id: "future".to_string(), version: 2 });
    assert_eq!(refusal(&unknown), "extension-unknown");

    let mut offered_unasked = consent();
    offered_unasked.slot = Some(SlotChoice {
        slot_id: "slot_1".to_string(),
        window: TimeWindow { earliest_secs: NOW + 10, latest_secs: NOW + 20 },
    });
    assert_eq!(
        terms_from_listing(&base(), OWNER, &offered_unasked, NOW).expect_err("refused").code(),
        "slot-not-offered"
    );

    assert_eq!(NotComplete::QuoteTooLarge.code(), "quote-too-large");
    assert_eq!(NotComplete::Invalid("deal-bug").code(), "deal-bug");
    assert_eq!(
        terms_from_listing(&base(), "did:key:zSomeoneElse", &consent(), NOW)
            .expect_err("refused")
            .code(),
        "listing-id-mismatch"
    );
}

#[test]
fn a_refusal_that_breaks_two_rules_reports_the_first_one() {
    let mut listing = base();
    listing.status = ListingStatus::Draft;
    listing.payment.as_mut().expect("payment").model = PaymentModel::PerHour;
    listing.payment.as_mut().expect("payment").tax_included = false;
    assert_eq!(refusal(&listing), "listing-not-active");

    let mut listing = base();
    listing.payment.as_mut().expect("payment").model = PaymentModel::PerHour;
    listing.payment.as_mut().expect("payment").tax_included = false;
    assert_eq!(refusal(&listing), "price-not-fixed");

    let mut listing = base();
    listing.payment.as_mut().expect("payment").tax_included = false;
    listing.location.as_mut().expect("location").where_ = ServiceLocation::AtReceiver;
    listing.deal_terms = None;
    assert_eq!(refusal(&listing), "tax-not-included");

    let mut listing = base();
    listing.location.as_mut().expect("location").where_ = ServiceLocation::AtReceiver;
    listing.deal_terms = None;
    assert_eq!(refusal(&listing), "at-receiver");

    let mut listing = base();
    listing.deal_terms = None;
    listing.extension =
        Some(crate::deal::ExtensionRef { id: SCHEDULING_EXTENSION.to_string(), version: 1 });
    assert_eq!(refusal(&listing), "no-deal-terms");
}

/// Listings that cover every refusal, every shipped extension and the
/// shapes just outside them.
fn generated_listings() -> Vec<ListingPayload> {
    let mut out = vec![base()];

    for status in [ListingStatus::Draft, ListingStatus::Withdrawn] {
        let mut listing = base();
        listing.status = status;
        out.push(listing);
    }
    for model in [PaymentModel::PerHour, PaymentModel::PerUnit, PaymentModel::QuoteOnly] {
        let mut listing = base();
        listing.payment.as_mut().expect("payment").model = model;
        out.push(listing);
    }
    let mut no_payment = base();
    no_payment.payment = None;
    out.push(no_payment);
    let mut tax_extra = base();
    tax_extra.payment.as_mut().expect("payment").tax_included = false;
    out.push(tax_extra);
    let mut at_receiver = base();
    at_receiver.location.as_mut().expect("location").where_ = ServiceLocation::AtReceiver;
    out.push(at_receiver);
    let mut no_terms = base();
    no_terms.deal_terms = None;
    out.push(no_terms);
    let mut empty_payee = base();
    empty_payee.payment.as_mut().expect("payment").payee = "   ".to_string();
    out.push(empty_payee);

    let mut slot_listing = base();
    slot_listing.extension =
        Some(crate::deal::ExtensionRef { id: SCHEDULING_EXTENSION.to_string(), version: 1 });
    slot_listing.booking = Some(listing::BookingTerms {
        lead_time_secs: 3_600,
        cancellation_window_secs: 86_400,
        max_per_booking: 2,
    });
    out.push(slot_listing.clone());

    let mut one_shot = base();
    one_shot.extension = Some(crate::deal::ExtensionRef { id: "one-shot".to_string(), version: 1 });
    out.push(one_shot);

    let mut unknown = base();
    unknown.extension = Some(crate::deal::ExtensionRef { id: "future".to_string(), version: 2 });
    out.push(unknown);

    out
}

fn has_reserve_hook(listing: &ListingPayload) -> bool {
    listing
        .extension
        .as_ref()
        .and_then(|ext| extension::registry().workflow(ext))
        .is_some_and(extension::Manifest::has_reserve_hook)
}

#[test]
fn ready_agrees_with_the_full_build_without_a_slot() {
    for listing in generated_listings() {
        if has_reserve_hook(&listing) {
            assert!(as_listed_ready(&listing).is_ok(), "{}", listing.slug);
            assert_eq!(
                terms_from_listing(&listing, OWNER, &consent(), NOW).expect_err("slot").code(),
                "slot-required"
            );
            continue;
        }
        assert_eq!(
            as_listed_ready(&listing).is_ok(),
            terms_from_listing(&listing, OWNER, &consent(), NOW).is_ok(),
            "{}",
            listing.slug
        );
    }
}

#[test]
fn a_built_quote_never_fails_validate() {
    for listing in generated_listings() {
        if listing.validate().is_err() || as_listed_ready(&listing).is_err() {
            continue;
        }
        let mut as_listed = consent();
        if has_reserve_hook(&listing) {
            let window = TimeWindow { earliest_secs: NOW + 86_400, latest_secs: NOW + 90_000 };
            let slot_id = listing::derive_slot_id(
                &listing.listing_id,
                window.earliest_secs,
                window.latest_secs,
            )
            .expect("slot id");
            as_listed.slot = Some(SlotChoice { slot_id, window });
        }
        let terms = terms_from_listing(&listing, OWNER, &as_listed, NOW).expect("terms");
        assert!(terms.validate().is_ok(), "{}", listing.slug);
    }
}

#[test]
fn the_largest_listing_builds_terms_that_pass_the_size_check() {
    let others = ["hi", "fr", "de", "es"];
    let title = "t".repeat(listing::MAX_TITLE_LEN);
    let summary = "s".repeat(listing::MAX_SUMMARY_LEN);
    let long = "x".repeat(crate::deal::MAX_TERMS_TEXT_LEN);
    let translations: Vec<ListingTranslation> = others
        .iter()
        .map(|code| ListingTranslation {
            lang: lang(code),
            title: title.clone(),
            summary: summary.clone(),
        })
        .collect();
    let translated = |body: &str| LocalText {
        lang: lang("en"),
        text: body.to_string(),
        translations: others
            .iter()
            .map(|code| TextTranslation { lang: lang(code), text: body.to_string() })
            .collect(),
    };

    let listing = ListingPayload {
        title: title.clone(),
        summary: summary.clone(),
        translations,
        deal_terms: Some(ListingDealTerms {
            cancellation_terms: translated(&long),
            refund_terms: translated(&long),
            dispute_path: translated(&long),
            quote_valid_secs: listing::as_listed::MAX_LISTING_QUOTE_VALID_SECS,
            payment_timing: Some(PaymentTiming::BeforeWork),
        }),
        ..base()
    };
    assert!(listing.validate().is_ok(), "the fixture itself must be a valid listing");

    let terms = built(&listing);
    let size = terms.disclosure_size().expect("size");
    assert!(size.total_bytes <= MAX_RECORD_DISCLOSURE_BYTES);
    assert!(size.details.iter().all(|(_, bytes)| *bytes <= MAX_DETAILS_DISCLOSURE_BYTES));
}

#[test]
fn the_amount_text_uses_a_dot_and_the_currency_exponent() {
    assert_eq!(money::plain_amount(123_456, "INR").as_deref(), Some("1234.56"));
    assert_eq!(money::plain_amount(1_234, "JPY").as_deref(), Some("1234"));
    assert_eq!(money::plain_amount(1_234, "BHD").as_deref(), Some("1.234"));
    assert_eq!(money::plain_amount(5, "EUR").as_deref(), Some("0.05"));
    assert_eq!(money::plain_amount(-123_456, "INR").as_deref(), Some("-1234.56"));
    assert!(money::plain_amount(1, "XYZ").is_none());

    for (currency, amount, text) in
        [("INR", 123_456, "INR 1234.56"), ("JPY", 1_234, "JPY 1234"), ("BHD", 1_234, "BHD 1.234")]
    {
        let mut listing = base();
        listing.payment = Some(fixed_payment(currency, amount));
        let terms = built(&listing);
        assert_eq!(terms.commitments[1].description.text, text);
    }
}

#[test]
fn expiry_is_now_plus_quote_valid_secs() {
    let mut listing = base();
    listing.deal_terms.as_mut().expect("terms").quote_valid_secs = 60;
    assert_eq!(built(&listing).quote_expires_at_secs, NOW + 60);

    listing.deal_terms.as_mut().expect("terms").quote_valid_secs =
        listing::as_listed::MAX_LISTING_QUOTE_VALID_SECS;
    assert!(listing.validate().is_ok());
    assert_eq!(built(&listing).quote_expires_at_secs, NOW + 7 * 86_400);
}

#[test]
fn a_listing_with_empty_payee_refuses_price_not_fixed() {
    for payee in ["", " ", "\t\n"] {
        let mut listing = base();
        listing.payment.as_mut().expect("payment").payee = payee.to_string();
        assert_eq!(refusal(&listing), "price-not-fixed");
        assert_eq!(as_listed_ready(&listing).expect_err("refused").code(), "price-not-fixed");
    }
}

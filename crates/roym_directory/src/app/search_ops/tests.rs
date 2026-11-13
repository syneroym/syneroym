use syneroym_roym_core::{
    directory::SearchQuery,
    lang::LangTag,
    listing::{
        ListingPayload, ListingStatus, ListingTranslation, PaymentModel, PaymentTerms,
        derive_listing_id,
    },
};

use super::{build_search_filter, index_text};

fn sample_payload() -> ListingPayload {
    ListingPayload {
        listing_id: derive_listing_id("did:key:zIssuer", "pipe-fix").unwrap(),
        slug: "pipe-fix".to_string(),
        title: "नल वाला".to_string(),
        summary: "काम अच्छा".to_string(),
        lang: LangTag::parse("hi").unwrap(),
        translations: vec![ListingTranslation {
            lang: LangTag::parse("en").unwrap(),
            title: "Tap fixer".to_string(),
            summary: "Good work".to_string(),
        }],
        categories: vec!["plumbing".to_string(), "repair".to_string()],
        conversation_address: "did:key:zListingOwnerConv".to_string(),
        status: ListingStatus::Active,
        booking: None,
        payment: Some(PaymentTerms {
            currency: "INR".to_string(),
            model: PaymentModel::PerHour,
            amount_minor: Some(50000),
            tax_included: true,
            fees_minor: None,
            methods: vec!["cash".to_string()],
            payee: "A. Plumber".to_string(),
        }),
        product: None,
        service: None,
        location: None,
        relationship: None,
        service_record: None,
    }
}

#[test]
fn index_text_joins_title_summary_categories_translations() {
    let payload = sample_payload();
    let text = index_text(&payload);
    assert_eq!(text, "नल वाला काम अच्छा plumbing repair Tap fixer Good work");
}

#[test]
fn build_search_filter_uses_search_clause_and_no_regex_on_text() {
    let query = SearchQuery {
        categories: vec!["plumbing".to_string()],
        area: None,
        text: Some("nal wala".to_string()),
        open_to: None,
        booking_mode: None,
        limit: None,
    };
    let filter = build_search_filter(&query, 1000).unwrap();
    let filter_str = filter.to_string();
    assert!(filter_str.contains(r#"{"text":{"$search":"nal wala"}}"#));
    assert!(!filter_str.contains(r#"{"text":{"$regex""#));

    // Blank text adds no text clause
    let query_blank = SearchQuery {
        categories: vec![],
        area: None,
        text: Some("   ".to_string()),
        open_to: None,
        booking_mode: None,
        limit: None,
    };
    let filter_blank = build_search_filter(&query_blank, 1000).unwrap();
    assert!(!filter_blank.to_string().contains(r#""text""#));
}

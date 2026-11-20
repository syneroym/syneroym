use syneroym_roym_core::{
    deal::ExtensionRef,
    directory::SearchQuery,
    lang::LangTag,
    listing::{
        ListingPayload, ListingStatus, ListingTranslation, PaymentModel, PaymentTerms,
        derive_listing_id,
    },
};

use super::{build_index_rows, build_search_filter, index_text};

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
        extension: None,
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
        extension: None,
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
        extension: None,
        limit: None,
    };
    let filter_blank = build_search_filter(&query_blank, 1000).unwrap();
    assert!(!filter_blank.to_string().contains(r#""text""#));
}

fn query_for_extension(extension: &str) -> SearchQuery {
    SearchQuery {
        categories: vec![],
        area: None,
        text: None,
        open_to: None,
        extension: Some(extension.to_string()),
        limit: None,
    }
}

#[test]
fn search_refuses_an_extension_filter_that_is_not_an_id() {
    for bad in ["", "Scheduling", "has space", "a_b", &"a".repeat(33)] {
        let err = build_search_filter(&query_for_extension(bad), 1000).unwrap_err();
        assert_eq!(err.error.as_ref().unwrap().code, -32602, "filter {bad:?}");
    }
}

#[test]
fn an_extension_filter_and_its_index_column_are_written() {
    let filter = build_search_filter(&query_for_extension("scheduling"), 1000).unwrap();
    assert!(filter.to_string().contains(r#"{"extension":"scheduling"}"#));

    let mut scheduled = sample_payload();
    scheduled.extension = Some(ExtensionRef { id: "scheduling".to_string(), version: 1 });
    let rows = build_index_rows(&scheduled, "rec_1", "did:key:zIssuer", 10, 10, (0, 0));
    assert_eq!(serde_json::to_value(&rows[0]).unwrap()["extension"], "scheduling");

    let plain = build_index_rows(&sample_payload(), "rec_2", "did:key:zIssuer", 10, 10, (0, 0));
    assert!(serde_json::to_value(&plain[0]).unwrap().get("extension").is_none());
}

use serde_json::Value;
use syneroym_roym_core::{
    listing::ServiceLocation,
    transaction::{PaymentTiming, QuoteLocation, TimeWindow},
};

use super::QuoteTermsInput;

#[test]
fn quote_terms_input_matches_agreed_terms_fields() {
    let input = QuoteTermsInput {
        scope: "gardening".to_string(),
        currency: "EUR".to_string(),
        amount_minor: 5000,
        tax_minor: 500,
        fees_minor: 100,
        payment_methods: vec!["sepa".to_string()],
        payee: "did:key:z6MkuPayee".to_string(),
        payment_timing: PaymentTiming::AfterWork,
        schedule: Some(TimeWindow { earliest_secs: 1000, latest_secs: 2000 }),
        location: QuoteLocation {
            where_: ServiceLocation::AtGiver,
            area: None,
            address: Some("Baker Street 221B".to_string()),
        },
        cancellation_terms: "24h".to_string(),
        refund_terms: "full refund".to_string(),
        dispute_path: "arbitration".to_string(),
    };

    let expires_at_secs = 999_999;
    let agreed = input.clone().into_agreed_terms(expires_at_secs);

    let mut agreed_json = serde_json::to_value(&agreed).expect("serialize agreed terms");
    let agreed_map = agreed_json.as_object_mut().expect("agreed terms is object");
    let removed_expiry = agreed_map.remove("quote_expires_at_secs");
    assert_eq!(removed_expiry, Some(Value::from(expires_at_secs)));

    let input_json = serde_json::to_value(&input).expect("serialize quote terms input");
    assert_eq!(input_json, agreed_json);
}

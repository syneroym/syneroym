//! No signed record, and no value a service answers with, may carry a
//! fixed seller or buyer word. A deal has a quote author and a
//! counterparty, and nothing on the wire says anything else.
//!
//! Each sample fills every optional field and runs through every variant of
//! the enums that name a party or a place, so a value cannot stay hidden
//! behind a sample that does not use it.

use serde::Serialize;
use serde_json::Value;

use crate::{
    area::Area,
    booking::{BookingProgressPayload, BookingState, ConflictReason, NextStep, TrackState},
    fulfilment::FulfilmentReceiptPayload,
    listing::{
        AddressDisclosure, BookingMode, BookingTerms, ListingPayload, ListingStatus, LocationTerms,
        OpenTo, PaymentModel, PaymentTerms, ProductCondition, ProductDetail, RelationshipTerms,
        ServiceDetail, ServiceLocation, ServiceRecordTerms,
    },
    payment::{PaymentAcknowledgementPayload, PaymentRequestPayload},
    transaction::{
        AgreedTerms, AgreementReceiptPayload, DealRole, PaymentTiming, QuoteLocation, QuotePayload,
        RequestPayload, TimeWindow,
    },
};

const OLD_WORDS: [&str; 3] = ["provider", "consumer", "customer"];

const COUNTERPARTY: &str = "did:key:zCounterparty";
const QUOTE_AUTHOR: &str = "did:key:zQuoteAuthor";

/// Records every key and every string value that holds an old word, with
/// its path in the JSON.
fn find_old_words(path: &str, value: &Value, found: &mut Vec<String>) {
    let has_old_word = |text: &str| {
        let lower = text.to_lowercase();
        OLD_WORDS.iter().any(|word| lower.contains(word))
    };
    match value {
        Value::String(text) if has_old_word(text) => found.push(format!("{path} = {text:?}")),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                find_old_words(&format!("{path}[{i}]"), item, found);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if has_old_word(key) {
                    found.push(format!("{path}.{key} (key)"));
                }
                find_old_words(&format!("{path}.{key}"), item, found);
            }
        }
        _ => {}
    }
}

fn assert_no_old_words(name: &str, sample: &impl Serialize) {
    let value = serde_json::to_value(sample).unwrap();
    let mut found = Vec::new();
    find_old_words(name, &value, &mut found);
    assert!(found.is_empty(), "old deal-role words on the wire: {found:#?}");
}

// Each list below sits next to a `match` without a wildcard, so adding a
// variant stops the build here until the new variant is listed.

fn every_role() -> [DealRole; 2] {
    let _ = |role: DealRole| match role {
        DealRole::QuoteAuthor | DealRole::Counterparty => (),
    };
    [DealRole::QuoteAuthor, DealRole::Counterparty]
}

fn every_place() -> [ServiceLocation; 3] {
    let _ = |place: ServiceLocation| match place {
        ServiceLocation::AtGiver | ServiceLocation::AtReceiver | ServiceLocation::Remote => (),
    };
    [ServiceLocation::AtGiver, ServiceLocation::AtReceiver, ServiceLocation::Remote]
}

fn every_audience() -> [OpenTo; 4] {
    let _ = |open_to: OpenTo| match open_to {
        OpenTo::Anyone | OpenTo::Members | OpenTo::Referral | OpenTo::PastCounterparties => (),
    };
    [OpenTo::Anyone, OpenTo::Members, OpenTo::Referral, OpenTo::PastCounterparties]
}

fn every_next_step() -> [NextStep; 7] {
    let _ = |step: NextStep| match step {
        NextStep::WaitForQuoteAuthor
        | NextStep::RequestPayment
        | NextStep::PayOutsideRoym
        | NextStep::ConfirmPaymentReceived
        | NextStep::MarkWorkComplete
        | NextStep::ConfirmWorkComplete
        | NextStep::Nothing => (),
    };
    [
        NextStep::WaitForQuoteAuthor,
        NextStep::RequestPayment,
        NextStep::PayOutsideRoym,
        NextStep::ConfirmPaymentReceived,
        NextStep::MarkWorkComplete,
        NextStep::ConfirmWorkComplete,
        NextStep::Nothing,
    ]
}

fn terms(place: ServiceLocation) -> AgreedTerms {
    AgreedTerms {
        scope: "Trim the hedge".to_string(),
        currency: "EUR".to_string(),
        amount_minor: 12_000,
        tax_minor: 2_000,
        fees_minor: 100,
        payment_methods: vec!["cash".to_string()],
        payee: "A. Gardener".to_string(),
        payment_timing: PaymentTiming::BeforeWork,
        schedule: Some(TimeWindow { earliest_secs: 1_000, latest_secs: 2_000 }),
        location: QuoteLocation {
            where_: place,
            area: Some(Area::Named { label: "North".to_string(), code: Some("N1".to_string()) }),
            address: Some("1 Garden Lane".to_string()),
        },
        cancellation_terms: "Free until a day before".to_string(),
        refund_terms: "Refund if the work is not done".to_string(),
        dispute_path: "Write to the quote author".to_string(),
        quote_expires_at_secs: 5_000,
    }
}

fn request() -> RequestPayload {
    RequestPayload {
        request_id: "req_1".to_string(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        listing_id: Some("lst_1".to_string()),
        categories: vec!["gardening".to_string()],
        description: "Trim the hedge".to_string(),
        area: Some(Area::Circle { lat_e6: 1, lon_e6: 2, radius_m: 3 }),
        window: Some(TimeWindow { earliest_secs: 1_000, latest_secs: 2_000 }),
        data_use_notice: crate::transaction::DEFAULT_DATA_USE_NOTICE.to_string(),
    }
}

fn booking_progress(
    cancelled_by: DealRole,
    conflict: ConflictReason,
    state: BookingState,
) -> BookingProgressPayload {
    BookingProgressPayload {
        agreement: "rec_agreement".to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: COUNTERPARTY.to_string(),
        quote_author_did: QUOTE_AUTHOR.to_string(),
        seq: 2,
        state,
        conflict: Some(conflict),
        payment: TrackState::Claimed,
        fulfilment: TrackState::Acknowledged,
        track_window_ends_at_secs: 9_000,
        cancelled_by: Some(cancelled_by),
        cancel_reason: Some("changed my mind".to_string()),
    }
}

fn listing(place: ServiceLocation, open_to: OpenTo) -> ListingPayload {
    ListingPayload {
        listing_id: "lst_1".to_string(),
        slug: "hedge-trimming".to_string(),
        title: "Hedge trimming".to_string(),
        summary: "Neat hedges.".to_string(),
        categories: vec!["gardening".to_string()],
        conversation_address: "did:key:zListingOwnerConv".to_string(),
        status: ListingStatus::Active,
        booking: Some(BookingTerms {
            mode: BookingMode::Slots,
            lead_time_secs: 3_600,
            cancellation_window_secs: 86_400,
            max_per_booking: 2,
        }),
        payment: Some(PaymentTerms {
            currency: "EUR".to_string(),
            model: PaymentModel::PerHour,
            amount_minor: Some(3_500),
            tax_included: true,
            fees_minor: Some(100),
            methods: vec!["cash".to_string()],
            payee: "A. Gardener".to_string(),
        }),
        product: Some(ProductDetail {
            unit: "hour".to_string(),
            pack_size: 1,
            condition: ProductCondition::New,
            sku: Some("HT-1".to_string()),
        }),
        service: Some(ServiceDetail {
            duration_secs: 3_600,
            includes: vec!["clippings removed".to_string()],
            excludes: vec!["ladders".to_string()],
            prerequisites: vec!["access to the garden".to_string()],
        }),
        location: Some(LocationTerms {
            where_: place,
            service_area: vec![Area::Circle { lat_e6: 1, lon_e6: 2, radius_m: 3 }],
            address_disclosure: AddressDisclosure::OnAgreement,
        }),
        relationship: Some(RelationshipTerms {
            open_to,
            member_of: Some("did:key:zGroup".to_string()),
        }),
        service_record: Some(ServiceRecordTerms {
            issues_fulfilment_receipt: true,
            warranty_secs: 10,
            retention_secs: 20,
        }),
    }
}

#[test]
fn the_deal_enums_say_no_old_word_in_any_variant() {
    for role in every_role() {
        assert_no_old_words("role", &role);
    }
    for place in every_place() {
        assert_no_old_words("place", &place);
    }
    for open_to in every_audience() {
        assert_no_old_words("open_to", &open_to);
    }
    for step in every_next_step() {
        assert_no_old_words("next_step", &step);
    }
}

#[test]
fn a_request_and_a_quote_say_no_old_word() {
    assert_no_old_words("request", &request());
    for place in every_place() {
        let quote = QuotePayload {
            quote_id: "quo_1".to_string(),
            conversation: "conv_1".to_string(),
            sequence: 1,
            request_record_id: "rec_request".to_string(),
            listing_id: Some("lst_1".to_string()),
            slot_id: Some("slot_1".to_string()),
            counterparty_did: COUNTERPARTY.to_string(),
            terms: terms(place),
        };
        assert_no_old_words("quote", &quote);
    }
}

#[test]
fn an_agreement_and_its_receipts_say_no_old_word() {
    for role in every_role() {
        for place in every_place() {
            let agreement = AgreementReceiptPayload {
                quote_record_id: "rec_quote".to_string(),
                counterparty_did: COUNTERPARTY.to_string(),
                quote_author_did: QUOTE_AUTHOR.to_string(),
                role,
                terms: terms(place),
            };
            assert_no_old_words("agreement-receipt", &agreement);
            let fulfilment = FulfilmentReceiptPayload {
                agreement: "rec_agreement".to_string(),
                conversation: "conv_1".to_string(),
                counterparty_did: COUNTERPARTY.to_string(),
                quote_author_did: QUOTE_AUTHOR.to_string(),
                role,
                terms: terms(place),
            };
            assert_no_old_words("fulfilment-receipt", &fulfilment);
        }
    }
}

#[test]
fn the_payment_records_say_no_old_word() {
    let request = PaymentRequestPayload {
        agreement: "rec_agreement".to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: COUNTERPARTY.to_string(),
        quote_author_did: QUOTE_AUTHOR.to_string(),
        currency: "EUR".to_string(),
        amount_minor: 12_000,
        note: Some("Please pay".to_string()),
    };
    assert_no_old_words("payment-request", &request);
    for role in every_role() {
        let ack = PaymentAcknowledgementPayload {
            agreement: "rec_agreement".to_string(),
            conversation: "conv_1".to_string(),
            counterparty_did: COUNTERPARTY.to_string(),
            quote_author_did: QUOTE_AUTHOR.to_string(),
            role,
            currency: "EUR".to_string(),
            amount_minor: 12_000,
            observed_at_secs: 3_000,
            method: Some("cash".to_string()),
            reference: Some("receipt 7".to_string()),
        };
        assert_no_old_words("payment-acknowledgement", &ack);
    }
}

#[test]
fn a_booking_progress_snapshot_says_no_old_word_for_either_canceller() {
    for role in every_role() {
        for conflict in [ConflictReason::SlotTaken, ConflictReason::SlotUnavailable] {
            for state in [BookingState::Cancelled, BookingState::Conflict] {
                assert_no_old_words("booking-progress", &booking_progress(role, conflict, state));
            }
        }
    }
}

#[test]
fn a_listing_says_no_old_word_for_any_place_or_audience() {
    for place in every_place() {
        for open_to in every_audience() {
            assert_no_old_words("listing", &listing(place, open_to));
        }
    }
}

#[test]
fn the_guard_finds_an_old_word_in_a_key_and_in_a_value() {
    let mut found = Vec::new();
    let sample = serde_json::json!({ "consumer_did": "x", "role": ["fine", "the Provider"] });
    find_old_words("sample", &sample, &mut found);
    assert_eq!(found, ["sample.consumer_did (key)", "sample.role[1] = \"the Provider\""]);
}

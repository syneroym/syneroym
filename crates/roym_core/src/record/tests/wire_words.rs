//! No signed record, and no value a service answers with, may carry a
//! fixed seller or buyer word. A deal has a quote author and a
//! counterparty, and nothing on the wire says anything else.
//!
//! Each sample fills every optional field and runs through every variant of
//! the enums that name a party or a place, so a value cannot stay hidden
//! behind a sample that does not use it.

use serde::Serialize;
use serde_json::Value;
use syneroym_signed_record::decode_disclosure;

use crate::{
    area::Area,
    commitment_failure::{CommitmentFailurePayload, FailureReason},
    deal::{
        Answers, Commitment, CommitmentKind, DealTerms, ExtensionRef, LocalText, ResourceKind,
        SettlementRole, TextTranslation, transition::CommitmentState,
    },
    lang::LangTag,
    listing::{
        AddressDisclosure, BookingTerms, ListingPayload, ListingStatus, ListingTranslation,
        LocationTerms, OpenTo, PaymentModel, PaymentTerms, ProductCondition, ProductDetail,
        RelationshipTerms, ServiceDetail, ServiceLocation, ServiceRecordTerms,
    },
    progress::{AgreementProgressPayload, AgreementState, CancelReason, CommitmentProgress},
    testing::sd::{
        self, PlainFulfilment, PlainPaymentAck, PlainPaymentRequest, PlainQuote, PlainStep,
    },
    transaction::{
        AgreementReceiptPayload, DEFAULT_DATA_USE_NOTICE, DealRole, EnquiryPayload, QuoteLocation,
        TimeWindow,
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

fn assert_no_old_words_in_disclosures(name: &str, disclosures: &[String]) {
    for (i, d) in disclosures.iter().enumerate() {
        if let Ok(decoded) = decode_disclosure(d) {
            let mut found = Vec::new();
            find_old_words(&format!("{name}.disclosures[{i}]"), &decoded.value, &mut found);
            assert!(found.is_empty(), "old deal-role words in disclosures: {found:#?}");
        }
    }
}

/// Lists every variant of an enum. The `match` has no wildcard, so adding a
/// variant stops the build until the new variant is listed here.
macro_rules! every {
    ($ty:ty, [$($variant:path),+ $(,)?]) => {{
        let _ = |value: $ty| match value {
            $($variant)|+ => (),
        };
        [$($variant),+]
    }};
}

fn every_role() -> [DealRole; 2] {
    every!(DealRole, [DealRole::QuoteAuthor, DealRole::Counterparty])
}

fn every_settlement_role() -> [SettlementRole; 2] {
    every!(SettlementRole, [SettlementRole::Giver, SettlementRole::Receiver])
}

fn every_place() -> [ServiceLocation; 3] {
    every!(
        ServiceLocation,
        [ServiceLocation::AtGiver, ServiceLocation::AtReceiver, ServiceLocation::Remote]
    )
}

fn every_audience() -> [OpenTo; 4] {
    every!(OpenTo, [OpenTo::Anyone, OpenTo::Members, OpenTo::Referral, OpenTo::PastCounterparties])
}

fn every_agreement_state() -> [AgreementState; 4] {
    every!(
        AgreementState,
        [
            AgreementState::Agreed,
            AgreementState::Closed,
            AgreementState::Cancelled,
            AgreementState::NotFormed,
        ]
    )
}

fn every_cancel_reason() -> [CancelReason; 4] {
    every!(
        CancelReason,
        [
            CancelReason::CannotDeliver,
            CancelReason::ScheduleConflict,
            CancelReason::CounterpartyAsked,
            CancelReason::Other,
        ]
    )
}

fn every_commitment_state() -> [CommitmentState; 8] {
    every!(
        CommitmentState,
        [
            CommitmentState::Promised,
            CommitmentState::PaymentRequested,
            CommitmentState::Claimed,
            CommitmentState::Fulfilled,
            CommitmentState::Acknowledged,
            CommitmentState::EndedUnconfirmed,
            CommitmentState::Failed,
            CommitmentState::Cancelled,
        ]
    )
}

fn terms(place: ServiceLocation) -> DealTerms {
    DealTerms {
        currency: "EUR".to_string(),
        quote_expires_at_secs: 5_000,
        home_exchange: None,
        commitments: vec![Commitment {
            commitment_id: "c1".to_string(),
            giver: DealRole::QuoteAuthor,
            resource_kind: ResourceKind::GoodsOrService,
            kind: CommitmentKind::Fulfilment,
            extension: None,
            after: Vec::new(),
            description: LocalText {
                lang: LangTag::parse("en").unwrap(),
                text: "Trim the hedge".to_string(),
                translations: vec![TextTranslation {
                    lang: LangTag::parse("hi").unwrap(),
                    text: "झाड़ी काटें".to_string(),
                }],
            },
            declared_value_minor: 12_000,
            details: None,
            window: Some(TimeWindow { earliest_secs: 1_000, latest_secs: 2_000 }),
            location: Some(QuoteLocation {
                where_: place,
                area: Some(Area::Named {
                    label: "North".to_string(),
                    code: Some("N1".to_string()),
                }),
                address: Some("1 Garden Lane".to_string()),
            }),
            payee: None,
            payment_methods: None,
            tax_minor: None,
            fees_minor: None,
            eep: None,
        }],
        listing_id: Some("lst_1".to_string()),
        scope: LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Trim the hedge".to_string(),
            translations: vec![TextTranslation {
                lang: LangTag::parse("hi").unwrap(),
                text: "झाड़ी काटें".to_string(),
            }],
        },
        cancellation_terms: LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Free until a day before".to_string(),
            translations: vec![TextTranslation {
                lang: LangTag::parse("hi").unwrap(),
                text: "रद्दीकरण".to_string(),
            }],
        },
        refund_terms: LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Refund if the work is not done".to_string(),
            translations: vec![TextTranslation {
                lang: LangTag::parse("hi").unwrap(),
                text: "वापसी".to_string(),
            }],
        },
        dispute_path: LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Write to the quote author".to_string(),
            translations: vec![TextTranslation {
                lang: LangTag::parse("hi").unwrap(),
                text: "विवाद".to_string(),
            }],
        },
    }
}

fn enquiry() -> EnquiryPayload {
    EnquiryPayload {
        enquiry_id: "enq_1".to_string(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        listing_id: Some("lst_1".to_string()),
        categories: vec!["gardening".to_string()],
        description: LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Trim the hedge".to_string(),
            translations: vec![TextTranslation {
                lang: LangTag::parse("hi").unwrap(),
                text: "झाड़ी काटें".to_string(),
            }],
        },
        area: Some(Area::Circle { lat_e6: 1, lon_e6: 2, radius_m: 3 }),
        window: Some(TimeWindow { earliest_secs: 1_000, latest_secs: 2_000 }),
        data_use_notice: DEFAULT_DATA_USE_NOTICE.to_string(),
    }
}

fn agreement_progress(
    cancel_reason: Option<CancelReason>,
    state: AgreementState,
) -> AgreementProgressPayload {
    AgreementProgressPayload {
        agreement: "rec_agreement".to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: COUNTERPARTY.to_string(),
        quote_author_did: QUOTE_AUTHOR.to_string(),
        seq: 2,
        state,
        state_reason: if state == AgreementState::NotFormed {
            Some("slot-taken".to_string())
        } else {
            None
        },
        cancel_reason,
        commitments: vec![CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Promised,
            reason: None,
            claimed: None,
            halves: vec!["rec_half1".to_string()],
        }],
    }
}

fn listing(place: ServiceLocation, open_to: OpenTo) -> ListingPayload {
    ListingPayload {
        listing_id: "lst_1".to_string(),
        slug: "hedge-trimming".to_string(),
        title: "Hedge trimming".to_string(),
        summary: "Neat hedges.".to_string(),
        lang: LangTag::parse("en").unwrap(),
        translations: vec![ListingTranslation {
            lang: LangTag::parse("hi").unwrap(),
            title: "झाड़ी कटाई".to_string(),
            summary: "साफ झाड़ियाँ।".to_string(),
        }],
        categories: vec!["gardening".to_string()],
        conversation_address: "did:key:zListingOwnerConv".to_string(),
        status: ListingStatus::Active,
        extension: Some(ExtensionRef { id: "scheduling".to_string(), version: 1 }),
        booking: Some(BookingTerms {
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
    for s_role in every_settlement_role() {
        assert_no_old_words("settlement role", &s_role);
    }
    for place in every_place() {
        assert_no_old_words("place", &place);
    }
    for open_to in every_audience() {
        assert_no_old_words("open_to", &open_to);
    }
}

#[test]
fn the_other_wire_enums_say_no_old_word_in_any_variant() {
    for state in every_agreement_state() {
        assert_no_old_words("agreement state", &state);
    }
    for reason in every_cancel_reason() {
        assert_no_old_words("cancel reason", &reason);
    }
    for c_state in every_commitment_state() {
        assert_no_old_words("commitment state", &c_state);
    }
    let statuses = every!(
        ListingStatus,
        [ListingStatus::Draft, ListingStatus::Active, ListingStatus::Withdrawn]
    );
    let models = every!(
        PaymentModel,
        [
            PaymentModel::Fixed,
            PaymentModel::PerHour,
            PaymentModel::PerUnit,
            PaymentModel::QuoteOnly
        ]
    );
    let conditions = every!(
        ProductCondition,
        [ProductCondition::New, ProductCondition::Used, ProductCondition::Refurbished]
    );
    let disclosures =
        every!(AddressDisclosure, [AddressDisclosure::OnAgreement, AddressDisclosure::Public]);
    assert_no_old_words("listing status", &statuses);
    assert_no_old_words("payment model", &models);
    assert_no_old_words("product condition", &conditions);
    assert_no_old_words("address disclosure", &disclosures);
}

#[test]
fn a_request_and_a_quote_say_no_old_word() {
    assert_no_old_words("enquiry", &enquiry());
    for place in every_place() {
        let plain_terms = terms(place);
        assert_no_old_words("plain terms", &plain_terms);
        let plain = PlainQuote {
            quote_id: "quo_1".to_string(),
            conversation: "conv_1".to_string(),
            sequence: 1,
            answers: Answers::Enquiry { record_id: "rec_enquiry".to_string() },
            counterparty_did: COUNTERPARTY.to_string(),
            terms: plain_terms,
        };
        let (quote, disclosures) = sd::conceal_quote(plain, 1);
        assert_no_old_words("quote", &quote);
        assert_no_old_words_in_disclosures("quote", &disclosures);
    }
}

#[test]
fn an_agreement_and_its_receipts_say_no_old_word() {
    for role in every_role() {
        for place in every_place() {
            let plain_terms = terms(place);
            assert_no_old_words("plain terms", &plain_terms);
            let mut salts = sd::seeded_salts(2);
            let concealed = plain_terms.conceal(&mut salts).unwrap();
            let agreement = AgreementReceiptPayload {
                quote_record_id: "rec_quote".to_string(),
                counterparty_did: COUNTERPARTY.to_string(),
                quote_author_did: QUOTE_AUTHOR.to_string(),
                role,
                viewed_lang: plain_terms.scope.lang,
                viewed_via_mt: false,
                terms: concealed.signed,
            };
            assert_no_old_words("agreement-receipt", &agreement);
            assert_no_old_words_in_disclosures("agreement", &concealed.disclosures);
        }
    }
    for s_role in every_settlement_role() {
        let plain = PlainFulfilment {
            agreement: "rec_agreement".to_string(),
            conversation: "conv_1".to_string(),
            commitment_id: "c1".to_string(),
            counterparty_did: COUNTERPARTY.to_string(),
            quote_author_did: QUOTE_AUTHOR.to_string(),
            role: s_role,
            settlement_note: None,
        };
        assert_no_old_words("plain fulfilment", &plain.settlement_note);
        let (fulfilment, disclosures) = sd::conceal_fulfilment(plain, 3);
        assert_no_old_words("fulfilment-receipt", &fulfilment);
        assert_no_old_words_in_disclosures("fulfilment", &disclosures);
    }
}

#[test]
fn the_commitment_failure_record_says_no_old_word() {
    let failure = CommitmentFailurePayload {
        agreement: "rec_agreement".to_string(),
        conversation: "conv_1".to_string(),
        commitment_id: "c1".to_string(),
        reason: every!(FailureReason, [FailureReason::ReservationLost])[0],
    };
    assert_no_old_words("commitment-failure", &failure);
}

#[test]
fn the_step_record_says_no_old_word() {
    for s_role in every_settlement_role() {
        let plain = PlainStep {
            agreement: "rec_agreement".to_string(),
            conversation: "conv_1".to_string(),
            commitment_id: "c1".to_string(),
            counterparty_did: COUNTERPARTY.to_string(),
            quote_author_did: QUOTE_AUTHOR.to_string(),
            role: s_role,
            extension: ExtensionRef { id: "scheduling".to_string(), version: 1 },
            step_type: "scheduling/started".to_string(),
            data: serde_json::json!({ "note": "left at the door" }),
        };
        let (step, disclosures) = sd::conceal_step(plain, 3);
        assert_no_old_words("step", &step);
        assert_no_old_words_in_disclosures("step", &disclosures);
    }
}

#[test]
fn the_payment_records_say_no_old_word() {
    let plain_req = PlainPaymentRequest {
        agreement: "rec_agreement".to_string(),
        conversation: "conv_1".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: COUNTERPARTY.to_string(),
        quote_author_did: QUOTE_AUTHOR.to_string(),
        note: Some(LocalText {
            lang: LangTag::parse("en").unwrap(),
            text: "Please pay".to_string(),
            translations: Vec::new(),
        }),
    };
    assert_no_old_words("plain payment-request note", &plain_req.note);
    let (request, req_disclosures) = sd::conceal_payment_request(plain_req, 4);
    assert_no_old_words("payment-request", &request);
    assert_no_old_words_in_disclosures("payment-request", &req_disclosures);

    for s_role in every_settlement_role() {
        let plain_ack = PlainPaymentAck {
            agreement: "rec_agreement".to_string(),
            conversation: "conv_1".to_string(),
            commitment_id: "c1".to_string(),
            counterparty_did: COUNTERPARTY.to_string(),
            quote_author_did: QUOTE_AUTHOR.to_string(),
            role: s_role,
            observed_at_secs: 3_000,
            method: Some("cash".to_string()),
            reference: Some("receipt 7".to_string()),
            settlement_note: None,
        };
        assert_no_old_words("plain payment-ack method", &plain_ack.method);
        let (ack, ack_disclosures) = sd::conceal_payment_ack(plain_ack, 5);
        assert_no_old_words("payment-acknowledgement", &ack);
        assert_no_old_words_in_disclosures("payment-acknowledgement", &ack_disclosures);
    }
}

#[test]
fn an_agreement_progress_snapshot_says_no_old_word() {
    for reason in every_cancel_reason() {
        for state in every_agreement_state() {
            assert_no_old_words("agreement-progress", &agreement_progress(Some(reason), state));
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

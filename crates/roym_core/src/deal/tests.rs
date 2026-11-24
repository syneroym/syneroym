use serde_json::json;

mod extension;

use crate::{
    deal::{
        Answers, Commitment, CommitmentKind, DealTerms, Details, DetailsExtra, ExtensionRef,
        LocalText, MAX_COMMITMENTS, MAX_DECLARED_VALUE_MINOR, ResourceKind, TextTranslation,
        commitment_id,
    },
    extension::{SCHEDULING_EXTENSION, SCHEDULING_SLOT_SCHEMA},
    lang::LangTag,
    listing,
    testing::sd,
    transaction::{AgreementReceiptPayload, DealRole, QuotePayload, TimeWindow},
};

fn sample_tag(s: &str) -> LangTag {
    LangTag::parse(s).expect("valid tag")
}

fn sample_local_text(text: &str) -> LocalText {
    LocalText { lang: sample_tag("en"), text: text.to_string(), translations: Vec::new() }
}

fn sample_service_commitment(id: &str, giver: DealRole, value: i64) -> Commitment {
    Commitment {
        commitment_id: id.to_string(),
        giver,
        resource_kind: ResourceKind::GoodsOrService,
        kind: CommitmentKind::Fulfilment,
        extension: None,
        after: Vec::new(),
        description: sample_local_text("Tutoring lesson"),
        declared_value_minor: value,
        details: None,
        window: None,
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }
}

fn sample_currency_commitment(id: &str, giver: DealRole, value: i64) -> Commitment {
    Commitment {
        commitment_id: id.to_string(),
        giver,
        resource_kind: ResourceKind::Currency,
        kind: CommitmentKind::Currency,
        extension: None,
        after: Vec::new(),
        description: sample_local_text("Payment for lesson"),
        declared_value_minor: value,
        details: None,
        window: None,
        location: None,
        payee: Some("did:key:z6MkuSamplePayeeKey".to_string()),
        payment_methods: Some(vec!["in-app".to_string()]),
        tax_minor: Some(0),
        fees_minor: Some(0),
        eep: None,
    }
}

fn sample_slot_commitment(id: &str, giver: DealRole, value: i64) -> Commitment {
    let window = TimeWindow { earliest_secs: 1_000_000, latest_secs: 1_003_600 };
    let listing_id = "listing_123";
    let slot_id = listing::derive_slot_id(listing_id, window.earliest_secs, window.latest_secs)
        .expect("derive slot id");
    Commitment {
        commitment_id: id.to_string(),
        giver,
        resource_kind: ResourceKind::ServiceInSlot,
        kind: CommitmentKind::Fulfilment,
        extension: Some(ExtensionRef { id: SCHEDULING_EXTENSION.to_string(), version: 1 }),
        after: Vec::new(),
        description: sample_local_text("Scheduled session"),
        declared_value_minor: value,
        details: Some(Details {
            extension: SCHEDULING_EXTENSION.to_string(),
            schema: SCHEDULING_SLOT_SCHEMA.to_string(),
            data: json!({
                "listing_id": listing_id,
                "slot_id": slot_id,
            }),
            extra: Vec::new(),
        }),
        window: Some(window),
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }
}

fn sample_deal_terms(commitments: Vec<Commitment>) -> DealTerms {
    DealTerms {
        currency: "INR".to_string(),
        quote_expires_at_secs: 2_000_000,
        home_exchange: None,
        commitments,
        listing_id: None,
        scope: sample_local_text("High school mathematics tutoring"),
        cancellation_terms: sample_local_text("Free cancellation up to 24h before start"),
        refund_terms: sample_local_text("Full refund if session is cancelled"),
        dispute_path: sample_local_text("Direct peer resolution or platform mediation"),
    }
}

#[test]
fn every_giver_can_give_every_built_resource_kind() {
    let roles = [DealRole::QuoteAuthor, DealRole::Counterparty];
    for author_giver in roles {
        let counterparty_giver = match author_giver {
            DealRole::QuoteAuthor => DealRole::Counterparty,
            DealRole::Counterparty => DealRole::QuoteAuthor,
        };

        // ServiceInSlot
        let c1 = sample_slot_commitment("c1", author_giver, 100);
        let c2 = sample_currency_commitment("c2", counterparty_giver, 100);
        assert!(sample_deal_terms(vec![c1, c2]).validate().is_ok());

        // GoodsOrService
        let c1 = sample_service_commitment("c1", author_giver, 100);
        let c2 = sample_currency_commitment("c2", counterparty_giver, 100);
        assert!(sample_deal_terms(vec![c1, c2]).validate().is_ok());

        // Currency
        let c1 = sample_currency_commitment("c1", author_giver, 100);
        let c2 = sample_service_commitment("c2", counterparty_giver, 100);
        assert!(sample_deal_terms(vec![c1, c2]).validate().is_ok());
    }
}

#[test]
fn commitment_count_table() {
    // 0 commitments
    let terms_0 = sample_deal_terms(Vec::new());
    assert_eq!(terms_0.validate().unwrap_err().code(), "commitment-count");

    // 1 commitment
    let terms_1 =
        sample_deal_terms(vec![sample_service_commitment("c1", DealRole::QuoteAuthor, 100)]);
    assert!(terms_1.validate().is_ok());

    // 16 commitments
    let mut comms_16 = Vec::new();
    for i in 0..MAX_COMMITMENTS {
        let id = commitment_id(i);
        comms_16.push(sample_service_commitment(&id, DealRole::QuoteAuthor, 10));
    }
    let terms_16 = sample_deal_terms(comms_16);
    assert!(terms_16.validate().is_ok());

    // 17 commitments
    let mut comms_17 = Vec::new();
    for i in 0..17 {
        let id = commitment_id(i);
        comms_17.push(sample_service_commitment(&id, DealRole::QuoteAuthor, 10));
    }
    let terms_17 = sample_deal_terms(comms_17);
    assert_eq!(terms_17.validate().unwrap_err().code(), "commitment-count");
}

#[test]
fn missing_or_wrong_commitment_fields_are_refused() {
    // ServiceInSlot missing extension
    let mut c1 = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    c1.extension = None;
    assert_eq!(
        sample_deal_terms(vec![c1]).validate().unwrap_err().code(),
        "commitment-kind-not-supported"
    );

    // Kind mismatch: ServiceInSlot with Currency kind
    let mut c2 = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    c2.kind = CommitmentKind::Currency;
    assert_eq!(sample_deal_terms(vec![c2]).validate().unwrap_err().code(), "kind-mismatch");
}

#[test]
fn commitment_ids_must_follow_their_position() {
    let c1 = sample_service_commitment("c2", DealRole::QuoteAuthor, 100);
    assert_eq!(sample_deal_terms(vec![c1]).validate().unwrap_err().code(), "commitment-id");

    let c1_ok = sample_service_commitment("c1", DealRole::QuoteAuthor, 100);
    let c2_wrong = sample_service_commitment("c3", DealRole::Counterparty, 100);
    assert_eq!(
        sample_deal_terms(vec![c1_ok, c2_wrong]).validate().unwrap_err().code(),
        "commitment-id"
    );
}

#[test]
fn unknown_currency_is_refused() {
    let mut terms =
        sample_deal_terms(vec![sample_service_commitment("c1", DealRole::QuoteAuthor, 100)]);
    terms.currency = "INVALID_CURRENCY".to_string();
    assert_eq!(terms.validate().unwrap_err().code(), "currency-unknown");
}

#[test]
fn declared_value_range() {
    // -1 value
    let c_neg = sample_service_commitment("c1", DealRole::QuoteAuthor, -1);
    assert_eq!(
        sample_deal_terms(vec![c_neg]).validate().unwrap_err().code(),
        "declared-value-range"
    );

    // 0 value on service: OK
    let c_zero = sample_service_commitment("c1", DealRole::QuoteAuthor, 0);
    assert!(sample_deal_terms(vec![c_zero]).validate().is_ok());

    // 0 value on currency: refused
    let c_curr_zero = sample_currency_commitment("c1", DealRole::QuoteAuthor, 0);
    assert_eq!(
        sample_deal_terms(vec![c_curr_zero]).validate().unwrap_err().code(),
        "currency-value-zero"
    );

    // 10^15: OK
    let c_max = sample_service_commitment("c1", DealRole::QuoteAuthor, MAX_DECLARED_VALUE_MINOR);
    assert!(sample_deal_terms(vec![c_max]).validate().is_ok());

    // 10^15 + 1: refused
    let c_over =
        sample_service_commitment("c1", DealRole::QuoteAuthor, MAX_DECLARED_VALUE_MINOR + 1);
    assert_eq!(
        sample_deal_terms(vec![c_over]).validate().unwrap_err().code(),
        "declared-value-range"
    );
}

#[test]
fn currency_only_fields_on_a_service_are_refused() {
    let mut c = sample_service_commitment("c1", DealRole::QuoteAuthor, 100);
    c.payee = Some("did:key:z6MkuPayee".to_string());
    assert_eq!(sample_deal_terms(vec![c]).validate().unwrap_err().code(), "currency-only-field");

    let mut c2 = sample_service_commitment("c1", DealRole::QuoteAuthor, 100);
    c2.tax_minor = Some(10);
    assert_eq!(sample_deal_terms(vec![c2]).validate().unwrap_err().code(), "currency-only-field");
}

#[test]
fn payee_is_required_on_money() {
    let mut c = sample_currency_commitment("c1", DealRole::QuoteAuthor, 100);
    c.payee = None;
    assert_eq!(sample_deal_terms(vec![c]).validate().unwrap_err().code(), "payee-empty");

    let mut c_empty = sample_currency_commitment("c1", DealRole::QuoteAuthor, 100);
    c_empty.payee = Some("   ".to_string());
    assert_eq!(sample_deal_terms(vec![c_empty]).validate().unwrap_err().code(), "payee-empty");
}

#[test]
fn tax_and_fees_never_exceed_the_value() {
    let mut c = sample_currency_commitment("c1", DealRole::QuoteAuthor, 100);
    c.tax_minor = Some(80);
    c.fees_minor = Some(30); // 80 + 30 = 110 > 100
    assert_eq!(
        sample_deal_terms(vec![c]).validate().unwrap_err().code(),
        "breakdown-exceeds-value"
    );

    let mut c_neg_tax = sample_currency_commitment("c1", DealRole::QuoteAuthor, 100);
    c_neg_tax.tax_minor = Some(-5);
    assert_eq!(
        sample_deal_terms(vec![c_neg_tax]).validate().unwrap_err().code(),
        "breakdown-exceeds-value"
    );
}

#[test]
fn after_names_known_ids_with_no_cycle() {
    // Unknown after id
    let mut c1 = sample_service_commitment("c1", DealRole::QuoteAuthor, 100);
    c1.after = vec!["c3".to_string()];
    let c2 = sample_currency_commitment("c2", DealRole::Counterparty, 100);
    assert_eq!(sample_deal_terms(vec![c1, c2]).validate().unwrap_err().code(), "after-unknown");

    // Self cycle
    let mut c1_self = sample_service_commitment("c1", DealRole::QuoteAuthor, 100);
    c1_self.after = vec!["c1".to_string()];
    assert_eq!(sample_deal_terms(vec![c1_self]).validate().unwrap_err().code(), "after-cycle");

    // Indirect cycle (c1 -> c2 and c2 -> c1)
    let mut c1_cyc = sample_service_commitment("c1", DealRole::QuoteAuthor, 100);
    c1_cyc.after = vec!["c2".to_string()];
    let mut c2_cyc = sample_currency_commitment("c2", DealRole::Counterparty, 100);
    c2_cyc.after = vec!["c1".to_string()];
    assert_eq!(
        sample_deal_terms(vec![c1_cyc, c2_cyc]).validate().unwrap_err().code(),
        "after-cycle"
    );

    // Valid acyclic order (c2 after c1)
    let c1_ok = sample_service_commitment("c1", DealRole::QuoteAuthor, 100);
    let mut c2_ok = sample_currency_commitment("c2", DealRole::Counterparty, 100);
    c2_ok.after = vec!["c1".to_string()];
    assert!(sample_deal_terms(vec![c1_ok, c2_ok]).validate().is_ok());
}

#[test]
fn scheduling_details_must_name_the_slot_of_the_window() {
    let mut c = sample_slot_commitment("c1", DealRole::QuoteAuthor, 100);
    let details = c.details.as_mut().unwrap();
    details.data = json!({
        "listing_id": "listing_123",
        "slot_id": "wrong_slot_id",
    });
    assert_eq!(sample_deal_terms(vec![c]).validate().unwrap_err().code(), "slot-details");
}

#[test]
fn every_text_shares_one_language_and_one_translation_set() {
    let mut terms =
        sample_deal_terms(vec![sample_service_commitment("c1", DealRole::QuoteAuthor, 100)]);

    // Scope in Hindi while cancellation terms in English
    terms.scope.lang = sample_tag("hi");
    assert_eq!(terms.validate().unwrap_err().code(), "text-lang-mismatch");

    // Same original language (English), but different translations set
    terms.scope.lang = sample_tag("en");
    terms.scope.translations = vec![TextTranslation {
        lang: sample_tag("hi"),
        text: "हिंदी अनुवाद".to_string(),
    }];
    assert_eq!(terms.validate().unwrap_err().code(), "translation-set-mismatch");

    // Matching translation sets across all texts: OK
    let trans = vec![TextTranslation {
        lang: sample_tag("hi"), text: "हिंदी पाठ".to_string()
    }];
    terms.scope.translations = trans.clone();
    terms.cancellation_terms.translations = trans.clone();
    terms.refund_terms.translations = trans.clone();
    terms.dispute_path.translations = trans.clone();
    terms.commitments[0].description.translations = trans;
    assert!(terms.validate().is_ok());
}

#[test]
fn a_quote_with_and_without_money_validate_the_same_way() {
    // Free service (no currency commitment)
    let free_deal =
        sample_deal_terms(vec![sample_service_commitment("c1", DealRole::QuoteAuthor, 0)]);
    assert!(free_deal.validate().is_ok());

    // Paid service (service + currency)
    let paid_deal = sample_deal_terms(vec![
        sample_service_commitment("c1", DealRole::QuoteAuthor, 100),
        sample_currency_commitment("c2", DealRole::Counterparty, 100),
    ]);
    assert!(paid_deal.validate().is_ok());
}

#[test]
fn answers_enquiry_and_want_variants_serialize_consistently() {
    let enquiry_answers = Answers::Enquiry { record_id: "rec_enquiry_123".to_string() };
    let want_answers = Answers::Want { intent_id: "int_want_456".to_string(), version: 1 };

    let enq_json = serde_json::to_value(&enquiry_answers).expect("serialize enquiry");
    assert_eq!(enq_json["kind"], "enquiry");
    assert_eq!(enq_json["record_id"], "rec_enquiry_123");

    let want_json = serde_json::to_value(&want_answers).expect("serialize want");
    assert_eq!(want_json["kind"], "want");
    assert_eq!(want_json["intent_id"], "int_want_456");
    assert_eq!(want_json["version"], 1);
}

#[test]
fn quote_for_an_enquiry_and_for_a_want_give_equal_receipts() {
    let cp_did = "did:key:z6MkuCounterparty";
    let author_did = "did:key:z6MkuAuthor";
    let terms = sample_deal_terms(vec![
        sample_service_commitment("c1", DealRole::QuoteAuthor, 100),
        sample_currency_commitment("c2", DealRole::Counterparty, 100),
    ]);
    let signed_terms =
        terms.conceal(&mut sd::seeded_salts(42)).expect("conceal terms for test").signed;

    let quote_enquiry = QuotePayload {
        quote_id: "quo_1".to_string(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enquiry_123".to_string() },
        counterparty_did: cp_did.to_string(),
        terms: signed_terms.clone(),
    };
    let quote_want = QuotePayload {
        quote_id: "quo_2".to_string(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Want { intent_id: "int_want_456".to_string(), version: 1 },
        counterparty_did: cp_did.to_string(),
        terms: signed_terms.clone(),
    };
    assert_ne!(quote_enquiry.answers, quote_want.answers);
    let rec_enquiry_id = "rec_quote_enquiry";
    let rec_want_id = "rec_quote_want";

    let receipt_enq = AgreementReceiptPayload {
        quote_record_id: rec_enquiry_id.to_string(),
        counterparty_did: cp_did.to_string(),
        quote_author_did: author_did.to_string(),
        role: DealRole::Counterparty,
        terms: signed_terms.clone(),
        viewed_lang: sample_tag("en"),
        viewed_via_mt: false,
    };
    let receipt_want = AgreementReceiptPayload {
        quote_record_id: rec_want_id.to_string(),
        counterparty_did: cp_did.to_string(),
        quote_author_did: author_did.to_string(),
        role: DealRole::Counterparty,
        terms: signed_terms,
        viewed_lang: sample_tag("en"),
        viewed_via_mt: false,
    };

    assert_ne!(receipt_enq.quote_record_id, receipt_want.quote_record_id);
    let mut receipt_want_matching = receipt_want.clone();
    receipt_want_matching.quote_record_id = receipt_enq.quote_record_id.clone();
    assert_eq!(receipt_enq, receipt_want_matching);

    let snap_enq = crate::progress::open(
        rec_enquiry_id.to_string(),
        "conv_1".to_string(),
        cp_did.to_string(),
        author_did.to_string(),
        &terms,
        crate::progress::FormedAs::Quote,
        None,
    );
    let snap_want = crate::progress::open(
        rec_want_id.to_string(),
        "conv_1".to_string(),
        cp_did.to_string(),
        author_did.to_string(),
        &terms,
        crate::progress::FormedAs::Quote,
        None,
    );
    assert_ne!(snap_enq.agreement, snap_want.agreement);
    let mut snap_want_matching = snap_want.clone();
    snap_want_matching.agreement = snap_enq.agreement.clone();
    assert_eq!(snap_enq, snap_want_matching);
}

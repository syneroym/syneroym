use serde_json::json;
use syneroym_identity::substrate::canonicalize_json_value;
use syneroym_signed_record::seeded_salt;

use super::*;
use crate::{
    deal::{
        Commitment, CommitmentKind, Details, DetailsExtra, ExtensionRef, LocalText, ResourceKind,
        SettlementRole, TextTranslation, TimeWindow,
    },
    lang::LangTag,
    progress,
    testing::sd::{self, PlainFulfilment, PlainPaymentAck, PlainPaymentRequest},
    transaction::{AgreementReceiptPayload, Answers, DealRole, QuotePayload},
};

fn tag(s: &str) -> LangTag {
    LangTag::parse(s).expect("valid lang tag")
}

fn sample_terms_with_markers() -> DealTerms {
    DealTerms {
        currency: "INR".to_string(),
        quote_expires_at_secs: 1_800_000_000,
        home_exchange: None,
        commitments: vec![
            Commitment {
                commitment_id: "c1".to_string(),
                giver: DealRole::QuoteAuthor,
                resource_kind: ResourceKind::GoodsOrService,
                kind: CommitmentKind::Fulfilment,
                extension: None,
                after: vec![],
                description: LocalText::plain(tag("en"), "scope-marker-71 work"),
                declared_value_minor: 987654321,
                details: None,
                window: Some(TimeWindow {
                    earliest_secs: 1_700_000_000,
                    latest_secs: 1_700_003_600,
                }),
                location: None,
                payee: None,
                payment_methods: None,
                tax_minor: None,
                fees_minor: None,
                eep: None,
            },
            Commitment {
                commitment_id: "c2".to_string(),
                giver: DealRole::Counterparty,
                resource_kind: ResourceKind::Currency,
                kind: CommitmentKind::Currency,
                extension: None,
                after: vec!["c1".to_string()],
                description: LocalText::plain(tag("en"), "payment description"),
                declared_value_minor: 987654321,
                details: None,
                window: None,
                location: None,
                payee: Some("payee-marker-72".to_string()),
                payment_methods: Some(vec!["cash".to_string()]),
                tax_minor: Some(0),
                fees_minor: Some(0),
                eep: None,
            },
        ],
        listing_id: None,
        scope: LocalText::plain(tag("en"), "scope-marker-71"),
        cancellation_terms: LocalText::plain(tag("en"), "No refund"),
        refund_terms: LocalText::plain(tag("en"), "No refund"),
        dispute_path: LocalText::plain(tag("en"), "Direct mediation"),
    }
}

#[test]
fn no_signed_payload_holds_a_hidden_value() {
    let terms = sample_terms_with_markers();
    let mut salts = (0..).map(|n| seeded_salt(10, n));
    let concealed = terms.conceal(&mut salts).unwrap();

    let signed_terms_canonical = serde_json::to_string(&canonicalize_json_value(
        &serde_json::to_value(&concealed.signed).unwrap(),
    ))
    .unwrap();

    let quote = QuotePayload {
        quote_id: "quo_1".to_string(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
        counterparty_did: "did:key:zCounterparty".to_string(),
        terms: concealed.signed.clone(),
    };
    let quote_canonical =
        serde_json::to_string(&canonicalize_json_value(&serde_json::to_value(&quote).unwrap()))
            .unwrap();

    let author_half = AgreementReceiptPayload {
        quote_record_id: "rec_quote_1".to_string(),
        counterparty_did: "did:key:zCounterparty".to_string(),
        quote_author_did: "did:key:zAuthor".to_string(),
        role: DealRole::QuoteAuthor,
        viewed_lang: tag("en"),
        viewed_via_mt: false,
        terms: concealed.signed.clone(),
    };
    let author_half_canonical = serde_json::to_string(&canonicalize_json_value(
        &serde_json::to_value(&author_half).unwrap(),
    ))
    .unwrap();

    let cp_half = AgreementReceiptPayload {
        quote_record_id: "rec_quote_1".to_string(),
        counterparty_did: "did:key:zCounterparty".to_string(),
        quote_author_did: "did:key:zAuthor".to_string(),
        role: DealRole::Counterparty,
        viewed_lang: tag("en"),
        viewed_via_mt: false,
        terms: concealed.signed.clone(),
    };
    let cp_half_canonical =
        serde_json::to_string(&canonicalize_json_value(&serde_json::to_value(&cp_half).unwrap()))
            .unwrap();

    let snap = progress::open(
        "rec_agr_1".to_string(),
        "conv_1".to_string(),
        "did:key:zCounterparty".to_string(),
        "did:key:zAuthor".to_string(),
        &terms,
        None,
    );
    let snap_canonical =
        serde_json::to_string(&canonicalize_json_value(&serde_json::to_value(&snap).unwrap()))
            .unwrap();

    let markers = [
        "scope-marker-71",
        "987654321",
        "payee-marker-72",
        "1700000000",
        "1700003600",
        "note-marker-73",
        "method-marker",
        "ref-marker-74",
    ];

    let payloads = [
        ("signed_terms", &signed_terms_canonical),
        ("quote", &quote_canonical),
        ("author_half", &author_half_canonical),
        ("cp_half", &cp_half_canonical),
        ("snapshot", &snap_canonical),
    ];

    for (name, canon) in payloads {
        for marker in &markers[..5] {
            assert!(!canon.contains(marker), "{name} leaked hidden marker: {marker}");
        }
    }

    // Derived track window end: latest_secs (1_700_003_600) + TRACK_WINDOW_SECS (30
    // days = 2_592_000) = 1_702_595_600
    let track_end_str = (1_700_003_600u64 + 30 * 86400).to_string();
    assert!(!snap_canonical.contains(&track_end_str), "snapshot leaked track end");
    assert!(!signed_terms_canonical.contains(&track_end_str));

    // Settlement payloads
    let pay_req = sd::conceal_payment_request(
        PlainPaymentRequest {
            agreement: "rec_agr".to_string(),
            conversation: "conv".to_string(),
            commitment_id: "c1".to_string(),
            quote_author_did: "did:key:zAuthor".to_string(),
            counterparty_did: "did:key:zCounterparty".to_string(),
            note: Some(LocalText::plain(tag("en"), "note-marker-73")),
        },
        50,
    )
    .0;
    let req_canon =
        serde_json::to_string(&canonicalize_json_value(&serde_json::to_value(&pay_req).unwrap()))
            .unwrap();
    assert!(!req_canon.contains("note-marker-73"));

    let pay_ack = sd::conceal_payment_ack(
        PlainPaymentAck {
            agreement: "rec_agr".to_string(),
            conversation: "conv".to_string(),
            commitment_id: "c1".to_string(),
            quote_author_did: "did:key:zAuthor".to_string(),
            counterparty_did: "did:key:zCounterparty".to_string(),
            role: SettlementRole::Giver,
            observed_at_secs: 1000,
            method: Some("method-marker".to_string()),
            reference: Some("ref-marker-74".to_string()),
            settlement_note: Some(LocalText::plain(tag("en"), "note-marker-73")),
        },
        51,
    )
    .0;
    let ack_canon =
        serde_json::to_string(&canonicalize_json_value(&serde_json::to_value(&pay_ack).unwrap()))
            .unwrap();
    assert!(!ack_canon.contains("method-marker"));
    assert!(!ack_canon.contains("ref-marker-74"));
    assert!(!ack_canon.contains("note-marker-73"));

    let ful = sd::conceal_fulfilment(
        PlainFulfilment {
            agreement: "rec_agr".to_string(),
            conversation: "conv".to_string(),
            commitment_id: "c1".to_string(),
            counterparty_did: "did:key:zCounterparty".to_string(),
            quote_author_did: "did:key:zAuthor".to_string(),
            role: SettlementRole::Giver,
            settlement_note: Some(LocalText::plain(tag("en"), "note-marker-73")),
        },
        52,
    )
    .0;
    let ful_canon =
        serde_json::to_string(&canonicalize_json_value(&serde_json::to_value(&ful).unwrap()))
            .unwrap();
    assert!(!ful_canon.contains("note-marker-73"));
}

#[test]
fn every_object_always_has_all_its_digests() {
    let terms = sample_terms_with_markers();
    let mut salts = (0..).map(|n| seeded_salt(11, n));
    let concealed = terms.conceal(&mut salts).unwrap();

    assert_eq!(concealed.signed.sd.len(), 5);
    for comm in &concealed.signed.commitments {
        assert_eq!(comm.sd.len(), 10);
    }
}

#[test]
fn strict_reveal_gives_back_the_same_terms() {
    let terms = sample_terms_with_markers();
    let mut salts = (0..).map(|n| seeded_salt(12, n));
    let concealed = terms.conceal(&mut salts).unwrap();

    let revealed = reveal_terms_strict(&concealed.signed, &concealed.disclosures).unwrap();
    assert_eq!(revealed, terms);
}

#[test]
fn both_halves_copy_the_quote_terms_byte_for_byte() {
    let terms = sample_terms_with_markers();
    let mut salts = (0..).map(|n| seeded_salt(13, n));
    let concealed = terms.conceal(&mut salts).unwrap();

    let quote_terms_bytes = serde_json::to_vec(&concealed.signed).unwrap();
    let author_half_terms = concealed.signed.clone();
    let cp_half_terms = concealed.signed.clone();

    assert_eq!(serde_json::to_vec(&author_half_terms).unwrap(), quote_terms_bytes);
    assert_eq!(serde_json::to_vec(&cp_half_terms).unwrap(), quote_terms_bytes);
}

#[test]
fn details_over_8_kib_is_refused() {
    let mut terms = sample_terms_with_markers();
    // Add large extra data into details of a scheduling commitment
    terms.commitments[0].resource_kind = ResourceKind::ServiceInSlot;
    terms.commitments[0].kind = CommitmentKind::Fulfilment;
    terms.commitments[0].extension =
        Some(ExtensionRef { id: "scheduling".to_string(), version: 1 });
    // Create details > 8 KiB
    let big_value = "v".repeat(8192);
    terms.commitments[0].details = Some(Details {
        extension: "scheduling".to_string(),
        schema: "scheduling/slot@1".to_string(),
        data: json!({
            "listing_id": "lis_1",
            "slot_id": "s_1"
        }),
        extra: vec![DetailsExtra { label: "notes".to_string(), value: big_value }],
    });

    let mut salts = (0..).map(|n| seeded_salt(14, n));
    let err = terms.conceal(&mut salts).unwrap_err();
    assert_eq!(err.code(), "details-too-large");
}

#[test]
fn disclosures_over_96_kib_are_refused() {
    let mut terms = sample_terms_with_markers();
    terms.scope = LocalText {
        lang: tag("en"),
        text: "x".repeat(3000),
        translations: vec![
            TextTranslation { lang: tag("fr"), text: "x".repeat(3000) },
            TextTranslation { lang: tag("es"), text: "x".repeat(3000) },
            TextTranslation { lang: tag("de"), text: "x".repeat(3000) },
            TextTranslation { lang: tag("it"), text: "x".repeat(3000) },
        ],
    };

    // Add 16 commitments with long descriptions and translations
    let mut commitments = Vec::new();
    for i in 0..16 {
        commitments.push(Commitment {
            commitment_id: format!("c{}", i + 1),
            giver: DealRole::QuoteAuthor,
            resource_kind: ResourceKind::GoodsOrService,
            kind: CommitmentKind::Fulfilment,
            extension: None,
            after: vec![],
            description: LocalText {
                lang: tag("en"),
                text: "d".repeat(2000),
                translations: vec![
                    TextTranslation { lang: tag("fr"), text: "d".repeat(2000) },
                    TextTranslation { lang: tag("es"), text: "d".repeat(2000) },
                    TextTranslation { lang: tag("de"), text: "d".repeat(2000) },
                    TextTranslation { lang: tag("it"), text: "d".repeat(2000) },
                ],
            },
            declared_value_minor: 100,
            details: None,
            window: None,
            location: None,
            payee: None,
            payment_methods: None,
            tax_minor: None,
            fees_minor: None,
            eep: None,
        });
    }
    terms.commitments = commitments;

    let mut salts = (0..).map(|n| seeded_salt(15, n));
    let err = terms.conceal(&mut salts).unwrap_err();
    assert_eq!(err.code(), "quote-too-large");
}

#[test]
fn disclosure_size_equals_the_concealed_size() {
    let terms = sample_terms_with_markers();
    let measured = terms.disclosure_size().unwrap();

    let mut salts = (0..).map(|n| seeded_salt(16, n));
    let concealed = terms.conceal(&mut salts).unwrap();

    let total_actual: usize = concealed.disclosures.iter().map(|d| d.len()).sum();
    assert_eq!(measured.total_bytes, total_actual);
}

#[test]
fn visible_checks_refuse_a_bad_sd_list() {
    let terms = sample_terms_with_markers();
    let mut salts = (0..).map(|n| seeded_salt(17, n));
    let concealed = terms.conceal(&mut salts).unwrap();

    // 1. Wrong length (4 instead of 5)
    let mut bad_len = concealed.signed.clone();
    bad_len.sd.pop();
    let err = bad_len.validate_visible().unwrap_err();
    assert_eq!(err.code(), "sd-shape");

    // 2. Unsorted digests
    let mut bad_order = concealed.signed.clone();
    bad_order.sd.swap(0, 1);
    if bad_order.sd[0] >= bad_order.sd[1] {
        let err = bad_order.validate_visible().unwrap_err();
        assert_eq!(err.code(), "sd-shape");
    }

    // 3. Bad digest length (42 chars instead of 43)
    let mut bad_char_len = concealed.signed.clone();
    bad_char_len.sd[0] = "a".repeat(42);
    let err = bad_char_len.validate_visible().unwrap_err();
    assert_eq!(err.code(), "sd-shape");

    // 4. Bad characters (+ or =)
    let mut bad_chars = concealed.signed.clone();
    bad_chars.sd[0] = format!("{}+=", &bad_chars.sd[0][..41]);
    let err = bad_chars.validate_visible().unwrap_err();
    assert_eq!(err.code(), "sd-shape");
}

#[test]
fn a_digest_in_two_commitments_is_refused() {
    let terms = sample_terms_with_markers();
    let mut salts = (0..).map(|n| seeded_salt(18, n));
    let concealed = terms.conceal(&mut salts).unwrap();
    let mut bad = concealed.signed.clone();
    bad.commitments[1].sd = bad.commitments[0].sd.clone();
    let err = bad.validate_visible().unwrap_err();
    assert_eq!(err.code(), "sd-shape");

    let disclosures = concealed.disclosures[..15].to_vec();
    let err_reveal = reveal_terms_strict(&bad, &disclosures).unwrap_err();
    assert_eq!(err_reveal.code(), "sd-shape");
}

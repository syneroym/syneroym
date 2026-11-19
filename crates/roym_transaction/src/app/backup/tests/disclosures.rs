use std::{collections::BTreeMap, sync::Arc};

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    backup::{
        Bundle, ImportInput, SECTION_AGREEMENTS, SECTION_CARDS, SECTION_DISCLOSURES,
        SECTION_QUOTE_HISTORY, SECTION_QUOTES,
    },
    conversation::Direction,
    deal::{Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind},
    disclosure::{Salt, encode_disclosure},
    lang::LangTag,
    method::NoParams,
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RecordDraft},
    signing,
    testing::sd::{self, PlainQuote},
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, Answers, DealRole, QUOTE_VERSION,
        QuotePayload, ReceiptHalf, derive_quote_id,
    },
};

use crate::app::{
    AgreementRow, CARDS, CardRow, DISCLOSURES, QUOTE_HISTORY, QUOTES, RecordPointerRow,
    SCHEMA_VERSION, backup,
    disclosures::DisclosureRow,
    get_row, put_bytes, put_row,
    tests::{fixture, fixture_with_master},
};

fn sample_deal_terms(now: u64) -> DealTerms {
    DealTerms {
        currency: "EUR".to_string(),
        commitments: vec![
            Commitment {
                commitment_id: "c1".to_string(),
                giver: DealRole::QuoteAuthor,
                resource_kind: ResourceKind::GoodsOrService,
                kind: CommitmentKind::Fulfilment,
                extension: None,
                after: vec![],
                description: LocalText::plain(
                    LangTag::parse_canonical("en").expect("lang"),
                    "Service",
                ),
                declared_value_minor: 1000,
                details: None,
                window: None,
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
                description: LocalText::plain(
                    LangTag::parse_canonical("en").expect("lang"),
                    "Currency",
                ),
                declared_value_minor: 1000,
                details: None,
                window: None,
                location: None,
                payee: Some("X".to_string()),
                payment_methods: None,
                tax_minor: None,
                fees_minor: None,
                eep: None,
            },
        ],
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Scope"),
        cancellation_terms: LocalText::plain(
            LangTag::parse_canonical("en").expect("lang"),
            "Terms",
        ),
        refund_terms: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Refund"),
        dispute_path: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Dispute"),
        quote_expires_at_secs: now + 3600,
    }
}

fn make_quote_with_disclosures(
    author: &Identity,
    cp_did: &str,
    now: u64,
) -> (String, String, QuotePayload, Vec<String>) {
    let author_did = substrate::derive_did_key(&author.public_key());
    let quote_id = derive_quote_id("conv_1", &author_did, 1).expect("quote id");
    let terms = sample_deal_terms(now);
    let plain = PlainQuote {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
        counterparty_did: cp_did.to_string(),
        terms,
    };
    let (payload, disclosures) = sd::conceal_quote(plain, 1);
    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: "quote".to_string(),
        subject: "conv_1".to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: Some(now + 3600),
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, author_did, None, now).expect("env");
    let sig = author.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let rec_id = env.record_id().expect("rec id");
    (env.to_json().expect("json"), rec_id, payload, disclosures)
}

fn make_test_receipt_half(
    signer: &Identity,
    role: DealRole,
    quote_rec_id: &str,
    quote_payload: &QuotePayload,
    quote_author_did: &str,
    now: u64,
) -> (String, String) {
    let signer_did = substrate::derive_did_key(&signer.public_key());
    let payload = AgreementReceiptPayload {
        role,
        quote_record_id: quote_rec_id.to_string(),
        counterparty_did: quote_payload.counterparty_did.clone(),
        quote_author_did: quote_author_did.to_string(),
        terms: quote_payload.terms.clone(),
        viewed_lang: LangTag::parse_canonical("en").expect("lang"),
        viewed_via_mt: false,
    };
    let draft = RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: quote_rec_id.to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: None,
        supersedes: None,
    };
    let (mut env, bytes) = Envelope::unsigned(draft, signer_did, None, now).expect("env");
    let sig = signer.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let rec_id = env.record_id().expect("rec id");
    (env.to_json().expect("json"), rec_id)
}

#[tokio::test]
async fn export_import_keeps_disclosures() {
    let f1 = fixture();
    let now = f1.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());

    let (q_env, q_rec_id, q_payload, disclosures) =
        make_quote_with_disclosures(&f1.master, &cp_did, now);

    put_bytes(&f1.host, QUOTE_HISTORY, &q_rec_id, q_env.as_bytes()).await.unwrap();
    let q_ptr = RecordPointerRow {
        envelope: q_env.clone(),
        record_id: q_rec_id.clone(),
        id: q_payload.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: f1.master_did.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(cp_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    put_row(&f1.host, QUOTES, &q_payload.quote_id, &q_ptr).await.unwrap();

    let disc_row = DisclosureRow {
        record_id: q_rec_id.clone(),
        record_type: "quote".to_string(),
        disclosures: disclosures.clone(),
        stored_at_secs: now,
    };
    put_row(&f1.host, DISCLOSURES, &q_rec_id, &disc_row).await.unwrap();

    let bundle = backup::export(&f1.host, NoParams {}).await.unwrap();

    assert!(bundle.sections.contains_key("disclosures"));

    let f2 = fixture_with_master(Arc::clone(&f1.master));
    let import_output = backup::import(&f2.host, ImportInput { bundle }).await.unwrap();
    assert!(import_output.imported);

    let stored: Option<DisclosureRow> = get_row(&f2.host, DISCLOSURES, &q_rec_id).await.unwrap();
    assert!(stored.is_some());
    assert_eq!(stored.unwrap().disclosures, disclosures);
}

#[tokio::test]
async fn import_rebuilds_card_data_from_disclosures() {
    let f1 = fixture();
    let now = f1.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());

    let (q_env, q_rec_id, q_payload, disclosures) =
        make_quote_with_disclosures(&f1.master, &cp_did, now);

    let mut sections = BTreeMap::new();
    let q_ptr = RecordPointerRow {
        envelope: q_env.clone(),
        record_id: q_rec_id.clone(),
        id: q_payload.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: f1.master_did.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(cp_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    sections.insert(
        SECTION_QUOTES.to_string(),
        vec![json!({ "id": q_payload.quote_id, "payload": q_ptr })],
    );
    sections.insert(
        SECTION_QUOTE_HISTORY.to_string(),
        vec![json!({ "id": q_rec_id, "payload": { "envelope": q_env } })],
    );
    let disc_row = DisclosureRow {
        record_id: q_rec_id.clone(),
        record_type: "quote".to_string(),
        disclosures: disclosures.clone(),
        stored_at_secs: now,
    };
    sections.insert(
        SECTION_DISCLOSURES.to_string(),
        vec![json!({ "id": q_rec_id, "payload": disc_row })],
    );
    let card_row = CardRow {
        message_id: "card_quote_1".to_string(),
        conversation: "conv_1".to_string(),
        direction: Direction::Outgoing,
        sender_timestamp_ms: (now * 1000) as i64,
        card_type: "quote".to_string(),
        version: 1,
        known: true,
        verified: false,
        expired: false,
        repeat: false,
        reason: None,
        issuer: Some(f1.master_did.clone()),
        record_id: Some(q_rec_id.clone()),
        revocation_status: None,
        data: None,
        stored_at_secs: now,
        declined: None,
        version_count: Some(1),
        deal_state: None,
        agreement_payee: None,
        agreement_payment_methods: Vec::new(),
    };
    sections.insert(
        SECTION_CARDS.to_string(),
        vec![json!({ "id": "card_quote_1", "payload": card_row })],
    );

    let mut bundle = Bundle::new(f1.master_did.clone(), SCHEMA_VERSION, sections).unwrap();
    signing::sign_bundle(&f1.host, &mut bundle, now).await.unwrap();

    let f2 = fixture_with_master(Arc::clone(&f1.master));
    let res = backup::import(&f2.host, ImportInput { bundle }).await.unwrap();
    assert!(res.imported);

    let imported_card: Option<CardRow> = get_row(&f2.host, CARDS, "card_quote_1").await.unwrap();
    assert!(imported_card.is_some());
    let card = imported_card.unwrap();
    assert!(card.verified);
    assert!(card.data.is_some());
    let data = card.data.unwrap();
    assert_eq!(data["terms"]["currency"], "EUR");
    assert!(data["terms"].get("_sd").is_none());
}

#[tokio::test]
async fn import_refuses_disclosures_that_match_no_digest() {
    let f1 = fixture();
    let now = f1.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());

    let (q_env, q_rec_id, q_payload, _disclosures) =
        make_quote_with_disclosures(&f1.master, &cp_did, now);

    let mut sections = BTreeMap::new();
    let q_ptr = RecordPointerRow {
        envelope: q_env.clone(),
        record_id: q_rec_id.clone(),
        id: q_payload.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: f1.master_did.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(cp_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    sections.insert(
        SECTION_QUOTES.to_string(),
        vec![json!({ "id": q_payload.quote_id, "payload": q_ptr })],
    );
    sections.insert(
        SECTION_QUOTE_HISTORY.to_string(),
        vec![json!({ "id": q_rec_id, "payload": { "envelope": q_env } })],
    );

    let salt = Salt::from_bytes([0x42; 16]);
    let bogus_disclosure =
        encode_disclosure(&salt, "bogus_field", &json!("bogus_value")).expect("encode disclosure");
    let disc_row = DisclosureRow {
        record_id: q_rec_id.clone(),
        record_type: "quote".to_string(),
        disclosures: vec![bogus_disclosure],
        stored_at_secs: now,
    };
    sections.insert(
        SECTION_DISCLOSURES.to_string(),
        vec![json!({ "id": q_rec_id, "payload": disc_row })],
    );

    let mut bundle = Bundle::new(f1.master_did.clone(), SCHEMA_VERSION, sections).unwrap();
    signing::sign_bundle(&f1.host, &mut bundle, now).await.unwrap();

    let f2 = fixture_with_master(Arc::clone(&f1.master));
    let res = backup::import(&f2.host, ImportInput { bundle }).await;
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.error.as_ref().unwrap().message.contains("disclosure unknown digest"));
}

#[tokio::test]
async fn import_refuses_agreement_terms_that_differ_from_the_quote() {
    let f1 = fixture();
    let now = f1.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());

    let (q_env, q_rec_id, q_payload, disclosures) =
        make_quote_with_disclosures(&f1.master, &cp_did, now);

    let (cp_half_env, cp_half_rec_id) = make_test_receipt_half(
        &cp,
        DealRole::Counterparty,
        &q_rec_id,
        &q_payload,
        &f1.master_did,
        now,
    );

    let mut sections = BTreeMap::new();
    let q_ptr = RecordPointerRow {
        envelope: q_env.clone(),
        record_id: q_rec_id.clone(),
        id: q_payload.quote_id.clone(),
        conversation: "conv_1".to_string(),
        sequence: 1,
        issuer: f1.master_did.clone(),
        mine: true,
        updated_at_secs: now,
        version_count: 1,
        issued_at_secs: now,
        request_record_id: None,
        counterparty_did: Some(cp_did.clone()),
        declined_at_secs: None,
        decline_note: None,
    };
    sections.insert(
        SECTION_QUOTES.to_string(),
        vec![json!({ "id": q_payload.quote_id, "payload": q_ptr })],
    );
    sections.insert(
        SECTION_QUOTE_HISTORY.to_string(),
        vec![json!({ "id": q_rec_id, "payload": { "envelope": q_env } })],
    );
    let disc_row = DisclosureRow {
        record_id: q_rec_id.clone(),
        record_type: "quote".to_string(),
        disclosures: disclosures.clone(),
        stored_at_secs: now,
    };
    sections.insert(
        SECTION_DISCLOSURES.to_string(),
        vec![json!({ "id": q_rec_id, "payload": disc_row })],
    );

    let mut modified_terms = sample_deal_terms(now);
    modified_terms.currency = "USD".to_string();

    let agr_row = AgreementRow {
        quote_record_id: q_rec_id.clone(),
        quote_id: q_payload.quote_id.clone(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_did.clone(),
        quote_author_did: f1.master_did.clone(),
        terms: modified_terms,
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: Vec::new(),
        agreed_at_secs: Some(now),
        counterparty: Some(ReceiptHalf {
            envelope: cp_half_env,
            record_id: cp_half_rec_id,
            issuer: cp_did.clone(),
            issued_at_secs: now,
        }),
        quote_author: None,
        updated_at_secs: now,
    };
    sections.insert(
        SECTION_AGREEMENTS.to_string(),
        vec![json!({ "id": q_rec_id, "payload": agr_row })],
    );

    let mut bundle = Bundle::new(f1.master_did.clone(), SCHEMA_VERSION, sections).unwrap();
    signing::sign_bundle(&f1.host, &mut bundle, now).await.unwrap();

    let f2 = fixture_with_master(Arc::clone(&f1.master));
    let res = backup::import(&f2.host, ImportInput { bundle }).await;
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.error.as_ref().unwrap().message.contains("agreement terms differ from quote"));
}

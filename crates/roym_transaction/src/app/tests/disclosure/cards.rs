use std::collections::HashSet;

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    card::{self, MAX_CARD_BODY_BYTES},
    clock,
    deal::{Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind, TextTranslation},
    disclosure::{
        MAX_RECORD_DISCLOSURE_BYTES, Salt, decode_disclosure, encode_base64url, encode_disclosure,
    },
    lang::LangTag,
    record::{Envelope, RECORD_AGREEMENT_RECEIPT, RECORD_QUOTE, RecordDraft},
    transaction::{AGREEMENT_RECEIPT_VERSION, Answers, DealRole, QUOTE_VERSION, derive_quote_id},
};
use z32;

use super::helpers::{
    build_and_sign_enquiry, build_and_sign_quote, make_counterparty_receipt, sample_terms_input,
    setup_agreed_currency_deal,
};
use crate::app::{
    AGREEMENTS, AgreementRow, CARDS, CardRow, DISCLOSURES, ENQUIRY_HISTORY, PAYMENTS,
    disclosures::DisclosureRow,
    get_row,
    payment_ops::{self, PaymentAckInput, PaymentRequestInput},
    put_bytes, put_row,
    quote_ops::{self, CommitmentInput, QuoteSetInput},
    sync,
    tests::fixture,
};

#[tokio::test]
async fn quote_card_carries_every_disclosure_and_the_peer_files_plain_terms() {
    let author_fix = fixture();
    let cp_fix = fixture();
    let now = author_fix.host.now();

    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp_fix.master, now);
    put_bytes(&author_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();
    put_bytes(&cp_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let res = quote_ops::set_at(
        &author_fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Service terms"),
        },
        now,
    )
    .await
    .unwrap();

    let body = author_fix.host.sent_cards().last().unwrap().clone();
    let card = card::parse_card(&body).unwrap();
    assert!(!card.disclosures.is_empty());

    let msg = json!({
        "id": "msg_quote_peer_1",
        "body": body,
        "sender_timestamp_ms": (now * 1000) as i64,
    });
    let file_res = sync::file_incoming_card(&cp_fix.host, &msg, "conv_1", now, &cp_fix.master_did)
        .await
        .unwrap();
    assert!(file_res.filed);

    let card_row: CardRow =
        get_row(&cp_fix.host, CARDS, "msg_quote_peer_1").await.unwrap().unwrap();
    assert!(card_row.verified);
    assert!(card_row.data.is_some());
    let data = card_row.data.unwrap();
    assert_eq!(data["terms"]["currency"], "EUR");
    assert!(data["terms"].get("_sd").is_none());

    let disc_row: DisclosureRow =
        get_row(&cp_fix.host, DISCLOSURES, &res.record_id).await.unwrap().unwrap();
    assert_eq!(disc_row.disclosures, card.disclosures);
}

#[tokio::test]
async fn a_quote_card_missing_a_disclosure_is_refused() {
    let author_fix = fixture();
    let cp_fix = fixture();
    let now = author_fix.host.now();

    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp_fix.master, now);
    put_bytes(&author_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();
    put_bytes(&cp_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let _res = quote_ops::set_at(
        &author_fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Service terms"),
        },
        now,
    )
    .await
    .unwrap();

    let body = author_fix.host.sent_cards().last().unwrap().clone();
    let mut card = card::parse_card(&body).unwrap();

    card.disclosures.pop();
    let bad_body =
        card::card_body(&card.card_type, card.version, &card.envelope, &card.disclosures).unwrap();

    let msg = json!({
        "id": "msg_bad_quote",
        "body": bad_body,
        "sender_timestamp_ms": (now * 1000) as i64,
    });
    let res = sync::file_incoming_card(&cp_fix.host, &msg, "conv_1", now, &cp_fix.master_did)
        .await
        .unwrap();
    assert!(res.refused);

    let card_row: CardRow = get_row(&cp_fix.host, CARDS, "msg_bad_quote").await.unwrap().unwrap();
    assert!(!card_row.verified);
    assert!(card_row.reason.unwrap().contains("quote disclosures invalid"));
}

#[tokio::test]
async fn a_quote_card_with_a_seventeen_byte_salt_is_refused() {
    let author_fix = fixture();
    let cp_fix = fixture();
    let now = author_fix.host.now();

    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp_fix.master, now);
    put_bytes(&author_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();
    put_bytes(&cp_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let _res = quote_ops::set_at(
        &author_fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("Service terms"),
        },
        now,
    )
    .await
    .unwrap();

    let body = author_fix.host.sent_cards().last().unwrap().clone();
    let mut card = card::parse_card(&body).unwrap();

    let salt17 = encode_base64url(&[0x42u8; 17]);
    let tuple = (salt17, "scope", json!({"lang": "en", "text": "Scope"}));
    let json_str = serde_json::to_string(&tuple).unwrap();
    let disclosure_17 = encode_base64url(json_str.as_bytes());

    card.disclosures[0] = disclosure_17;
    let bad_body =
        card::card_body(&card.card_type, card.version, &card.envelope, &card.disclosures).unwrap();

    let msg = json!({
        "id": "msg_bad_salt",
        "body": bad_body,
        "sender_timestamp_ms": (now * 1000) as i64,
    });
    let res = sync::file_incoming_card(&cp_fix.host, &msg, "conv_1", now, &cp_fix.master_did)
        .await
        .unwrap();
    assert!(res.refused);

    let card_row: CardRow = get_row(&cp_fix.host, CARDS, "msg_bad_salt").await.unwrap().unwrap();
    assert!(!card_row.verified);
    assert!(card_row.reason.unwrap().contains("salt-length"));
}

#[tokio::test]
async fn each_quote_version_gets_fresh_salts() {
    let author_fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = author_fix.host.now();

    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp, now);
    put_bytes(&author_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let q1 = quote_ops::set_at(
        &author_fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: sample_terms_input("V1 terms"),
        },
        now,
    )
    .await
    .unwrap();

    let disc1: DisclosureRow =
        get_row(&author_fix.host, DISCLOSURES, &q1.record_id).await.unwrap().unwrap();

    let q2 = quote_ops::set_at(
        &author_fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: Some(q1.quote_id.clone()),
            expires_in_secs: 3600,
            terms: sample_terms_input("V2 terms"),
        },
        now + 10,
    )
    .await
    .unwrap();

    let disc2: DisclosureRow =
        get_row(&author_fix.host, DISCLOSURES, &q2.record_id).await.unwrap().unwrap();

    let salts1: HashSet<String> =
        disc1.disclosures.iter().map(|d| decode_disclosure(d).unwrap().salt).collect();
    let salts2: HashSet<String> =
        disc2.disclosures.iter().map(|d| decode_disclosure(d).unwrap().salt).collect();

    assert!(!salts1.is_empty());
    assert!(!salts2.is_empty());
    assert!(salts1.is_disjoint(&salts2));
}

#[tokio::test]
async fn quote_set_refuses_quote_too_large() {
    let author_fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = author_fix.host.now();

    let (enq_rec, enq_env) = build_and_sign_enquiry(&cp, now);
    put_bytes(&author_fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let mut large_terms = sample_terms_input("Large terms");
    let en = LangTag::parse_canonical("en").expect("lang");
    let fr = LangTag::parse_canonical("fr").expect("lang");
    let es = LangTag::parse_canonical("es").expect("lang");
    let de = LangTag::parse_canonical("de").expect("lang");
    let make_lt = |text: &str| LocalText {
        lang: en.clone(),
        text: text.to_string(),
        translations: vec![
            TextTranslation { lang: fr.clone(), text: text.to_string() },
            TextTranslation { lang: es.clone(), text: text.to_string() },
            TextTranslation { lang: de.clone(), text: text.to_string() },
        ],
    };
    large_terms.scope = make_lt("Large terms scope");
    large_terms.cancellation_terms = make_lt("Cancel");
    large_terms.refund_terms = make_lt("Refund");
    large_terms.dispute_path = make_lt("Dispute");

    let desc = "x".repeat(1800);
    large_terms.commitments = (0..16)
        .map(|_| CommitmentInput {
            giver: DealRole::QuoteAuthor,
            resource_kind: ResourceKind::GoodsOrService,
            kind: CommitmentKind::Fulfilment,
            extension: None,
            after: vec![],
            description: make_lt(&desc),
            declared_value_minor: 1000,
            details: None,
            window: None,
            location: None,
            payee: None,
            payment_methods: None,
            tax_minor: None,
            fees_minor: None,
            eep: None,
        })
        .collect();

    let err = quote_ops::set_at(
        &author_fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: large_terms,
        },
        now,
    )
    .await
    .unwrap_err();

    let err_obj = err.error.as_ref().unwrap();
    let data = err_obj.data.as_ref().unwrap();
    assert_eq!(data.get("code").and_then(|c| c.as_str()), Some("quote-too-large"));
}

#[tokio::test]
async fn largest_quote_card_fits_in_the_card_body() {
    let fix = fixture();
    let now = fix.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());

    let desc_len = 4100;
    let mut commitments = Vec::new();
    for i in 0..16 {
        commitments.push(Commitment {
            commitment_id: format!("c{i:02}"),
            giver: DealRole::QuoteAuthor,
            resource_kind: ResourceKind::GoodsOrService,
            kind: CommitmentKind::Fulfilment,
            extension: None,
            after: vec![],
            description: LocalText::plain(
                LangTag::parse_canonical("en").expect("lang"),
                "a".repeat(desc_len),
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
        });
    }

    let terms = DealTerms {
        currency: "EUR".to_string(),
        commitments,
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
    };

    let author_did = fix.master_did.clone();
    let quote_id = derive_quote_id("conv_1", &author_did, 1).expect("quote id");
    let plain = syneroym_roym_core::testing::sd::PlainQuote {
        quote_id,
        conversation: "conv_1".to_string(),
        sequence: 1,
        answers: Answers::Enquiry { record_id: "rec_enq_1".to_string() },
        counterparty_did: cp_did,
        terms,
    };
    let (payload, disclosures) = syneroym_roym_core::testing::sd::conceal_quote(plain, 1);
    let disc_bytes: usize = disclosures.iter().map(|d| d.len()).sum();
    assert!(disc_bytes < MAX_RECORD_DISCLOSURE_BYTES, "must be under 96 KiB: {disc_bytes}");
    assert!(disc_bytes > 80_000, "must be near 96 KiB: {disc_bytes}");

    let draft = RecordDraft {
        version: QUOTE_VERSION,
        record_type: RECORD_QUOTE.to_string(),
        subject: "conv_1".to_string(),
        payload: serde_json::to_value(&payload).expect("payload"),
        expires_at_secs: Some(now + 3600),
        supersedes: None,
    };
    let (mut env, bytes) =
        Envelope::unsigned(draft, author_did, Some(fix.cert_json.clone()), now).expect("env");
    let sig = fix.service_key.sign(&bytes).to_bytes();
    env.attach_signature(z32::encode(&sig)).expect("sig");
    let env_json = env.to_json().expect("env json");

    let card_res = card::card_body(RECORD_QUOTE, QUOTE_VERSION, &env_json, &disclosures);
    assert!(card_res.is_ok(), "card_body failed: {:?}", card_res.err());
    let body = card_res.unwrap();
    assert!(
        body.len() <= MAX_CARD_BODY_BYTES,
        "card body exceeds max: {} > {}",
        body.len(),
        MAX_CARD_BODY_BYTES
    );
}

#[tokio::test]
async fn a_receipt_card_with_disclosures_is_refused() {
    let fix = fixture();
    let now = fix.host.now();
    let cp = Identity::generate().expect("cp");
    let cp_did = substrate::derive_did_key(&cp.public_key());

    let (q_held, q_rec_id, _) =
        build_and_sign_quote(&fix.master, &cp_did, 1, None, now + 3600, now);
    let (receipt_env, _) =
        make_counterparty_receipt(&cp, &fix.master_did, &q_rec_id, &q_held.signed.terms, now);

    let salt = Salt::from_bytes([0x42; 16]);
    let disc = encode_disclosure(&salt, "foo", &json!("bar")).unwrap();

    let body =
        card::card_body(RECORD_AGREEMENT_RECEIPT, AGREEMENT_RECEIPT_VERSION, &receipt_env, &[disc])
            .unwrap();

    let msg = json!({
        "id": "msg_bad_receipt",
        "body": body,
        "sender_timestamp_ms": (now * 1000) as i64,
    });
    let res =
        sync::file_incoming_card(&fix.host, &msg, "conv_1", now, &fix.master_did).await.unwrap();
    assert!(res.refused);

    let card_row: CardRow = get_row(&fix.host, CARDS, "msg_bad_receipt").await.unwrap().unwrap();
    assert!(!card_row.verified);
    assert!(
        card_row.reason.unwrap().contains("disclosures not permitted on agreement receipt card")
    );
}

#[tokio::test]
async fn a_resent_payment_acknowledgement_carries_its_disclosures() {
    use syneroym_app_host::AppDataLayer;

    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let agreed_at = clock::now_secs();

    let (agreement, _terms) = setup_agreed_currency_deal(&fix, &cp, agreed_at).await;

    let input = PaymentAckInput {
        agreement: agreement.clone(),
        commitment_id: "c2".to_string(),
        observed_at_secs: None,
        settlement_note: Some(LocalText::plain(
            LangTag::parse_canonical("en").expect("lang"),
            "First note",
        )),
        method: Some("sepa".to_string()),
        reference: Some("tx_123".to_string()),
        supersedes: None,
    };
    let out1 = payment_ops::acknowledge(&fix.host, input.clone()).await.unwrap();

    let ack_cards1: Vec<_> = fix
        .host
        .sent_cards()
        .into_iter()
        .filter_map(|b| card::parse_card(&b).ok())
        .filter(|c| c.card_type == "payment-acknowledgement")
        .collect();
    assert_eq!(ack_cards1.len(), 1);
    let card1 = &ack_cards1[0];
    assert!(!card1.disclosures.is_empty());

    // Crash after fence: delete payments row so repeat call recovers via fence
    // collision
    let p_id = payment_ops::payment_id(&agreement, "c2");
    let _ = AppDataLayer::delete(&fix.host, PAYMENTS.to_string(), p_id).await;

    let out2 = payment_ops::acknowledge(&fix.host, input).await.unwrap();
    let ack_cards2: Vec<_> = fix
        .host
        .sent_cards()
        .into_iter()
        .filter_map(|b| card::parse_card(&b).ok())
        .filter(|c| c.card_type == "payment-acknowledgement")
        .collect();
    assert_eq!(ack_cards2.len(), 2);
    let card2 = &ack_cards2[1];

    assert_eq!(out1.record_id, out2.record_id);
    assert_eq!(card1.disclosures, card2.disclosures);
}

#[tokio::test]
async fn a_settlement_card_note_is_filed_plain_and_signed_hidden() {
    let fix = fixture();
    let cp_fix = fixture();
    let agreed_at = clock::now_secs();

    let (agreement, _terms) = setup_agreed_currency_deal(&fix, &cp_fix.master, agreed_at).await;
    let agr_row: AgreementRow = get_row(&fix.host, AGREEMENTS, &agreement).await.unwrap().unwrap();
    put_row(&cp_fix.host, AGREEMENTS, &agreement, &agr_row).await.unwrap();

    let input = PaymentRequestInput {
        agreement: agreement.clone(),
        commitment_id: "c2".to_string(),
        note: Some(LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Invoice note")),
    };
    let _out = payment_ops::request(&fix.host, input).await.unwrap();
    let body = fix.host.sent_cards().last().unwrap().clone();

    let card = card::parse_card(&body).unwrap();
    let env = Envelope::from_json(&card.envelope).unwrap();
    assert!(env.payload.get("_sd").is_some());
    assert!(env.payload.get("note").is_none());

    let msg = json!({
        "id": "msg_pay_req",
        "body": body,
        "sender_timestamp_ms": ((agreed_at + 10) * 1000) as i64,
    });
    let res =
        sync::file_incoming_card(&cp_fix.host, &msg, "conv_1", agreed_at + 10, &cp_fix.master_did)
            .await
            .unwrap();
    assert!(res.filed);

    let card_row: CardRow = get_row(&cp_fix.host, CARDS, "msg_pay_req").await.unwrap().unwrap();
    assert!(card_row.verified);
    assert!(card_row.data.is_some());
    let data = card_row.data.unwrap();
    assert_eq!(data["note"]["text"], "Invoice note");
}

use syneroym_app_host::types::signing::SigningIdentity;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    deal::{CommitmentKind, ExtensionRef, LocalText, ResourceKind, TextTranslation},
    lang::LangTag,
    testing::TestHost,
    transaction::DealRole,
};

use super::helpers::{
    build_and_sign_enquiry_conv, sample_deal_terms, sample_terms_input, slot_commitment,
};
use crate::app::{
    DISCLOSURES, ENQUIRY_HISTORY,
    disclosures::DisclosureRow,
    get_row, put_bytes,
    quote_ops::{self, CommitmentInput, QuoteMeasureInput, QuoteSetInput},
    tests::fixture,
};

#[tokio::test]
async fn quote_measure_matches_quote_set() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = fix.host.now();

    let conv_id = format!("conv:{}", "0".repeat(64));
    let (enq_rec, enq_env) = build_and_sign_enquiry_conv(&cp, &conv_id, now);
    put_bytes(&fix.host, ENQUIRY_HISTORY, &enq_rec, enq_env.as_bytes()).await.unwrap();

    let (slot_comm, _) = slot_commitment("c1", "listing_123", 1_000_000, 1_003_600);
    let mut terms_input = sample_terms_input("Slot lesson");
    terms_input.commitments = vec![CommitmentInput {
        giver: DealRole::QuoteAuthor,
        resource_kind: ResourceKind::ServiceInSlot,
        kind: CommitmentKind::Fulfilment,
        extension: Some(ExtensionRef { id: "scheduling".to_string(), version: 1 }),
        after: vec![],
        description: LocalText::plain(LangTag::parse_canonical("en").expect("lang"), "Lesson"),
        declared_value_minor: 5000,
        details: slot_comm.details,
        window: None,
        location: None,
        payee: None,
        payment_methods: None,
        tax_minor: None,
        fees_minor: None,
        eep: None,
    }];

    let measure =
        quote_ops::measure_at(&fix.host, QuoteMeasureInput { terms: terms_input.clone() }, now)
            .await
            .unwrap();

    let set_out = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec.clone(),
            quote_id: None,
            expires_in_secs: 3600,
            terms: terms_input.clone(),
        },
        now,
    )
    .await
    .unwrap();

    let disc_row: DisclosureRow =
        get_row(&fix.host, DISCLOSURES, &set_out.record_id).await.unwrap().unwrap();
    let real_bytes: usize = disc_row.disclosures.iter().map(|d| d.len()).sum();

    assert_eq!(measure.bytes, real_bytes as u64);
    assert!(measure.fits);
    assert_eq!(measure.details.len(), 1);
    assert_eq!(measure.details[0].commitment_id, "c1");
    assert_eq!(substrate::derive_did_key(&cp.public_key()).len(), 64);
    assert_eq!(fix.master_did.len(), 64);
    assert_eq!(conv_id.len(), 69);
    assert_eq!(set_out.record_id.len(), 56);

    let body = fix.host.sent_cards().last().unwrap().clone();
    assert!(measure.card_bytes >= body.len() as u64);

    let _set_v2_out = quote_ops::set_at(
        &fix.host,
        QuoteSetInput {
            enquiry_record_id: enq_rec,
            quote_id: Some(set_out.quote_id),
            expires_in_secs: 3600,
            terms: terms_input,
        },
        now + 10,
    )
    .await
    .unwrap();
    let body_v2 = fix.host.sent_cards().last().unwrap().clone();
    assert!(measure.card_bytes >= body_v2.len() as u64);
}

#[tokio::test]
async fn quote_measure_says_when_a_card_does_not_fit_a_mailbox() {
    let fix = fixture();
    let now = fix.host.now();

    let typical_terms = sample_deal_terms(now);
    let measure1 =
        quote_ops::measure_at(&fix.host, QuoteMeasureInput { terms: typical_terms }, now)
            .await
            .unwrap();
    assert!(measure1.fits);
    assert!(measure1.mailbox_fits);

    let mut long_terms = sample_deal_terms(now);
    let long_desc = "x".repeat(3000);
    let tag_fr = LangTag::parse_canonical("fr").unwrap();
    let tag_es = LangTag::parse_canonical("es").unwrap();
    let tag_de = LangTag::parse_canonical("de").unwrap();
    long_terms.scope = LocalText {
        lang: LangTag::parse_canonical("en").unwrap(),
        text: long_desc.clone(),
        translations: vec![
            TextTranslation { lang: tag_fr, text: long_desc.clone() },
            TextTranslation { lang: tag_es, text: long_desc.clone() },
            TextTranslation { lang: tag_de, text: long_desc.clone() },
        ],
    };
    let measure2 = quote_ops::measure_at(&fix.host, QuoteMeasureInput { terms: long_terms }, now)
        .await
        .unwrap();
    assert!(measure2.fits);
    assert!(!measure2.mailbox_fits);
}

#[tokio::test]
async fn quote_measure_answers_signing_not_enrolled_like_quote_set() {
    let host = TestHost::default();
    let now = 1000;
    host.set_now(now);

    let master = Identity::generate().expect("master");
    let master_did = substrate::derive_did_key(&master.public_key());
    let service_key = Identity::generate().expect("service");
    let service_did = substrate::derive_did_key(&service_key.public_key());
    let pubkey_hex: String =
        service_key.public_key().as_bytes().iter().map(|b| format!("{b:02x}")).collect();

    host.set_signing_id(SigningIdentity {
        signing_did: service_did,
        pubkey_hex,
        owner_did: Some(master_did),
    });

    let terms = sample_deal_terms(now);
    let measure_err =
        quote_ops::measure_at(&host, QuoteMeasureInput { terms }, now).await.unwrap_err();

    let set_input = QuoteSetInput {
        enquiry_record_id: "rec_1".to_string(),
        quote_id: None,
        expires_in_secs: 3600,
        terms: sample_terms_input("Terms"),
    };
    let set_err = quote_ops::set_at(&host, set_input, now).await.unwrap_err();

    let m_err = measure_err.error.as_ref().unwrap();
    let s_err = set_err.error.as_ref().unwrap();
    assert_eq!(m_err.code, -32602);
    assert_eq!(m_err.message, "signing-not-enrolled");
    assert_eq!(m_err.code, s_err.code);
    assert_eq!(m_err.message, s_err.message);
}

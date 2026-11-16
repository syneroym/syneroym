use syneroym_roym_core::{
    deal::{CommitmentKind, LocalText, ResourceKind},
    lang::LangTag,
    transaction::DealRole,
};

use super::{CommitmentInput, DealTermsInput};

#[test]
fn deal_terms_input_assigns_commitment_ids_and_expiry() {
    let input = DealTermsInput {
        currency: "INR".to_string(),
        home_exchange: None,
        commitments: vec![
            CommitmentInput {
                giver: DealRole::QuoteAuthor,
                resource_kind: ResourceKind::GoodsOrService,
                kind: CommitmentKind::Fulfilment,
                extension: None,
                after: vec![],
                description: LocalText::plain(LangTag::parse_canonical("en").unwrap(), "Delivery"),
                declared_value_minor: 10000,
                details: None,
                window: None,
                location: None,
                payee: None,
                payment_methods: None,
                tax_minor: None,
                fees_minor: None,
                eep: None,
            },
            CommitmentInput {
                giver: DealRole::Counterparty,
                resource_kind: ResourceKind::Currency,
                kind: CommitmentKind::Currency,
                extension: None,
                after: vec!["c1".to_string()],
                description: LocalText::plain(
                    LangTag::parse_canonical("en").unwrap(),
                    "INR 100.00",
                ),
                declared_value_minor: 10000,
                details: None,
                window: None,
                location: None,
                payee: Some("did:key:z6MkuPayee".to_string()),
                payment_methods: Some(vec!["upi".to_string()]),
                tax_minor: Some(0),
                fees_minor: Some(0),
                eep: None,
            },
        ],
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").unwrap(), "Delivery of goods"),
        cancellation_terms: LocalText::plain(LangTag::parse_canonical("en").unwrap(), "24h"),
        refund_terms: LocalText::plain(LangTag::parse_canonical("en").unwrap(), "Full refund"),
        dispute_path: LocalText::plain(LangTag::parse_canonical("en").unwrap(), "Mediation"),
    };

    let expires = 1_234_567;
    let deal = input.into_deal_terms(expires);
    assert_eq!(deal.quote_expires_at_secs, expires);
    assert_eq!(deal.commitments.len(), 2);
    assert_eq!(deal.commitments[0].commitment_id, "c1");
    assert_eq!(deal.commitments[1].commitment_id, "c2");
}

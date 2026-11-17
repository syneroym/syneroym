use std::collections::HashMap;

use serde_json::json;
use syneroym_roym_core::{
    deal::{Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind},
    lang::LangTag,
    transaction::{Answers, DealRole, QuotePayload, ReceiptHalf},
};

use crate::app::{
    backup::{ImportedQuote, QuotesByRecordId, sections::validate_ledger},
    decide::{CountersignRow, DecisionRow, DecisionState},
    hold::{HoldLedgerRow, SeatLedgerRow},
    ledger::{LedgerKind, LedgerRow},
};

fn sample_quote_context(quote_rec_id: &str) -> QuotesByRecordId {
    let mut quotes = HashMap::new();
    let terms = DealTerms {
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
        quote_expires_at_secs: 10000,
    };

    quotes.insert(
        quote_rec_id.to_string(),
        ImportedQuote {
            quote_author_did: "did:key:author".to_string(),
            payload: QuotePayload {
                quote_id: "quote_1".to_string(),
                conversation: "conv_1".to_string(),
                sequence: 1,
                counterparty_did: "did:key:cp".to_string(),
                terms,
                answers: Answers::Enquiry { record_id: "enq_1".to_string() },
            },
        },
    );
    quotes
}

#[test]
fn hold_row_refused_on_unheld_agreement_or_unknown_commitment() {
    let quotes = sample_quote_context("quote_held");

    let bad_agr_row = HoldLedgerRow {
        kind: "hold".to_string(),
        slot_id: "slot_1".to_string(),
        seat: 1,
        agreement: "unheld_quote".to_string(),
        commitment_id: "c1".to_string(),
        created_at_secs: 100,
    };
    let res = validate_ledger(
        vec![json!({ "id": "hold:unheld_quote:c1", "payload": bad_agr_row })],
        &quotes,
        100,
    );
    assert!(res.is_err());
    assert!(res.unwrap_err().error.unwrap().message.contains("names unheld agreement"));

    let bad_cid_row = HoldLedgerRow {
        kind: "hold".to_string(),
        slot_id: "slot_1".to_string(),
        seat: 1,
        agreement: "quote_held".to_string(),
        commitment_id: "c_unknown".to_string(),
        created_at_secs: 100,
    };
    let res2 = validate_ledger(
        vec![json!({ "id": "hold:quote_held:c_unknown", "payload": bad_cid_row })],
        &quotes,
        100,
    );
    assert!(res2.is_err());
    assert!(res2.unwrap_err().error.unwrap().message.contains("unknown commitment id"));
}

#[test]
fn decision_row_refused_on_unheld_agreement_or_unknown_state() {
    let quotes = sample_quote_context("quote_held");

    let bad_agr_row = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Agreed,
        agreement: "unheld_quote".to_string(),
        quote_id: "q1".to_string(),
        reason: None,
        created_at_secs: 100,
    };
    let res =
        validate_ledger(vec![json!({ "id": "decision:q1", "payload": bad_agr_row })], &quotes, 100);
    assert!(res.is_err());
    assert!(res.unwrap_err().error.unwrap().message.contains("names unheld agreement"));

    let bad_state_json = json!({
        "kind": "decision",
        "state": "invalid_state",
        "agreement": "quote_held",
        "quote_id": "q1",
        "created_at_secs": 100,
    });
    let res2 = validate_ledger(
        vec![json!({ "id": "decision:q1", "payload": bad_state_json })],
        &quotes,
        100,
    );
    assert!(res2.is_err());
    assert!(res2.unwrap_err().error.unwrap().message.contains("invalid decision row"));
}

#[test]
fn countersign_row_refused_on_unheld_quote_or_bad_signature() {
    let quotes = sample_quote_context("quote_held");

    let bad_cs_row = CountersignRow {
        kind: "countersign".to_string(),
        agreement: "unheld_quote".to_string(),
        half: ReceiptHalf {
            envelope: "bad_envelope".to_string(),
            record_id: "r1".to_string(),
            issuer: "did:key:author".to_string(),
            issued_at_secs: 100,
        },
        message_id: None,
        created_at_secs: 100,
    };
    let res = validate_ledger(
        vec![json!({ "id": "countersign:unheld_quote", "payload": bad_cs_row })],
        &quotes,
        100,
    );
    assert!(res.is_err());
    assert!(res.unwrap_err().error.unwrap().message.contains("countersign does not verify"));
}

#[test]
fn payreq_row_refused_on_non_currency_commitment() {
    let quotes = sample_quote_context("quote_held");

    let bad_payreq = LedgerRow {
        kind: LedgerKind::Fence,
        agreement: "quote_held".to_string(),
        state: None,
        quote_id: None,
        commitment_id: Some("c1".to_string()),
        slot_id: None,
        seat: None,
        reason: None,
        half: None,
        created_at_secs: 100,
    };
    let res = validate_ledger(
        vec![json!({ "id": "payreq:quote_held:c1", "payload": bad_payreq })],
        &quotes,
        100,
    );
    assert!(res.is_err());
    assert!(
        res.unwrap_err().error.unwrap().message.contains("payreq commitment 'c1' is not currency")
    );
}

#[test]
fn fulfil_row_refused_on_currency_commitment() {
    let quotes = sample_quote_context("quote_held");

    let bad_fulfil = LedgerRow {
        kind: LedgerKind::Fence,
        agreement: "quote_held".to_string(),
        state: None,
        quote_id: None,
        commitment_id: Some("c2".to_string()),
        slot_id: None,
        seat: None,
        reason: None,
        half: None,
        created_at_secs: 100,
    };
    let res = validate_ledger(
        vec![json!({ "id": "fulfil:quote_held:c2:giver", "payload": bad_fulfil })],
        &quotes,
        100,
    );
    assert!(res.is_err());
    assert!(
        res.unwrap_err()
            .error
            .unwrap()
            .message
            .contains("fulfil commitment 'c2' is not goods-or-service")
    );
}

#[test]
fn seat_row_refused_without_matching_active_hold() {
    let quotes = sample_quote_context("quote_held");

    let orphan_seat = SeatLedgerRow {
        kind: "seat".to_string(),
        slot_id: "slot_1".to_string(),
        seat: 1,
        agreement: "quote_held".to_string(),
        commitment_id: "c1".to_string(),
        created_at_secs: 100,
    };
    let res = validate_ledger(
        vec![json!({ "id": "seat:slot_1:1", "payload": orphan_seat })],
        &quotes,
        100,
    );
    assert!(res.is_err());
    assert!(res.unwrap_err().error.unwrap().message.contains("seat has no matching active hold"));
}

#[test]
fn unknown_ledger_row_kind_is_refused() {
    let quotes = sample_quote_context("quote_held");

    let res = validate_ledger(
        vec![json!({ "id": "unknown_kind:123", "payload": { "foo": "bar" } })],
        &quotes,
        100,
    );
    assert!(res.is_err());
    assert!(res.unwrap_err().error.unwrap().message.contains("unknown ledger kind"));
}

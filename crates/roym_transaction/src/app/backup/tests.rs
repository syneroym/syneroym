use std::collections::HashMap;

use serde_json::json;
use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    deal::{Commitment, CommitmentKind, DealTerms, LocalText, ResourceKind},
    lang::LangTag,
    record::{Envelope, RECORD_DEAL_DECLINE, RecordDraft},
    transaction::{
        Answers, DEAL_DECLINE_VERSION, DealDeclinePayload, DealRole, QuotePayload, ReceiptHalf,
    },
};

use crate::app::{
    AgreementRow,
    backup::{ImportedQuote, QuotesByRecordId, sections::validate_ledger},
    decide::{CountersignRow, DecisionRow, DecisionState, RefusedRow},
    decline_ops::DeclineRow,
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
        &HashMap::new(),
        &HashMap::new(),
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
        &HashMap::new(),
        &HashMap::new(),
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
    let res = validate_ledger(
        vec![json!({ "id": "decision:q1", "payload": bad_agr_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
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
        &HashMap::new(),
        &HashMap::new(),
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
        &HashMap::new(),
        &HashMap::new(),
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
        &HashMap::new(),
        &HashMap::new(),
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
        &HashMap::new(),
        &HashMap::new(),
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
        &HashMap::new(),
        &HashMap::new(),
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
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(res.is_err());
    assert!(res.unwrap_err().error.unwrap().message.contains("unknown ledger kind"));
}

fn make_test_decline_envelope(
    signer: &Identity,
    declines: &str,
    deal: &str,
    conv: &str,
    now: u64,
) -> (String, String) {
    let signer_did = substrate::derive_did_key(&signer.public_key());
    let payload = DealDeclinePayload {
        declines: declines.to_string(),
        deal: deal.to_string(),
        conversation: conv.to_string(),
        reason: None,
    };
    let draft = RecordDraft {
        version: DEAL_DECLINE_VERSION,
        record_type: RECORD_DEAL_DECLINE.to_string(),
        subject: declines.to_string(),
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

#[test]
fn decline_row_refused_on_payload_mismatch_or_unheld_target() {
    let quotes = sample_quote_context("rec_quote_1");
    let key = Identity::generate().expect("key");
    let (env_json, rec_id) =
        make_test_decline_envelope(&key, "rec_quote_1", "quote_1", "conv_1", 100);

    let valid_row = DeclineRow {
        kind: "decline".to_string(),
        deal: "quote_1".to_string(),
        declines: "rec_quote_1".to_string(),
        envelope: env_json.clone(),
        record_id: rec_id.clone(),
        message_id: None,
        created_at_secs: 100,
    };

    let ok = validate_ledger(
        vec![json!({ "id": "decline:quote_1", "payload": valid_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(ok.is_ok());

    let bad_deal_row = DeclineRow {
        kind: "decline".to_string(),
        deal: "quote_1".to_string(),
        declines: "rec_quote_1".to_string(),
        envelope: env_json.clone(),
        record_id: rec_id.clone(),
        message_id: None,
        created_at_secs: 100,
    };
    let err_deal = validate_ledger(
        vec![json!({ "id": "decline:quote_other", "payload": bad_deal_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err_deal.unwrap_err().error.unwrap().message.contains("deal id mismatch"));

    let bad_rec_row = DeclineRow {
        kind: "decline".to_string(),
        deal: "quote_1".to_string(),
        declines: "rec_quote_1".to_string(),
        envelope: env_json.clone(),
        record_id: "rec_other".to_string(),
        message_id: None,
        created_at_secs: 100,
    };
    let err_rec = validate_ledger(
        vec![json!({ "id": "decline:quote_1", "payload": bad_rec_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err_rec.unwrap_err().error.unwrap().message.contains("record id mismatch"));

    let (unheld_env, unheld_rec) =
        make_test_decline_envelope(&key, "rec_unheld", "quote_1", "conv_1", 100);
    let unheld_row = DeclineRow {
        kind: "decline".to_string(),
        deal: "quote_1".to_string(),
        declines: "rec_unheld".to_string(),
        envelope: unheld_env,
        record_id: unheld_rec,
        message_id: None,
        created_at_secs: 100,
    };
    let err_unheld = validate_ledger(
        vec![json!({ "id": "decline:quote_1", "payload": unheld_row })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err_unheld.unwrap_err().error.unwrap().message.contains("names unheld target"));

    let enqs = HashMap::from([("rec_enq_1".to_string(), "enq_deal_1".to_string())]);
    let (enq_env, enq_rec) =
        make_test_decline_envelope(&key, "rec_enq_1", "enq_deal_1", "conv_1", 100);
    let enq_row = DeclineRow {
        kind: "decline".to_string(),
        deal: "enq_deal_1".to_string(),
        declines: "rec_enq_1".to_string(),
        envelope: enq_env,
        record_id: enq_rec,
        message_id: None,
        created_at_secs: 100,
    };
    let ok_enq = validate_ledger(
        vec![json!({ "id": "decline:enq_deal_1", "payload": enq_row })],
        &HashMap::new(),
        &HashMap::new(),
        &enqs,
        100,
    );
    assert!(ok_enq.is_ok());
}

#[test]
fn refused_row_refused_on_unheld_quote_or_counterparty_half() {
    let quotes = sample_quote_context("rec_quote_1");

    let empty_quote = RefusedRow {
        kind: "refused".to_string(),
        quote_id: String::new(),
        code: "invalid-record".to_string(),
        created_at_secs: 100,
    };
    let err1 = validate_ledger(
        vec![json!({ "id": "refused:rec_half_1", "payload": empty_quote })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err1.unwrap_err().error.unwrap().message.contains("empty quote_id"));

    let unheld_quote = RefusedRow {
        kind: "refused".to_string(),
        quote_id: "unknown_quote".to_string(),
        code: "invalid-record".to_string(),
        created_at_secs: 100,
    };
    let err2 = validate_ledger(
        vec![json!({ "id": "refused:rec_half_1", "payload": unheld_quote })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err2.unwrap_err().error.unwrap().message.contains("not in backup"));

    let unheld_half = RefusedRow {
        kind: "refused".to_string(),
        quote_id: "quote_1".to_string(),
        code: "invalid-record".to_string(),
        created_at_secs: 100,
    };
    let err3 = validate_ledger(
        vec![json!({ "id": "refused:rec_half_unheld", "payload": unheld_half.clone() })],
        &quotes,
        &HashMap::new(),
        &HashMap::new(),
        100,
    );
    assert!(err3.unwrap_err().error.unwrap().message.contains("names unheld counterparty half"));

    let sample_quote = quotes.get("rec_quote_1").expect("quote");
    let held_agreement = AgreementRow {
        quote_record_id: "rec_quote_1".to_string(),
        quote_id: "quote_1".to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: "did:key:cp".to_string(),
        quote_author_did: "did:key:author".to_string(),
        terms: sample_quote.payload.terms.clone(),
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: Vec::new(),
        agreed_at_secs: None,
        counterparty: Some(ReceiptHalf {
            envelope: "env".to_string(),
            record_id: "rec_half_1".to_string(),
            issuer: "did:key:cp".to_string(),
            issued_at_secs: 100,
        }),
        quote_author: None,
        updated_at_secs: 100,
    };
    let agreements = HashMap::from([("agr_1".to_string(), held_agreement)]);

    let ok = validate_ledger(
        vec![json!({ "id": "refused:rec_half_1", "payload": unheld_half })],
        &quotes,
        &agreements,
        &HashMap::new(),
        100,
    );
    assert!(ok.is_ok());
}

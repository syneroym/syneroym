use serde_json::Value;

use super::{
    records::{sign_test_fulfilment_receipt, sign_test_payment_ack, sign_test_payment_request},
    sample_two_commitment_terms, setup_test_deal, sign_test_record,
};
use crate::{
    deal::{SettlementRole, transition::CommitmentState},
    disclosure::{MAX_RECORD_DISCLOSURE_BYTES, Salt, encode_disclosure},
    presentation::{
        types::{MAX_PRESENTATION_RECORD_BYTES, Presentation},
        verify::verify_presentation,
    },
    progress::{
        AGREEMENT_PROGRESS_VERSION, AgreementProgressPayload, AgreementState, CommitmentProgress,
        FormedAs, RECORD_AGREEMENT_PROGRESS,
    },
};

#[test]
fn payer_signed_acknowledgement_with_role_receiver_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // In sample_two_commitment_terms(), commitment c2 has giver: Counterparty
    // (payer). Counterparty key signs a payment acknowledgement claiming role:
    // Receiver.
    let (ack_env, _) = sign_test_payment_ack(
        &fix.cp_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "c2",
        SettlementRole::Receiver,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, ack_env],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100)
        .expect_err("payer cannot sign acknowledgement as receiver");
    assert_eq!(err.code(), "party-mismatch");
}

#[test]
fn payer_signed_payment_request_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // In sample_two_commitment_terms(), commitment c2 has giver: Counterparty
    // (payer). Counterparty key signs a payment request (only receiver may
    // request payment).
    let (req_env, _) = sign_test_payment_request(
        &fix.cp_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "c2",
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, req_env],
        disclosures: vec![],
    };

    let err =
        verify_presentation(&presentation, 1_100).expect_err("payer cannot sign payment request");
    assert_eq!(err.code(), "party-mismatch");
}

#[test]
fn receiver_signed_fulfilment_receipt_with_role_giver_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // In sample_two_commitment_terms(), commitment c1 has giver: QuoteAuthor.
    // Counterparty (receiver) signs a fulfilment receipt with role: Giver.
    let (ful_env, _) = sign_test_fulfilment_receipt(
        &fix.cp_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "c1",
        SettlementRole::Giver,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, ful_env],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100)
        .expect_err("receiver cannot sign fulfilment receipt as giver");
    assert_eq!(err.code(), "party-mismatch");
}

#[test]
fn settlement_view_includes_issuer_supersedes_and_observed_at_secs() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // c2 payee is QuoteAuthor. QuoteAuthor signs payment request.
    let (req_env, _) = sign_test_payment_request(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "c2",
    );

    // c2 payee (QuoteAuthor) signs payment ack with role: Receiver and
    // observed_at_secs: 1200.
    let (ack_env, _) = sign_test_payment_ack(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "c2",
        SettlementRole::Receiver,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, req_env, ack_env],
        disclosures: vec![],
    };

    let view = verify_presentation(&presentation, 1_300).expect("verify presentation");
    assert_eq!(view.settlement.len(), 2);

    let req_view = &view.settlement[0];
    assert_eq!(req_view.issuer, fix.author_did);
    assert_eq!(req_view.role, SettlementRole::Receiver);
    assert_eq!(req_view.observed_at_secs, None);
    assert_eq!(req_view.supersedes, None);

    let ack_view = &view.settlement[1];
    assert_eq!(ack_view.issuer, fix.author_did);
    assert_eq!(ack_view.role, SettlementRole::Receiver);
    assert_eq!(ack_view.observed_at_secs, Some(1_200));
    assert_eq!(ack_view.supersedes, None);
}

#[test]
fn records_with_mismatched_conversation_are_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // Snapshot with different conversation id
    let progress_payload = AgreementProgressPayload {
        agreement: fix.quote_record_id.clone(),
        conversation: "conv_different_deal_9999".to_string(),
        counterparty_did: fix.counterparty_did.clone(),
        quote_author_did: fix.author_did.clone(),
        seq: 1,
        state: AgreementState::Agreed,
        state_reason: None,
        formed_as: FormedAs::Quote,
        cancel_reason: None,
        commitments: vec![CommitmentProgress {
            commitment_id: "c1".to_string(),
            state: CommitmentState::Promised,
            reason: None,
            claimed: Some(false),
            halves: vec![],
        }],
    };

    let snap_env = sign_test_record(
        &fix.author_key,
        RECORD_AGREEMENT_PROGRESS,
        AGREEMENT_PROGRESS_VERSION,
        &fix.quote_record_id,
        serde_json::to_value(&progress_payload).expect("json"),
        None,
        1_200,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, snap_env],
        disclosures: vec![],
    };

    let err =
        verify_presentation(&presentation, 1_300).expect_err("mismatched conversation must fail");
    assert_eq!(err.code(), "multiple-deals");
}

#[test]
fn lone_counterparty_half_without_quote_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // Presentation with only the counterparty half and no quote
    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.cp_half_env],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100)
        .expect_err("lone counterparty half without quote must fail");
    assert_eq!(err.code(), "missing-terms");

    // Lone quote author half without quote is accepted
    let presentation_author = Presentation {
        presentation_version: 1,
        records: vec![fix.author_half_env],
        disclosures: vec![],
    };
    let view = verify_presentation(&presentation_author, 1_100)
        .expect("lone quote author half is accepted");
    assert!(!view.deal.quote_checked);
}

#[test]
fn quote_expired_and_revocation_status_are_reported() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // Before expiry (quote_expires_at_secs is 10_000)
    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env.clone(), fix.author_half_env.clone()],
        disclosures: vec![],
    };
    let view_before = verify_presentation(&presentation, 1_100).expect("verify before expiry");
    assert!(!view_before.deal.quote_expired);
    assert_eq!(view_before.records[0].revocation_status.as_deref(), Some("unknown"));

    // After expiry
    let view_after = verify_presentation(&presentation, 10_500).expect("verify after expiry");
    assert!(view_after.deal.quote_expired);
}

#[test]
fn oversized_record_string_and_oversized_disclosures_are_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // Oversized record string
    let huge_record = "x".repeat(MAX_PRESENTATION_RECORD_BYTES + 1);
    let pres_huge_rec =
        Presentation { presentation_version: 1, records: vec![huge_record], disclosures: vec![] };
    let err_rec = verify_presentation(&pres_huge_rec, 1_100).expect_err("huge record refused");
    assert_eq!(err_rec.code(), "record-refused");

    // Total disclosures exceeding terms budget
    let salt = Salt::from_bytes([1u8; 16]);
    let huge_disc = encode_disclosure(
        &salt,
        "scope",
        &Value::String("y".repeat(MAX_RECORD_DISCLOSURE_BYTES + 10)),
    )
    .unwrap();
    let pres_huge_disc = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env],
        disclosures: vec![huge_disc],
    };
    let err_disc =
        verify_presentation(&pres_huge_disc, 1_100).expect_err("huge disclosures refused");
    assert_eq!(err_disc.code(), "quote-too-large");
}

#[test]
fn optional_settlement_fields_shown_as_null_are_accepted() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    let (req_env, req_discs) = sign_test_payment_request(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "c2",
    );

    let (ack_env, ack_discs) = sign_test_payment_ack(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "c2",
        SettlementRole::Receiver,
    );

    let (ful_env, ful_discs) = sign_test_fulfilment_receipt(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "c1",
        SettlementRole::Giver,
    );

    let mut all_disclosures = Vec::new();
    all_disclosures.extend(req_discs);
    all_disclosures.extend(ack_discs);
    all_disclosures.extend(ful_discs);

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![
            fix.quote_env,
            fix.author_half_env,
            fix.cp_half_env,
            req_env,
            ack_env,
            ful_env,
        ],
        disclosures: all_disclosures,
    };

    let view = verify_presentation(&presentation, 1_300)
        .expect("optional settlement fields disclosed as null must be accepted");
    assert_eq!(view.settlement.len(), 3);

    let req_view = &view.settlement[0];
    assert_eq!(req_view.shown.get("note"), Some(&Value::Null));

    let ack_view = &view.settlement[1];
    assert_eq!(ack_view.shown.get("method"), Some(&Value::Null));
    assert_eq!(ack_view.shown.get("reference"), Some(&Value::Null));
    assert_eq!(ack_view.shown.get("settlement_note"), Some(&Value::Null));

    let ful_view = &view.settlement[2];
    assert_eq!(ful_view.shown.get("settlement_note"), Some(&Value::Null));
}

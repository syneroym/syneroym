use syneroym_identity::{Identity, substrate};

use super::*;
use crate::{
    deal::{SettlementRole, transition::CommitmentState},
    fulfilment::FULFILMENT_RECEIPT_VERSION,
    payment::{PAYMENT_ACKNOWLEDGEMENT_VERSION, PAYMENT_REQUEST_VERSION},
    progress::{
        AGREEMENT_PROGRESS_VERSION, AgreementProgressPayload, AgreementState, CommitmentProgress,
        FormedAs,
    },
    record::{
        RECORD_AGREEMENT_PROGRESS, RECORD_AGREEMENT_RECEIPT, RECORD_FULFILMENT_RECEIPT,
        RECORD_PAYMENT_ACKNOWLEDGEMENT, RECORD_PAYMENT_REQUEST,
    },
    testing::sd::{
        PlainFulfilment, PlainPaymentAck, PlainPaymentRequest, conceal_fulfilment,
        conceal_payment_ack, conceal_payment_request,
    },
    transaction::{AGREEMENT_RECEIPT_VERSION, DealRole},
};

pub(super) fn sign_test_payment_request(
    key: &Identity,
    agreement: &str,
    author_did: &str,
    cp_did: &str,
    cid: &str,
) -> (String, Vec<String>) {
    let plain = PlainPaymentRequest {
        agreement: agreement.to_string(),
        conversation: "conv_music_deal_1234".to_string(),
        commitment_id: cid.to_string(),
        quote_author_did: author_did.to_string(),
        counterparty_did: cp_did.to_string(),
        note: None,
    };
    let (payload, disclosures) = conceal_payment_request(plain, 10);
    let env = sign_test_record(
        key,
        RECORD_PAYMENT_REQUEST,
        PAYMENT_REQUEST_VERSION,
        agreement,
        serde_json::to_value(&payload).expect("json"),
        None,
        1_200,
    );
    (env, disclosures)
}

pub(super) fn sign_test_payment_ack(
    key: &Identity,
    agreement: &str,
    author_did: &str,
    cp_did: &str,
    cid: &str,
    role: SettlementRole,
) -> (String, Vec<String>) {
    let plain = PlainPaymentAck {
        agreement: agreement.to_string(),
        conversation: "conv_music_deal_1234".to_string(),
        commitment_id: cid.to_string(),
        quote_author_did: author_did.to_string(),
        counterparty_did: cp_did.to_string(),
        role,
        observed_at_secs: 1_200,
        method: None,
        reference: None,
        settlement_note: None,
    };
    let (payload, disclosures) = conceal_payment_ack(plain, 20);
    let env = sign_test_record(
        key,
        RECORD_PAYMENT_ACKNOWLEDGEMENT,
        PAYMENT_ACKNOWLEDGEMENT_VERSION,
        agreement,
        serde_json::to_value(&payload).expect("json"),
        None,
        1_200,
    );
    (env, disclosures)
}

pub(super) fn sign_test_fulfilment_receipt(
    key: &Identity,
    agreement: &str,
    author_did: &str,
    cp_did: &str,
    cid: &str,
    role: SettlementRole,
) -> (String, Vec<String>) {
    let plain = PlainFulfilment {
        agreement: agreement.to_string(),
        conversation: "conv_music_deal_1234".to_string(),
        commitment_id: cid.to_string(),
        counterparty_did: cp_did.to_string(),
        quote_author_did: author_did.to_string(),
        role,
        settlement_note: None,
    };
    let (payload, disclosures) = conceal_fulfilment(plain, 30);
    let env = sign_test_record(
        key,
        RECORD_FULFILMENT_RECEIPT,
        FULFILMENT_RECEIPT_VERSION,
        agreement,
        serde_json::to_value(&payload).expect("json"),
        None,
        1_200,
    );
    (env, disclosures)
}

#[test]
fn halves_signed_by_other_keys_than_the_quotes_parties_are_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());
    let stranger_a = Identity::generate().expect("key");
    let stranger_b = Identity::generate().expect("key");
    let stranger_a_did = substrate::derive_did_key(&stranger_a.public_key());
    let stranger_b_did = substrate::derive_did_key(&stranger_b.public_key());

    let half_a = AgreementReceiptPayload {
        quote_record_id: fix.quote_record_id.clone(),
        counterparty_did: stranger_b_did.clone(),
        quote_author_did: stranger_a_did.clone(),
        role: DealRole::QuoteAuthor,
        terms: fix.terms.clone(),
        viewed_lang: lang_en(),
        viewed_via_mt: false,
    };
    let half_a_env = sign_test_record(
        &stranger_a,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &fix.quote_record_id,
        serde_json::to_value(&half_a).expect("json"),
        None,
        1_050,
    );

    let half_b = AgreementReceiptPayload {
        quote_record_id: fix.quote_record_id.clone(),
        counterparty_did: stranger_b_did,
        quote_author_did: stranger_a_did,
        role: DealRole::Counterparty,
        terms: fix.terms.clone(),
        viewed_lang: lang_en(),
        viewed_via_mt: false,
    };
    let half_b_env = sign_test_record(
        &stranger_b,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &fix.quote_record_id,
        serde_json::to_value(&half_b).expect("json"),
        None,
        1_060,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, half_a_env, half_b_env],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100).expect_err("stranger halves must fail");
    assert_eq!(err.code(), "party-mismatch");
}

#[test]
fn counterparty_half_signed_by_other_key_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());
    let stranger = Identity::generate().expect("key");
    let stranger_did = substrate::derive_did_key(&stranger.public_key());

    let forged_cp_half = AgreementReceiptPayload {
        quote_record_id: fix.quote_record_id.clone(),
        counterparty_did: stranger_did,
        quote_author_did: fix.author_did.clone(),
        role: DealRole::Counterparty,
        terms: fix.terms.clone(),
        viewed_lang: lang_en(),
        viewed_via_mt: false,
    };
    let forged_cp_env = sign_test_record(
        &stranger,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &fix.quote_record_id,
        serde_json::to_value(&forged_cp_half).expect("json"),
        None,
        1_060,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, forged_cp_env],
        disclosures: vec![],
    };

    let err =
        verify_presentation(&presentation, 1_100).expect_err("forged counterparty half must fail");
    assert_eq!(err.code(), "party-mismatch");
}

#[test]
fn quote_checked_field_reflects_quote_presence() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // Presentation with quote
    let p_with_quote = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env.clone(), fix.author_half_env.clone(), fix.cp_half_env.clone()],
        disclosures: vec![],
    };
    let view_with = verify_presentation(&p_with_quote, 1_100).expect("verify with quote");
    assert!(view_with.deal.quote_checked);

    // Presentation without quote
    let p_without_quote = Presentation {
        presentation_version: 1,
        records: vec![fix.author_half_env, fix.cp_half_env],
        disclosures: vec![],
    };
    let view_without = verify_presentation(&p_without_quote, 1_100).expect("verify without quote");
    assert!(!view_without.deal.quote_checked);
}

#[test]
fn payment_request_with_forged_party_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());
    let stranger = Identity::generate().expect("key");
    let stranger_did = substrate::derive_did_key(&stranger.public_key());

    let (req_env, _) = sign_test_payment_request(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &stranger_did,
        "c2",
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, req_env],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100).expect_err("forged party in payment req");
    assert_eq!(err.code(), "party-mismatch");
}

#[test]
fn payment_request_with_unknown_commitment_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    let (req_env, _) = sign_test_payment_request(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "no-such-commitment",
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, req_env],
        disclosures: vec![],
    };

    let err =
        verify_presentation(&presentation, 1_100).expect_err("unknown commitment in payment req");
    assert_eq!(err.code(), "commitment-mismatch");
}

#[test]
fn payment_ack_with_forged_party_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());
    let stranger = Identity::generate().expect("key");
    let stranger_did = substrate::derive_did_key(&stranger.public_key());

    let (ack_env, _) = sign_test_payment_ack(
        &stranger,
        &fix.quote_record_id,
        &fix.author_did,
        &stranger_did,
        "c2",
        SettlementRole::Receiver,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, ack_env],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100).expect_err("forged party in payment ack");
    assert_eq!(err.code(), "party-mismatch");
}

#[test]
fn payment_ack_with_unknown_commitment_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    let (ack_env, _) = sign_test_payment_ack(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "no-such-commitment",
        SettlementRole::Receiver,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, ack_env],
        disclosures: vec![],
    };

    let err =
        verify_presentation(&presentation, 1_100).expect_err("unknown commitment in payment ack");
    assert_eq!(err.code(), "commitment-mismatch");
}

#[test]
fn fulfilment_with_forged_party_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());
    let stranger = Identity::generate().expect("key");
    let stranger_did = substrate::derive_did_key(&stranger.public_key());

    let (ful_env, _) = sign_test_fulfilment_receipt(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &stranger_did,
        "c1",
        SettlementRole::Giver,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, ful_env],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100).expect_err("forged party in fulfilment");
    assert_eq!(err.code(), "party-mismatch");
}

#[test]
fn fulfilment_with_unknown_commitment_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    let (ful_env, _) = sign_test_fulfilment_receipt(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "no-such-commitment",
        SettlementRole::Giver,
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, ful_env],
        disclosures: vec![],
    };

    let err =
        verify_presentation(&presentation, 1_100).expect_err("unknown commitment in fulfilment");
    assert_eq!(err.code(), "commitment-mismatch");
}

#[test]
fn snapshot_with_forged_party_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());
    let stranger = Identity::generate().expect("key");
    let stranger_did = substrate::derive_did_key(&stranger.public_key());

    let progress_payload = AgreementProgressPayload {
        agreement: fix.quote_record_id.clone(),
        conversation: "conv_music_deal_1234".to_string(),
        counterparty_did: stranger_did,
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
        records: vec![fix.author_half_env, fix.cp_half_env, snap_env],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100).expect_err("forged party in snapshot");
    assert_eq!(err.code(), "party-mismatch");
}

#[test]
fn snapshot_with_unknown_commitment_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    let progress_payload = AgreementProgressPayload {
        agreement: fix.quote_record_id.clone(),
        conversation: "conv_music_deal_1234".to_string(),
        counterparty_did: fix.counterparty_did.clone(),
        quote_author_did: fix.author_did.clone(),
        seq: 1,
        state: AgreementState::Agreed,
        state_reason: None,
        formed_as: FormedAs::Quote,
        cancel_reason: None,
        commitments: vec![CommitmentProgress {
            commitment_id: "no-such-commitment".to_string(),
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
        records: vec![fix.author_half_env, fix.cp_half_env, snap_env],
        disclosures: vec![],
    };

    let err =
        verify_presentation(&presentation, 1_100).expect_err("unknown commitment in snapshot");
    assert_eq!(err.code(), "commitment-mismatch");
}

#[test]
fn settlement_record_with_mismatched_commitment_kind_is_refused() {
    let fix = setup_test_deal(sample_two_commitment_terms());

    // c1 is fulfilment, but we create payment request for it
    let (bad_pay_req, _) = sign_test_payment_request(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "c1",
    );

    let presentation = Presentation {
        presentation_version: 1,
        records: vec![
            fix.quote_env.clone(),
            fix.author_half_env.clone(),
            fix.cp_half_env.clone(),
            bad_pay_req,
        ],
        disclosures: vec![],
    };

    let err = verify_presentation(&presentation, 1_100).expect_err("payment on non-currency");
    assert_eq!(err.code(), "commitment-mismatch");

    // c2 is currency, but we create fulfilment receipt for it
    let (bad_ful, _) = sign_test_fulfilment_receipt(
        &fix.author_key,
        &fix.quote_record_id,
        &fix.author_did,
        &fix.counterparty_did,
        "c2",
        SettlementRole::Giver,
    );

    let presentation2 = Presentation {
        presentation_version: 1,
        records: vec![fix.quote_env, fix.author_half_env, fix.cp_half_env, bad_ful],
        disclosures: vec![],
    };

    let err2 = verify_presentation(&presentation2, 1_100).expect_err("fulfilment on currency");
    assert_eq!(err2.code(), "commitment-mismatch");
}

fn sign_test_settlement_records_with_notes(
    fix: &TestDealFixture,
) -> (String, Vec<String>, String, Vec<String>, String, Vec<String>) {
    let plain_req = PlainPaymentRequest {
        agreement: fix.quote_record_id.clone(),
        conversation: "conv_music_deal_1234".to_string(),
        commitment_id: "c2".to_string(),
        quote_author_did: fix.author_did.clone(),
        counterparty_did: fix.counterparty_did.clone(),
        note: Some(LocalText::plain(lang_en(), "Payment request note")),
    };
    let (req_payload, req_discs) = conceal_payment_request(plain_req, 10);
    let req_env = sign_test_record(
        &fix.author_key,
        RECORD_PAYMENT_REQUEST,
        PAYMENT_REQUEST_VERSION,
        &fix.quote_record_id,
        serde_json::to_value(&req_payload).expect("json"),
        None,
        1_200,
    );

    let plain_ack = PlainPaymentAck {
        agreement: fix.quote_record_id.clone(),
        conversation: "conv_music_deal_1234".to_string(),
        commitment_id: "c2".to_string(),
        quote_author_did: fix.author_did.clone(),
        counterparty_did: fix.counterparty_did.clone(),
        role: SettlementRole::Receiver,
        observed_at_secs: 1_200,
        method: Some("upi".to_string()),
        reference: Some("ref-123".to_string()),
        settlement_note: Some(LocalText::plain(lang_en(), "Ack note")),
    };
    let (ack_payload, ack_discs) = conceal_payment_ack(plain_ack, 20);
    let ack_env = sign_test_record(
        &fix.author_key,
        RECORD_PAYMENT_ACKNOWLEDGEMENT,
        PAYMENT_ACKNOWLEDGEMENT_VERSION,
        &fix.quote_record_id,
        serde_json::to_value(&ack_payload).expect("json"),
        None,
        1_200,
    );

    let plain_ful = PlainFulfilment {
        agreement: fix.quote_record_id.clone(),
        conversation: "conv_music_deal_1234".to_string(),
        commitment_id: "c1".to_string(),
        counterparty_did: fix.counterparty_did.clone(),
        quote_author_did: fix.author_did.clone(),
        role: SettlementRole::Giver,
        settlement_note: Some(LocalText::plain(lang_en(), "Fulfilment note")),
    };
    let (ful_payload, ful_discs) = conceal_fulfilment(plain_ful, 30);
    let ful_env = sign_test_record(
        &fix.author_key,
        RECORD_FULFILMENT_RECEIPT,
        FULFILMENT_RECEIPT_VERSION,
        &fix.quote_record_id,
        serde_json::to_value(&ful_payload).expect("json"),
        None,
        1_200,
    );

    (req_env, req_discs, ack_env, ack_discs, ful_env, ful_discs)
}

#[test]
fn settlement_records_presented_with_notes_shown_and_hidden() {
    let fix = setup_test_deal(sample_two_commitment_terms());
    let (req_env, req_discs, ack_env, ack_discs, ful_env, ful_discs) =
        sign_test_settlement_records_with_notes(&fix);

    let pres_hidden = Presentation {
        presentation_version: 1,
        records: vec![
            fix.quote_env.clone(),
            fix.author_half_env.clone(),
            fix.cp_half_env.clone(),
            req_env.clone(),
            ack_env.clone(),
            ful_env.clone(),
        ],
        disclosures: vec![],
    };
    let view_hidden = verify_presentation(&pres_hidden, 1_300).expect("verify hidden succeeds");
    assert_eq!(view_hidden.settlement.len(), 3);
    for s in &view_hidden.settlement {
        assert!(s.shown.as_object().unwrap().is_empty());
        assert!(!s.hidden.is_empty());
    }

    let mut all_discs = Vec::new();
    all_discs.extend(req_discs);
    all_discs.extend(ack_discs);
    all_discs.extend(ful_discs);

    let pres_shown = Presentation {
        presentation_version: 1,
        records: vec![
            fix.quote_env,
            fix.author_half_env,
            fix.cp_half_env,
            req_env,
            ack_env,
            ful_env,
        ],
        disclosures: all_discs,
    };
    let view_shown = verify_presentation(&pres_shown, 1_300).expect("verify shown succeeds");
    assert_eq!(view_shown.settlement.len(), 3);
    let req_view = view_shown
        .settlement
        .iter()
        .find(|s| {
            s.commitment_id == "c2" && s.role == SettlementRole::Receiver && s.hidden.is_empty()
        })
        .expect("req view");
    assert!(req_view.shown.get("note").is_some());

    let ack_view = view_shown
        .settlement
        .iter()
        .find(|s| s.commitment_id == "c2" && s.shown.get("method").is_some())
        .expect("ack view");
    assert_eq!(ack_view.shown.get("method").and_then(Value::as_str), Some("upi"));
    assert_eq!(ack_view.shown.get("reference").and_then(Value::as_str), Some("ref-123"));
    assert!(ack_view.shown.get("settlement_note").is_some());

    let ful_view =
        view_shown.settlement.iter().find(|s| s.commitment_id == "c1").expect("ful view");
    assert!(ful_view.shown.get("settlement_note").is_some());
}

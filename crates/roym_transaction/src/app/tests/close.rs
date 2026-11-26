use serde_json::Value;
use syneroym_identity::Identity;
use syneroym_roym_core::{
    clock,
    deal::{Commitment, CommitmentKind, LocalText, ResourceKind},
    envelope::Response,
    lang::LangTag,
    progress::AgreementState,
    transaction::DealRole,
};

use super::{
    fixture,
    settle::{make_currency_commitment, make_service_commitment, setup_agreed_deal},
};
use crate::app::{
    AGREEMENTS, AgreementRow, FULFILMENTS, FulfilmentsRow, SendState,
    close_ops::{AgreementCloseInput, CloseAs, close_at},
    fulfilment_ops::fulfilment_id,
    get_row, put_row,
};

fn code_of(err: &Response) -> Value {
    err.error.as_ref().and_then(|e| e.data.clone()).unwrap_or(Value::Null)
}

fn message_of(err: &Response) -> String {
    err.error.as_ref().map(|e| e.message.clone()).unwrap_or_default()
}

fn make_cp_service_commitment(id: &str) -> Commitment {
    Commitment {
        commitment_id: id.to_string(),
        giver: DealRole::Counterparty,
        resource_kind: ResourceKind::GoodsOrService,
        kind: CommitmentKind::Fulfilment,
        extension: None,
        after: vec![],
        description: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse lang"),
            "Delivery service",
        ),
        declared_value_minor: 3000,
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

#[tokio::test]
async fn one_call_settles_every_receivable_commitment_and_repeat_skips() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();

    let c1 = make_cp_service_commitment("c1");
    let c2 = make_cp_service_commitment("c2");
    let c3 = make_currency_commitment("c3", &fix.master_did);
    let (agreement, _) = setup_agreed_deal(&fix, &cp, vec![c1, c2, c3], now).await;

    let res = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement: agreement.clone(),
            close_as: CloseAs::Receiver,
            commitments: vec!["c1".to_string(), "c2".to_string(), "c3".to_string()],
        },
        now + 10,
    )
    .await
    .expect("close call succeeds");

    assert_eq!(res.records.len(), 3);
    assert!(res.skipped.is_empty());
    assert_eq!(res.records[0].commitment_id, "c1");
    assert_eq!(res.records[1].commitment_id, "c2");
    assert_eq!(res.records[2].commitment_id, "c3");

    let agr: AgreementRow =
        get_row(&fix.host, AGREEMENTS, &agreement).await.expect("get").expect("agr exists");
    assert_eq!(agr.state, Some(AgreementState::Closed));

    let repeat = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement,
            close_as: CloseAs::Receiver,
            commitments: vec!["c1".to_string(), "c2".to_string(), "c3".to_string()],
        },
        now + 20,
    )
    .await
    .expect("repeat call succeeds");

    assert!(repeat.records.is_empty());
    assert_eq!(repeat.skipped.len(), 3);
    for s in &repeat.skipped {
        assert_eq!(s.reason, "final");
    }
}

#[tokio::test]
async fn repeated_claim_returns_stored_records() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();

    let c1 = make_service_commitment("c1");
    let c2 = make_service_commitment("c2");
    let (agreement, _) = setup_agreed_deal(&fix, &cp, vec![c1, c2], now).await;

    let first = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement: agreement.clone(),
            close_as: CloseAs::Giver,
            commitments: vec!["c1".to_string(), "c2".to_string()],
        },
        now + 10,
    )
    .await
    .expect("first claim succeeds");

    assert_eq!(first.records.len(), 2);
    assert!(first.skipped.is_empty());
    assert_ne!(first.records[0].state, SendState::AlreadyRecorded);
    assert_ne!(first.records[1].state, SendState::AlreadyRecorded);

    let second = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement,
            close_as: CloseAs::Giver,
            commitments: vec!["c1".to_string(), "c2".to_string()],
        },
        now + 20,
    )
    .await
    .expect("repeat claim succeeds");

    assert_eq!(second.records.len(), 2);
    assert!(second.skipped.is_empty());
    assert_eq!(second.records[0].record_id, first.records[0].record_id);
    assert_eq!(second.records[1].record_id, first.records[1].record_id);
    assert_eq!(second.records[0].state, SendState::AlreadyRecorded);
    assert_eq!(second.records[1].state, SendState::AlreadyRecorded);
}

#[tokio::test]
async fn closed_agreement_skips_all_as_final() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();

    let c1 = make_service_commitment("c1");
    let (agreement, _) = setup_agreed_deal(&fix, &cp, vec![c1], now).await;

    let mut agr: AgreementRow =
        get_row(&fix.host, AGREEMENTS, &agreement).await.expect("get").expect("agr");
    agr.state = Some(AgreementState::Closed);
    put_row(&fix.host, AGREEMENTS, &agreement, &agr).await.expect("put closed");

    let res = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement,
            close_as: CloseAs::Giver,
            commitments: vec!["c1".to_string()],
        },
        now + 10,
    )
    .await
    .expect("call on closed agreement succeeds");

    assert!(res.records.is_empty());
    assert_eq!(res.skipped.len(), 1);
    assert_eq!(res.skipped[0].commitment_id, "c1");
    assert_eq!(res.skipped[0].reason, "final");
}

#[tokio::test]
async fn bad_list_refuses_and_signs_nothing() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();

    let c1 = make_service_commitment("c1");
    let (agreement, _) = setup_agreed_deal(&fix, &cp, vec![c1], now).await;

    // 1. Empty list
    let err_empty = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement: agreement.clone(),
            close_as: CloseAs::Giver,
            commitments: vec![],
        },
        now,
    )
    .await
    .unwrap_err();
    assert_eq!(message_of(&err_empty), "bad-commitment-list");
    assert_eq!(code_of(&err_empty)["code"], "bad-commitment-list");

    // 2. Duplicate commitment
    let err_dup = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement: agreement.clone(),
            close_as: CloseAs::Giver,
            commitments: vec!["c1".to_string(), "c1".to_string()],
        },
        now,
    )
    .await
    .unwrap_err();
    assert_eq!(message_of(&err_dup), "bad-commitment-list");
    assert_eq!(code_of(&err_dup)["code"], "bad-commitment-list");

    // 3. Foreign commitment
    let err_foreign = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement: agreement.clone(),
            close_as: CloseAs::Giver,
            commitments: vec!["c99".to_string()],
        },
        now,
    )
    .await
    .unwrap_err();
    assert_eq!(message_of(&err_foreign), "no-such-commitment");
    assert_eq!(code_of(&err_foreign)["code"], "no-such-commitment");

    // 4. Not your side (QuoteAuthor is giver of c1, not receiver)
    let err_side = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement: agreement.clone(),
            close_as: CloseAs::Receiver,
            commitments: vec!["c1".to_string()],
        },
        now,
    )
    .await
    .unwrap_err();
    assert_eq!(message_of(&err_side), "not-your-side");
    assert_eq!(code_of(&err_side)["code"], "not-your-side");

    // 5. Not open (Cancelled)
    let mut agr: AgreementRow =
        get_row(&fix.host, AGREEMENTS, &agreement).await.expect("get").expect("agr");
    agr.state = Some(AgreementState::Cancelled);
    put_row(&fix.host, AGREEMENTS, &agreement, &agr).await.expect("put cancelled");

    let err_not_open = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement: agreement.clone(),
            close_as: CloseAs::Giver,
            commitments: vec!["c1".to_string()],
        },
        now,
    )
    .await
    .unwrap_err();
    assert_eq!(message_of(&err_not_open), "agreement-not-open");
    assert_eq!(code_of(&err_not_open)["code"], "agreement-not-open");

    // Verify nothing was signed
    let f_row: Option<FulfilmentsRow> =
        get_row(&fix.host, FULFILMENTS, &fulfilment_id(&agreement, "c1")).await.expect("get f");
    assert!(f_row.is_none());
}

#[tokio::test]
async fn failure_in_second_signature_leaves_first_intact_and_retry_finishes() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();

    let c1 = make_cp_service_commitment("c1");
    let c2 = make_cp_service_commitment("c2");
    let (agreement, _) = setup_agreed_deal(&fix, &cp, vec![c1, c2], now).await;

    // Fail write on c2
    let c2_f_id = fulfilment_id(&agreement, "c2");
    fix.host.fail_write(FULFILMENTS, &c2_f_id);

    let err = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement: agreement.clone(),
            close_as: CloseAs::Receiver,
            commitments: vec!["c1".to_string(), "c2".to_string()],
        },
        now + 10,
    )
    .await
    .unwrap_err();
    let msg = message_of(&err);
    assert!(msg.contains("fail_write triggered") || err.error.as_ref().unwrap().code == -32603);

    // c1 half should exist
    let c1_f_id = fulfilment_id(&agreement, "c1");
    let c1_row: Option<FulfilmentsRow> =
        get_row(&fix.host, FULFILMENTS, &c1_f_id).await.expect("get c1");
    assert!(c1_row.is_some());
    assert!(c1_row.unwrap().quote_author.is_some());

    // c2 half should NOT exist
    let c2_row: Option<FulfilmentsRow> =
        get_row(&fix.host, FULFILMENTS, &c2_f_id).await.expect("get c2");
    assert!(c2_row.is_none());

    // Clear failure and retry
    fix.host.restart();

    let retry = close_at(
        &fix.host,
        AgreementCloseInput {
            agreement: agreement.clone(),
            close_as: CloseAs::Receiver,
            commitments: vec!["c1".to_string(), "c2".to_string()],
        },
        now + 20,
    )
    .await
    .expect("retry succeeds");

    // c1 was fulfilled by receiver signing in call 1 -> skipped as final
    // c2 was signed now
    assert_eq!(retry.records.len(), 1);
    assert_eq!(retry.records[0].commitment_id, "c2");
    assert_eq!(retry.skipped.len(), 1);
    assert_eq!(retry.skipped[0].commitment_id, "c1");
    assert_eq!(retry.skipped[0].reason, "final");

    let final_agr: AgreementRow =
        get_row(&fix.host, AGREEMENTS, &agreement).await.expect("get").expect("agr");
    assert_eq!(final_agr.state, Some(AgreementState::Closed));
}

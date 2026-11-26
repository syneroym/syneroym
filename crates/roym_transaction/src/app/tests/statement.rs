use syneroym_identity::{Identity, substrate};
use syneroym_roym_core::{
    clock,
    deal::{LocalText, ResourceKind, transition::CommitmentState},
    eventlog::Direction,
    lang::LangTag,
    progress::{AgreementProgressPayload, AgreementState, CommitmentProgress, FormedAs},
    statement::EepSection,
    transaction::DealRole,
};

use super::{
    fixture, fixture_with_master,
    settle::{make_service_commitment, sample_note, setup_agreed_deal},
};
use crate::app::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow,
    decide::SnapshotRow,
    fulfilment_ops::{self, FulfilmentSignInput},
    put_row,
    statement_ops::{self, StatementExportInput},
};

async fn store_progress_snapshot(
    fix: &super::TestFixture,
    agreement: &str,
    seq: u32,
    state: AgreementState,
    commitments: Vec<CommitmentProgress>,
    issued_at_secs: u64,
) {
    let payload = AgreementProgressPayload {
        agreement: agreement.to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: "did:key:zCp".to_string(),
        quote_author_did: fix.master_did.clone(),
        seq,
        state,
        formed_as: FormedAs::Quote,
        state_reason: None,
        cancel_reason: None,
        commitments,
    };
    let (env, record_id) =
        crate::app::progress::sign(&fix.host, &payload).await.expect("sign snapshot");
    let snap_key = crate::app::decide::snapshot_id(agreement, seq);
    let snap_row = SnapshotRow {
        seq,
        agreement: agreement.to_string(),
        conversation: "conv_1".to_string(),
        snapshot: payload,
        envelope: env,
        record_id,
        message_id: None,
        stored_at_secs: issued_at_secs,
    };
    put_row(&fix.host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await.expect("put snapshot row");
}

fn make_prog(id: &str, state: CommitmentState, halves: Vec<String>) -> CommitmentProgress {
    CommitmentProgress { commitment_id: id.to_string(), state, reason: None, claimed: None, halves }
}

#[tokio::test]
async fn statement_export_refuses_bad_period() {
    let fix = fixture();
    let res =
        statement_ops::export(&fix.host, StatementExportInput { from_secs: 200, to_secs: 100 })
            .await;

    let err = res.expect_err("should refuse bad period");
    assert_eq!(err.error.as_ref().unwrap().message, "bad-period");
    assert_eq!(err.error.as_ref().unwrap().data.as_ref().unwrap()["code"], "bad-period");

    let res_equal =
        statement_ops::export(&fix.host, StatementExportInput { from_secs: 100, to_secs: 100 })
            .await;
    assert_eq!(res_equal.expect_err("equal period").error.as_ref().unwrap().message, "bad-period");
}

async fn setup_cp_deal(cp_fix: &super::TestFixture, author_did: &str, now: u64) -> AgreementRow {
    let comm = make_service_commitment("c1");
    let terms = syneroym_roym_core::deal::DealTerms {
        currency: "EUR".to_string(),
        commitments: vec![comm],
        home_exchange: None,
        listing_id: None,
        scope: LocalText::plain(LangTag::parse_canonical("en").expect("parse"), "Lesson"),
        cancellation_terms: LocalText::plain(
            LangTag::parse_canonical("en").expect("parse"),
            "none",
        ),
        refund_terms: LocalText::plain(LangTag::parse_canonical("en").expect("parse"), "none"),
        dispute_path: LocalText::plain(LangTag::parse_canonical("en").expect("parse"), "none"),
        quote_expires_at_secs: now + 3600,
    };

    crate::app::ensure_collections(&cp_fix.host).await.unwrap();
    let agr_row = AgreementRow {
        quote_record_id: "rec_deal_cp".to_string(),
        formed_as: None,
        quote_id: "q_cp".to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_fix.master_did.clone(),
        quote_author_did: author_did.to_string(),
        terms,
        agreed_at_secs: Some(now),
        counterparty: None,
        quote_author: None,
        state: Some(AgreementState::Agreed),
        state_reason: None,
        cancel_reason: None,
        seq: Some(1),
        commitments: vec![make_prog("c1", CommitmentState::Promised, vec![])],
        updated_at_secs: now,
    };
    put_row(&cp_fix.host, AGREEMENTS, "rec_deal_cp", &agr_row).await.unwrap();

    let f_row = crate::app::FulfilmentsRow {
        agreement: "rec_deal_cp".to_string(),
        commitment_id: "c1".to_string(),
        conversation: "conv_1".to_string(),
        counterparty: Some(syneroym_roym_core::transaction::ReceiptHalf {
            record_id: "h_cp".to_string(),
            envelope: "{}".to_string(),
            issuer: cp_fix.master_did.clone(),
            issued_at_secs: now + 5,
        }),
        quote_author: None,
        updated_at_secs: now + 5,
    };
    put_row(
        &cp_fix.host,
        crate::app::FULFILMENTS,
        &crate::app::fulfilment_ops::fulfilment_id("rec_deal_cp", "c1"),
        &f_row,
    )
    .await
    .unwrap();

    let formed_snap = AgreementProgressPayload {
        agreement: "rec_deal_cp".to_string(),
        conversation: "conv_1".to_string(),
        counterparty_did: cp_fix.master_did.clone(),
        quote_author_did: author_did.to_string(),
        seq: 1,
        state: AgreementState::Agreed,
        formed_as: FormedAs::Quote,
        state_reason: None,
        cancel_reason: None,
        commitments: vec![make_prog("c1", CommitmentState::Promised, vec![])],
    };
    let snap_key1 = crate::app::decide::snapshot_id("rec_deal_cp", 1);
    let snap_row1 = SnapshotRow {
        seq: 1,
        agreement: "rec_deal_cp".to_string(),
        conversation: "conv_1".to_string(),
        snapshot: formed_snap,
        envelope: "{}".to_string(),
        record_id: "snap_rec_1".to_string(),
        message_id: None,
        stored_at_secs: now,
    };
    put_row(&cp_fix.host, AGREEMENT_PROGRESS, &snap_key1, &snap_row1).await.unwrap();
    agr_row
}

#[tokio::test]
async fn statement_export_before_and_after_counterparty_files_snapshot() {
    let cp_key = std::sync::Arc::new(Identity::generate().expect("cp"));
    let cp_fix = fixture_with_master(cp_key);
    let author = Identity::generate().expect("author");
    let author_did = substrate::derive_did_key(&author.public_key());
    let now = clock::now_secs();

    let agr_row = setup_cp_deal(&cp_fix, &author_did, now).await;

    let stmt_before = statement_ops::export(
        &cp_fix.host,
        StatementExportInput { from_secs: now, to_secs: now + 100 },
    )
    .await
    .expect("export before snapshot");

    assert_eq!(stmt_before.rows.len(), 0);
    assert_eq!(stmt_before.open_at_end.len(), 1);
    assert_eq!(stmt_before.open_at_end[0].commitment_id, "c1");

    let mut updated_agr = agr_row.clone();
    updated_agr.seq = Some(2);
    updated_agr.commitments =
        vec![make_prog("c1", CommitmentState::Fulfilled, vec!["h_cp".into()])];
    put_row(&cp_fix.host, AGREEMENTS, "rec_deal_cp", &updated_agr).await.unwrap();

    store_progress_snapshot(
        &cp_fix,
        "rec_deal_cp",
        2,
        AgreementState::Agreed,
        vec![make_prog("c1", CommitmentState::Fulfilled, vec!["h_cp".into()])],
        now + 10,
    )
    .await;

    let stmt_after = statement_ops::export(
        &cp_fix.host,
        StatementExportInput { from_secs: now, to_secs: now + 100 },
    )
    .await
    .expect("export after snapshot");

    assert_eq!(stmt_after.rows.len(), 1);
    assert_eq!(stmt_after.rows[0].commitment_id, "c1");
    assert_eq!(stmt_after.rows[0].settled_at_secs, now + 5);
    assert_eq!(stmt_after.open_at_end.len(), 0);
    assert_eq!(stmt_after.eeps, EepSection::NotAvailable);
}

#[tokio::test]
async fn statement_export_with_and_without_money() {
    let fix = fixture();
    let cp = Identity::generate().expect("cp");
    let now = clock::now_secs();

    let mut service1 = make_service_commitment("c1");
    service1.giver = DealRole::Counterparty;
    let (agreement, _) = setup_agreed_deal(&fix, &cp, vec![service1], now).await;

    fix.host.set_now(now + 10);
    fulfilment_ops::sign_at(
        &fix.host,
        FulfilmentSignInput {
            agreement: agreement.clone(),
            commitment_id: "c1".to_string(),
            settlement_note: Some(sample_note("Service delivered successfully")),
        },
        now + 10,
    )
    .await
    .expect("sign fulfilment");

    let stmt = statement_ops::export(
        &fix.host,
        StatementExportInput { from_secs: now, to_secs: now + 100 },
    )
    .await
    .expect("export statement");

    assert_eq!(stmt.rows.len(), 1);
    assert_eq!(stmt.rows[0].commitment_id, "c1");
    assert_eq!(stmt.rows[0].direction, Direction::Received);
    assert_eq!(stmt.rows[0].resource_kind, ResourceKind::GoodsOrService);
    assert_eq!(
        stmt.rows[0].settlement_note.as_ref().unwrap().text,
        "Service delivered successfully"
    );
    assert_eq!(stmt.totals.len(), 1);
    assert_eq!(stmt.totals[0].currency, "EUR");
    assert_eq!(stmt.totals[0].direction, Direction::Received);
    assert_eq!(stmt.totals[0].resource_kind, ResourceKind::GoodsOrService);
    assert_eq!(stmt.totals[0].sum_minor, 5000);
}

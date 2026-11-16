use syneroym_roym_core::progress::{AgreementProgressPayload, AgreementState};

use crate::app::{
    AGREEMENT_PROGRESS,
    decide::{SnapshotRow, snapshot_id},
    put_row,
    sync::receipts::check_progress_fork,
    tests::fixture,
};

#[tokio::test]
async fn test_progress_fork_detection() {
    let f = fixture();
    let now = f.host.now();
    let agr_id = "rec_quote_1";
    let snap_key = snapshot_id(agr_id, 1);

    let initial_payload = AgreementProgressPayload {
        agreement: agr_id.to_string(),
        conversation: "conv_1".to_string(),
        seq: 1,
        counterparty_did: "did:key:cp".to_string(),
        quote_author_did: f.master_did.clone(),
        state: AgreementState::Agreed,
        state_reason: None,
        cancel_reason: None,
        commitments: vec![],
    };

    let outcome1 = check_progress_fork(
        &f.host,
        &initial_payload,
        &snap_key,
        "rec_snap_orig",
        "{\"envelope\":\"orig\"}",
        now,
    )
    .await
    .unwrap();
    assert_eq!(outcome1, None);

    let orig_row = SnapshotRow {
        seq: 1,
        agreement: agr_id.to_string(),
        conversation: "conv_1".to_string(),
        snapshot: initial_payload.clone(),
        envelope: "{\"envelope\":\"orig\"}".to_string(),
        record_id: "rec_snap_orig".to_string(),
        message_id: Some("msg_1".to_string()),
        stored_at_secs: now,
    };
    put_row(&f.host, AGREEMENT_PROGRESS, &snap_key, &orig_row).await.unwrap();

    let outcome2 = check_progress_fork(
        &f.host,
        &initial_payload,
        &snap_key,
        "rec_snap_orig",
        "{\"envelope\":\"orig\"}",
        now,
    )
    .await
    .unwrap();
    assert_eq!(outcome2, None);

    let conflicting_payload = initial_payload.clone();
    let outcome3 = check_progress_fork(
        &f.host,
        &conflicting_payload,
        &snap_key,
        "rec_snap_forked",
        "{\"envelope\":\"forked\"}",
        now,
    )
    .await
    .unwrap();
    assert_eq!(outcome3, Some("progress-fork".to_string()));

    let fork_key = format!("fork:{}:1:rec_snap_forked", agr_id);
    let fork_row: Option<SnapshotRow> =
        crate::app::get_row(&f.host, AGREEMENT_PROGRESS, &fork_key).await.unwrap();
    assert!(fork_row.is_some(), "fork row must be stored");
    let fr = fork_row.unwrap();
    assert_eq!(fr.record_id, "rec_snap_forked");

    let stored_orig: Option<SnapshotRow> =
        crate::app::get_row(&f.host, AGREEMENT_PROGRESS, &snap_key).await.unwrap();
    assert_eq!(stored_orig.unwrap().record_id, "rec_snap_orig");
}

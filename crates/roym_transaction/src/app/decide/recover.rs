//! Recovery of deciding rows and reservations cleanup.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    deal::ResourceKind,
    paging,
    progress::{self as core_progress, AgreementState},
    transaction::DealRole,
};

use super::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, CountersignRow, DecisionRow, DecisionState,
    EVENT_PROGRESS, LEDGER, SnapshotRow, countersign_id, decision_id, ensure_snapshot_sent,
    finalize_agreed_deal, get_row, handle_not_formed, progress, put_row, repeat_both,
    repeat_countersigned, reserve_author_holds, snapshot_id,
};
use crate::app::hold::{Reservations, SeatLedger};

pub(crate) async fn release_all_author_holds<H: AppHost, R: Reservations>(
    host: &H,
    row: &AgreementRow,
    reservations: &R,
) {
    for c in &row.terms.commitments {
        if c.giver == DealRole::QuoteAuthor && c.resource_kind == ResourceKind::ServiceInSlot {
            let _ = reservations.release(host, &row.quote_record_id, &c.commitment_id).await;
        }
    }
}

pub(crate) async fn repeat_not_formed<H: AppHost, R: Reservations>(
    host: &H,
    row: &mut AgreementRow,
    mut snap_row: SnapshotRow,
    reservations: &R,
    now: u64,
) -> Result<bool, String> {
    release_all_author_holds(host, row, reservations).await;

    if !ensure_snapshot_sent(host, &row.conversation, &mut snap_row, now).await? {
        return Ok(false);
    }

    row.state = Some(AgreementState::NotFormed);
    row.state_reason = snap_row.snapshot.state_reason.clone();
    row.cancel_reason = None;
    row.seq = Some(1);
    row.commitments = snap_row.snapshot.commitments.clone();
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;

    let dec_not_formed = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::NotFormed.as_str().to_string(),
        agreement: row.quote_record_id.clone(),
        quote_id: row.quote_id.clone(),
        reason: snap_row.snapshot.state_reason.clone(),
        created_at_secs: now,
    };
    put_row(host, LEDGER, &decision_id(&row.quote_id), &dec_not_formed).await?;

    let _ = host
        .record_event(
            EVENT_PROGRESS.to_string(),
            row.conversation.clone(),
            Some(row.quote_record_id.clone()),
        )
        .await;
    Ok(false)
}

async fn finish_expired_deciding<H: AppHost, R: Reservations>(
    host: &H,
    row: &mut AgreementRow,
    reservations: &R,
    now: u64,
) -> Result<bool, String> {
    let snapshot = core_progress::open(
        row.quote_record_id.clone(),
        row.conversation.clone(),
        row.counterparty_did.clone(),
        row.quote_author_did.clone(),
        &row.terms,
        Some("quote-expired".to_string()),
    );
    let (snap_env, snap_rec_id) = progress::sign(host, &snapshot).await?;
    let snap_key = snapshot_id(&row.quote_record_id, 1);
    let mut snap_row = SnapshotRow {
        seq: 1,
        agreement: row.quote_record_id.clone(),
        conversation: row.conversation.clone(),
        snapshot: snapshot.clone(),
        envelope: snap_env,
        record_id: snap_rec_id,
        message_id: None,
        stored_at_secs: now,
    };
    put_row(host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await?;

    release_all_author_holds(host, row, reservations).await;

    if !ensure_snapshot_sent(host, &row.conversation, &mut snap_row, now).await? {
        return Ok(false);
    }

    row.state = Some(AgreementState::NotFormed);
    row.state_reason = Some("quote-expired".to_string());
    row.cancel_reason = None;
    row.seq = Some(1);
    row.commitments = snapshot.commitments;
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;

    let dec_not_formed = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::NotFormed.as_str().to_string(),
        agreement: row.quote_record_id.clone(),
        quote_id: row.quote_id.clone(),
        reason: Some("quote-expired".to_string()),
        created_at_secs: now,
    };
    put_row(host, LEDGER, &decision_id(&row.quote_id), &dec_not_formed).await?;

    let _ = host
        .record_event(
            EVENT_PROGRESS.to_string(),
            row.conversation.clone(),
            Some(row.quote_record_id.clone()),
        )
        .await;

    Ok(false)
}

pub(crate) async fn finish_deciding_row_with_reservations<H: AppHost, R: Reservations>(
    host: &H,
    quote_id: &str,
    reservations: R,
    now: u64,
) -> Result<bool, String> {
    let dec_key = decision_id(quote_id);
    let dec = match get_row::<DecisionRow, _>(host, LEDGER, &dec_key).await? {
        Some(d) if d.decision_state() == Some(DecisionState::Deciding) => d,
        _ => return Ok(false),
    };
    let mut row = match get_row::<AgreementRow, _>(host, AGREEMENTS, &dec.agreement).await? {
        Some(r) => r,
        None => return Ok(false),
    };

    let snap_key = snapshot_id(&row.quote_record_id, 1);
    let stored_snap: Option<SnapshotRow> = get_row(host, AGREEMENT_PROGRESS, &snap_key).await?;

    let cs_key = countersign_id(&row.quote_record_id);
    let stored_cs: Option<CountersignRow> = get_row(host, LEDGER, &cs_key).await?;

    if let Some(snap_row) = stored_snap {
        if snap_row.snapshot.state == AgreementState::NotFormed {
            return repeat_not_formed(host, &mut row, snap_row, &reservations, now).await;
        }
        if let Some(cs) = stored_cs {
            return repeat_both(host, &mut row, snap_row, cs, now).await;
        }
    }

    if stored_cs.is_none() && now >= row.terms.quote_expires_at_secs {
        return finish_expired_deciding(host, &mut row, &reservations, now).await;
    }

    if let Some(cs) = stored_cs {
        return repeat_countersigned(host, &mut row, cs, now).await;
    }

    let (reserved, failed) = reserve_author_holds(host, &row, &reservations, now).await?;
    if let Some(reason) = failed {
        return handle_not_formed(host, &mut row, &reservations, &reserved, reason, now).await;
    }

    for cid in &reserved {
        if let Err(reason) = reservations.confirm(host, &row.quote_record_id, cid).await {
            return handle_not_formed(host, &mut row, &reservations, &reserved, &reason, now).await;
        }
    }

    finalize_agreed_deal(host, &mut row, now).await
}

pub(crate) async fn finish_deciding_row<H: AppHost>(
    host: &H,
    quote_id: &str,
    now: u64,
) -> Result<bool, String> {
    finish_deciding_row_with_reservations(host, quote_id, SeatLedger, now).await
}

pub(crate) async fn recover_deciding_rows<H: AppHost>(host: &H, now: u64) -> Result<(), String> {
    let filter = serde_json::json!({
        "kind": "decision",
        "state": DecisionState::Deciding.as_str(),
    })
    .to_string();
    let deciding_rows: Vec<DecisionRow> = paging::query_all(host, LEDGER, Some(filter)).await?;
    for dec in deciding_rows {
        let _ = finish_deciding_row(host, &dec.quote_id, now).await;
    }
    Ok(())
}

//! Agreement progress operations: transitions, cancel, progress queries, and
//! ticks.

use serde::{Deserialize, Serialize};
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    clock,
    deal::{ResourceKind, transition::track_end},
    envelope::Response,
    method::Reply,
    paging,
    progress::{
        self as core_progress, AGREEMENT_PROGRESS_VERSION, AgreementEvent,
        AgreementProgressPayload, AgreementState, CancelReason, RECORD_AGREEMENT_PROGRESS,
        TransitionError,
    },
    signing,
    transaction::{DealRole, EVENT_PROGRESS},
};

use super::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRef, AgreementRow,
    agreement_ops::AgreementView,
    decide::{SnapshotRow, snapshot_id},
    get_row,
    hold::{Reservations, SeatLedger},
    progress, put_row, send_card_and_file,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementCancelInput {
    pub agreement: String,
    pub reason: CancelReason,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgreementProgressOutput {
    pub snapshots: Vec<AgreementProgressSnapshotRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgreementProgressSnapshotRow {
    pub seq: u32,
    pub snapshot: AgreementProgressPayload,
    pub envelope: String,
    pub record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

pub(crate) async fn transition_at<H: AppHost>(
    host: &H,
    agreement: &str,
    event: AgreementEvent,
    now: u64,
) -> Reply<AgreementRow> {
    let mut row: AgreementRow = match get_row(host, AGREEMENTS, agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let highest_seq = row.seq.unwrap_or(1);
    let snap_key = snapshot_id(agreement, highest_seq);
    let cur_snap_row: SnapshotRow = match get_row(host, AGREEMENT_PROGRESS, &snap_key).await {
        Ok(Some(sr)) => sr,
        Ok(None) => return Err(Response::invalid_params("no-progress-snapshot")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let agreed_at = row.agreed_at_secs.unwrap_or(now);
    let next_snap =
        match core_progress::apply(&cur_snap_row.snapshot, &row.terms, agreed_at, &event) {
            Ok(None) => {
                progress::resend_if_unsent(host, agreement, highest_seq, &row.conversation).await;
                return Ok(row);
            }
            Ok(Some(mut n)) => {
                n.seq = highest_seq + 1;
                n
            }
            Err(TransitionError::CannotCancel) => {
                return Err(Response::invalid_params("commitment-moved"));
            }
            Err(e) => return Err(Response::invalid_params(e.to_string())),
        };

    let (snap_env, snap_rec_id) = match progress::sign(host, &next_snap).await {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e)),
    };

    let next_seq = next_snap.seq;
    let next_snap_key = snapshot_id(agreement, next_seq);
    let mut snap_row = SnapshotRow {
        seq: next_seq,
        agreement: agreement.to_string(),
        conversation: row.conversation.clone(),
        snapshot: next_snap.clone(),
        envelope: snap_env.clone(),
        record_id: snap_rec_id,
        message_id: None,
        stored_at_secs: now,
    };

    let payload = match serde_json::to_vec(&snap_row) {
        Ok(b) => b,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let write = RecordWriteValue { id: next_snap_key.clone(), payload };
    if let Err(e) = AppDataLayer::create(host, AGREEMENT_PROGRESS.to_string(), vec![write]).await {
        return Err(Response::internal_error(e.to_string()));
    }

    if next_snap.state == AgreementState::Cancelled {
        let reservations = SeatLedger;
        for c in &row.terms.commitments {
            if c.giver == DealRole::QuoteAuthor && c.resource_kind == ResourceKind::ServiceInSlot {
                let _ = reservations.release(host, agreement, &c.commitment_id).await;
            }
        }
    }

    let (message_id, _, _) = send_card_and_file(
        host,
        &row.conversation,
        RECORD_AGREEMENT_PROGRESS,
        AGREEMENT_PROGRESS_VERSION,
        &snap_env,
        now,
        None,
    )
    .await;

    if !message_id.is_empty() {
        snap_row.message_id = Some(message_id);
        let _ = put_row(host, AGREEMENT_PROGRESS, &next_snap_key, &snap_row).await;
    }

    row.state = Some(next_snap.state);
    row.state_reason = next_snap.state_reason;
    row.cancel_reason = next_snap.cancel_reason;
    row.seq = Some(next_seq);
    row.commitments = next_snap.commitments;
    row.updated_at_secs = now;
    if let Err(e) = put_row(host, AGREEMENTS, agreement, &row).await {
        return Err(Response::internal_error(e));
    }

    let _ = host
        .record_event(
            EVENT_PROGRESS.to_string(),
            row.conversation.clone(),
            Some(agreement.to_string()),
        )
        .await;

    Ok(row)
}

pub(crate) async fn cancel<H: AppHost>(
    host: &H,
    input: AgreementCancelInput,
) -> Reply<AgreementView> {
    let now = clock::now_secs();
    let agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    if owner != agr.quote_author_did {
        return Err(Response::invalid_params("quote-author-only"));
    }

    let updated =
        transition_at(host, &input.agreement, AgreementEvent::Cancel { reason: input.reason }, now)
            .await?;

    Ok(AgreementView::from(updated))
}

pub(crate) async fn progress<H: AppHost>(
    host: &H,
    input: AgreementRef,
) -> Reply<AgreementProgressOutput> {
    progress_at(host, input, clock::now_secs()).await
}

pub(crate) async fn progress_at<H: AppHost>(
    host: &H,
    input: AgreementRef,
    now: u64,
) -> Reply<AgreementProgressOutput> {
    let mut agr: AgreementRow = match get_row(host, AGREEMENTS, &input.agreement).await {
        Ok(Some(a)) => a,
        Ok(None) => return Err(Response::invalid_params("no-such-agreement")),
        Err(e) => return Err(Response::internal_error(e)),
    };

    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    if owner == agr.quote_author_did {
        let _ = tick_agreement_at(host, &mut agr, now).await;
    } else if owner != agr.counterparty_did {
        return Err(Response::invalid_params("not-a-party"));
    }

    let highest_seq = agr.seq.unwrap_or(0);
    let mut snapshots = Vec::new();
    for seq in (1..=highest_seq).rev() {
        let snap_key = snapshot_id(&input.agreement, seq);
        if let Ok(Some(sr)) = get_row::<SnapshotRow, _>(host, AGREEMENT_PROGRESS, &snap_key).await {
            snapshots.push(AgreementProgressSnapshotRow {
                seq: sr.seq,
                snapshot: sr.snapshot,
                envelope: sr.envelope,
                record_id: sr.record_id,
                message_id: sr.message_id,
            });
        }
    }

    Ok(AgreementProgressOutput { snapshots })
}

pub(crate) async fn tick_agreement_at<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    now: u64,
) -> Result<(), String> {
    if row.state != Some(AgreementState::Agreed) {
        return Ok(());
    }
    let agreed_at = match row.agreed_at_secs {
        Some(a) => a,
        None => return Ok(()),
    };
    let needs_tick = row.terms.commitments.iter().any(|c| {
        let end = track_end(c.window.as_ref(), agreed_at);
        now >= end
    });
    if needs_tick {
        let res =
            transition_at(host, &row.quote_record_id, AgreementEvent::Tick { now_secs: now }, now)
                .await;
        if let Ok(updated) = res {
            *row = updated;
        }
    }
    Ok(())
}

pub(crate) async fn tick_conversation_at<H: AppHost>(
    host: &H,
    conversation: &str,
    now: u64,
) -> Result<(), String> {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(e.to_string()),
    };
    let filter = serde_json::json!({ "conversation": conversation }).to_string();
    let mut agreements: Vec<AgreementRow> =
        match paging::query_all(host, AGREEMENTS, Some(filter)).await {
            Ok(a) => a,
            Err(e) => return Err(e),
        };

    for row in &mut agreements {
        if owner == row.quote_author_did {
            let _ = tick_agreement_at(host, row, now).await;
        }
    }
    Ok(())
}

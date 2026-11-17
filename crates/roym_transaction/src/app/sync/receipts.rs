//! Inbound filing for agreement progress cards.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    deal::ResourceKind,
    progress::{self, AgreementState, MAX_DEFER_SECS},
    record::Envelope,
    transaction::{self, DealRole},
};

use super::{
    super::{
        AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, CardRow, EVENT_PROGRESS, QUOTE_HISTORY,
        decide::{SnapshotRow, snapshot_id},
        get_bytes, get_row,
        hold::{Reservations, SeatLedger},
        put_card, put_row,
    },
    FileCardResult, refuse_card,
};

pub(crate) async fn defer_or_refuse<H: AppHost>(
    host: &H,
    row: CardRow,
    envelope_json: &str,
    now: u64,
    reason: &str,
) -> Result<FileCardResult, String> {
    let issued_at = Envelope::from_json(envelope_json).map(|e| e.issued_at_secs).unwrap_or(0);
    if issued_at + MAX_DEFER_SECS > now {
        Ok(FileCardResult {
            filed: false,
            refused: false,
            unknown: false,
            countersigned: false,
            deferred: true,
        })
    } else {
        refuse_card(host, row, reason).await
    }
}

async fn load_progress_quote<H: AppHost>(
    host: &H,
    agreement: &str,
    now: u64,
) -> Result<Result<transaction::RecordVerdict<transaction::QuotePayload>, Option<String>>, String> {
    let quote_bytes = match get_bytes(host, QUOTE_HISTORY, agreement).await? {
        Some(b) => b,
        None => return Ok(Err(None)),
    };
    let quote_env = String::from_utf8_lossy(&quote_bytes);
    let qv = transaction::verify_quote(&quote_env, now);
    if !qv.verified {
        return Ok(Err(Some("stored quote does not verify".to_string())));
    }
    Ok(Ok(qv))
}

fn validate_progress_parties<'a>(
    qv: &'a transaction::RecordVerdict<transaction::QuotePayload>,
    p: &progress::AgreementProgressPayload,
) -> Result<&'a transaction::QuotePayload, &'static str> {
    let qp = match qv.payload.as_ref() {
        Some(q) => q,
        None => return Err("stored quote has no payload"),
    };
    if p.quote_author_did != qv.issuer.as_deref().unwrap_or_default()
        || p.counterparty_did != qp.counterparty_did
    {
        return Err("names the wrong parties");
    }
    Ok(qp)
}

struct WriterProgressContext<'a> {
    snap_key: &'a str,
    rec_id: &'a str,
    envelope: &'a str,
    message_id: &'a str,
    now: u64,
}

async fn apply_writer_progress<H: AppHost>(
    host: &H,
    p: &progress::AgreementProgressPayload,
    agr: AgreementRow,
    ctx: WriterProgressContext<'_>,
) -> Result<(), String> {
    if p.seq > agr.seq.unwrap_or(0) {
        let mut updated_agr = agr;
        updated_agr.seq = Some(p.seq);
        updated_agr.state = Some(p.state);
        updated_agr.state_reason = p.state_reason.clone();
        updated_agr.cancel_reason = p.cancel_reason;
        updated_agr.commitments = p.commitments.clone();
        updated_agr.updated_at_secs = ctx.now;
        put_row(host, AGREEMENTS, &p.agreement, &updated_agr).await?;
    }
    if get_row::<SnapshotRow, _>(host, AGREEMENT_PROGRESS, ctx.snap_key).await?.is_none() {
        let snap_row = SnapshotRow {
            seq: p.seq,
            agreement: p.agreement.clone(),
            conversation: p.conversation.clone(),
            snapshot: p.clone(),
            envelope: ctx.envelope.to_string(),
            record_id: ctx.rec_id.to_string(),
            message_id: Some(ctx.message_id.to_string()),
            stored_at_secs: ctx.now,
        };
        put_row(host, AGREEMENT_PROGRESS, ctx.snap_key, &snap_row).await?;
    }
    Ok(())
}

fn init_progress_agreement_row(
    p: &progress::AgreementProgressPayload,
    qp: &transaction::QuotePayload,
    conversation: &str,
    now: u64,
) -> AgreementRow {
    AgreementRow {
        quote_record_id: p.agreement.clone(),
        quote_id: qp.quote_id.clone(),
        conversation: conversation.to_string(),
        counterparty_did: p.counterparty_did.clone(),
        quote_author_did: p.quote_author_did.clone(),
        terms: qp.terms.clone(),
        state: None,
        state_reason: None,
        cancel_reason: None,
        seq: None,
        commitments: Vec::new(),
        agreed_at_secs: None,
        counterparty: None,
        quote_author: None,
        updated_at_secs: now,
    }
}

pub(crate) async fn file_progress_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = progress::verify_agreement_progress(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "agreement progress does not verify".to_string());
        return refuse_card(host, row, reason).await;
    }
    let p = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, row, "missing payload").await,
    };
    if p.conversation != conversation {
        return refuse_card(host, row, "card names another conversation").await;
    }
    let is_counterparty = owner == p.counterparty_did;
    let is_writer = owner == p.quote_author_did;
    if !is_counterparty && !is_writer {
        return refuse_card(
            host,
            row,
            "progress for an agreement this node is neither the counterparty nor quote author of",
        )
        .await;
    }
    let qv = match load_progress_quote(host, &p.agreement, now).await? {
        Ok(qv) => qv,
        Err(Some(err)) => return refuse_card(host, row, err).await,
        Err(None) => {
            return defer_or_refuse(
                host,
                row,
                envelope,
                now,
                "names a quote this node does not hold",
            )
            .await;
        }
    };
    if v.issuer != qv.signer_did {
        return refuse_card(host, row, "not signed by the service that signed the quote").await;
    }
    let qp = match validate_progress_parties(&qv, p) {
        Ok(qp) => qp,
        Err(err) => return refuse_card(host, row, err).await,
    };

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &p.agreement).await? {
        Some(a) => a,
        None => init_progress_agreement_row(p, qp, conversation, now),
    };

    let rec_id = v.record_id.clone().unwrap_or_default();
    let snap_key = snapshot_id(&p.agreement, p.seq);

    if is_writer {
        let ctx = WriterProgressContext {
            snap_key: &snap_key,
            rec_id: &rec_id,
            envelope,
            message_id: &row.message_id,
            now,
        };
        apply_writer_progress(host, p, agr, ctx).await?;
    } else if p.seq > agr.seq.unwrap_or(0) {
        let ctx = NewerSnapshotContext {
            snap_key: &snap_key,
            rec_id,
            envelope,
            issued_at: v.issued_at_secs,
            now,
        };
        apply_newer_snapshot(host, p, agr, ctx).await?;
    } else if p.seq == agr.seq.unwrap_or(0)
        && let Some(reason) =
            check_progress_fork(host, p, &snap_key, &rec_id, envelope, now).await?
    {
        row.reason = Some(reason);
    }

    row.known = true;
    row.verified = true;
    row.record_id = v.record_id;
    row.data = serde_json::to_value(p).ok();
    put_card(host, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

struct NewerSnapshotContext<'a> {
    snap_key: &'a str,
    rec_id: String,
    envelope: &'a str,
    issued_at: Option<u64>,
    now: u64,
}

async fn apply_newer_snapshot<H: AppHost>(
    host: &H,
    p: &progress::AgreementProgressPayload,
    mut agr: AgreementRow,
    ctx: NewerSnapshotContext<'_>,
) -> Result<(), String> {
    let snap_row = SnapshotRow {
        seq: p.seq,
        agreement: p.agreement.clone(),
        conversation: p.conversation.clone(),
        snapshot: p.clone(),
        envelope: ctx.envelope.to_string(),
        record_id: ctx.rec_id,
        message_id: None,
        stored_at_secs: ctx.now,
    };
    put_row(host, AGREEMENT_PROGRESS, ctx.snap_key, &snap_row).await?;

    agr.state = Some(p.state);
    agr.state_reason = p.state_reason.clone();
    agr.cancel_reason = p.cancel_reason;
    agr.seq = Some(p.seq);
    agr.commitments = p.commitments.clone();
    if agr.agreed_at_secs.is_none() && p.state == AgreementState::Agreed {
        let agreed_at = agr
            .quote_author
            .as_ref()
            .map_or_else(|| ctx.issued_at.unwrap_or(ctx.now), |h| h.issued_at_secs);
        agr.agreed_at_secs = Some(agreed_at);
    }
    agr.updated_at_secs = ctx.now;
    put_row(host, AGREEMENTS, &p.agreement, &agr).await?;

    if p.state == AgreementState::NotFormed || p.state == AgreementState::Cancelled {
        for c in &agr.terms.commitments {
            if c.giver == DealRole::Counterparty && c.resource_kind == ResourceKind::ServiceInSlot {
                let _ = SeatLedger.release(host, &p.agreement, &c.commitment_id).await;
            }
        }
    }

    let _ = host
        .record_event(EVENT_PROGRESS.to_string(), p.conversation.clone(), Some(p.agreement.clone()))
        .await;
    Ok(())
}

pub(crate) async fn check_progress_fork<H: AppHost>(
    host: &H,
    p: &progress::AgreementProgressPayload,
    snap_key: &str,
    rec_id: &str,
    envelope: &str,
    now: u64,
) -> Result<Option<String>, String> {
    if let Ok(Some(existing_snap)) =
        get_row::<SnapshotRow, _>(host, AGREEMENT_PROGRESS, snap_key).await
        && existing_snap.record_id != rec_id
    {
        let fork_key = format!("fork:{}:{}:{}", p.agreement, p.seq, rec_id);
        let snap_row = SnapshotRow {
            seq: p.seq,
            agreement: p.agreement.clone(),
            conversation: p.conversation.clone(),
            snapshot: p.clone(),
            envelope: envelope.to_string(),
            record_id: rec_id.to_string(),
            message_id: None,
            stored_at_secs: now,
        };
        put_row(host, AGREEMENT_PROGRESS, &fork_key, &snap_row).await?;
        return Ok(Some("progress-fork".to_string()));
    }
    Ok(None)
}

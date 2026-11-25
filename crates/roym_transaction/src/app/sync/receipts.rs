//! Inbound filing for agreement progress cards.

use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    progress::{self, AgreementState, MAX_DEFER_SECS},
    record::Envelope,
    transaction::{DealRole, RecordVerdict},
};

use super::{
    super::{
        AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, CardRow, EVENT_PROGRESS,
        decide::{SnapshotRow, snapshot_id},
        disclosures::{self, HeldQuote, LoadQuoteError},
        get_row,
        hooks::{self, Ended, ExtensionHooks},
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
) -> Result<Result<HeldQuote, Option<String>>, String> {
    match disclosures::load_quote(host, agreement, now).await {
        Ok(held) => Ok(Ok(held)),
        Err(LoadQuoteError::Missing) => Ok(Err(None)),
        Err(LoadQuoteError::Invalid(e)) => {
            Ok(Err(Some(format!("stored quote does not verify: {e}"))))
        }
        Err(LoadQuoteError::Storage(e)) => Err(e),
    }
}

fn validate_progress_parties(
    held: &HeldQuote,
    p: &progress::AgreementProgressPayload,
) -> Result<(), &'static str> {
    if p.quote_author_did != held.verdict.issuer.as_deref().unwrap_or_default()
        || p.counterparty_did != held.signed.counterparty_did
    {
        return Err("names the wrong parties");
    }
    Ok(())
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
        updated_agr.formed_as = Some(p.formed_as);
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
    held: &HeldQuote,
    conversation: &str,
    now: u64,
) -> AgreementRow {
    AgreementRow {
        formed_as: Some(p.formed_as),
        quote_record_id: p.agreement.clone(),
        quote_id: held.signed.quote_id.clone(),
        conversation: conversation.to_string(),
        counterparty_did: p.counterparty_did.clone(),
        quote_author_did: p.quote_author_did.clone(),
        terms: held.terms.clone(),
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

/// Verifies the card's record and checks it names this conversation. Returns
/// the verdict and the payload, because the caller still needs the signer.
fn verify_progress_card(
    envelope: &str,
    conversation: &str,
    now: u64,
) -> Result<
    (RecordVerdict<progress::AgreementProgressPayload>, progress::AgreementProgressPayload),
    String,
> {
    let v = progress::verify_agreement_progress(envelope, now);
    if !v.verified {
        return Err(v.reason.unwrap_or_else(|| "agreement progress does not verify".to_string()));
    }
    let p = v.payload.clone().ok_or_else(|| "missing payload".to_string())?;
    if p.conversation != conversation {
        return Err("card names another conversation".to_string());
    }
    Ok((v, p))
}

pub(crate) async fn file_progress_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    envelope: &str,
    disclosures: &[String],
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    if !disclosures.is_empty() {
        return refuse_card(host, row, "disclosures not permitted on agreement progress card")
            .await;
    }
    let (verdict, p) = match verify_progress_card(envelope, conversation, now) {
        Ok(v) => v,
        Err(reason) => return refuse_card(host, row, &reason).await,
    };
    let v = &verdict;
    let p = &p;
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
    let held = match load_progress_quote(host, &p.agreement, now).await? {
        Ok(held) => held,
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
    if v.issuer != held.verdict.signer_did {
        return refuse_card(host, row, "not signed by the service that signed the quote").await;
    }
    if let Err(err) = validate_progress_parties(&held, p) {
        return refuse_card(host, row, err).await;
    }

    let agr: AgreementRow = match get_row(host, AGREEMENTS, &p.agreement).await? {
        Some(a) => a,
        None => init_progress_agreement_row(p, &held, conversation, now),
    };
    if let Some(held_as) = agr.formed_as
        && held_as != p.formed_as
    {
        return refuse_card(host, row, "formed-as-changed").await;
    }

    let rec_id = v.record_id.clone().unwrap_or_default();
    let snap_key = snapshot_id(&p.agreement, p.seq);

    if is_writer {
        if let Ok(Some(existing_snap)) =
            get_row::<SnapshotRow, _>(host, AGREEMENT_PROGRESS, &snap_key).await
            && existing_snap.record_id == rec_id
        {
            row.repeat = true;
        }
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
    } else if p.seq == agr.seq.unwrap_or(0) {
        if let Some(reason) =
            check_progress_fork(host, p, &snap_key, &rec_id, envelope, now).await?
        {
            row.reason = Some(reason);
        } else {
            row.repeat = true;
        }
    }

    row.known = true;
    row.verified = true;
    row.record_id = v.record_id.clone();
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
    agr.formed_as = Some(p.formed_as);
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

    let ended = Ended { state: p.state, commitments: &p.commitments };
    hooks::release_ended(host, &ExtensionHooks, &agr, DealRole::Counterparty, ended, ctx.now).await;

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

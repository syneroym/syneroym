//! The writer's decision, in its fixed step order.

use syneroym_app_host::{AppDataLayer, AppHost, AppSigning, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    deal::{Commitment, SignedTerms},
    progress::{
        self as core_progress, AGREEMENT_PROGRESS_VERSION, AgreementState, CommitmentProgress,
        RECORD_AGREEMENT_PROGRESS,
    },
    record::{Envelope, RECORD_AGREEMENT_RECEIPT},
    signing,
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, DealRole, QuotePayload, ReceiptHalf,
    },
};

use super::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, EVENT_PROGRESS, LEDGER, OutgoingCard, QUOTES,
    RecordPointerRow,
    disclosures::HeldQuote,
    get_row,
    hooks::{self, ExtensionHooks, Reservations},
    progress, put_row, send_card_and_file,
};

pub(crate) mod recover;
pub(crate) mod rows;

pub(crate) use recover::*;
pub(crate) use rows::*;

pub(crate) async fn run<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    quote: &HeldQuote,
    now: u64,
) -> Result<bool, String> {
    run_with_reservations(host, row, quote, ExtensionHooks, now).await
}

pub(crate) async fn run_with_reservations<H: AppHost, R: Reservations>(
    host: &H,
    row: &mut AgreementRow,
    quote: &HeldQuote,
    reservations: R,
    now: u64,
) -> Result<bool, String> {
    let (cp_issuer, cp_record_id) = match row.counterparty.as_ref() {
        Some(h) => (h.issuer.clone(), h.record_id.clone()),
        None => return Ok(false),
    };

    let ref_key = refused_id(&cp_record_id);
    if get_row::<RefusedRow, _>(host, LEDGER, &ref_key).await?.is_some() {
        return Ok(false);
    }

    let dec_key = decision_id(&quote.signed.quote_id);
    let cs_key = countersign_id(&row.quote_record_id);

    let existing_dec: Option<DecisionRow> = get_row(host, LEDGER, &dec_key).await?;
    let existing_cs: Option<CountersignRow> = get_row(host, LEDGER, &cs_key).await?;

    if let Some(dec) = &existing_dec {
        if dec.agreement == row.quote_record_id {
            return continue_decision(host, row, dec, &reservations, quote, now).await;
        }
        return handle_refusal(
            host,
            row,
            &quote.signed,
            &cp_record_id,
            "quote-already-agreed",
            now,
        )
        .await;
    }
    if existing_cs.is_some() {
        return repeat_path(host, row, &reservations, quote, now).await;
    }

    if cp_issuer != row.counterparty_did {
        return Ok(false);
    }

    if let Some(ptr) = get_row::<RecordPointerRow, _>(host, QUOTES, &quote.signed.quote_id).await? {
        if ptr.record_id != row.quote_record_id {
            return handle_refusal(
                host,
                row,
                &quote.signed,
                &cp_record_id,
                "quote-superseded",
                now,
            )
            .await;
        }
        row.formed_as = ptr.formed_as;
    }

    if now >= quote.signed.terms.quote_expires_at_secs {
        return handle_refusal(host, row, &quote.signed, &cp_record_id, "quote-expired", now).await;
    }

    if !claim_decision_fence(host, row, &quote.signed, &dec_key, now).await? {
        if let Ok(Some(existing_dec)) = get_row::<DecisionRow, _>(host, LEDGER, &dec_key).await {
            if existing_dec.agreement == row.quote_record_id {
                return continue_decision(host, row, &existing_dec, &reservations, quote, now)
                    .await;
            }
            return handle_refusal(
                host,
                row,
                &quote.signed,
                &cp_record_id,
                "quote-already-agreed",
                now,
            )
            .await;
        }
        return Ok(false);
    }

    // 3. Reserve holds through Reservations
    let (reserved, failed) = reserve_author_holds(host, row, &reservations).await?;
    if let Some(reason) = failed {
        return handle_not_formed(host, row, &reservations, &reserved, &reason, now).await;
    }

    // 4. Confirm holds
    if let Some(reason) =
        hooks::confirm_each(host, &reservations, &row.quote_record_id, &reserved).await?
    {
        return handle_not_formed(host, row, &reservations, &reserved, &reason, now).await;
    }

    // 5-7. Store countersign, snapshots, send cards, and complete agreed state
    finalize_agreed_deal(host, row, &quote.signed.terms, &quote.disclosures, now).await
}

pub(crate) async fn stored_or_new_seq1_snapshot<H: AppHost>(
    host: &H,
    row: &AgreementRow,
    reason: &str,
    now: u64,
) -> Result<SnapshotRow, String> {
    let snap_key = snapshot_id(&row.quote_record_id, 1);
    if let Some(stored) = get_row(host, AGREEMENT_PROGRESS, &snap_key).await? {
        return Ok(stored);
    }
    let snapshot = core_progress::open(
        row.quote_record_id.clone(),
        row.conversation.clone(),
        row.counterparty_did.clone(),
        row.quote_author_did.clone(),
        &row.terms,
        row.formed_as(),
        Some(reason.to_string()),
    );
    let (snap_env, snap_rec_id) = progress::sign(host, &snapshot).await?;
    let snap = SnapshotRow {
        seq: 1,
        agreement: row.quote_record_id.clone(),
        conversation: row.conversation.clone(),
        snapshot,
        envelope: snap_env,
        record_id: snap_rec_id,
        message_id: None,
        stored_at_secs: now,
    };
    let write = RecordWriteValue {
        id: snap_key.clone(),
        payload: serde_json::to_vec(&snap).map_err(|e| e.to_string())?,
    };
    if let Some(err) = AppDataLayer::create(host, AGREEMENT_PROGRESS.to_string(), vec![write])
        .await
        .map_err(|e| e.to_string())?
    {
        if let Some(existing) = get_row(host, AGREEMENT_PROGRESS, &snap_key).await? {
            Ok(existing)
        } else {
            Err(format!("failed to store snapshot: {err:?}"))
        }
    } else {
        Ok(snap)
    }
}

async fn handle_refusal<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    quote: &QuotePayload,
    half_record_id: &str,
    reason: &str,
    now: u64,
) -> Result<bool, String> {
    let mut snap_row = stored_or_new_seq1_snapshot(host, row, reason, now).await?;

    if !ensure_snapshot_sent(host, &row.conversation, &mut snap_row, now).await? {
        return Ok(false);
    }

    row.state = Some(AgreementState::NotFormed);
    row.state_reason = Some(reason.to_string());
    row.cancel_reason = None;
    row.seq = Some(1);
    row.commitments = snap_row.snapshot.commitments.clone();
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;

    let ref_row = RefusedRow {
        kind: "refused".to_string(),
        quote_id: quote.quote_id.clone(),
        code: reason.to_string(),
        created_at_secs: now,
    };
    put_row(host, LEDGER, &refused_id(half_record_id), &ref_row).await?;

    let _ = host
        .record_event(
            EVENT_PROGRESS.to_string(),
            row.conversation.clone(),
            Some(row.quote_record_id.clone()),
        )
        .await;

    Ok(false)
}

async fn ensure_countersign_sent<H: AppHost>(
    host: &H,
    conversation: &str,
    cs: &mut CountersignRow,
    disclosures: &[String],
    now: u64,
) -> Result<bool, String> {
    if cs.message_id.is_some() {
        return Ok(true);
    }
    let card = OutgoingCard {
        card_type: RECORD_AGREEMENT_RECEIPT,
        version: AGREEMENT_RECEIPT_VERSION,
        envelope: &cs.half.envelope,
        send: &[],
        view: disclosures,
    };
    let (msg_id, _, err) = send_card_and_file(host, conversation, &card, now, None).await;
    if !msg_id.is_empty() {
        cs.message_id = Some(msg_id);
        let _ = put_row(host, LEDGER, &countersign_id(&cs.agreement), cs).await;
    }
    if let Some(err) = err {
        if err.is_transport() {
            return Err(err.to_string());
        }
        return Ok(false);
    }
    Ok(true)
}

async fn ensure_snapshot_sent<H: AppHost>(
    host: &H,
    conversation: &str,
    snap: &mut SnapshotRow,
    now: u64,
) -> Result<bool, String> {
    if snap.message_id.is_some() {
        return Ok(true);
    }
    let card = OutgoingCard {
        card_type: RECORD_AGREEMENT_PROGRESS,
        version: AGREEMENT_PROGRESS_VERSION,
        envelope: &snap.envelope,
        send: &[],
        view: &[],
    };
    let (msg_id, _, err) = send_card_and_file(host, conversation, &card, now, None).await;
    if !msg_id.is_empty() {
        snap.message_id = Some(msg_id);
        let _ =
            put_row(host, AGREEMENT_PROGRESS, &snapshot_id(&snap.agreement, snap.seq), snap).await;
    }
    if let Some(err) = err {
        if err.is_transport() {
            return Err(err.to_string());
        }
        return Ok(false);
    }
    Ok(true)
}

async fn continue_decision<H: AppHost, R: Reservations>(
    host: &H,
    row: &mut AgreementRow,
    existing_dec: &DecisionRow,
    reservations: &R,
    quote: &HeldQuote,
    now: u64,
) -> Result<bool, String> {
    match existing_dec.state {
        DecisionState::Agreed | DecisionState::NotFormed => {
            resend_completed_decision(host, row, existing_dec.state, &quote.disclosures, now).await
        }
        _ => repeat_path(host, row, reservations, quote, now).await,
    }
}

pub(crate) async fn mark_decision_agreed<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    half: ReceiptHalf,
    commitments: Vec<CommitmentProgress>,
    now: u64,
) -> Result<bool, String> {
    let stored: Option<AgreementRow> = get_row(host, AGREEMENTS, &row.quote_record_id).await?;
    let seq = stored.as_ref().and_then(|r| r.seq).unwrap_or(0);
    if seq > 1 {
        if let Some(mut s) = stored {
            let mut changed = false;
            if s.agreed_at_secs.is_none() {
                s.agreed_at_secs = Some(half.issued_at_secs);
                changed = true;
            }
            if s.quote_author.is_none() {
                s.quote_author = Some(half);
                changed = true;
            }
            if changed {
                s.updated_at_secs = now;
                put_row(host, AGREEMENTS, &row.quote_record_id, &s).await?;
            }
            *row = s;
        }
    } else {
        row.state = Some(AgreementState::Agreed);
        row.state_reason = None;
        row.cancel_reason = None;
        row.seq = Some(1);
        row.commitments = commitments;
        row.agreed_at_secs = Some(half.issued_at_secs);
        row.quote_author = Some(half);
        row.updated_at_secs = now;
        put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;
    }

    let dec_agreed = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Agreed,
        agreement: row.quote_record_id.clone(),
        quote_id: row.quote_id.clone(),
        reason: None,
        created_at_secs: now,
    };
    put_row(host, LEDGER, &decision_id(&row.quote_id), &dec_agreed).await?;

    let _ = host
        .record_event(
            EVENT_PROGRESS.to_string(),
            row.conversation.clone(),
            Some(row.quote_record_id.clone()),
        )
        .await;

    Ok(true)
}

async fn resend_completed_decision<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    state: DecisionState,
    disclosures: &[String],
    now: u64,
) -> Result<bool, String> {
    let snap_key = snapshot_id(&row.quote_record_id, 1);
    let mut stored_snap: Option<SnapshotRow> =
        super::get_row(host, AGREEMENT_PROGRESS, &snap_key).await?;

    let cs_key = countersign_id(&row.quote_record_id);
    let mut stored_cs: Option<CountersignRow> = super::get_row(host, LEDGER, &cs_key).await?;

    if state == DecisionState::Agreed {
        if let Some(cs) = stored_cs.as_mut()
            && !ensure_countersign_sent(host, &row.conversation, cs, disclosures, now).await?
        {
            return Ok(false);
        }
        if let Some(snap) = stored_snap.as_mut()
            && !ensure_snapshot_sent(host, &row.conversation, snap, now).await?
        {
            return Ok(false);
        }
        Ok(true)
    } else {
        if let Some(snap) = stored_snap.as_mut()
            && !ensure_snapshot_sent(host, &row.conversation, snap, now).await?
        {
            return Ok(false);
        }
        Ok(false)
    }
}

async fn claim_decision_fence<H: AppHost>(
    host: &H,
    row: &AgreementRow,
    quote: &QuotePayload,
    dec_key: &str,
    now: u64,
) -> Result<bool, String> {
    let dec_fence = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::Deciding,
        agreement: row.quote_record_id.clone(),
        quote_id: quote.quote_id.clone(),
        reason: None,
        created_at_secs: now,
    };
    let fence_write = RecordWriteValue {
        id: dec_key.to_string(),
        payload: serde_json::to_vec(&dec_fence).map_err(|e| e.to_string())?,
    };
    let outcome = AppDataLayer::create(host, LEDGER.to_string(), vec![fence_write])
        .await
        .map_err(|e| e.to_string())?;
    Ok(outcome.is_none())
}

pub(crate) async fn reserve_author_holds<H: AppHost, R: Reservations>(
    host: &H,
    row: &AgreementRow,
    reservations: &R,
) -> Result<(Vec<Commitment>, Option<String>), String> {
    let reserved = hooks::reserve_each(
        host,
        reservations,
        &row.quote_record_id,
        hooks::reserved_by(&row.terms, DealRole::QuoteAuthor),
    )
    .await;
    match reserved.stopped {
        None => Ok((reserved.held, None)),
        Some(hooks::Stop::Refused(reason)) => Ok((reserved.held, Some(reason))),
        Some(hooks::Stop::Fault(fault)) => Err(fault),
    }
}

pub(crate) async fn finalize_agreed_deal<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    signed_terms: &SignedTerms,
    disclosures: &[String],
    now: u64,
) -> Result<bool, String> {
    let cs_key = countersign_id(&row.quote_record_id);
    let mut cs_row = match get_row::<CountersignRow, _>(host, LEDGER, &cs_key).await? {
        Some(cs) => cs,
        None => {
            let half = sign_author_receipt_half(host, row, signed_terms, now).await?;
            let cs = CountersignRow {
                kind: "countersign".to_string(),
                agreement: row.quote_record_id.clone(),
                half,
                message_id: None,
                created_at_secs: now,
            };
            put_row(host, LEDGER, &cs_key, &cs).await?;
            cs
        }
    };

    let snap_key = snapshot_id(&row.quote_record_id, 1);
    let mut snap_row = match get_row::<SnapshotRow, _>(host, AGREEMENT_PROGRESS, &snap_key).await? {
        Some(s) => s,
        None => {
            let snapshot = core_progress::open(
                row.quote_record_id.clone(),
                row.conversation.clone(),
                row.counterparty_did.clone(),
                row.quote_author_did.clone(),
                &row.terms,
                row.formed_as(),
                None,
            );
            let (snap_env, snap_rec_id) = progress::sign(host, &snapshot).await?;
            let s = SnapshotRow {
                seq: 1,
                agreement: row.quote_record_id.clone(),
                conversation: row.conversation.clone(),
                snapshot,
                envelope: snap_env,
                record_id: snap_rec_id,
                message_id: None,
                stored_at_secs: now,
            };
            put_row(host, AGREEMENT_PROGRESS, &snap_key, &s).await?;
            s
        }
    };

    if !ensure_countersign_sent(host, &row.conversation, &mut cs_row, disclosures, now).await? {
        return Ok(false);
    }
    if !ensure_snapshot_sent(host, &row.conversation, &mut snap_row, now).await? {
        return Ok(false);
    }

    mark_decision_agreed(host, row, cs_row.half, snap_row.snapshot.commitments, now).await
}

pub(crate) async fn repeat_both<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    mut snap_row: SnapshotRow,
    mut cs: CountersignRow,
    disclosures: &[String],
    now: u64,
) -> Result<bool, String> {
    if !ensure_countersign_sent(host, &row.conversation, &mut cs, disclosures, now).await? {
        return Ok(false);
    }
    if !ensure_snapshot_sent(host, &row.conversation, &mut snap_row, now).await? {
        return Ok(false);
    }
    mark_decision_agreed(host, row, cs.half, snap_row.snapshot.commitments, now).await
}

pub(crate) async fn repeat_countersigned<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    mut cs: CountersignRow,
    disclosures: &[String],
    now: u64,
) -> Result<bool, String> {
    let snap_key = snapshot_id(&row.quote_record_id, 1);
    let snapshot = core_progress::open(
        row.quote_record_id.clone(),
        row.conversation.clone(),
        row.counterparty_did.clone(),
        row.quote_author_did.clone(),
        &row.terms,
        row.formed_as(),
        None,
    );
    let (snap_env, snap_rec_id) = progress::sign(host, &snapshot).await?;
    let mut snap_row = SnapshotRow {
        seq: 1,
        agreement: row.quote_record_id.clone(),
        conversation: row.conversation.clone(),
        snapshot: snapshot.clone(),
        envelope: snap_env.clone(),
        record_id: snap_rec_id,
        message_id: None,
        stored_at_secs: now,
    };
    put_row(host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await?;

    if !ensure_countersign_sent(host, &row.conversation, &mut cs, disclosures, now).await? {
        return Ok(false);
    }
    if !ensure_snapshot_sent(host, &row.conversation, &mut snap_row, now).await? {
        return Ok(false);
    }

    mark_decision_agreed(host, row, cs.half, snapshot.commitments, now).await
}

pub(crate) async fn repeat_fresh<H: AppHost, R: Reservations>(
    host: &H,
    row: &mut AgreementRow,
    reservations: &R,
    signed_terms: &SignedTerms,
    disclosures: &[String],
    now: u64,
) -> Result<bool, String> {
    if now >= row.terms.quote_expires_at_secs {
        return Ok(false);
    }
    let (reserved, failed) = reserve_author_holds(host, row, reservations).await?;
    if let Some(reason) = failed {
        return handle_not_formed(host, row, reservations, &reserved, &reason, now).await;
    }

    if let Some(reason) =
        hooks::confirm_each(host, reservations, &row.quote_record_id, &reserved).await?
    {
        return handle_not_formed(host, row, reservations, &reserved, &reason, now).await;
    }

    finalize_agreed_deal(host, row, signed_terms, disclosures, now).await
}

async fn repeat_path<H: AppHost, R: Reservations>(
    host: &H,
    row: &mut AgreementRow,
    reservations: &R,
    quote: &HeldQuote,
    now: u64,
) -> Result<bool, String> {
    let snap_key = snapshot_id(&row.quote_record_id, 1);
    let stored_snap: Option<SnapshotRow> =
        super::get_row(host, AGREEMENT_PROGRESS, &snap_key).await?;

    let cs_key = countersign_id(&row.quote_record_id);
    let stored_cs: Option<CountersignRow> = super::get_row(host, LEDGER, &cs_key).await?;

    if let Some(snap_row) = stored_snap {
        if snap_row.snapshot.state == AgreementState::NotFormed {
            return repeat_not_formed(host, row, snap_row, reservations, now).await;
        }
        if let Some(cs) = stored_cs {
            return repeat_both(host, row, snap_row, cs, &quote.disclosures, now).await;
        }
    } else if let Some(cs) = stored_cs {
        return repeat_countersigned(host, row, cs, &quote.disclosures, now).await;
    } else {
        return repeat_fresh(host, row, reservations, &quote.signed.terms, &quote.disclosures, now)
            .await;
    }

    Ok(false)
}

pub(crate) async fn handle_not_formed<H: AppHost, R: Reservations>(
    host: &H,
    row: &mut AgreementRow,
    reservations: &R,
    reserved_to_release: &[Commitment],
    reason: &str,
    now: u64,
) -> Result<bool, String> {
    let snapshot = core_progress::open(
        row.quote_record_id.clone(),
        row.conversation.clone(),
        row.counterparty_did.clone(),
        row.quote_author_did.clone(),
        &row.terms,
        row.formed_as(),
        Some(reason.to_string()),
    );
    let (snap_env, snap_rec_id) = progress::sign(host, &snapshot).await?;
    let snap_key = snapshot_id(&row.quote_record_id, 1);
    let mut snap_row = SnapshotRow {
        seq: 1,
        agreement: row.quote_record_id.clone(),
        conversation: row.conversation.clone(),
        snapshot: snapshot.clone(),
        envelope: snap_env.clone(),
        record_id: snap_rec_id,
        message_id: None,
        stored_at_secs: now,
    };
    put_row(host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await?;

    hooks::release_each(host, reservations, &row.quote_record_id, reserved_to_release).await;

    if !ensure_snapshot_sent(host, &row.conversation, &mut snap_row, now).await? {
        return Ok(false);
    }

    row.state = Some(AgreementState::NotFormed);
    row.state_reason = Some(reason.to_string());
    row.cancel_reason = None;
    row.seq = Some(1);
    row.commitments = snapshot.commitments;
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;

    let dec_not_formed = DecisionRow {
        kind: "decision".to_string(),
        state: DecisionState::NotFormed,
        agreement: row.quote_record_id.clone(),
        quote_id: row.quote_id.clone(),
        reason: Some(reason.to_string()),
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

async fn sign_author_receipt_half<H: AppHost>(
    host: &H,
    row: &AgreementRow,
    signed_terms: &SignedTerms,
    now: u64,
) -> Result<ReceiptHalf, String> {
    let (principal, _master) = match signing::person_principal(host, now).await {
        Ok(p) => p,
        Err(e) => return Err(e.to_string()),
    };
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(e.to_string()),
    };

    let payload = AgreementReceiptPayload {
        quote_record_id: row.quote_record_id.clone(),
        counterparty_did: row.counterparty_did.clone(),
        quote_author_did: row.quote_author_did.clone(),
        role: DealRole::QuoteAuthor,
        terms: signed_terms.clone(),
        viewed_lang: row.terms.scope.lang.clone(),
        viewed_via_mt: false,
    };
    let payload_str = serde_json::to_string(&payload).map_err(|e| e.to_string())?;

    let draft = syneroym_app_host::types::signing::RecordDraft {
        version: AGREEMENT_RECEIPT_VERSION,
        record_type: RECORD_AGREEMENT_RECEIPT.to_string(),
        subject: row.quote_record_id.clone(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes: None,
    };

    let envelope_json =
        AppSigning::sign_record(host, draft, principal).await.map_err(|e| e.to_string())?;

    let envelope = Envelope::from_json(&envelope_json).map_err(|e| e.to_string())?;
    let record_id = envelope.record_id().map_err(|e| e.to_string())?;

    Ok(ReceiptHalf {
        envelope: envelope_json,
        record_id,
        issuer: owner,
        issued_at_secs: envelope.issued_at_secs,
    })
}

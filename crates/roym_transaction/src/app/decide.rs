//! Writer-side decision execution for deal formation (D-C1-16).

use serde::{Deserialize, Serialize};
use syneroym_app_host::{AppDataLayer, AppHost, AppSigning, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    deal::ResourceKind,
    progress::{
        self as core_progress, AGREEMENT_PROGRESS_VERSION, AgreementProgressPayload,
        AgreementState, RECORD_AGREEMENT_PROGRESS,
    },
    record::{Envelope, RECORD_AGREEMENT_RECEIPT},
    signing,
    transaction::{
        AGREEMENT_RECEIPT_VERSION, AgreementReceiptPayload, DealRole, QuotePayload, ReceiptHalf,
    },
};

use super::{
    AGREEMENT_PROGRESS, AGREEMENTS, AgreementRow, EVENT_PROGRESS, LEDGER,
    hold::{Reservations, ReserveOutcome, SeatLedger},
    progress, put_row, send_card_and_file,
};

pub(crate) fn decision_id(quote_id: &str) -> String {
    format!("decision:{quote_id}")
}

pub(crate) fn countersign_id(agreement: &str) -> String {
    format!("countersign:{agreement}")
}

pub(crate) fn snapshot_id(agreement: &str, seq: u32) -> String {
    format!("snapshot:{agreement}:{seq}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DecisionRow {
    pub(crate) kind: String,
    pub(crate) state: String,
    pub(crate) agreement: String,
    pub(crate) quote_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(alias = "at_secs")]
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CountersignRow {
    pub(crate) kind: String,
    pub(crate) agreement: String,
    pub(crate) half: ReceiptHalf,
    #[serde(alias = "at_secs")]
    pub(crate) created_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SnapshotRow {
    pub(crate) seq: u32,
    pub(crate) agreement: String,
    pub(crate) conversation: String,
    pub(crate) snapshot: AgreementProgressPayload,
    pub(crate) envelope: String,
    pub(crate) record_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message_id: Option<String>,
    pub(crate) stored_at_secs: u64,
}

pub(crate) async fn run<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    quote: &QuotePayload,
    now: u64,
) -> Result<bool, String> {
    let dec_key = decision_id(&quote.quote_id);

    // 1. Look up decision:<quote_id> first.
    if let Ok(Some(existing_dec)) = super::get_row::<DecisionRow, _>(host, LEDGER, &dec_key).await {
        if existing_dec.agreement != row.quote_record_id {
            // Other agreement: leave half filed with no countersign and no snapshot
            return Ok(false);
        }
        return repeat_path(host, row, quote, now).await;
    }

    // 2. Step-1 validation: check signer and expiry before creating fence.
    if !precheck_acceptance(row, quote, now) {
        return Ok(false);
    }

    if !claim_decision_fence(host, row, quote, &dec_key, now).await? {
        if let Ok(Some(existing_dec)) =
            super::get_row::<DecisionRow, _>(host, LEDGER, &dec_key).await
            && existing_dec.agreement == row.quote_record_id
        {
            return repeat_path(host, row, quote, now).await;
        }
        return Ok(false);
    }

    // 3. Reserve holds through Reservations
    let reserved = match reserve_author_holds(host, row, now).await? {
        Ok(res) => res,
        Err(reason) => return handle_not_formed(host, row, quote, reason, now).await,
    };

    // 4. Confirm holds
    let reservations = SeatLedger;
    for cid in &reserved {
        reservations.confirm(host, &row.quote_record_id, cid).await?;
    }

    // 5-7. Store countersign, snapshots, send cards, and complete agreed state
    finalize_agreed_deal(host, row, quote, &dec_key, now).await
}

fn precheck_acceptance(row: &AgreementRow, quote: &QuotePayload, now: u64) -> bool {
    let cp_half = match row.counterparty.as_ref() {
        Some(h) => h,
        None => return false,
    };
    if cp_half.issuer != row.counterparty_did {
        return false;
    }
    if now >= quote.terms.quote_expires_at_secs {
        return false;
    }
    true
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
        state: "deciding".to_string(),
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

async fn reserve_author_holds<H: AppHost>(
    host: &H,
    row: &AgreementRow,
    now: u64,
) -> Result<Result<Vec<String>, &'static str>, String> {
    let reservations = SeatLedger;
    let mut reserved = Vec::new();
    for c in &row.terms.commitments {
        if c.giver == DealRole::QuoteAuthor && c.resource_kind == ResourceKind::ServiceInSlot {
            let slot_id = c
                .details
                .as_ref()
                .and_then(|d| d.data.get("slot_id"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "missing slot_id".to_string())?;

            match reservations
                .reserve(host, &row.quote_record_id, &c.commitment_id, slot_id, now)
                .await?
            {
                ReserveOutcome::Held { .. } | ReserveOutcome::AlreadyHeld { .. } => {
                    reserved.push(c.commitment_id.clone());
                }
                ReserveOutcome::SlotUnavailable | ReserveOutcome::SlotTaken => {
                    for cid in &reserved {
                        let _ = reservations.release(host, &row.quote_record_id, cid).await;
                    }
                    return Ok(Err("slot-taken"));
                }
            }
        }
    }
    Ok(Ok(reserved))
}

async fn finalize_agreed_deal<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    quote: &QuotePayload,
    dec_key: &str,
    now: u64,
) -> Result<bool, String> {
    let half = sign_author_receipt_half(host, row, now).await?;
    let cs_row = CountersignRow {
        kind: "countersign".to_string(),
        agreement: row.quote_record_id.clone(),
        half: half.clone(),
        created_at_secs: now,
    };
    put_row(host, LEDGER, &countersign_id(&row.quote_record_id), &cs_row).await?;

    let snapshot = core_progress::open(
        row.quote_record_id.clone(),
        row.conversation.clone(),
        row.counterparty_did.clone(),
        row.quote_author_did.clone(),
        &row.terms,
        None,
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

    let (_cs_msg, _cs_state, cs_err) = send_card_and_file(
        host,
        &row.conversation,
        RECORD_AGREEMENT_RECEIPT,
        AGREEMENT_RECEIPT_VERSION,
        &half.envelope,
        now,
        None,
    )
    .await;
    if let Some(err) = cs_err
        && is_transport_error(&err)
    {
        return Err(err);
    }

    let (snap_msg, _, snap_err) = send_card_and_file(
        host,
        &row.conversation,
        RECORD_AGREEMENT_PROGRESS,
        AGREEMENT_PROGRESS_VERSION,
        &snap_env,
        now,
        None,
    )
    .await;
    if let Some(err) = snap_err
        && is_transport_error(&err)
    {
        return Err(err);
    }

    if !snap_msg.is_empty() {
        snap_row.message_id = Some(snap_msg);
        let _ = put_row(host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await;
    }

    let dec_agreed = DecisionRow {
        kind: "decision".to_string(),
        state: "agreed".to_string(),
        agreement: row.quote_record_id.clone(),
        quote_id: quote.quote_id.clone(),
        reason: None,
        created_at_secs: now,
    };
    put_row(host, LEDGER, dec_key, &dec_agreed).await?;

    row.state = Some(AgreementState::Agreed);
    row.state_reason = None;
    row.cancel_reason = None;
    row.seq = Some(1);
    row.commitments = snapshot.commitments;
    row.agreed_at_secs = Some(now);
    row.quote_author = Some(half);
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;

    let _ = host
        .record_event(
            EVENT_PROGRESS.to_string(),
            row.conversation.clone(),
            Some(row.quote_record_id.clone()),
        )
        .await;

    Ok(true)
}

async fn repeat_path<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    quote: &QuotePayload,
    now: u64,
) -> Result<bool, String> {
    let snap_key = snapshot_id(&row.quote_record_id, 1);
    let stored_snap: Option<SnapshotRow> =
        super::get_row(host, AGREEMENT_PROGRESS, &snap_key).await?;

    let cs_key = countersign_id(&row.quote_record_id);
    let stored_cs: Option<CountersignRow> = super::get_row(host, LEDGER, &cs_key).await?;

    if let Some(snap_row) = stored_snap {
        if let Some(cs) = stored_cs {
            if snap_row.message_id.is_none() {
                let (snap_msg, _, err) = send_card_and_file(
                    host,
                    &row.conversation,
                    RECORD_AGREEMENT_PROGRESS,
                    AGREEMENT_PROGRESS_VERSION,
                    &snap_row.envelope,
                    now,
                    None,
                )
                .await;
                if let Some(e) = err
                    && is_transport_error(&e)
                {
                    return Err(e);
                }
                if !snap_msg.is_empty() {
                    let mut s = snap_row.clone();
                    s.message_id = Some(snap_msg);
                    let _ = put_row(host, AGREEMENT_PROGRESS, &snap_key, &s).await;
                }
            }
            row.state = Some(snap_row.snapshot.state);
            row.state_reason = snap_row.snapshot.state_reason;
            row.cancel_reason = snap_row.snapshot.cancel_reason;
            row.seq = Some(snap_row.seq);
            row.commitments = snap_row.snapshot.commitments;
            row.quote_author = Some(cs.half);
            row.agreed_at_secs = Some(snap_row.stored_at_secs);
            row.updated_at_secs = now;
            put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;
            return Ok(true);
        }
    } else if stored_cs.is_none() && now >= quote.terms.quote_expires_at_secs {
        return Ok(false);
    }
    Ok(false)
}

async fn handle_not_formed<H: AppHost>(
    host: &H,
    row: &mut AgreementRow,
    quote: &QuotePayload,
    reason: &str,
    now: u64,
) -> Result<bool, String> {
    let snapshot = core_progress::open(
        row.quote_record_id.clone(),
        row.conversation.clone(),
        row.counterparty_did.clone(),
        row.quote_author_did.clone(),
        &row.terms,
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

    let (snap_msg, _, snap_err) = send_card_and_file(
        host,
        &row.conversation,
        RECORD_AGREEMENT_PROGRESS,
        AGREEMENT_PROGRESS_VERSION,
        &snap_env,
        now,
        None,
    )
    .await;
    if let Some(err) = snap_err
        && is_transport_error(&err)
    {
        return Err(err);
    }
    if !snap_msg.is_empty() {
        snap_row.message_id = Some(snap_msg);
        let _ = put_row(host, AGREEMENT_PROGRESS, &snap_key, &snap_row).await;
    }

    let dec_not_formed = DecisionRow {
        kind: "decision".to_string(),
        state: "not-formed".to_string(),
        agreement: row.quote_record_id.clone(),
        quote_id: quote.quote_id.clone(),
        reason: Some(reason.to_string()),
        created_at_secs: now,
    };
    put_row(host, LEDGER, &decision_id(&quote.quote_id), &dec_not_formed).await?;

    row.state = Some(AgreementState::NotFormed);
    row.state_reason = Some(reason.to_string());
    row.cancel_reason = None;
    row.seq = Some(1);
    row.commitments = snapshot.commitments;
    row.updated_at_secs = now;
    put_row(host, AGREEMENTS, &row.quote_record_id, row).await?;

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
        terms: row.terms.clone(),
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

fn is_transport_error(msg: &str) -> bool {
    msg.contains("transport") || msg.contains("timeout") || msg.contains("connection")
}

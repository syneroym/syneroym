//! The counterparty's confirm of its own reservations.
//!
//! When the writer's countersigned half is filed, an agreement exists. A
//! counterparty that gives a reserved resource then confirms its own
//! reservation. If the extension refuses, the giver cannot keep the resource,
//! so the node signs one `commitment-failure` record and sends it to the
//! writer.

use serde_json::json;
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{data_layer::RecordWriteValue, signing::RecordDraft},
};
use syneroym_roym_core::{
    commitment_failure::{COMMITMENT_FAILURE_VERSION, CommitmentFailurePayload, FailureReason},
    deal::{Commitment, transition::CommitmentState},
    method::scheduling::code,
    paging,
    record::{Envelope, RECORD_COMMITMENT_FAILURE},
    transaction::DealRole,
};

use super::super::{
    AGREEMENTS, AgreementRow, LEDGER, OutgoingCard, get_row,
    hooks::{self, ConfirmOutcome, Reservations, ReserveOutcome},
    ledger::{FailureRow, LedgerKind, LedgerRow, confirmed_id, failure_id},
    put_row, resolve_principal_and_owner, send_card_and_file,
};

/// The commitments of `row` that this node gives, that must still be
/// confirmed: the writer's half is held, the agreement has not ended, and the
/// latest snapshot (if any) still shows the commitment `promised`. A
/// commitment that ended `failed` or `cancelled` was released, so a confirm of
/// it could only be refused.
fn awaiting_confirm(row: &AgreementRow) -> impl Iterator<Item = &Commitment> {
    let formed = row.quote_author.is_some() && !row.state.is_some_and(|s| s.is_terminal());
    hooks::reserved_by(&row.terms, DealRole::Counterparty).filter(move |c| {
        formed
            && row
                .commitments
                .iter()
                .find(|p| p.commitment_id == c.commitment_id)
                .is_none_or(|p| p.state == CommitmentState::Promised)
    })
}

/// Confirms every reservation this node owns in `conversation` that still
/// waits for it. Answers how many failure records could not be signed or sent
/// in this pass: the next sync tries them again. A storage error stops only
/// the commitment it happened on, and the next sync tries that one again too,
/// so the pass never fails the sync that runs it.
pub(crate) async fn confirm_own_reservations<H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    conversation: &str,
    owner: &str,
    now: u64,
) -> u32 {
    let filter = json!({ "conversation": conversation }).to_string();
    let Ok(rows) = paging::query_all::<_, AgreementRow>(host, AGREEMENTS, Some(filter)).await
    else {
        return 0;
    };
    let mut unsent = 0;
    for row in rows.iter().filter(|r| r.counterparty_did == owner) {
        for c in awaiting_confirm(row) {
            unsent += confirm_one(host, hooks, row, c, now).await.unwrap_or(0);
        }
    }
    unsent
}

/// One commitment. Answers 1 when its failure record is not sent yet.
async fn confirm_one<H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    row: &AgreementRow,
    c: &Commitment,
    now: u64,
) -> Result<u32, String> {
    let agreement = &row.quote_record_id;
    let confirmed = confirmed_id(agreement, &c.commitment_id);
    if get_row::<LedgerRow, _>(host, LEDGER, &confirmed).await?.is_some() {
        return Ok(0);
    }
    if let Some(stored) =
        get_row::<FailureRow, _>(host, LEDGER, &failure_id(agreement, &c.commitment_id)).await?
    {
        return send_failure(host, stored, now).await;
    }
    match confirm_or_retake(host, hooks, agreement, c).await {
        Ok(ConfirmOutcome::Confirmed) => {
            let marker = LedgerRow {
                kind: LedgerKind::Confirmed,
                agreement: agreement.clone(),
                state: None,
                quote_id: None,
                commitment_id: Some(c.commitment_id.clone()),
                reason: None,
                half: None,
                created_at_secs: now,
            };
            put_row(host, LEDGER, &confirmed, &marker).await?;
            Ok(0)
        }
        // Every other refusal means the giver cannot keep the resource, whatever
        // code the extension gave.
        Ok(ConfirmOutcome::Refused { .. }) => make_failure(host, row, c, now).await,
        // A fault is not an answer: the next sync asks again.
        Err(_) => Ok(0),
    }
}

/// Confirms the reservation. An extension that has no reservation for the key
/// answers `reservation-unknown`. That is the state of a restore that brought
/// the agreement back before the extension's own data, so the node takes the
/// reservation again, as the import does, and confirms that. Only a refusal of
/// the new reserve or confirm is a lost reservation.
async fn confirm_or_retake<H: AppHost, R: Reservations>(
    host: &H,
    hooks: &R,
    agreement: &str,
    c: &Commitment,
) -> Result<ConfirmOutcome, String> {
    match hooks.confirm(host, agreement, c).await? {
        ConfirmOutcome::Refused { reason } if reason == code::RESERVATION_UNKNOWN => {
            match hooks.reserve(host, agreement, c).await? {
                ReserveOutcome::Held => hooks.confirm(host, agreement, c).await,
                ReserveOutcome::Refused { reason } => Ok(ConfirmOutcome::Refused { reason }),
            }
        }
        other => Ok(other),
    }
}

/// Signs the record, stores it, then sends it. When it cannot be signed, no
/// row is written, so an unsigned record can never be sent.
async fn make_failure<H: AppHost>(
    host: &H,
    row: &AgreementRow,
    c: &Commitment,
    now: u64,
) -> Result<u32, String> {
    let payload = CommitmentFailurePayload {
        agreement: row.quote_record_id.clone(),
        conversation: row.conversation.clone(),
        commitment_id: c.commitment_id.clone(),
        reason: FailureReason::ReservationLost,
    };
    let Ok((envelope, record_id)) = sign_failure(host, &payload, now).await else {
        return Ok(1);
    };
    let stored = FailureRow {
        kind: LedgerKind::Failure,
        agreement: payload.agreement,
        commitment_id: payload.commitment_id,
        conversation: payload.conversation,
        record_id,
        envelope,
        created_at_secs: now,
        message_id: None,
    };
    let stored = claim_failure_row(host, stored).await?;
    send_failure(host, stored, now).await
}

async fn sign_failure<H: AppHost>(
    host: &H,
    payload: &CommitmentFailurePayload,
    now: u64,
) -> Result<(String, String), String> {
    payload.validate().map_err(|e| e.to_string())?;
    let (principal, _) = resolve_principal_and_owner(host, now)
        .await
        .map_err(|e| e.error.map_or_else(|| "signing failed".to_string(), |error| error.message))?;
    let draft = RecordDraft {
        version: COMMITMENT_FAILURE_VERSION,
        record_type: RECORD_COMMITMENT_FAILURE.to_string(),
        subject: payload.agreement.clone(),
        payload: serde_json::to_string(payload).map_err(|e| e.to_string())?,
        expires_at_secs: None,
        supersedes: None,
    };
    let envelope =
        AppSigning::sign_record(host, draft, principal).await.map_err(|e| e.to_string())?;
    let record_id =
        Envelope::from_json(&envelope).and_then(|e| e.record_id()).map_err(|e| e.to_string())?;
    Ok((envelope, record_id))
}

/// Writes the row. If another sync wrote one first, that row is used and
/// this one is dropped, so one record is made for one commitment.
async fn claim_failure_row<H: AppHost>(host: &H, row: FailureRow) -> Result<FailureRow, String> {
    let id = failure_id(&row.agreement, &row.commitment_id);
    let payload = serde_json::to_vec(&row).map_err(|e| e.to_string())?;
    let write = RecordWriteValue { id: id.clone(), payload };
    match AppDataLayer::create(host, LEDGER.to_string(), vec![write]).await {
        Ok(None) => Ok(row),
        Ok(Some(_)) => {
            get_row(host, LEDGER, &id).await?.ok_or_else(|| format!("failure row '{id}' vanished"))
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Sends the stored record as a card, unless it was sent already. Answers 1
/// when the send did not work: the row stays, and the next sync sends the
/// same record again.
async fn send_failure<H: AppHost>(host: &H, mut row: FailureRow, now: u64) -> Result<u32, String> {
    if row.message_id.is_some() {
        return Ok(0);
    }
    let card = OutgoingCard {
        card_type: RECORD_COMMITMENT_FAILURE,
        version: COMMITMENT_FAILURE_VERSION,
        envelope: &row.envelope,
        send: &[],
        view: &[],
    };
    let (message_id, _, _) = send_card_and_file(host, &row.conversation, &card, now, None).await;
    if message_id.is_empty() {
        return Ok(1);
    }
    row.message_id = Some(message_id);
    put_row(host, LEDGER, &failure_id(&row.agreement, &row.commitment_id), &row).await?;
    Ok(0)
}

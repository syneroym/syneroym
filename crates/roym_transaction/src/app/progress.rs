//! Signing and sending booking progress snapshots as service-signed records.

use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    booking::{BOOKING_PROGRESS_VERSION, BookingProgressPayload},
    clock,
    record::{Envelope, RECORD_BOOKING_PROGRESS},
};

use super::{
    BookingRow, LEDGER, get_row,
    ledger::{LedgerRow, step_id},
    put_row, send_card_and_file,
};

pub(crate) async fn sign<H: AppHost>(
    host: &H,
    s: &BookingProgressPayload,
) -> Result<(String, String), String> {
    let payload = serde_json::to_string(s).map_err(|e| e.to_string())?;
    let draft = RecordDraft {
        version: BOOKING_PROGRESS_VERSION,
        record_type: RECORD_BOOKING_PROGRESS.to_string(),
        subject: s.agreement.clone(),
        payload,
        expires_at_secs: None,
        supersedes: None,
    };
    let envelope_json = AppSigning::sign_record(host, draft, Principal::Service)
        .await
        .map_err(|e| e.to_string())?;
    let env = Envelope::from_json(&envelope_json).map_err(|e| e.to_string())?;
    let record_id = env.record_id().map_err(|e| e.to_string())?;
    Ok((envelope_json, record_id))
}

pub(crate) async fn send<H: AppHost>(host: &H, row: &BookingRow) {
    let step_key = step_id(&row.agreement, row.snapshot.seq);
    let step_row_res: Result<Option<LedgerRow>, String> = get_row(host, LEDGER, &step_key).await;
    let mut lr = match step_row_res {
        Ok(Some(lr)) => lr,
        _ => return,
    };
    let envelope = match &lr.step {
        Some(s) if s.message_id.is_none() => s.envelope.clone(),
        _ => return,
    };

    let now = clock::now_secs();
    let (message_id, _, _) =
        send_card_and_file(host, &row.conversation, "booking-progress", 1, &envelope, now, None)
            .await;

    if !message_id.is_empty() {
        if let Some(ref mut step) = lr.step {
            step.message_id = Some(message_id);
        }
        let _ = put_row(host, LEDGER, &step_key, &lr).await;
    }
}

pub(crate) async fn resend_if_unsent<H: AppHost>(host: &H, row: &BookingRow) {
    let step_key = step_id(&row.agreement, row.snapshot.seq);
    if let Ok(Some(lr)) = get_row::<LedgerRow, _>(host, LEDGER, &step_key).await
        && let Some(step) = lr.step
        && step.message_id.is_none()
    {
        send(host, row).await;
    }
}

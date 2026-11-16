//! Signing and sending agreement progress snapshots as service-signed records.

use syneroym_app_host::{
    AppHost, AppSigning,
    types::signing::{Principal, RecordDraft},
};
use syneroym_roym_core::{
    clock,
    progress::{AGREEMENT_PROGRESS_VERSION, AgreementProgressPayload, RECORD_AGREEMENT_PROGRESS},
    record::Envelope,
};

use super::{AGREEMENT_PROGRESS, decide::SnapshotRow, get_row, put_row, send_card_and_file};

pub(crate) async fn sign<H: AppHost>(
    host: &H,
    s: &AgreementProgressPayload,
) -> Result<(String, String), String> {
    let payload = serde_json::to_string(s).map_err(|e| e.to_string())?;
    let draft = RecordDraft {
        version: AGREEMENT_PROGRESS_VERSION,
        record_type: RECORD_AGREEMENT_PROGRESS.to_string(),
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

pub(crate) async fn resend_if_unsent<H: AppHost>(
    host: &H,
    agreement: &str,
    seq: u32,
    conversation: &str,
) {
    let snap_key = format!("snapshot:{agreement}:{seq}");
    if let Ok(Some(mut sr)) = get_row::<SnapshotRow, _>(host, AGREEMENT_PROGRESS, &snap_key).await
        && sr.message_id.is_none()
    {
        let now = clock::now_secs();
        let (message_id, _, _) = send_card_and_file(
            host,
            conversation,
            "agreement-progress",
            1,
            &sr.envelope,
            now,
            None,
        )
        .await;
        if !message_id.is_empty() {
            sr.message_id = Some(message_id);
            let _ = put_row(host, AGREEMENT_PROGRESS, &snap_key, &sr).await;
        }
    }
}

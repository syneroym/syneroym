//! Operations for declining a deal (quote or enquiry).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use syneroym_app_host::{AppDataLayer, AppHost, AppSigning, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    clock,
    deal::LocalText,
    envelope::Response,
    lang::LangTag,
    method::Reply,
    paging,
    progress::AgreementState,
    record::{Envelope, RECORD_DEAL_DECLINE},
    transaction::{self, DEAL_DECLINE_VERSION, DealDeclinePayload},
};

use super::{
    AGREEMENTS, AgreementRow, ENQUIRIES, ENQUIRY_HISTORY, LEDGER, QUOTE_HISTORY, QUOTES,
    RecordPointerRow, ensure_collections, get_bytes, get_row, is_deal_level_not_formed_reason,
    put_row, resolve_principal_and_owner, send_card_and_file,
};

pub fn decline_id(deal: &str) -> String {
    format!("decline:{deal}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeclineRow {
    pub(crate) kind: String,
    pub(crate) deal: String,
    pub(crate) declines: String,
    pub(crate) record_id: String,
    pub(crate) envelope: String,
    #[serde(alias = "at_secs")]
    pub(crate) created_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DealDeclineInput {
    pub declines: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DealDeclineOutput {
    pub declines: String,
    pub deal: String,
    pub record_id: String,
    pub declined: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

pub(crate) async fn decline<H: AppHost>(
    host: &H,
    input: DealDeclineInput,
) -> Reply<DealDeclineOutput> {
    let now = clock::now_secs();
    decline_at(host, input, now).await
}

async fn resolve_declined_target<H: AppHost>(
    host: &H,
    declines: &str,
    owner: &str,
    now: u64,
) -> Result<(String, String), Response> {
    if let Some(q_bytes) =
        get_bytes(host, QUOTE_HISTORY, declines).await.map_err(Response::internal_error)?
    {
        let q_env = String::from_utf8(q_bytes)
            .map_err(|e| Response::internal_error(format!("invalid quote envelope utf8: {e}")))?;
        let verdict = transaction::verify_quote(&q_env, now);
        if !verdict.verified {
            return Err(Response::invalid_params(format!(
                "the quote does not verify: {}",
                verdict.reason.as_deref().unwrap_or("unknown")
            )));
        }
        let q_payload = verdict
            .payload
            .ok_or_else(|| Response::internal_error("missing quote payload after verification"))?;

        if owner != q_payload.counterparty_did {
            return Err(Response::invalid_params("only the counterparty can decline this quote"));
        }

        if has_unrefused_counterparty_half(host, &q_payload.quote_id)
            .await
            .map_err(Response::internal_error)?
        {
            return Err(Response::invalid_params(
                "cannot decline a quote that has already been accepted",
            ));
        }

        Ok((q_payload.quote_id, q_payload.conversation))
    } else if let Some(e_bytes) =
        get_bytes(host, ENQUIRY_HISTORY, declines).await.map_err(Response::internal_error)?
    {
        let e_env = String::from_utf8(e_bytes)
            .map_err(|e| Response::internal_error(format!("invalid enquiry envelope utf8: {e}")))?;
        let verdict = transaction::verify_enquiry(&e_env, now);
        if !verdict.verified {
            return Err(Response::invalid_params(format!(
                "the enquiry does not verify: {}",
                verdict.reason.as_deref().unwrap_or("unknown")
            )));
        }
        let e_payload = verdict.payload.ok_or_else(|| {
            Response::internal_error("missing enquiry payload after verification")
        })?;

        if has_unrefused_counterparty_half_for_enquiry(host, &e_payload.enquiry_id)
            .await
            .map_err(Response::internal_error)?
        {
            return Err(Response::invalid_params(
                "cannot decline an enquiry with an accepted quote",
            ));
        }

        Ok((e_payload.enquiry_id, e_payload.conversation))
    } else {
        Err(Response::invalid_params("no such quote or enquiry to decline"))
    }
}

pub(crate) async fn decline_at<H: AppHost>(
    host: &H,
    input: DealDeclineInput,
    now: u64,
) -> Reply<DealDeclineOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;
    let (principal, owner) = resolve_principal_and_owner(host, now).await?;

    let (deal, conversation) = resolve_declined_target(host, &input.declines, &owner, now).await?;

    // Idempotent through fence row decline:<deal>
    let dec_key = decline_id(&deal);
    if let Ok(Some(existing)) = get_row::<DeclineRow, _>(host, LEDGER, &dec_key).await {
        return repeat_decline(host, &dec_key, &conversation, existing, &deal, input.note, now)
            .await;
    }

    let (envelope_json, record_id) = build_and_sign_decline(
        host,
        principal,
        &input.declines,
        &deal,
        &conversation,
        input.note.as_deref(),
        input.lang.as_deref(),
    )
    .await?;

    let mut decline_row = DeclineRow {
        kind: "decline".to_string(),
        deal: deal.clone(),
        declines: input.declines.clone(),
        record_id: record_id.clone(),
        envelope: envelope_json.clone(),
        created_at_secs: now,
        message_id: None,
    };

    let write_val = RecordWriteValue {
        id: dec_key.clone(),
        payload: serde_json::to_vec(&decline_row)
            .map_err(|e| Response::internal_error(e.to_string()))?,
    };

    let outcome = AppDataLayer::create(host, LEDGER.to_string(), vec![write_val])
        .await
        .map_err(|e| Response::internal_error(e.to_string()))?;

    if outcome.is_some() {
        // Race: another caller claimed the fence row
        if let Ok(Some(existing)) = get_row::<DeclineRow, _>(host, LEDGER, &dec_key).await {
            return repeat_decline(host, &dec_key, &conversation, existing, &deal, input.note, now)
                .await;
        }
    }

    // Send the card and file it
    let (msg_id, _send_state, send_err) = send_card_and_file(
        host,
        &conversation,
        RECORD_DEAL_DECLINE,
        DEAL_DECLINE_VERSION,
        &envelope_json,
        now,
        None,
    )
    .await;

    if let Some(err) = send_err {
        return Err(Response::internal_error(format!("failed to send decline card: {err:?}")));
    }

    if !msg_id.is_empty() {
        decline_row.message_id = Some(msg_id);
        put_row(host, LEDGER, &dec_key, &decline_row).await.map_err(Response::internal_error)?;
    }

    // Mark deal or enquiry declined in local state
    mark_target_declined(host, &deal, &input.note, now).await?;

    Ok(DealDeclineOutput {
        declines: input.declines,
        deal,
        record_id,
        declined: true,
        note: input.note,
    })
}

async fn repeat_decline<H: AppHost>(
    host: &H,
    dec_key: &str,
    conversation: &str,
    mut existing: DeclineRow,
    deal: &str,
    note: Option<String>,
    now: u64,
) -> Reply<DealDeclineOutput> {
    if existing.message_id.is_none() {
        let (msg_id, _send_state, send_err) = send_card_and_file(
            host,
            conversation,
            RECORD_DEAL_DECLINE,
            DEAL_DECLINE_VERSION,
            &existing.envelope,
            now,
            None,
        )
        .await;
        if let Some(err) = send_err {
            return Err(Response::internal_error(format!("failed to send decline card: {err:?}")));
        }
        if !msg_id.is_empty() {
            existing.message_id = Some(msg_id);
            put_row(host, LEDGER, dec_key, &existing).await.map_err(Response::internal_error)?;
        }
    }
    mark_target_declined(host, deal, &note, now).await?;
    Ok(DealDeclineOutput {
        declines: existing.declines,
        deal: existing.deal,
        record_id: existing.record_id,
        declined: true,
        note,
    })
}

async fn build_and_sign_decline<H: AppHost>(
    host: &H,
    principal: syneroym_app_host::types::signing::Principal,
    declines: &str,
    deal: &str,
    conversation: &str,
    note: Option<&str>,
    lang: Option<&str>,
) -> Reply<(String, String)> {
    let reason = match note {
        Some(n) => {
            let lang_tag = lang.unwrap_or("en");
            let lang = LangTag::parse_canonical(lang_tag)
                .map_err(|e| Response::invalid_params(format!("invalid lang tag: {e}")))?;
            Some(LocalText::plain(lang, n.to_string()))
        }
        None => None,
    };

    let payload = DealDeclinePayload {
        declines: declines.to_string(),
        deal: deal.to_string(),
        conversation: conversation.to_string(),
        reason,
    };
    let payload_str =
        serde_json::to_string(&payload).map_err(|e| Response::internal_error(e.to_string()))?;

    let draft = syneroym_app_host::types::signing::RecordDraft {
        version: DEAL_DECLINE_VERSION,
        record_type: RECORD_DEAL_DECLINE.to_string(),
        subject: declines.to_string(),
        payload: payload_str,
        expires_at_secs: None,
        supersedes: None,
    };

    let envelope_json = AppSigning::sign_record(host, draft, principal)
        .await
        .map_err(|e| Response::internal_error(e.to_string()))?;
    let envelope =
        Envelope::from_json(&envelope_json).map_err(|e| Response::internal_error(e.to_string()))?;
    let record_id = envelope.record_id().map_err(|e| Response::internal_error(e.to_string()))?;
    Ok((envelope_json, record_id))
}

async fn mark_target_declined<H: AppHost>(
    host: &H,
    deal: &str,
    note: &Option<String>,
    now: u64,
) -> Result<(), Response> {
    if let Some(mut ptr) = get_row::<RecordPointerRow, _>(host, QUOTES, deal)
        .await
        .map_err(Response::internal_error)?
    {
        ptr.declined_at_secs = Some(now);
        ptr.decline_note = note.clone();
        ptr.updated_at_secs = now;
        put_row(host, QUOTES, deal, &ptr).await.map_err(Response::internal_error)?;
    } else if let Some(mut ptr) = get_row::<RecordPointerRow, _>(host, ENQUIRIES, deal)
        .await
        .map_err(Response::internal_error)?
    {
        ptr.declined_at_secs = Some(now);
        ptr.decline_note = note.clone();
        ptr.updated_at_secs = now;
        put_row(host, ENQUIRIES, deal, &ptr).await.map_err(Response::internal_error)?;
    }
    Ok(())
}

pub(crate) async fn has_unrefused_counterparty_half<H: AppHost>(
    host: &H,
    quote_id: &str,
) -> Result<bool, String> {
    let filter = serde_json::json!({ "quote_id": quote_id }).to_string();
    let agr_rows: Vec<AgreementRow> = paging::query_all(host, AGREEMENTS, Some(filter)).await?;
    for agr in agr_rows {
        if let Some(cp_half) = &agr.counterparty {
            let ref_key = format!("refused:{}", cp_half.record_id);
            if get_row::<serde_json::Value, _>(host, LEDGER, &ref_key).await?.is_none() {
                let is_refused_by_snapshot = agr.state == Some(AgreementState::NotFormed)
                    && !is_deal_level_not_formed_reason(agr.state_reason.as_deref());
                if !is_refused_by_snapshot {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

pub(crate) async fn has_unrefused_counterparty_half_for_enquiry<H: AppHost>(
    host: &H,
    enquiry_id: &str,
) -> Result<bool, String> {
    let mut record_ids = HashSet::new();
    if let Some(ptr) = get_row::<RecordPointerRow, _>(host, ENQUIRIES, enquiry_id).await? {
        record_ids.insert(ptr.record_id);
    }
    let history = paging::envelope_history(host, ENQUIRY_HISTORY, "enquiry_id", enquiry_id).await?;
    for env_json in history {
        if let Ok(env) = Envelope::from_json(&env_json)
            && let Ok(rid) = env.record_id()
        {
            record_ids.insert(rid);
        }
    }
    for rec_id in record_ids {
        let filter = serde_json::json!({ "request_record_id": rec_id }).to_string();
        let quotes: Vec<RecordPointerRow> = paging::query_all(host, QUOTES, Some(filter)).await?;
        for q in quotes {
            if has_unrefused_counterparty_half(host, &q.id).await? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

//! Filing and verification of deal-decline cards.

use serde_json::Value;
use syneroym_app_host::AppHost;
use syneroym_roym_core::transaction::{self, DealDeclinePayload};

use super::{FileCardResult, refuse_card};
use crate::app::{
    CardRow, ENQUIRIES, ENQUIRY_HISTORY, LEDGER, QUOTE_HISTORY, QUOTES, RecordPointerRow,
    decline_ops, get_bytes, get_row, put_card, put_row,
};

async fn verify_and_apply_declined_quote<H: AppHost>(
    host: &H,
    payload: &DealDeclinePayload,
    q_bytes: &[u8],
    signer: &str,
    conversation: &str,
    now: u64,
) -> Result<(), &'static str> {
    let q_str = match String::from_utf8(q_bytes.to_vec()) {
        Ok(s) => s,
        Err(_) => return Err("stored quote envelope is invalid utf8"),
    };
    let qv = transaction::verify_quote(&q_str, now);
    if !qv.verified {
        return Err("the quote it declines does not verify");
    }
    let qp = match qv.payload.as_ref() {
        Some(qp) => qp,
        None => return Err("quote missing payload"),
    };
    if qp.conversation != conversation {
        return Err("declined quote belongs to another conversation");
    }
    if payload.deal != qp.quote_id {
        return Err("deal does not match quote_id");
    }
    if signer != qp.counterparty_did {
        return Err("only the counterparty can decline this quote");
    }
    if decline_ops::has_unrefused_counterparty_half(host, &qp.quote_id)
        .await
        .map_err(|_| "unrefused half check failed")?
    {
        return Err("cannot decline a quote with an unrefused acceptance");
    }

    if let Ok(Some(mut ptr)) = get_row::<RecordPointerRow, _>(host, QUOTES, &qp.quote_id).await {
        ptr.declined_at_secs = Some(now);
        ptr.decline_note = payload.reason.as_ref().map(|r| r.text.clone());
        ptr.updated_at_secs = now;
        put_row(host, QUOTES, &qp.quote_id, &ptr)
            .await
            .map_err(|_| "failed to write quote pointer")?;
    }
    Ok(())
}

async fn verify_and_apply_declined_enquiry<H: AppHost>(
    host: &H,
    payload: &DealDeclinePayload,
    e_bytes: &[u8],
    signer: &str,
    conversation: &str,
    now: u64,
) -> Result<(), &'static str> {
    let e_str = match String::from_utf8(e_bytes.to_vec()) {
        Ok(s) => s,
        Err(_) => return Err("stored enquiry envelope is invalid utf8"),
    };
    let ev = transaction::verify_enquiry(&e_str, now);
    if !ev.verified {
        return Err("the enquiry it declines does not verify");
    }
    let ep = match ev.payload.as_ref() {
        Some(ep) => ep,
        None => return Err("enquiry missing payload"),
    };
    if ep.conversation != conversation {
        return Err("declined enquiry belongs to another conversation");
    }
    if payload.deal != ep.enquiry_id {
        return Err("deal does not match enquiry_id");
    }
    let mut ptr = match get_row::<RecordPointerRow, _>(host, ENQUIRIES, &ep.enquiry_id).await {
        Ok(Some(p)) => p,
        _ => return Err("enquiry pointer not found"),
    };
    let enq_issuer = ev.issuer.as_deref().unwrap_or(&ptr.issuer);
    if signer != enq_issuer {
        if let Some(cp) = ptr.counterparty_did.as_deref() {
            if signer != cp {
                return Err("only the counterparty or quote author can decline this enquiry");
            }
        } else if ptr.mine {
            ptr.counterparty_did = Some(signer.to_string());
        } else {
            return Err("only the counterparty or quote author can decline this enquiry");
        }
    }
    if decline_ops::has_unrefused_counterparty_half_for_enquiry(host, &ep.enquiry_id)
        .await
        .map_err(|_| "unrefused half check failed")?
    {
        return Err("cannot decline an enquiry with an accepted quote");
    }

    ptr.declined_at_secs = Some(now);
    ptr.decline_note = payload.reason.as_ref().map(|r| r.text.clone());
    ptr.updated_at_secs = now;
    put_row(host, ENQUIRIES, &ep.enquiry_id, &ptr)
        .await
        .map_err(|_| "failed to write enquiry pointer")?;
    Ok(())
}

pub(crate) async fn file_deal_decline_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    _sender: &str,
    envelope: &str,
    conversation: &str,
    now: u64,
    _owner: &str,
) -> Result<FileCardResult, String> {
    let v = transaction::verify_deal_decline(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "deal decline does not verify".to_string());
        return refuse_card(host, row, reason).await;
    }
    let payload = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, row, "missing payload").await,
    };
    if payload.conversation != conversation {
        return refuse_card(host, row, "card names another conversation").await;
    }

    let signer = v.issuer.as_deref().unwrap_or_default();

    if let Some(q_bytes) = get_bytes(host, QUOTE_HISTORY, &payload.declines).await? {
        if let Err(reason) =
            verify_and_apply_declined_quote(host, payload, &q_bytes, signer, conversation, now)
                .await
        {
            return refuse_card(host, row, reason).await;
        }
    } else if let Some(e_bytes) = get_bytes(host, ENQUIRY_HISTORY, &payload.declines).await? {
        if let Err(reason) =
            verify_and_apply_declined_enquiry(host, payload, &e_bytes, signer, conversation, now)
                .await
        {
            return refuse_card(host, row, reason).await;
        }
    } else {
        return refuse_card(host, row, "declined record this node does not hold").await;
    }

    let dec_key = decline_ops::decline_id(&payload.deal);
    if let (Some(rid), Ok(Some(existing_dec))) = (
        v.record_id.as_deref(),
        get_row::<decline_ops::DeclineRow, _>(host, LEDGER, &dec_key).await,
    ) && rid == existing_dec.record_id
    {
        row.repeat = true;
    }
    let dec_row = decline_ops::DeclineRow {
        kind: "decline".to_string(),
        deal: payload.deal.clone(),
        declines: payload.declines.clone(),
        record_id: v.record_id.clone().unwrap_or_default(),
        envelope: envelope.to_string(),
        created_at_secs: now,
        message_id: Some(row.message_id.clone()),
    };
    put_row(host, LEDGER, &dec_key, &dec_row).await?;

    row.verified = true;
    row.data = Some(serde_json::to_value(payload).unwrap_or(Value::Null));
    row.issuer = v.issuer;
    row.record_id = v.record_id;
    row.revocation_status = v.revocation_status;
    put_card(host, &row).await?;

    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

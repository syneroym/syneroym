//! Transaction synchronization across conversation history.

use serde::Deserialize;
use serde_json::{Value, json};
use syneroym_app_host::{AppDataLayer, AppHost};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    clock,
    conversation::Direction,
    envelope::{Request, Response},
    signing,
    transaction::{self, MAX_CARDS_PER_CONVERSATION, ReceiptHalf, SYNC_OVERLAP, SYNC_WINDOW},
};

use super::{
    AGREEMENTS, AgreementRow, CARDS, CardRow, QUOTE_HISTORY, REQUEST_HISTORY, SYNC_STATE,
    SyncStateRow, agreement_ops::maybe_countersign, conversation_call,
    count_cards_for_conversation, ensure_collections, get_bytes, get_row, put_row,
    quote_ops::store_received_quote, request_ops::store_received_request,
};

#[derive(Debug, Deserialize)]
struct SyncParams {
    conversation: String,
    #[serde(default)]
    full: bool,
}

struct FileCardResult {
    filed: bool,
    refused: bool,
    unknown: bool,
    countersigned: bool,
}

pub(crate) async fn sync<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let params: SyncParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid params: {e}")),
    };

    if let Err(e) = ensure_collections(host).await {
        return Response::internal_error(e);
    }

    let sync_state: SyncStateRow = match get_row(host, SYNC_STATE, &params.conversation).await {
        Ok(Some(s)) => s,
        Ok(None) => SyncStateRow { scanned_count: 0 },
        Err(e) => return Response::internal_error(e),
    };

    let start = if params.full { 0 } else { sync_state.scanned_count.saturating_sub(SYNC_OVERLAP) };

    let page_resp = match conversation_call(
        host,
        "conversation.history",
        json!({
            "conversation": params.conversation,
            "limit": SYNC_WINDOW,
            "cursor": start,
        }),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    };

    if let Some(err) = page_resp.error {
        return Response::internal_error(err.message);
    }

    let res = page_resp.result.unwrap_or(Value::Null);
    let messages = res.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();

    let mut card_count = match count_cards_for_conversation(host, &params.conversation).await {
        Ok(c) => c,
        Err(e) => return Response::internal_error(e),
    };

    let mut scanned: u32 = 0;
    let mut filed: u32 = 0;
    let mut refused: u32 = 0;
    let mut unknown: u32 = 0;
    let mut countersigned: u32 = 0;

    let mut offset = start;
    let mut first_declined: Option<u64> = None;
    let owner = signing::owner_did(host).await.unwrap_or_default();

    for m in messages {
        offset += 1;
        scanned += 1;
        let content_type = m.get("content_type").and_then(Value::as_str).unwrap_or("");
        if content_type != CARD_CONTENT_TYPE {
            continue;
        }
        if m.get("deleted_at_secs").and_then(Value::as_u64).is_some() {
            continue;
        }
        let msg_id = match m.get("id").and_then(Value::as_str) {
            Some(id) => id,
            None => continue,
        };
        let exists = match AppDataLayer::get(host, CARDS.to_string(), msg_id.to_string()).await {
            Ok(o) => o.is_some(),
            Err(e) => return Response::internal_error(e.to_string()),
        };
        if exists {
            continue;
        }
        if card_count >= MAX_CARDS_PER_CONVERSATION {
            first_declined = first_declined.or(Some(offset - 1));
            continue;
        }

        let file_res = match file_incoming_card(host, &m, &params.conversation, now, &owner).await {
            Ok(res) => res,
            Err(e) => return Response::internal_error(e),
        };
        card_count += 1;
        if file_res.filed {
            filed += 1;
        }
        if file_res.refused {
            refused += 1;
        }
        if file_res.unknown {
            unknown += 1;
        }
        if file_res.countersigned {
            countersigned += 1;
        }
    }

    let new_scanned_count = match first_declined {
        Some(o) => o,
        None => sync_state.scanned_count.max(offset),
    };
    if let Err(e) = put_row(
        host,
        SYNC_STATE,
        &params.conversation,
        &SyncStateRow { scanned_count: new_scanned_count },
    )
    .await
    {
        return Response::internal_error(e);
    }

    Response::ok(json!({
        "scanned": scanned,
        "filed": filed,
        "refused": refused,
        "unknown": unknown,
        "countersigned": countersigned,
        "scanned_count": new_scanned_count,
    }))
}

async fn file_incoming_card<H: AppHost>(
    host: &H,
    m: &Value,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let msg_id = m.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let direction_str = m.get("direction").and_then(Value::as_str).unwrap_or("incoming");
    let direction =
        if direction_str == "outgoing" { Direction::Outgoing } else { Direction::Incoming };
    let sender_timestamp_ms =
        m.get("sender_timestamp_ms").and_then(Value::as_i64).unwrap_or(now as i64 * 1000);

    let mut row = CardRow {
        message_id: msg_id.clone(),
        conversation: conversation.to_string(),
        direction,
        sender_timestamp_ms,
        card_type: String::new(),
        version: 0,
        known: false,
        verified: false,
        expired: false,
        reason: None,
        issuer: None,
        record_id: None,
        revocation_status: None,
        data: None,
        stored_at_secs: now,
        declined: None,
        version_count: None,
    };

    let body = match m.get("body").and_then(Value::as_str) {
        Some(b) => b,
        None => return refuse_card(host, &msg_id, row, "no body").await,
    };

    let card = match card::parse_card(body) {
        Ok(c) => c,
        Err(e) => return refuse_card(host, &msg_id, row, e.to_string()).await,
    };

    row.card_type = card.card_type.clone();
    row.version = card.version;
    row.known = card::is_known_card(&card.card_type, card.version);
    if !row.known {
        put_row(host, CARDS, &msg_id, &row).await?;
        return Ok(FileCardResult {
            filed: true,
            refused: false,
            unknown: true,
            countersigned: false,
        });
    }

    match card.card_type.as_str() {
        "request" => {
            file_request_card(host, &msg_id, row, &card.envelope, conversation, now, owner).await
        }
        "quote" => {
            file_quote_card(host, &msg_id, row, &card.envelope, conversation, now, owner).await
        }
        "agreement-receipt" => {
            file_agreement_receipt_card(
                host,
                &msg_id,
                row,
                &card.envelope,
                conversation,
                now,
                owner,
            )
            .await
        }
        _ => {
            refuse_card(host, &msg_id, row, "a known card type with no producer in this build")
                .await
        }
    }
}

/// Stores `row` with `reason` and reports it as filed-but-refused.
async fn refuse_card<H: AppHost>(
    host: &H,
    msg_id: &str,
    mut row: CardRow,
    reason: impl Into<String>,
) -> Result<FileCardResult, String> {
    row.reason = Some(reason.into());
    put_row(host, CARDS, msg_id, &row).await?;
    Ok(FileCardResult { filed: true, refused: true, unknown: false, countersigned: false })
}

async fn file_request_card<H: AppHost>(
    host: &H,
    msg_id: &str,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = transaction::verify_request(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "request does not verify".to_string());
        return refuse_card(host, msg_id, row, reason).await;
    }
    let payload = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, msg_id, row, "missing payload").await,
    };
    if payload.conversation != conversation {
        return refuse_card(host, msg_id, row, "card names another conversation").await;
    }
    let version_count = match store_received_request(host, envelope, &v, payload, now, owner).await
    {
        Ok(vc) => vc,
        Err(e) => return refuse_card(host, msg_id, row, e).await,
    };
    row.verified = true;
    row.version_count = Some(version_count);
    row.data = Some(serde_json::to_value(payload).unwrap_or(Value::Null));
    row.issuer = v.issuer;
    row.record_id = v.record_id;
    row.revocation_status = v.revocation_status;
    put_row(host, CARDS, msg_id, &row).await?;
    Ok(FileCardResult { filed: true, refused: false, unknown: false, countersigned: false })
}

async fn file_quote_card<H: AppHost>(
    host: &H,
    msg_id: &str,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = transaction::verify_quote(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "quote does not verify".to_string());
        return refuse_card(host, msg_id, row, reason).await;
    }
    let payload = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, msg_id, row, "missing payload").await,
    };
    if payload.conversation != conversation {
        return refuse_card(host, msg_id, row, "card names another conversation").await;
    }
    let req_bytes = match get_bytes(host, REQUEST_HISTORY, &payload.request_record_id).await? {
        Some(b) => b,
        None => {
            return refuse_card(host, msg_id, row, "answers a request this node does not hold")
                .await;
        }
    };
    let req_str = match String::from_utf8(req_bytes) {
        Ok(s) => s,
        Err(_) => {
            return refuse_card(host, msg_id, row, "stored request envelope is invalid utf8").await;
        }
    };
    let req_v = transaction::verify_request(&req_str, now);
    if !req_v.verified || req_v.issuer.as_deref() != Some(payload.consumer_did.as_str()) {
        return refuse_card(host, msg_id, row, "quote consumer_did does not match request issuer")
            .await;
    }
    let version_count = match store_received_quote(host, envelope, &v, payload, now, owner).await {
        Ok(vc) => vc,
        Err(e) => return refuse_card(host, msg_id, row, e).await,
    };
    row.verified = true;
    row.expired = v.expired;
    row.version_count = Some(version_count);
    row.data = Some(serde_json::to_value(payload).unwrap_or(Value::Null));
    row.issuer = v.issuer;
    row.record_id = v.record_id;
    row.revocation_status = v.revocation_status;
    put_row(host, CARDS, msg_id, &row).await?;
    Ok(FileCardResult { filed: true, refused: false, unknown: false, countersigned: false })
}

async fn file_agreement_receipt_card<H: AppHost>(
    host: &H,
    msg_id: &str,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = transaction::verify_agreement_receipt(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "agreement receipt does not verify".to_string());
        return refuse_card(host, msg_id, row, reason).await;
    }
    let payload = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, msg_id, row, "missing payload").await,
    };
    let q_bytes = match get_bytes(host, QUOTE_HISTORY, &payload.quote_record_id).await? {
        Some(b) => b,
        None => {
            return refuse_card(host, msg_id, row, "attests a quote this node does not hold").await;
        }
    };
    let q_str = match String::from_utf8(q_bytes) {
        Ok(s) => s,
        Err(_) => {
            return refuse_card(host, msg_id, row, "stored quote envelope is invalid utf8").await;
        }
    };
    let qv = transaction::verify_quote(&q_str, now);
    if !qv.verified {
        return refuse_card(host, msg_id, row, "the quote it attests does not verify").await;
    }
    let q_payload = match qv.payload.as_ref() {
        Some(qp) => qp,
        None => return refuse_card(host, msg_id, row, "quote missing payload").await,
    };
    if q_payload.terms != payload.terms {
        return refuse_card(host, msg_id, row, "terms differ from the quote").await;
    }
    if payload.consumer_did != q_payload.consumer_did
        || payload.provider_did != qv.issuer.as_deref().unwrap_or("")
    {
        return refuse_card(host, msg_id, row, "names the wrong parties").await;
    }
    let issued_at = v.issued_at_secs.unwrap_or(0);
    if issued_at >= payload.terms.quote_expires_at_secs {
        return refuse_card(host, msg_id, row, "accepted after the quote expired").await;
    }
    let mut row_agr: AgreementRow =
        match get_row(host, AGREEMENTS, &payload.quote_record_id).await? {
            Some(r) => r,
            None => AgreementRow {
                quote_record_id: payload.quote_record_id.clone(),
                conversation: conversation.to_string(),
                consumer_did: payload.consumer_did.clone(),
                provider_did: payload.provider_did.clone(),
                terms: q_payload.terms.clone(),
                consumer: None,
                provider: None,
                updated_at_secs: now,
            },
        };
    if row_agr.half(payload.role).is_none() {
        let half = ReceiptHalf {
            envelope: envelope.to_string(),
            record_id: v.record_id.clone().unwrap_or_default(),
            issuer: v.issuer.clone().unwrap_or_default(),
            issued_at_secs: issued_at,
        };
        row_agr.set_half(payload.role, half);
        row_agr.updated_at_secs = now;
        put_row(host, AGREEMENTS, &payload.quote_record_id, &row_agr).await?;
    }
    row.verified = true;
    row.data = Some(serde_json::to_value(payload).unwrap_or(Value::Null));
    row.issuer = v.issuer;
    row.record_id = v.record_id;
    row.revocation_status = v.revocation_status;
    put_row(host, CARDS, msg_id, &row).await?;
    let countersigned = maybe_countersign(host, &mut row_agr, &qv, now, owner).await?;
    Ok(FileCardResult { filed: true, refused: false, unknown: false, countersigned })
}

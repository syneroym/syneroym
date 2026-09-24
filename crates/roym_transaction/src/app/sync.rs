//! Transaction synchronization across conversation history.

pub(crate) mod receipts;

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

#[derive(Debug, Default)]
pub(crate) struct FileCardResult {
    pub(crate) filed: bool,
    pub(crate) refused: bool,
    pub(crate) unknown: bool,
    pub(crate) countersigned: bool,
    pub(crate) deferred: bool,
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
    let messages = match fetch_sync_messages(host, &params.conversation, start).await {
        Ok(m) => m,
        Err(resp) => return resp,
    };

    let mut card_count = match count_cards_for_conversation(host, &params.conversation).await {
        Ok(c) => c,
        Err(e) => return Response::internal_error(e),
    };

    let mut stats = SyncStats::default();
    let mut offset = start;
    let mut first_declined: Option<u64> = None;
    let owner = signing::owner_did(host).await.unwrap_or_default();

    for m in messages {
        offset += 1;
        stats.scanned += 1;
        match classify_sync_message(host, &m, &params.conversation, now, &owner, card_count).await {
            Ok(SyncOutcome::Skip) => {}
            Ok(SyncOutcome::Declined) => {
                first_declined = first_declined.or(Some(offset - 1));
            }
            Ok(SyncOutcome::Deferred) => {
                first_declined = first_declined.or(Some(offset - 1));
                stats.deferred += 1;
            }
            Ok(SyncOutcome::Filed(file_res)) => {
                card_count += 1;
                stats.record_filed(&file_res);
            }
            Err(e) => return Response::internal_error(e),
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
        "scanned": stats.scanned,
        "filed": stats.filed,
        "refused": stats.refused,
        "unknown": stats.unknown,
        "countersigned": stats.countersigned,
        "deferred": stats.deferred,
        "scanned_count": new_scanned_count,
    }))
}

async fn fetch_sync_messages<H: AppHost>(
    host: &H,
    conversation: &str,
    start: u64,
) -> Result<Vec<Value>, Response> {
    let page_resp = match conversation_call(
        host,
        "conversation.history",
        json!({
            "conversation": conversation,
            "limit": SYNC_WINDOW,
            "cursor": start,
        }),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return Err(Response::internal_error(e)),
    };

    if let Some(err) = page_resp.error {
        return Err(Response::internal_error(err.message));
    }

    let res = page_resp.result.unwrap_or(Value::Null);
    Ok(res.get("messages").and_then(Value::as_array).cloned().unwrap_or_default())
}

#[derive(Default)]
struct SyncStats {
    scanned: u32,
    filed: u32,
    refused: u32,
    unknown: u32,
    countersigned: u32,
    deferred: u32,
}

impl SyncStats {
    fn record_filed(&mut self, res: &FileCardResult) {
        if res.filed {
            self.filed += 1;
        }
        if res.refused {
            self.refused += 1;
        }
        if res.unknown {
            self.unknown += 1;
        }
        if res.countersigned {
            self.countersigned += 1;
        }
    }
}

/// What a single history message meant for sync, once it has been classified.
enum SyncOutcome {
    /// Not a card, deleted, missing an id, or already filed -- nothing to do.
    Skip,
    /// A card this node would otherwise file, but the conversation is at its
    /// card-count cap.
    Declined,
    /// A card whose prerequisite is missing but still within the deferral
    /// window.
    Deferred,
    /// Filed (successfully or as a refusal); carries the counters to fold in.
    Filed(FileCardResult),
}

/// Decides what to do with one history message: skip it, decline it for
/// being over the per-conversation card cap, or file it as a card.
async fn classify_sync_message<H: AppHost>(
    host: &H,
    m: &Value,
    conversation: &str,
    now: u64,
    owner: &str,
    card_count: usize,
) -> Result<SyncOutcome, String> {
    let content_type = m.get("content_type").and_then(Value::as_str).unwrap_or("");
    if content_type != CARD_CONTENT_TYPE {
        return Ok(SyncOutcome::Skip);
    }
    if m.get("deleted_at_secs").and_then(Value::as_u64).is_some() {
        return Ok(SyncOutcome::Skip);
    }
    let msg_id = match m.get("id").and_then(Value::as_str) {
        Some(id) => id,
        None => return Ok(SyncOutcome::Skip),
    };
    let exists = match AppDataLayer::get(host, CARDS.to_string(), msg_id.to_string()).await {
        Ok(o) => o.is_some(),
        Err(e) => return Err(e.to_string()),
    };
    if exists {
        return Ok(SyncOutcome::Skip);
    }
    if card_count >= MAX_CARDS_PER_CONVERSATION {
        return Ok(SyncOutcome::Declined);
    }

    let file_res = file_incoming_card(host, m, conversation, now, owner).await?;
    if file_res.deferred { Ok(SyncOutcome::Deferred) } else { Ok(SyncOutcome::Filed(file_res)) }
}

fn init_card_row(m: &Value, conversation: &str, now: u64) -> (String, CardRow) {
    let msg_id = m.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let direction_str = m.get("direction").and_then(Value::as_str).unwrap_or("incoming");
    let direction =
        if direction_str == "outgoing" { Direction::Outgoing } else { Direction::Incoming };
    let sender_timestamp_ms =
        m.get("sender_timestamp_ms").and_then(Value::as_i64).unwrap_or(now as i64 * 1000);

    let row = CardRow {
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
        agreement_payee: None,
        agreement_payment_methods: Vec::new(),
    };
    (msg_id, row)
}

async fn dispatch_known_card<H: AppHost>(
    host: &H,
    msg_id: &str,
    row: CardRow,
    card: &card::Card,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    match card.card_type.as_str() {
        "request" => {
            file_request_card(host, msg_id, row, &card.envelope, conversation, now, owner).await
        }
        "quote" => {
            file_quote_card(host, msg_id, row, &card.envelope, conversation, now, owner).await
        }
        "agreement-receipt" => {
            file_agreement_receipt_card(host, msg_id, row, &card.envelope, conversation, now, owner)
                .await
        }
        "booking-progress" => {
            receipts::file_progress_card(
                host,
                msg_id,
                row,
                &card.envelope,
                conversation,
                now,
                owner,
            )
            .await
        }
        "payment-request" => {
            receipts::file_payment_request_card(
                host,
                msg_id,
                row,
                &card.envelope,
                conversation,
                now,
                owner,
            )
            .await
        }
        "payment-acknowledgement" => {
            receipts::file_payment_ack_card(
                host,
                msg_id,
                row,
                &card.envelope,
                conversation,
                now,
                owner,
            )
            .await
        }
        "fulfilment-receipt" => {
            receipts::file_fulfilment_card(
                host,
                msg_id,
                row,
                &card.envelope,
                conversation,
                now,
                owner,
            )
            .await
        }
        _ => refuse_card(host, msg_id, row, "a known card type this build does not file").await,
    }
}

async fn file_incoming_card<H: AppHost>(
    host: &H,
    m: &Value,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let (msg_id, mut row) = init_card_row(m, conversation, now);

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
            deferred: false,
        });
    }

    dispatch_known_card(host, &msg_id, row, &card, conversation, now, owner).await
}

/// Stores `row` with `reason` and reports it as filed-but-refused.
pub(crate) async fn refuse_card<H: AppHost>(
    host: &H,
    msg_id: &str,
    mut row: CardRow,
    reason: impl Into<String>,
) -> Result<FileCardResult, String> {
    row.reason = Some(reason.into());
    put_row(host, CARDS, msg_id, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: true,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
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
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
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
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
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
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned,
        deferred: false,
    })
}

//! Transaction synchronization across conversation history.

pub(crate) mod receipts;
pub(crate) mod settle_receipts;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{AppDataLayer, AppHost};
use syneroym_roym_core::{
    card::{self, CARD_CONTENT_TYPE},
    clock,
    conversation::{Direction, group::CARDS_NOT_IN_GROUPS_MESSAGE},
    envelope::Response,
    method::{
        Reply, SiblingError, call_sibling,
        conversation::{ChangesInput, ChangesOutput},
    },
    paging, services, signing,
    transaction::{
        self, AgreementReceiptPayload, DealDeclinePayload, MAX_CARDS_PER_CONVERSATION,
        QuotePayload, ReceiptHalf, SYNC_WINDOW,
    },
};

use super::{
    AGREEMENTS, AgreementRow, CARDS, CardRow, ENQUIRIES, ENQUIRY_HISTORY, LEDGER, QUOTE_HISTORY,
    QUOTES, RecordPointerRow, SYNC_STATE, SyncStateRow, decide, decline_ops,
    enquiry_ops::store_received_enquiry, ensure_collections, get_bytes, get_row, progress_ops,
    put_card, put_row, quote_ops::store_received_quote,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncInput {
    pub conversation: String,
    #[serde(default)]
    pub full: bool,
}

#[derive(Debug, Serialize)]
pub struct SyncOutput {
    pub scanned: u32,
    pub filed: u32,
    pub refused: u32,
    pub unknown: u32,
    pub countersigned: u32,
    pub deferred: u32,
    pub last_seq: u64,
}

#[derive(Debug, Default)]
pub(crate) struct FileCardResult {
    pub(crate) filed: bool,
    pub(crate) refused: bool,
    pub(crate) unknown: bool,
    pub(crate) countersigned: bool,
    pub(crate) deferred: bool,
}

pub(crate) async fn sync<H: AppHost>(host: &H, input: SyncInput) -> Reply<SyncOutput> {
    sync_at(host, input, clock::now_secs()).await
}

pub(crate) async fn sync_at<H: AppHost>(host: &H, input: SyncInput, now: u64) -> Reply<SyncOutput> {
    ensure_collections(host).await.map_err(Response::internal_error)?;

    let sync_state: SyncStateRow = match get_row(host, SYNC_STATE, &input.conversation).await {
        Ok(Some(s)) => s,
        Ok(None) => SyncStateRow { last_seq: 0 },
        Err(e) => return Err(Response::internal_error(e)),
    };

    let start = if input.full { 0 } else { sync_state.last_seq };
    let (messages, last_seq, kind) = fetch_sync_messages(host, &input.conversation, start).await?;
    if kind.as_deref() == Some("group") {
        return Err(Response::invalid_params(CARDS_NOT_IN_GROUPS_MESSAGE));
    }

    let card_filter = json!({ "conversation": input.conversation }).to_string();
    let mut card_count = match paging::count(host, CARDS, Some(card_filter)).await {
        Ok(c) => c,
        Err(e) => return Err(Response::internal_error(e)),
    };

    let mut stats = SyncStats::default();
    let mut first_declined: Option<u64> = None;
    let owner = signing::owner_did(host).await.unwrap_or_default();

    for m in messages {
        stats.scanned += 1;
        let vseq = m.get("visible_seq").and_then(Value::as_u64);
        match classify_sync_message(host, &m, &input.conversation, now, &owner, card_count).await {
            Ok(SyncOutcome::Skip) => {}
            Ok(SyncOutcome::Declined) => {
                first_declined = first_declined.or(vseq.map(|s| s.saturating_sub(1)));
            }
            Ok(SyncOutcome::Deferred) => {
                first_declined = first_declined.or(vseq.map(|s| s.saturating_sub(1)));
                stats.deferred += 1;
            }
            Ok(SyncOutcome::Filed(file_res)) => {
                card_count += 1;
                stats.record_filed(&file_res);
            }
            Err(e) => return Err(Response::internal_error(e)),
        }
    }

    let new_last_seq = match first_declined {
        Some(o) => o,
        None => sync_state.last_seq.max(last_seq),
    };
    if let Err(e) =
        put_row(host, SYNC_STATE, &input.conversation, &SyncStateRow { last_seq: new_last_seq })
            .await
    {
        return Err(Response::internal_error(e));
    }

    let _ = progress_ops::tick_conversation_at(host, &input.conversation, now).await;
    let _ = decide::recover_deciding_rows(host, now).await;

    Ok(SyncOutput {
        scanned: stats.scanned,
        filed: stats.filed,
        refused: stats.refused,
        unknown: stats.unknown,
        countersigned: stats.countersigned,
        deferred: stats.deferred,
        last_seq: new_last_seq,
    })
}

async fn fetch_sync_messages<H: AppHost>(
    host: &H,
    conversation: &str,
    start: u64,
) -> Result<(Vec<Value>, u64, Option<String>), Response> {
    let out = match call_sibling::<_, ChangesInput, ChangesOutput>(
        host,
        services::CONVERSATION,
        "conversation.changes",
        &ChangesInput {
            conversation: conversation.to_string(),
            after_seq: Some(start),
            limit: Some(SYNC_WINDOW),
        },
    )
    .await
    {
        Ok(o) => o,
        Err(SiblingError::Refused(e)) => return Err(Response::internal_error(e.message)),
        Err(SiblingError::Transport(e) | SiblingError::BadReply(e)) => {
            return Err(Response::internal_error(e));
        }
    };

    let kind = serde_json::to_value(out.kind).ok().and_then(|v| v.as_str().map(str::to_string));
    let messages = serde_json::to_value(out.messages)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    Ok((messages, out.last_seq, kind))
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

fn init_card_row(m: &Value, conversation: &str, now: u64) -> CardRow {
    let msg_id = m.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let direction_str = m.get("direction").and_then(Value::as_str).unwrap_or("incoming");
    let direction =
        if direction_str == "outgoing" { Direction::Outgoing } else { Direction::Incoming };
    let sender_timestamp_ms =
        m.get("sender_timestamp_ms").and_then(Value::as_i64).unwrap_or(now as i64 * 1000);

    CardRow {
        message_id: msg_id,
        conversation: conversation.to_string(),
        direction,
        sender_timestamp_ms,
        card_type: String::new(),
        version: 0,
        known: false,
        verified: false,
        expired: false,
        repeat: false,
        reason: None,
        issuer: None,
        record_id: None,
        revocation_status: None,
        data: None,
        stored_at_secs: now,
        declined: None,
        deal_state: None,
        version_count: None,
        agreement_payee: None,
        agreement_payment_methods: Vec::new(),
    }
}

async fn dispatch_known_card<H: AppHost>(
    host: &H,
    row: CardRow,
    sender: &str,
    card: &card::Card,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    match card.card_type.as_str() {
        "enquiry" => file_enquiry_card(host, row, &card.envelope, conversation, now, owner).await,
        "quote" => file_quote_card(host, row, &card.envelope, conversation, now, owner).await,
        "agreement-receipt" => {
            file_agreement_receipt_card(host, row, &card.envelope, conversation, now, owner).await
        }
        "agreement-progress" => {
            receipts::file_progress_card(host, row, &card.envelope, conversation, now, owner).await
        }
        "payment-request" => {
            settle_receipts::file_payment_request_card(
                host,
                row,
                &card.envelope,
                conversation,
                now,
                owner,
            )
            .await
        }
        "payment-acknowledgement" => {
            settle_receipts::file_payment_ack_card(
                host,
                row,
                &card.envelope,
                conversation,
                now,
                owner,
            )
            .await
        }
        "fulfilment-receipt" => {
            settle_receipts::file_fulfilment_card(
                host,
                row,
                &card.envelope,
                conversation,
                now,
                owner,
            )
            .await
        }
        "deal-decline" => {
            file_deal_decline_card(host, row, sender, &card.envelope, conversation, now, owner)
                .await
        }
        _ => refuse_card(host, row, "a known card type this build does not file").await,
    }
}

pub(crate) async fn file_incoming_card<H: AppHost>(
    host: &H,
    m: &Value,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let mut row = init_card_row(m, conversation, now);
    let sender = m.get("author").and_then(Value::as_str).unwrap_or_default();

    let body = match m.get("body").and_then(Value::as_str) {
        Some(b) => b,
        None => return refuse_card(host, row, "no body").await,
    };

    let card = match card::parse_card(body) {
        Ok(c) => c,
        Err(e) => return refuse_card(host, row, e.to_string()).await,
    };

    row.card_type = card.card_type.clone();
    row.version = card.version;
    row.known = card::is_known_card(&card.card_type, card.version);
    if !row.known {
        put_card(host, &row).await?;
        return Ok(FileCardResult {
            filed: true,
            refused: false,
            unknown: true,
            countersigned: false,
            deferred: false,
        });
    }

    dispatch_known_card(host, row, sender, &card, conversation, now, owner).await
}

/// Stores `row` with `reason` and reports it as filed-but-refused.
pub(crate) async fn refuse_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    reason: impl Into<String>,
) -> Result<FileCardResult, String> {
    row.reason = Some(reason.into());
    put_card(host, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: true,
        unknown: false,
        countersigned: false,
        deferred: false,
    })
}

async fn file_enquiry_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = transaction::verify_enquiry(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "enquiry does not verify".to_string());
        return refuse_card(host, row, reason).await;
    }
    let payload = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, row, "missing payload").await,
    };
    if payload.conversation != conversation {
        return refuse_card(host, row, "card names another conversation").await;
    }
    let already_held = match &v.record_id {
        Some(rid) => get_bytes(host, ENQUIRY_HISTORY, rid).await?.is_some(),
        None => false,
    };
    let version_count = match store_received_enquiry(host, envelope, &v, payload, now, owner).await
    {
        Ok(vc) => vc,
        Err(e) => return refuse_card(host, row, e).await,
    };
    row.verified = true;
    row.repeat = already_held;
    row.version_count = Some(version_count);
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

async fn file_quote_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = transaction::verify_quote(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "quote does not verify".to_string());
        return refuse_card(host, row, reason).await;
    }
    let payload = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, row, "missing payload").await,
    };
    if payload.conversation != conversation {
        return refuse_card(host, row, "card names another conversation").await;
    }
    let enquiry_rec_id = match &payload.answers {
        transaction::Answers::Enquiry { record_id } => record_id,
        transaction::Answers::Want { .. } => {
            return refuse_card(host, row, "answers a want, which is not supported").await;
        }
    };
    let req_bytes = match get_bytes(host, ENQUIRY_HISTORY, enquiry_rec_id).await? {
        Some(b) => b,
        None => {
            return refuse_card(host, row, "answers an enquiry this node does not hold").await;
        }
    };
    let req_str = match String::from_utf8(req_bytes) {
        Ok(s) => s,
        Err(_) => {
            return refuse_card(host, row, "stored enquiry envelope is invalid utf8").await;
        }
    };
    let req_v = transaction::verify_enquiry(&req_str, now);
    if !req_v.verified || req_v.issuer.as_deref() != Some(payload.counterparty_did.as_str()) {
        return refuse_card(host, row, "quote counterparty_did does not match enquiry issuer")
            .await;
    }
    let already_held = match &v.record_id {
        Some(rid) => get_bytes(host, QUOTE_HISTORY, rid).await?.is_some(),
        None => false,
    };
    let version_count = match store_received_quote(host, envelope, &v, payload, now, owner).await {
        Ok(vc) => vc,
        Err(e) => return refuse_card(host, row, e).await,
    };
    row.verified = true;
    row.repeat = already_held;
    row.expired = v.expired;
    row.version_count = Some(version_count);
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

async fn validate_receipt_against_quote<H: AppHost>(
    host: &H,
    payload: &AgreementReceiptPayload,
    issued_at: u64,
    now: u64,
) -> Result<QuotePayload, &'static str> {
    let q_bytes = match get_bytes(host, QUOTE_HISTORY, &payload.quote_record_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Err("attests a quote this node does not hold"),
        Err(_) => return Err("storage error loading quote"),
    };
    let q_str = match String::from_utf8(q_bytes) {
        Ok(s) => s,
        Err(_) => return Err("stored quote envelope is invalid utf8"),
    };
    let qv = transaction::verify_quote(&q_str, now);
    if !qv.verified {
        return Err("the quote it attests does not verify");
    }
    let q_payload = match qv.payload {
        Some(qp) => qp,
        None => return Err("quote missing payload"),
    };
    if q_payload.terms != payload.terms {
        return Err("terms differ from the quote");
    }
    if payload.counterparty_did != q_payload.counterparty_did
        || payload.quote_author_did != qv.issuer.as_deref().unwrap_or("")
    {
        return Err("names the wrong parties");
    }
    if issued_at >= payload.terms.quote_expires_at_secs {
        return Err("accepted after the quote expired");
    }
    Ok(q_payload)
}

async fn file_agreement_receipt_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    envelope: &str,
    conversation: &str,
    now: u64,
    owner: &str,
) -> Result<FileCardResult, String> {
    let v = transaction::verify_agreement_receipt(envelope, now);
    if !v.verified {
        let reason = v.reason.unwrap_or_else(|| "agreement receipt does not verify".to_string());
        return refuse_card(host, row, reason).await;
    }
    let payload = match v.payload.as_ref() {
        Some(p) => p,
        None => return refuse_card(host, row, "missing payload").await,
    };
    let issued_at = v.issued_at_secs.unwrap_or(0);
    let q_payload = match validate_receipt_against_quote(host, payload, issued_at, now).await {
        Ok(qp) => qp,
        Err(reason) => return refuse_card(host, row, reason).await,
    };
    let mut row_agr: AgreementRow =
        match get_row(host, AGREEMENTS, &payload.quote_record_id).await? {
            Some(r) => r,
            None => AgreementRow {
                quote_record_id: payload.quote_record_id.clone(),
                quote_id: q_payload.quote_id.clone(),
                conversation: conversation.to_string(),
                counterparty_did: payload.counterparty_did.clone(),
                quote_author_did: payload.quote_author_did.clone(),
                terms: q_payload.terms.clone(),
                state: None,
                state_reason: None,
                cancel_reason: None,
                seq: None,
                commitments: Vec::new(),
                agreed_at_secs: None,
                counterparty: None,
                quote_author: None,
                updated_at_secs: now,
            },
        };
    if let (Some(rid), Some(half)) = (&v.record_id, row_agr.half(payload.role))
        && rid == &half.record_id
    {
        row.repeat = true;
    }
    if row_agr.half(payload.role).is_none() {
        let half = ReceiptHalf {
            envelope: envelope.to_string(),
            record_id: v.record_id.clone().unwrap_or_default(),
            issuer: v.issuer.clone().unwrap_or_default(),
            issued_at_secs: issued_at,
        };
        row_agr.set_half(payload.role, half);
        if payload.role == transaction::DealRole::QuoteAuthor {
            row_agr.agreed_at_secs = Some(issued_at);
        }
        row_agr.updated_at_secs = now;
        put_row(host, AGREEMENTS, &payload.quote_record_id, &row_agr).await?;
    }
    row.verified = true;
    row.data = Some(serde_json::to_value(payload).unwrap_or(Value::Null));
    row.issuer = v.issuer;
    row.record_id = v.record_id;
    row.revocation_status = v.revocation_status;
    let countersigned = if payload.role == transaction::DealRole::Counterparty
        && owner == row_agr.quote_author_did
    {
        match decide::run(host, &mut row_agr, &q_payload, now).await {
            Ok(cs) => cs,
            Err(e) => {
                let dec_key = decide::decision_id(&q_payload.quote_id);
                if let Ok(Some(dec)) =
                    get_row::<decide::DecisionRow, _>(host, LEDGER, &dec_key).await
                    && dec.state == decide::DecisionState::Deciding
                {
                    false
                } else {
                    return Err(e);
                }
            }
        }
    } else {
        false
    };
    put_card(host, &row).await?;
    Ok(FileCardResult {
        filed: true,
        refused: false,
        unknown: false,
        countersigned,
        deferred: false,
    })
}

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
    sender: &str,
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
    let enq_cp = ptr.counterparty_did.as_deref().unwrap_or_default();
    if signer != enq_issuer && signer != enq_cp && signer != sender {
        return Err("only the counterparty or quote author can decline this enquiry");
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

async fn file_deal_decline_card<H: AppHost>(
    host: &H,
    mut row: CardRow,
    sender: &str,
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
        if let Err(reason) = verify_and_apply_declined_enquiry(
            host,
            payload,
            &e_bytes,
            signer,
            sender,
            conversation,
            now,
        )
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

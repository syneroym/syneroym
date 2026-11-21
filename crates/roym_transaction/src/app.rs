//! Transaction service application logic, target-independent.

pub mod agreement_ops;
pub mod backup;
pub mod decide;
pub mod decline_ops;
pub mod disclosures;
pub mod enquiry_ops;
mod extension_rules;
pub mod fulfilment_ops;
pub mod hold;
pub mod ledger;
pub mod parties;
pub mod payment_ops;
pub mod present_ops;
pub mod progress;
pub mod progress_ops;
pub mod quote_ops;
pub mod step_ops;
pub mod sync;
pub mod thread;

#[cfg(test)]
pub mod tests;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::{
        data_layer::{CollectionSchema, IndexDefinition, IndexType, RecordWriteValue},
        signing::Principal,
    },
};
use syneroym_roym_core::{
    admit,
    card::{self, CARD_CONTENT_TYPE},
    conversation::{Direction, StoredState},
    deal::DealTerms,
    envelope::{Request, Response},
    method::{
        SiblingError, call_sibling,
        conversation::{SendInput, SendOutput},
    },
    paging,
    progress::{AgreementState, CancelReason, CommitmentProgress},
    record::Envelope,
    services,
    signing::{self, CertificateError},
    transaction::{DealRole, EVENT_CARD, EVENT_PROGRESS, ReceiptHalf},
};

use crate::api;

// Schema version 6 switches deal and settlement records to selective
// disclosure.
pub const SCHEMA_VERSION: u32 = 6;

pub(crate) const ENQUIRIES: &str = "enquiries";
pub(crate) const ENQUIRY_HISTORY: &str = "enquiry_history";
pub(crate) const QUOTES: &str = "quotes";
pub(crate) const QUOTE_HISTORY: &str = "quote_history";
pub(crate) const AGREEMENTS: &str = "agreements";
pub(crate) const CARDS: &str = "cards";
pub(crate) const SYNC_STATE: &str = "sync_state";
pub(crate) const LEDGER: &str = "ledger";
pub(crate) const AGREEMENT_PROGRESS: &str = "agreement_progress";
pub(crate) const PAYMENTS: &str = "payments";
pub(crate) const FULFILMENTS: &str = "fulfilments";
pub(crate) const STEPS: &str = "steps";
pub(crate) const DISCLOSURES: &str = "disclosures";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SendState {
    Pending,
    Deposited,
    Delivered,
    Failed,
    NotSent,
    AlreadyAccepted,
    AlreadyRecorded,
}

impl From<StoredState> for SendState {
    fn from(s: StoredState) -> Self {
        match s {
            StoredState::Pending => Self::Pending,
            StoredState::Deposited => Self::Deposited,
            StoredState::Delivered => Self::Delivered,
            StoredState::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementRef {
    #[serde(alias = "quote_record_id")]
    pub agreement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HalfRecordedOutput {
    pub record_id: String,
    pub role: DealRole,
    pub message_id: Option<String>,
    pub state: SendState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FenceOutcome<O> {
    Claimed,
    Answered(O),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListInput {
    #[serde(default)]
    pub conversation: Option<String>,
    #[serde(default)]
    pub mine: Option<bool>,
    #[serde(default = "default_list_limit_u64")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

pub(crate) fn default_list_limit_u64() -> u64 {
    200
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RecordPointerRow {
    pub(crate) envelope: String,
    pub(crate) record_id: String,
    pub(crate) id: String,
    pub(crate) conversation: String,
    pub(crate) sequence: u32,
    pub(crate) issuer: String,
    pub(crate) mine: bool,
    pub(crate) updated_at_secs: u64,
    pub(crate) version_count: u64,
    #[serde(default)]
    pub(crate) issued_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) request_record_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) counterparty_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) declined_at_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) decline_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AgreementRow {
    pub(crate) quote_record_id: String,
    pub(crate) quote_id: String,
    pub(crate) conversation: String,
    pub(crate) counterparty_did: String,
    pub(crate) quote_author_did: String,
    pub(crate) terms: DealTerms,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) state: Option<AgreementState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) state_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cancel_reason: Option<CancelReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) seq: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) commitments: Vec<CommitmentProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agreed_at_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) counterparty: Option<ReceiptHalf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) quote_author: Option<ReceiptHalf>,
    pub(crate) updated_at_secs: u64,
}

impl AgreementRow {
    pub(crate) fn half(&self, role: DealRole) -> Option<&ReceiptHalf> {
        match role {
            DealRole::Counterparty => self.counterparty.as_ref(),
            DealRole::QuoteAuthor => self.quote_author.as_ref(),
        }
    }

    pub(crate) fn set_half(&mut self, role: DealRole, half: ReceiptHalf) {
        match role {
            DealRole::Counterparty => self.counterparty = Some(half),
            DealRole::QuoteAuthor => self.quote_author = Some(half),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardRow {
    pub message_id: String,
    pub conversation: String,
    pub direction: Direction,
    pub sender_timestamp_ms: i64,
    pub card_type: String,
    pub version: u32,
    pub known: bool,
    pub verified: bool,
    #[serde(default)]
    pub expired: bool,
    #[serde(default)]
    pub repeat: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    pub stored_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declined: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deal_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agreement_payee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agreement_payment_methods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SyncStateRow {
    pub(crate) last_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct PaymentsRow {
    pub(crate) agreement: String,
    #[serde(default)]
    pub(crate) commitment_id: String,
    pub(crate) conversation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) request: Option<ReceiptHalf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) counterparty: Vec<ReceiptHalf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) quote_author: Vec<ReceiptHalf>,
    pub(crate) updated_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct FulfilmentsRow {
    pub(crate) agreement: String,
    #[serde(default)]
    pub(crate) commitment_id: String,
    pub(crate) conversation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) counterparty: Option<ReceiptHalf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) quote_author: Option<ReceiptHalf>,
    pub(crate) updated_at_secs: u64,
}

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::TRANSACTION.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if let Some(resp) = admit::require_internal(host).await {
        return resp;
    }
    if let Some(resp) = api::dispatch(host, &req).await {
        return resp;
    }
    Response::method_not_found(&req.method)
}

pub(crate) async fn resolve_principal_and_owner<H: AppHost>(
    host: &H,
    now: u64,
) -> Result<(Principal, String), Response> {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(CertificateError::NoOwner) => {
            return Err(Response::invalid_params("this installation has no recorded owner"));
        }
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let (principal, _master) = match signing::person_principal(host, now).await {
        Ok(res) => res,
        Err(CertificateError::NotEnrolled) => {
            return Err(Response::invalid_params("signing-not-enrolled"));
        }
        Err(CertificateError::Expired(t)) => {
            return Err(Response::invalid_params(format!("signing-certificate-expired at {t}")));
        }
        Err(CertificateError::Stale { installed_for, current }) => {
            return Err(Response::invalid_params(format!(
                "signing-certificate-stale: {installed_for} vs {current}"
            )));
        }
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    Ok((principal, owner))
}

async fn ensure_coll<H: AppHost>(
    host: &H,
    name: &str,
    indexes: &[IndexDefinition],
) -> Result<(), String> {
    AppDataLayer::create_collection(
        host,
        CollectionSchema { name: name.to_string(), indexes: indexes.to_vec(), text_fields: vec![] },
    )
    .await
    .map_err(|e| e.to_string())
}

fn idx(field: &str, ty: IndexType) -> IndexDefinition {
    IndexDefinition { field_name: field.to_string(), type_: ty }
}

pub(crate) async fn ensure_collections<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(
        host,
        ENQUIRIES,
        &[idx("conversation", IndexType::String), idx("updated_at_secs", IndexType::Numeric)],
    )
    .await?;
    ensure_coll(host, ENQUIRY_HISTORY, &[]).await?;
    ensure_coll(
        host,
        QUOTES,
        &[idx("conversation", IndexType::String), idx("updated_at_secs", IndexType::Numeric)],
    )
    .await?;
    ensure_coll(host, QUOTE_HISTORY, &[]).await?;
    ensure_coll(host, disclosures::DISCLOSURES, &[]).await?;
    ensure_coll(
        host,
        AGREEMENTS,
        &[idx("conversation", IndexType::String), idx("quote_id", IndexType::String)],
    )
    .await?;
    ensure_coll(
        host,
        CARDS,
        &[idx("conversation", IndexType::String), idx("sender_timestamp_ms", IndexType::Numeric)],
    )
    .await?;
    ensure_coll(host, SYNC_STATE, &[]).await?;
    ensure_coll(
        host,
        LEDGER,
        &[
            idx("kind", IndexType::String),
            idx("agreement", IndexType::String),
            idx("quote_id", IndexType::String),
            idx("state", IndexType::String),
        ],
    )
    .await?;
    ensure_coll(
        host,
        AGREEMENT_PROGRESS,
        &[idx("conversation", IndexType::String), idx("agreement", IndexType::String)],
    )
    .await?;
    ensure_coll(
        host,
        PAYMENTS,
        &[idx("conversation", IndexType::String), idx("agreement", IndexType::String)],
    )
    .await?;
    ensure_coll(
        host,
        FULFILMENTS,
        &[idx("conversation", IndexType::String), idx("agreement", IndexType::String)],
    )
    .await?;
    ensure_coll(
        host,
        STEPS,
        &[idx("conversation", IndexType::String), idx("agreement", IndexType::String)],
    )
    .await?;
    Ok(())
}

pub(crate) async fn put_row<H: AppHost, T: Serialize>(
    host: &H,
    collection: &str,
    id: &str,
    val: &T,
) -> Result<(), String> {
    let payload = serde_json::to_vec(val).map_err(|e| e.to_string())?;
    AppDataLayer::put(
        host,
        collection.to_string(),
        RecordWriteValue { id: id.to_string(), payload },
    )
    .await
    .map_err(|e| e.to_string())
}

pub(crate) async fn put_card<H: AppHost>(host: &H, card: &CardRow) -> Result<(), String> {
    put_row(host, CARDS, &card.message_id, card).await?;
    // The row is stored either way; a missed event only delays a client's refresh.
    let _ = host
        .record_event(
            EVENT_CARD.to_string(),
            card.conversation.clone(),
            Some(card.message_id.clone()),
        )
        .await;
    Ok(())
}

pub(crate) async fn get_row<T: for<'a> Deserialize<'a>, H: AppHost>(
    host: &H,
    collection: &str,
    id: &str,
) -> Result<Option<T>, String> {
    let row = AppDataLayer::get(host, collection.to_string(), id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

pub(crate) async fn put_bytes<H: AppHost>(
    host: &H,
    collection: &str,
    id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    AppDataLayer::put(
        host,
        collection.to_string(),
        RecordWriteValue { id: id.to_string(), payload: bytes.to_vec() },
    )
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
pub(crate) async fn delete_row<H: AppHost>(
    host: &H,
    collection: &str,
    id: &str,
) -> Result<(), String> {
    AppDataLayer::delete(host, collection.to_string(), id.to_string())
        .await
        .map_err(|e| e.to_string())
}

pub(crate) async fn get_bytes<H: AppHost>(
    host: &H,
    collection: &str,
    id: &str,
) -> Result<Option<Vec<u8>>, String> {
    let row = AppDataLayer::get(host, collection.to_string(), id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.map(|r| r.payload))
}

/// How many of `owner`'s own records `collection` holds in `conversation`.
pub(crate) async fn count_mine<H: AppHost>(
    host: &H,
    collection: &str,
    conversation: &str,
    owner: &str,
) -> Result<u32, String> {
    let filter = json!({
        "conversation": conversation,
        "issuer": owner,
    })
    .to_string();
    Ok(paging::count(host, collection, Some(filter)).await? as u32)
}

/// An outgoing card to be sent over conversation and filed locally.
#[derive(Debug, Clone)]
pub(crate) struct OutgoingCard<'a> {
    pub(crate) card_type: &'a str,
    pub(crate) version: u32,
    pub(crate) envelope: &'a str,
    pub(crate) send: &'a [String],
    pub(crate) view: &'a [String],
}

pub(crate) async fn file_own_card<H: AppHost>(
    host: &H,
    message_id: &str,
    conversation: &str,
    card: &OutgoingCard<'_>,
    sender_timestamp_ms: i64,
    now: u64,
    version_count: Option<u64>,
) -> Result<(), String> {
    let envelope = Envelope::from_json(card.envelope).map_err(|e| e.to_string())?;
    let record_id = envelope.record_id().map_err(|e| e.to_string())?;
    let plain_data = disclosures::plain_view(&envelope.payload, card.view);
    let card_row = CardRow {
        message_id: message_id.to_string(),
        conversation: conversation.to_string(),
        direction: Direction::Outgoing,
        sender_timestamp_ms,
        card_type: card.card_type.to_string(),
        version: card.version,
        known: true,
        verified: true,
        expired: false,
        repeat: false,
        reason: None,
        issuer: Some(envelope.issuer),
        record_id: Some(record_id),
        revocation_status: Some("not-revoked".to_string()),
        data: Some(plain_data),
        stored_at_secs: now,
        declined: None,
        deal_state: None,
        agreement_payee: None,
        agreement_payment_methods: Vec::new(),
        version_count,
    };
    put_card(host, &card_row).await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SendCardError {
    Transport(String),
    Refused(String),
}

impl SendCardError {
    pub(crate) fn is_transport(&self) -> bool {
        matches!(self, Self::Transport(_))
    }
}

impl std::fmt::Display for SendCardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(s) | Self::Refused(s) => write!(f, "{s}"),
        }
    }
}

/// Sends a record as a card on `conversation`, then files this node's own
/// copy of that card under the message id the send returned. A send or
/// file-own-card failure is reported back as `send_error` rather than
/// aborting: the signed record is already durably stored by the caller, so
/// the record exists here either way and only the notification/own-card
/// bookkeeping is at risk.
pub(crate) async fn send_card_and_file<H: AppHost>(
    host: &H,
    conversation: &str,
    card: &OutgoingCard<'_>,
    now: u64,
    version_count: Option<u64>,
) -> (String, SendState, Option<SendCardError>) {
    let body = match card::card_body(card.card_type, card.version, card.envelope, card.send) {
        Ok(b) => b,
        Err(e) => {
            return (
                String::new(),
                SendState::NotSent,
                Some(SendCardError::Refused(e.to_string())),
            );
        }
    };

    let send_resp = call_sibling::<_, SendInput, SendOutput>(
        host,
        services::CONVERSATION,
        "conversation.send",
        &SendInput {
            conversation: conversation.to_string(),
            body,
            content_type: Some(CARD_CONTENT_TYPE.to_string()),
            languages: Vec::new(),
        },
    )
    .await;

    let (message_id, state, sender_timestamp_ms, mut send_error) = match send_resp {
        Ok(out) => (out.message_id, SendState::from(out.state), out.sender_timestamp_ms, None),
        Err(SiblingError::Refused(err)) => (
            String::new(),
            SendState::NotSent,
            now as i64 * 1000,
            Some(SendCardError::Refused(err.message)),
        ),
        Err(SiblingError::Transport(e) | SiblingError::BadReply(e)) => (
            String::new(),
            SendState::NotSent,
            now as i64 * 1000,
            Some(SendCardError::Transport(e)),
        ),
    };

    if !message_id.is_empty()
        && let Err(e) = file_own_card(
            host,
            &message_id,
            conversation,
            card,
            sender_timestamp_ms,
            now,
            version_count,
        )
        .await
    {
        send_error = send_error
            .or_else(|| Some(SendCardError::Transport(format!("file own card failed: {e}"))));
    }

    (message_id, state, send_error)
}

/// Builds a `{"conversation": ..., "mine": ...}`-shaped filter from whichever
/// of the two a list endpoint's params supplied, or no filter at all.
pub(crate) fn conversation_mine_filter(
    conversation: Option<&str>,
    mine: Option<bool>,
) -> Option<String> {
    let mut filter_obj = Map::new();
    if let Some(c) = conversation {
        filter_obj.insert("conversation".to_string(), json!(c));
    }
    if let Some(m) = mine {
        filter_obj.insert("mine".to_string(), json!(m));
    }
    if filter_obj.is_empty() { None } else { Some(Value::Object(filter_obj).to_string()) }
}

/// Checks whether a deal (quote id) has ended by any of:
/// - clock expiry
/// - decline (pointer declined_at_secs or decline:<deal> in LEDGER)
/// - decision row in state "not-formed"
/// - terminal agreement state (closed, cancelled, not-formed)
/// - refusal with quote-expired
pub(crate) fn is_deal_level_not_formed_reason(reason: Option<&str>) -> bool {
    !matches!(reason, Some("quote-superseded" | "quote-already-agreed"))
}

pub(crate) async fn is_deal_ended<H: AppHost>(
    host: &H,
    quote_id: &str,
    quote_expires_at_secs: Option<u64>,
    now: u64,
) -> Result<bool, String> {
    if let Some(exp) = quote_expires_at_secs
        && now >= exp
    {
        return Ok(true);
    }
    if let Some(ptr) = get_row::<RecordPointerRow, _>(host, QUOTES, quote_id).await? {
        if ptr.declined_at_secs.is_some() {
            return Ok(true);
        }
        if let Ok(env) = Envelope::from_json(&ptr.envelope)
            && let Some(exp) = env
                .payload
                .get("terms")
                .and_then(|t| t.get("quote_expires_at_secs"))
                .and_then(Value::as_u64)
            && now >= exp
        {
            return Ok(true);
        }
    }
    let dec_key = format!("decision:{quote_id}");
    if let Some(dec) = get_row::<decide::DecisionRow, _>(host, LEDGER, &dec_key).await?
        && dec.state == decide::DecisionState::NotFormed
    {
        return Ok(true);
    }
    let dec_id = decline_ops::decline_id(quote_id);
    if get_row::<decline_ops::DeclineRow, _>(host, LEDGER, &dec_id).await?.is_some() {
        return Ok(true);
    }
    let filter = serde_json::json!({ "quote_id": quote_id }).to_string();
    let agr_rows: Vec<AgreementRow> = paging::query_all(host, AGREEMENTS, Some(filter)).await?;
    for agr in agr_rows {
        if let Some(state) = agr.state
            && state.is_terminal()
            && is_deal_level_not_formed_reason(agr.state_reason.as_deref())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

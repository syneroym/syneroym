//! Transaction service application logic, target-independent.

pub mod agreement_ops;
pub mod backup;
pub mod booking_ops;
pub mod fulfilment_ops;
pub mod ledger;
pub mod payment_ops;
pub mod progress;
pub mod quote_ops;
pub mod request_ops;
pub mod sync;
pub mod thread;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::{
        data_layer::{CollectionSchema, IndexDefinition, IndexType, RecordWriteValue},
        proxy::CallTarget,
        signing::Principal,
    },
};
use syneroym_roym_core::{
    admit,
    booking::{BookingProgressPayload, BookingState},
    card::{self, CARD_CONTENT_TYPE},
    conversation::{Direction, StoredState},
    envelope::{Request, Response},
    method::{
        SiblingError, call_sibling,
        conversation::{SendInput, SendOutput},
    },
    paging,
    record::Envelope,
    services,
    signing::{self, CertificateError},
    transaction::{AgreedTerms, DealRole, ReceiptHalf},
};

use crate::api;

// Schema version 4 names the deal roles by action (counterparty, quote
// author). Version 3 added the booking vertical, payments, fulfilments and
// the ledger.
pub const SCHEMA_VERSION: u32 = 4;

pub(crate) const REQUESTS: &str = "requests";
pub(crate) const REQUEST_HISTORY: &str = "request_history";
pub(crate) const QUOTES: &str = "quotes";
pub(crate) const QUOTE_HISTORY: &str = "quote_history";
pub(crate) const AGREEMENTS: &str = "agreements";
pub(crate) const CARDS: &str = "cards";
pub(crate) const SYNC_STATE: &str = "sync_state";
pub(crate) const LEDGER: &str = "ledger";
pub(crate) const BOOKINGS: &str = "bookings";
pub(crate) const PROGRESS: &str = "progress";
pub(crate) const PAYMENTS: &str = "payments";
pub(crate) const FULFILMENTS: &str = "fulfilments";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BookingWriter {
    #[serde(rename = "self")]
    Mine,
    Peer,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementRef {
    pub agreement: String,
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
    pub(crate) conversation: String,
    pub(crate) counterparty_did: String,
    pub(crate) quote_author_did: String,
    pub(crate) terms: AgreedTerms,
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
    pub agreement_payee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agreement_payment_methods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SyncStateRow {
    pub(crate) last_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BookingRow {
    pub(crate) agreement: String,
    pub(crate) conversation: String,
    pub(crate) state: BookingState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) slot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) seat: Option<u32>,
    pub(crate) snapshot: BookingProgressPayload,
    pub(crate) progress_record_id: String,
    pub(crate) updated_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProgressRow {
    pub(crate) agreement: String,
    pub(crate) conversation: String,
    pub(crate) seq: u32,
    pub(crate) snapshot: BookingProgressPayload,
    pub(crate) envelope: String,
    pub(crate) record_id: String,
    pub(crate) writer: String,
    pub(crate) received_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct PaymentsRow {
    pub(crate) agreement: String,
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

    match req.method.as_str() {
        "booking.get" => booking_ops::booking_get(host, &req).await,
        "booking.list" => booking_ops::booking_list(host, &req).await,
        "booking.start" => booking_ops::booking_start(host, &req).await,
        "booking.cancel" => booking_ops::booking_cancel(host, &req).await,
        "booking.history" => booking_ops::booking_history(host, &req).await,

        "payment.request" => payment_ops::payment_request(host, &req).await,
        "payment.acknowledge" => payment_ops::payment_acknowledge(host, &req).await,
        "payment.get" => payment_ops::payment_get(host, &req).await,

        "fulfilment.sign" => fulfilment_ops::fulfilment_sign(host, &req).await,
        "fulfilment.get" => fulfilment_ops::fulfilment_get(host, &req).await,
        other => Response::method_not_found(other),
    }
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
        CollectionSchema { name: name.to_string(), indexes: indexes.to_vec() },
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
        REQUESTS,
        &[idx("conversation", IndexType::String), idx("updated_at_secs", IndexType::Numeric)],
    )
    .await?;
    ensure_coll(host, REQUEST_HISTORY, &[]).await?;
    ensure_coll(
        host,
        QUOTES,
        &[idx("conversation", IndexType::String), idx("updated_at_secs", IndexType::Numeric)],
    )
    .await?;
    ensure_coll(host, QUOTE_HISTORY, &[]).await?;
    ensure_coll(host, AGREEMENTS, &[idx("conversation", IndexType::String)]).await?;
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
        &[idx("kind", IndexType::String), idx("agreement", IndexType::String)],
    )
    .await?;
    ensure_coll(
        host,
        BOOKINGS,
        &[
            idx("conversation", IndexType::String),
            idx("state", IndexType::String),
            idx("slot_id", IndexType::String),
        ],
    )
    .await?;
    ensure_coll(host, PROGRESS, &[idx("conversation", IndexType::String)]).await?;
    ensure_coll(host, PAYMENTS, &[idx("conversation", IndexType::String)]).await?;
    ensure_coll(host, FULFILMENTS, &[idx("conversation", IndexType::String)]).await?;
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

pub(crate) async fn sibling_call<H: AppHost>(
    host: &H,
    service: &str,
    interface: &str,
    method: &str,
    params: Value,
) -> Result<Response, String> {
    let req = json!({ "method": method, "params": params }).to_string();
    let raw = host
        .call(
            CallTarget::Dependency(service.to_string()),
            interface.to_string(),
            "invoke".to_string(),
            json!([req]).to_string(),
            None,
        )
        .await
        .map_err(|e| format!("{method}: {e:?}"))?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

pub(crate) async fn catalog_call<H: AppHost>(
    host: &H,
    method: &str,
    params: Value,
) -> Result<Response, String> {
    sibling_call(host, services::CATALOG.name, services::CATALOG.interface, method, params).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn file_own_card<H: AppHost>(
    host: &H,
    message_id: &str,
    conversation: &str,
    card_type: &str,
    version: u32,
    envelope_json: &str,
    sender_timestamp_ms: i64,
    now: u64,
    version_count: Option<u64>,
) -> Result<(), String> {
    let envelope = Envelope::from_json(envelope_json).map_err(|e| e.to_string())?;
    let record_id = envelope.record_id().map_err(|e| e.to_string())?;
    let card_row = CardRow {
        message_id: message_id.to_string(),
        conversation: conversation.to_string(),
        direction: Direction::Outgoing,
        sender_timestamp_ms,
        card_type: card_type.to_string(),
        version,
        known: true,
        verified: true,
        expired: false,
        reason: None,
        issuer: Some(envelope.issuer),
        record_id: Some(record_id),
        revocation_status: Some("not-revoked".to_string()),
        data: Some(envelope.payload),
        stored_at_secs: now,
        declined: None,
        agreement_payee: None,
        agreement_payment_methods: Vec::new(),
        version_count,
    };
    put_row(host, CARDS, message_id, &card_row).await
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
    card_type: &str,
    version: u32,
    envelope_json: &str,
    now: u64,
    version_count: Option<u64>,
) -> (String, SendState, Option<String>) {
    let body = match card::card_body(card_type, version, envelope_json) {
        Ok(b) => b,
        Err(e) => return (String::new(), SendState::NotSent, Some(e.to_string())),
    };

    let send_resp = call_sibling::<_, SendInput, SendOutput>(
        host,
        services::CONVERSATION,
        "conversation.send",
        &SendInput {
            conversation: conversation.to_string(),
            body,
            content_type: Some(CARD_CONTENT_TYPE.to_string()),
        },
    )
    .await;

    let (message_id, state, sender_timestamp_ms, mut send_error) = match send_resp {
        Ok(out) => (out.message_id, SendState::from(out.state), out.sender_timestamp_ms, None),
        Err(SiblingError::Refused(err)) => {
            (String::new(), SendState::NotSent, now as i64 * 1000, Some(err.message))
        }
        Err(SiblingError::Transport(e) | SiblingError::BadReply(e)) => {
            (String::new(), SendState::NotSent, now as i64 * 1000, Some(e))
        }
    };

    if !message_id.is_empty()
        && let Err(e) = file_own_card(
            host,
            &message_id,
            conversation,
            card_type,
            version,
            envelope_json,
            sender_timestamp_ms,
            now,
            version_count,
        )
        .await
    {
        send_error = send_error.or(Some(format!("file own card failed: {e}")));
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

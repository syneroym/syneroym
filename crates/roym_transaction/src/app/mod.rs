//! Transaction service application logic, target-independent.

pub mod agreement_ops;
pub mod backup;
pub mod quote_ops;
pub mod request_ops;
pub mod sync;
pub mod thread;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::{
        data_layer::{
            CollectionSchema, IndexDefinition, IndexType, QueryOptions, RecordWriteValue,
        },
        proxy::CallTarget,
        signing::Principal,
    },
};
use syneroym_roym_core::{
    admit,
    conversation::Direction,
    envelope::{Request, Response},
    record::Envelope,
    services,
    signing::{self, CertificateError},
    transaction::{AgreedTerms, ReceiptHalf, Role},
};

use self::agreement_ops::RecordKind;

// Schema version 2: the service gains its first state.
pub const SCHEMA_VERSION: u32 = 2;

pub(crate) const REQUESTS: &str = "requests";
pub(crate) const REQUEST_HISTORY: &str = "request_history";
pub(crate) const QUOTES: &str = "quotes";
pub(crate) const QUOTE_HISTORY: &str = "quote_history";
pub(crate) const AGREEMENTS: &str = "agreements";
pub(crate) const CARDS: &str = "cards";
pub(crate) const SYNC_STATE: &str = "sync_state";

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
    pub(crate) consumer_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) declined_at_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) decline_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AgreementRow {
    pub(crate) quote_record_id: String,
    pub(crate) conversation: String,
    pub(crate) consumer_did: String,
    pub(crate) provider_did: String,
    pub(crate) terms: AgreedTerms,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) consumer: Option<ReceiptHalf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) provider: Option<ReceiptHalf>,
    pub(crate) updated_at_secs: u64,
}

impl AgreementRow {
    pub(crate) fn half(&self, role: Role) -> Option<&ReceiptHalf> {
        match role {
            Role::Consumer => self.consumer.as_ref(),
            Role::Provider => self.provider.as_ref(),
        }
    }

    pub(crate) fn set_half(&mut self, role: Role, half: ReceiptHalf) {
        match role {
            Role::Consumer => self.consumer = Some(half),
            Role::Provider => self.provider = Some(half),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CardRow {
    pub(crate) message_id: String,
    pub(crate) conversation: String,
    pub(crate) direction: Direction,
    pub(crate) sender_timestamp_ms: i64,
    pub(crate) card_type: String,
    pub(crate) version: u32,
    pub(crate) known: bool,
    pub(crate) verified: bool,
    #[serde(default)]
    pub(crate) expired: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) record_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) revocation_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<Value>,
    pub(crate) stored_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) declined: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) version_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SyncStateRow {
    pub(crate) scanned_count: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListParams {
    #[serde(default)]
    pub(crate) conversation: Option<String>,
    #[serde(default)]
    pub(crate) mine: Option<bool>,
    #[serde(default = "default_list_limit")]
    pub(crate) limit: usize,
    #[serde(default)]
    pub(crate) offset: usize,
}

pub(crate) fn default_list_limit() -> usize {
    200
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
    if let Some(resp) = signing::handle_certificate_verb(host, "transaction.", &req).await {
        return resp;
    }

    match req.method.as_str() {
        "request.ping" | "quote.ping" | "agreement.ping" | "receipt.ping" => {
            Response::ok(json!({ "service": services::TRANSACTION.name }))
        }
        "request.set" => request_ops::request_set(host, &req).await,
        "request.get" => request_ops::request_get(host, &req).await,
        "request.list" => request_ops::request_list(host, &req).await,
        "request.history" => request_ops::request_history(host, &req).await,
        "request.verify" => agreement_ops::verify_verb(host, &req, RecordKind::Request).await,

        "quote.set" => quote_ops::quote_set(host, &req).await,
        "quote.get" => quote_ops::quote_get(host, &req).await,
        "quote.list" => quote_ops::quote_list(host, &req).await,
        "quote.history" => quote_ops::quote_history(host, &req).await,
        "quote.verify" => agreement_ops::verify_verb(host, &req, RecordKind::Quote).await,
        "quote.decline" => quote_ops::quote_decline(host, &req).await,

        "agreement.accept" => agreement_ops::agreement_accept(host, &req).await,
        "agreement.get" => agreement_ops::agreement_get(host, &req).await,
        "agreement.list" => agreement_ops::agreement_list(host, &req).await,
        "agreement.verify" => {
            agreement_ops::verify_verb(host, &req, RecordKind::AgreementReceipt).await
        }

        "transaction.sync" => sync::sync(host, &req).await,
        "transaction.thread" => thread::thread(host, &req).await,
        "transaction.export" => backup::export(host).await,
        "transaction.import" => backup::import(host, &req).await,
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

pub(crate) async fn collect<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            collection.to_string(),
            QueryOptions { filter: None, limit: Some(500), cursor: cursor.clone() },
        )
        .await
        .map_err(|e| e.to_string())?;
        for r in page.records {
            if let Ok(parsed) = serde_json::from_slice::<Value>(&r.payload) {
                out.push(json!({ "id": r.id, "payload": parsed }));
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(out)
}

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
    let mut count = 0;
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            collection.to_string(),
            QueryOptions { filter: Some(filter.clone()), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        .map_err(|e| e.to_string())?;
        count += page.records.len() as u32;
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(count)
}

pub(crate) async fn count_cards_for_conversation<H: AppHost>(
    host: &H,
    conversation: &str,
) -> Result<usize, String> {
    let filter = json!({ "conversation": conversation }).to_string();
    let mut count = 0;
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            CARDS.to_string(),
            QueryOptions { filter: Some(filter.clone()), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        .map_err(|e| e.to_string())?;
        count += page.records.len();
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(count)
}

pub(crate) async fn conversation_call<H: AppHost>(
    host: &H,
    method: &str,
    params: Value,
) -> Result<Response, String> {
    let req = json!({ "method": method, "params": params }).to_string();
    let raw = host
        .call(
            CallTarget::Dependency(services::CONVERSATION.name.to_string()),
            services::CONVERSATION.interface.to_string(),
            "invoke".to_string(),
            json!([req]).to_string(),
            None,
        )
        .await
        .map_err(|e| format!("{method}: {e:?}"))?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
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
        version_count,
    };
    put_row(host, CARDS, message_id, &card_row).await
}

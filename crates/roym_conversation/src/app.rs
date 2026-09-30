//! Conversation service application logic, target-independent.
//!
//! Two things live here: Roym's own copy of every message it sends and
//! receives (what export, search and delete act on), and the product's
//! inbox -- the enforcement point for the block list and the first-contact
//! rate limit.

pub mod backup;
pub mod group;
pub mod inbox;
pub mod messages;

pub use inbox::{on_delivery_state, on_message};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::{
        data_layer::{CollectionSchema, IndexDefinition, IndexType, RecordWriteValue},
        proxy::CallTarget,
    },
};
use syneroym_roym_core::{
    admit,
    conversation::{ConversationRow, MessageRow},
    envelope::{Request, Response},
    services, signing,
};

/// Bumped in this slice: group conversations added.
pub const SCHEMA_VERSION: u32 = 3;

pub const CONVERSATIONS: &str = "conversations";
pub const MESSAGES: &str = "messages";
pub const REFUSED_MESSAGES: &str = "refused_messages";

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::CONVERSATION.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

fn idx(field: &str, ty: IndexType) -> IndexDefinition {
    IndexDefinition { field_name: field.to_string(), type_: ty }
}

pub(crate) async fn ensure_coll<H: AppHost>(
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

pub(crate) async fn ensure_conversations<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(host, CONVERSATIONS, &[idx("last_activity_ms", IndexType::Numeric)]).await
}

pub(crate) async fn ensure_messages<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(
        host,
        MESSAGES,
        &[
            idx("conversation", IndexType::String),
            idx("sender_timestamp_ms", IndexType::Numeric),
            idx("state", IndexType::String),
        ],
    )
    .await
}

pub(crate) async fn ensure_refused<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(
        host,
        REFUSED_MESSAGES,
        &[idx("at_secs", IndexType::Numeric), idx("conversation", IndexType::String)],
    )
    .await
}

pub(crate) async fn put_conversation<H: AppHost>(
    host: &H,
    row: &ConversationRow,
) -> Result<(), String> {
    ensure_conversations(host).await?;
    let bytes = serde_json::to_vec(row).map_err(|e| e.to_string())?;
    AppDataLayer::put(
        host,
        CONVERSATIONS.to_string(),
        RecordWriteValue { id: row.id.clone(), payload: bytes },
    )
    .await
    .map_err(|e| e.to_string())
}

pub(crate) async fn create_conversation<H: AppHost>(
    host: &H,
    row: &ConversationRow,
) -> Result<bool, String> {
    ensure_conversations(host).await?;
    let bytes = serde_json::to_vec(row).map_err(|e| e.to_string())?;
    let val = RecordWriteValue { id: row.id.clone(), payload: bytes };
    let res = AppDataLayer::create(host, CONVERSATIONS.to_string(), vec![val])
        .await
        .map_err(|e| e.to_string())?;
    Ok(res.is_none())
}

pub(crate) async fn put_message<H: AppHost>(host: &H, row: &MessageRow) -> Result<(), String> {
    ensure_messages(host).await?;
    let bytes = serde_json::to_vec(row).map_err(|e| e.to_string())?;
    AppDataLayer::put(
        host,
        MESSAGES.to_string(),
        RecordWriteValue { id: row.id.clone(), payload: bytes },
    )
    .await
    .map_err(|e| e.to_string())
}

pub(crate) async fn load_message<H: AppHost>(
    host: &H,
    id: &str,
) -> Result<Option<MessageRow>, String> {
    ensure_messages(host).await?;
    let row = AppDataLayer::get(host, MESSAGES.to_string(), id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

pub(crate) async fn load_conversation<H: AppHost>(
    host: &H,
    id: &str,
) -> Result<Option<ConversationRow>, String> {
    ensure_conversations(host).await?;
    let row = AppDataLayer::get(host, CONVERSATIONS.to_string(), id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// One `invoke` call into the sibling `profile` service.
pub(crate) async fn profile_call<H: AppHost>(
    host: &H,
    method: &str,
    params: Value,
) -> Result<Response, String> {
    let req = json!({ "method": method, "params": params }).to_string();
    let raw = host
        .call(
            CallTarget::Dependency(services::PROFILE.name.to_string()),
            services::PROFILE.interface.to_string(),
            "invoke".to_string(),
            json!([req]).to_string(),
            None,
        )
        .await
        .map_err(|e| format!("{method}: {e:?}"))?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

/// Map of conversation_address -> person_did from contacts.list.
pub(crate) async fn contacts_map<H: AppHost>(
    host: &H,
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Ok(resp) = profile_call(host, "contacts.list", json!({})).await
        && let Some(rows) = resp.result.and_then(|v| v.as_array().cloned())
    {
        for row in rows {
            if let (Some(addr), Some(did)) = (
                row.get("conversation_address").and_then(Value::as_str),
                row.get("person_did").and_then(Value::as_str),
            ) {
                map.insert(addr.to_string(), did.to_string());
            }
        }
    }
    map
}

/// This peer's person DID as far as this product can say, from its own
/// contacts. `None` when no contact carries the address.
pub(crate) async fn person_did_for_address<H: AppHost>(host: &H, address: &str) -> Option<String> {
    contacts_map(host).await.remove(address)
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if let Some(resp) = admit::require_internal(host).await {
        return resp;
    }
    if let Some(resp) = signing::handle_certificate_verb(host, "conversation.", &req).await {
        return resp;
    }

    match req.method.as_str() {
        "conversation.ping" => Response::ok(json!({ "service": services::CONVERSATION.name })),
        "conversation.open" => messages::open(host, &req).await,
        "conversation.list" => messages::list(host, &req).await,
        "conversation.send" => messages::send(host, &req).await,
        "conversation.history" => messages::history(host, &req).await,
        "conversation.delivery-status" => messages::delivery_status(host, &req).await,
        "conversation.outbox" => messages::outbox(host).await,
        "conversation.retry" => messages::retry(host, &req).await,
        "conversation.delete-message" => messages::delete_message(host, &req).await,
        "conversation.search" => messages::search(host, &req).await,
        "conversation.transcript-digest" => messages::transcript_digest(host, &req).await,
        "conversation.export" => backup::export(host).await,
        "conversation.import" => backup::import(host, &req).await,
        "group.create" => group::create(host, &req).await,
        "group.rename" => group::rename(host, &req).await,
        "group.add-member" => group::add_member(host, &req).await,
        "group.remove-member" => group::remove_member(host, &req).await,
        "group.info" => group::info(host, &req).await,
        "group.sync" => group::sync(host, &req).await,
        "group.hide" => group::hide(host, &req).await,
        "group.unhide" => group::unhide(host, &req).await,
        other => Response::method_not_found(other),
    }
}

//! Conversation service application logic, target-independent.

pub mod backup;
pub mod group;
pub mod inbox;
pub mod messages;

use std::collections::HashMap;

pub use inbox::on_message;
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::{
        conversation::ConversationError,
        data_layer::{CollectionSchema, IndexDefinition, IndexType, RecordWriteValue},
        proxy::CallTarget,
    },
};
use syneroym_roym_core::{
    admit,
    envelope::{Request, Response},
    services, signing,
};

pub const SCHEMA_VERSION: u32 = 4;
pub const ADMISSIONS: &str = "admissions";
pub const FIRST_CONTACT_CHARGES: &str = "first_contact_charges";

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

pub(crate) async fn ensure_admissions<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(host, ADMISSIONS, &[]).await
}

pub(crate) async fn ensure_charges<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(host, FIRST_CONTACT_CHARGES, &[idx("at_secs", IndexType::Numeric)]).await
}

pub(crate) async fn load_admission_info<H: AppHost>(
    host: &H,
    conversation_id: &str,
) -> Result<(Option<String>, Option<String>), String> {
    ensure_admissions(host).await?;
    let row = AppDataLayer::get(host, ADMISSIONS.to_string(), conversation_id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => {
            let v: Value = serde_json::from_slice(&r.payload).map_err(|e| e.to_string())?;
            let state = v.get("state").and_then(Value::as_str).map(str::to_string);
            let peer = v.get("peer_address").and_then(Value::as_str).map(str::to_string);
            Ok((state, peer))
        }
        None => Ok((None, None)),
    }
}

pub(crate) async fn load_admission<H: AppHost>(
    host: &H,
    conversation_id: &str,
) -> Result<Option<String>, String> {
    let (state, _) = load_admission_info(host, conversation_id).await?;
    Ok(state)
}

pub(crate) async fn set_admission<H: AppHost>(
    host: &H,
    conversation_id: &str,
    state: &str,
) -> Result<(), String> {
    set_admission_peer(host, conversation_id, state, None).await
}

pub(crate) async fn set_admission_peer<H: AppHost>(
    host: &H,
    conversation_id: &str,
    state: &str,
    peer_address: Option<&str>,
) -> Result<(), String> {
    ensure_admissions(host).await?;
    let mut obj = json!({ "state": state });
    if let Some(peer) = peer_address {
        obj["peer_address"] = json!(peer);
    }
    let payload = serde_json::to_vec(&obj).map_err(|e| e.to_string())?;
    AppDataLayer::put(
        host,
        ADMISSIONS.to_string(),
        RecordWriteValue { id: conversation_id.to_string(), payload },
    )
    .await
    .map_err(|e| e.to_string())
}

pub(crate) fn from_host(err: ConversationError) -> Response {
    match err {
        ConversationError::NotFound => Response::invalid_params("not found"),
        ConversationError::InvalidArgument(m) => Response::invalid_params(m),
        ConversationError::PermissionDenied => Response::err(-32003, "permission denied"),
        ConversationError::Unreachable(m) => Response::internal_error(m),
        ConversationError::QuotaExceeded => Response::err(-32004, "quota exceeded"),
        ConversationError::Internal(m) => Response::internal_error(m),
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
pub(crate) async fn contacts_map<H: AppHost>(host: &H) -> HashMap<String, String> {
    let mut map = HashMap::new();
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
        "conversation.changes" => messages::changes(host, &req).await,
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

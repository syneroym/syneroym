//! Conversation service application logic, target-independent.

pub mod backup;
pub mod group;
pub mod inbox;
pub mod messages;

use std::collections::HashMap;

pub use inbox::on_message;
use serde_json::json;
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::{
        conversation::ConversationError,
        data_layer::{CollectionSchema, IndexDefinition, IndexType, RecordWriteValue},
    },
};
use syneroym_roym_core::{
    admit,
    envelope::{Request, Response},
    method::{
        call_sibling,
        profile::{ContactRow, ContactsListInput},
    },
    services,
};

use crate::api;

pub const SCHEMA_VERSION: u32 = 4;
pub const ADMISSIONS: &str = "admissions";
pub const FIRST_CONTACT_CHARGES: &str = "first_contact_charges";
/// Small bookkeeping rows that belong to no conversation, such as when the
/// old first-contact charges were last pruned.
pub const CONVERSATION_META: &str = "conversation_meta";

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

pub(crate) async fn ensure_meta<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(host, CONVERSATION_META, &[]).await
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
            let v: serde_json::Value =
                serde_json::from_slice(&r.payload).map_err(|e| e.to_string())?;
            let state = v.get("state").and_then(serde_json::Value::as_str).map(str::to_string);
            let peer =
                v.get("peer_address").and_then(serde_json::Value::as_str).map(str::to_string);
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

/// Map of conversation_address -> person_did from contacts.list.
pub(crate) async fn contacts_map<H: AppHost>(host: &H) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let rows_res = call_sibling::<_, ContactsListInput, Vec<ContactRow>>(
        host,
        services::PROFILE,
        "contacts.list",
        &ContactsListInput::default(),
    )
    .await;
    if let Ok(rows) = rows_res {
        for row in rows {
            map.insert(row.conversation_address, row.person_did);
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

    api::dispatch(host, &req).await.unwrap_or_else(|| Response::method_not_found(&req.method))
}

//! Group conversation operations: create, rename, membership, info, sync, and
//! visibility.

use serde_json::{Value, json};
use syneroym_app_host::{AppConversation, AppHost, types::conversation::ConversationError};
use syneroym_roym_core::{
    conversation::group::{
        GROUP_ADD_UNREACHABLE_MESSAGE, GROUP_DELIVERY_NOTICE, GROUP_JOIN_BOUNDARY_NOTICE,
        GROUP_KEY_TRUST_NOTICE, GROUP_REMOVED_NOTICE, GROUP_RESTORED_NOTICE, OWNER_CAN_READ_NOTICE,
        validate_group_name,
    },
    envelope::{Request, Response},
};

use super::{
    contacts_map, from_host, load_admission, messages::resolve_open_address, set_admission,
};

fn extract_conversation_param(req: &Request) -> Option<String> {
    req.params
        .get("conversation")
        .or_else(|| req.params.get("group"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(crate) async fn create<H: AppHost>(host: &H, req: &Request) -> Response {
    let name = match req.params.get("name").and_then(Value::as_str) {
        Some(raw) => match validate_group_name(raw) {
            Ok(n) => Some(n),
            Err(e) => return Response::invalid_params(e.to_string()),
        },
        None => None,
    };
    let group_id = match AppConversation::create_group(host).await {
        Ok(id) => id,
        Err(e) => return from_host(e),
    };
    if let Some(n) = name
        && let Err(e) = AppConversation::set_group_name(host, group_id.clone(), n).await
    {
        return from_host(e);
    }
    if let Err(e) = set_admission(host, &group_id, "shown").await {
        return Response::internal_error(e);
    }
    let info = match AppConversation::group_info(host, group_id.clone()).await {
        Ok(i) => i,
        Err(e) => return from_host(e),
    };
    Response::ok(json!({
        "conversation_id": group_id,
        "owner_address": info.owner,
    }))
}

pub(crate) async fn rename<H: AppHost>(host: &H, req: &Request) -> Response {
    let group = match extract_conversation_param(req) {
        Some(g) => g,
        None => return Response::invalid_params("conversation is required"),
    };
    let name = match req.params.get("name").and_then(Value::as_str) {
        Some(n) => n,
        None => return Response::invalid_params("name is required"),
    };
    let valid_name = match validate_group_name(name) {
        Ok(n) => n,
        Err(e) => return Response::invalid_params(e.to_string()),
    };
    match AppConversation::set_group_name(host, group, valid_name.clone()).await {
        Ok(()) => Response::ok(json!({ "renamed": true, "name": valid_name })),
        Err(e) => from_host(e),
    }
}

pub(crate) async fn add_member<H: AppHost>(host: &H, req: &Request) -> Response {
    let group = match extract_conversation_param(req) {
        Some(g) => g,
        None => return Response::invalid_params("conversation is required"),
    };
    let address = match resolve_open_address(host, req).await {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    if let Err(e) = AppConversation::add_member(host, group.clone(), address.clone()).await {
        return match e {
            ConversationError::Unreachable(_) => {
                Response::invalid_params(GROUP_ADD_UNREACHABLE_MESSAGE)
            }
            ConversationError::QuotaExceeded => Response::invalid_params("this group is full"),
            err => from_host(err),
        };
    }

    let updated_info = match AppConversation::group_info(host, group.clone()).await {
        Ok(i) => i,
        Err(e) => return from_host(e),
    };

    Response::ok(json!({
        "added": address,
        "epoch": updated_info.epoch,
    }))
}

pub(crate) async fn remove_member<H: AppHost>(host: &H, req: &Request) -> Response {
    let group = match extract_conversation_param(req) {
        Some(g) => g,
        None => return Response::invalid_params("conversation is required"),
    };
    let address = match resolve_open_address(host, req).await {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    match AppConversation::remove_member(host, group, address).await {
        Ok(()) => Response::ok(json!({ "removed": true })),
        Err(e) => from_host(e),
    }
}

pub(crate) async fn info<H: AppHost>(host: &H, req: &Request) -> Response {
    let group = match extract_conversation_param(req) {
        Some(g) => g,
        None => return Response::invalid_params("conversation is required"),
    };
    let info = match AppConversation::group_info(host, group.clone()).await {
        Ok(i) => i,
        Err(e) => return from_host(e),
    };
    let admission = load_admission(host, &group)
        .await
        .unwrap_or(None)
        .unwrap_or_else(|| if info.restored { "hidden".to_string() } else { "shown".to_string() });

    let contacts = contacts_map(host).await;
    let members: Vec<Value> = info
        .members
        .iter()
        .map(|m| {
            json!({
                "address": m,
                "person_did": contacts.get(m),
                "is_owner": *m == info.owner,
            })
        })
        .collect();
    let can_read = info.key_epoch >= info.epoch && info.is_member && !info.restored;
    Response::ok(json!({
        "conversation_id": group,
        "name": info.name,
        "owner_address": info.owner,
        "owner_person_did": contacts.get(&info.owner),
        "is_owner": info.is_owner,
        "is_member": info.is_member,
        "restored_only": info.restored,
        "admission": { "state": admission },
        "epoch": info.epoch,
        "key_epoch": info.key_epoch,
        "key_stored_at_ms": info.key_stored_at,
        "members": members,
        "can_read_new_messages": can_read,
        "notices": {
            "owner_can_read": OWNER_CAN_READ_NOTICE,
            "key_trust": GROUP_KEY_TRUST_NOTICE,
            "delivery": GROUP_DELIVERY_NOTICE,
            "join_boundary": GROUP_JOIN_BOUNDARY_NOTICE,
            "removed": if !info.is_member { Some(GROUP_REMOVED_NOTICE) } else { None },
            "restored": if info.restored { Some(GROUP_RESTORED_NOTICE) } else { None },
        }
    }))
}

pub(crate) async fn sync<H: AppHost>(host: &H, req: &Request) -> Response {
    let group = match extract_conversation_param(req) {
        Some(g) => g,
        None => return Response::invalid_params("conversation is required"),
    };
    if let Err(e) = AppConversation::group_info(host, group.clone()).await {
        return from_host(e);
    }
    match AppConversation::sync_now(host, group).await {
        Ok(()) => Response::ok(json!({ "synced": true })),
        Err(e) => from_host(e),
    }
}

pub(crate) async fn hide<H: AppHost>(host: &H, req: &Request) -> Response {
    let group = match extract_conversation_param(req) {
        Some(g) => g,
        None => return Response::invalid_params("conversation is required"),
    };
    if let Err(e) = AppConversation::group_info(host, group.clone()).await {
        return from_host(e);
    }
    if let Err(e) = set_admission(host, &group, "hidden").await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "admission": { "state": "hidden" } }))
}

pub(crate) async fn unhide<H: AppHost>(host: &H, req: &Request) -> Response {
    let group = match extract_conversation_param(req) {
        Some(g) => g,
        None => return Response::invalid_params("conversation is required"),
    };
    if let Err(e) = AppConversation::group_info(host, group.clone()).await {
        return from_host(e);
    }
    if let Err(e) = set_admission(host, &group, "shown").await {
        return Response::internal_error(e);
    }
    let filled_in =
        match AppConversation::readmit(host, group, vec!["group-hidden".to_string()]).await {
            Ok(n) => n,
            Err(e) => return from_host(e),
        };
    Response::ok(json!({
        "admission": { "state": "shown" },
        "filled_in": filled_in,
    }))
}

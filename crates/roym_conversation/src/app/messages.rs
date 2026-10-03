//! Conversation messaging operations: open, send, history, changes, search, and
//! deletion.

use std::{cmp::Reverse, collections::HashMap};

use serde_json::{Value, json};
use syneroym_app_host::{
    AppConversation, AppHost,
    types::conversation::{ConversationError, ConversationKind, HistoryItem, Message},
};
use syneroym_roym_core::{
    conversation::{
        DELETION_REQUEST_CONTENT_TYPE, StoredState, encode_body,
        group::{
            CARDS_NOT_IN_GROUPS_MESSAGE, GROUP_PROFILE_CONTENT_TYPE, GROUP_REMOVED_NOTICE,
            GROUP_RESTORED_NOTICE, MEMBERSHIP_EVENT_CONTENT_TYPE, group_profile_body,
            is_group_system_type, membership_event_body,
        },
    },
    envelope::{Request, Response},
};

use super::{contacts_map, from_host, load_admission_info, profile_call, set_admission_peer};

pub(crate) async fn resolve_open_address<H: AppHost>(
    host: &H,
    req: &Request,
) -> Result<String, Response> {
    if let Some(addr) = req.params.get("address").and_then(Value::as_str) {
        return Ok(addr.to_string());
    }
    let Some(person_did) = req.params.get("person_did").and_then(Value::as_str) else {
        return Err(Response::invalid_params("person_did or address is required"));
    };
    let resp = profile_call(host, "contacts.resolve-address", json!({ "person_did": person_did }))
        .await
        .map_err(Response::internal_error)?;
    if let Some(err) = resp.error {
        return Err(Response::err(err.code, err.message));
    }
    resp.result
        .and_then(|v| v.get("conversation_address").and_then(Value::as_str).map(str::to_string))
        .ok_or_else(|| Response::internal_error("contacts.resolve-address returned no address"))
}

pub(crate) async fn open<H: AppHost>(host: &H, req: &Request) -> Response {
    let address = match resolve_open_address(host, req).await {
        Ok(a) => a,
        Err(resp) => return resp,
    };
    let conversation_id = match AppConversation::open_direct(host, address.clone()).await {
        Ok(id) => id,
        Err(e) => return from_host(e),
    };
    if let Err(e) = set_admission_peer(host, &conversation_id, "accepted", Some(&address)).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "conversation_id": conversation_id, "peer_address": address }))
}

pub(crate) async fn list<H: AppHost>(host: &H, req: &Request) -> Response {
    let offset = req.params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
    let kind_filter = req.params.get("kind").and_then(Value::as_str);
    let include_hidden = req.params.get("include_hidden").and_then(Value::as_bool).unwrap_or(false);

    let summaries = match AppConversation::conversations(host).await {
        Ok(s) => s,
        Err(e) => return from_host(e),
    };
    let contacts = contacts_map(host).await;
    let mut admissions: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for s in &summaries {
        if let Ok(info) = load_admission_info(host, &s.id).await {
            admissions.insert(s.id.clone(), info);
        }
    }
    let mut rows = Vec::new();

    for s in summaries {
        let is_group = s.kind == ConversationKind::Group;
        if let Some("direct") = kind_filter
            && is_group
        {
            continue;
        }
        if let Some("group") = kind_filter
            && !is_group
        {
            continue;
        }

        let admission_info = admissions.get(&s.id);
        let admission_str = admission_info
            .and_then(|(st, _)| st.as_deref())
            .unwrap_or(if is_group { "shown" } else { "accepted" });

        if is_group && !include_hidden && admission_str == "hidden" {
            continue;
        }

        let peer_addr =
            if is_group { String::new() } else { s.peer_address.clone().unwrap_or_default() };
        let person_did = contacts.get(&peer_addr).cloned();

        let row = json!({
            "id": s.id,
            "kind": if is_group { "group" } else { "direct" },
            "peer_address": peer_addr,
            "peer_person_did": person_did,
            "opened_at_secs": s.created_at,
            "last_activity_ms": s.last_activity_at,
            "message_count": s.message_count,
            "name": s.name,
            "restored": s.restored,
            "group": if is_group {
                Some(json!({
                    "name": s.name,
                    "admission": { "state": admission_str },
                }))
            } else {
                None
            },
        });
        rows.push((s.last_activity_at, row));
    }

    rows.sort_by_key(|(act, _)| Reverse(*act));
    let out: Vec<Value> = rows.into_iter().skip(offset).take(limit).map(|(_, r)| r).collect();
    Response::ok(json!({ "conversations": out }))
}

pub(crate) async fn send<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match req.params.get("conversation").and_then(Value::as_str) {
        Some(c) => c.to_string(),
        None => return Response::invalid_params("conversation is required"),
    };
    let body = match req.params.get("body").and_then(Value::as_str) {
        Some(b) => b.to_string(),
        None => return Response::invalid_params("body is required"),
    };
    let content_type =
        req.params.get("content_type").and_then(Value::as_str).unwrap_or("text/plain");

    let is_group = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(info) => {
            if info.restored {
                return Response::invalid_params(GROUP_RESTORED_NOTICE);
            }
            if !info.is_member {
                return Response::invalid_params(GROUP_REMOVED_NOTICE);
            }
            true
        }
        Err(ConversationError::InvalidArgument(_) | ConversationError::NotFound) => false,
        Err(e) => return from_host(e),
    };

    if content_type == MEMBERSHIP_EVENT_CONTENT_TYPE
        || content_type == DELETION_REQUEST_CONTENT_TYPE
        || content_type == GROUP_PROFILE_CONTENT_TYPE
        || (is_group && content_type == "application/vnd.roym.card+json")
    {
        return Response::invalid_params(if content_type == "application/vnd.roym.card+json" {
            CARDS_NOT_IN_GROUPS_MESSAGE
        } else {
            "this content type is reserved"
        });
    }

    let message_id = match AppConversation::send(
        host,
        conversation.clone(),
        content_type.to_string(),
        body.into_bytes(),
    )
    .await
    {
        Ok(id) => id,
        Err(e) => return from_host(e),
    };

    let host_msg = AppConversation::get_message(host, message_id.clone()).await.ok();
    Response::ok(json!({
        "message_id": message_id,
        "state": host_msg.as_ref().map(|m| StoredState::from(m.state)).unwrap_or(StoredState::Pending),
        "sender_timestamp_ms": host_msg.map(|m| m.sender_timestamp).unwrap_or(0),
    }))
}

pub(crate) fn message_to_json(m: &Message) -> Value {
    let (body_encoding, body_str) = encode_body(&m.content_type, &m.body);
    let mut val = json!({
        "id": m.id,
        "conversation": m.conversation,
        "author": m.author,
        "direction": if m.outgoing { "outgoing" } else { "incoming" },
        "sender_timestamp_ms": m.sender_timestamp,
        "content_type": m.content_type,
        "body_encoding": body_encoding,
        "state": StoredState::from(m.state),
        "last_error": m.last_error,
        "stored_at_secs": m.received_at,
        "refused": m.refused,
        "restored": m.restored,
        "visible_seq": m.visible_seq,
        "verified": m.verified,
    });
    if let Some(del) = m.deleted_at
        && let Some(obj) = val.as_object_mut()
    {
        obj.insert("deleted_at_secs".to_string(), json!(del));
    }
    if m.deleted_at.is_none()
        && let Some(obj) = val.as_object_mut()
    {
        obj.insert("body".to_string(), Value::String(body_str));
    }
    val
}

fn history_item_to_json(conversation: &str, item: HistoryItem) -> Value {
    match item {
        HistoryItem::Message(m) => message_to_json(&m),
        HistoryItem::Membership(ev) => {
            let body = membership_event_body(&ev.action, &ev.subject, ev.epoch);
            json!({
                "id": ev.entry,
                "conversation": conversation,
                "author": ev.subject,
                "direction": "incoming",
                "sender_timestamp_ms": ev.sender_timestamp,
                "content_type": MEMBERSHIP_EVENT_CONTENT_TYPE,
                "body_encoding": "utf8",
                "body": body,
                "state": "delivered",
                "last_error": Value::Null,
                "stored_at_secs": ev.sender_timestamp / 1000,
                "refused": Value::Null,
                "restored": false,
                "visible_seq": 0,
                "verified": true,
            })
        }
        HistoryItem::GroupName(ev) => {
            let body = String::from_utf8(group_profile_body(&ev.name))
                .unwrap_or_else(|_| format!("{{\"name\":\"{}\"}}", ev.name));
            json!({
                "id": ev.entry,
                "conversation": conversation,
                "author": String::new(),
                "direction": "incoming",
                "sender_timestamp_ms": ev.sender_timestamp,
                "content_type": GROUP_PROFILE_CONTENT_TYPE,
                "body_encoding": "utf8",
                "body": body,
                "state": "delivered",
                "last_error": Value::Null,
                "stored_at_secs": ev.sender_timestamp / 1000,
                "refused": Value::Null,
                "restored": false,
                "visible_seq": 0,
                "verified": true,
            })
        }
    }
}

pub(crate) async fn history<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match req.params.get("conversation").and_then(Value::as_str) {
        Some(c) => c.to_string(),
        None => return Response::invalid_params("conversation is required"),
    };
    let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(200) as u32;
    let cursor = req.params.get("cursor").and_then(|v| {
        if let Some(s) = v.as_str() {
            if s.is_empty() { None } else { Some(s.to_string()) }
        } else {
            None
        }
    });

    let is_group = match AppConversation::group_info(host, conversation.clone()).await {
        Ok(_) => true,
        Err(ConversationError::InvalidArgument(_) | ConversationError::NotFound) => false,
        Err(e) => return from_host(e),
    };

    let page = match AppConversation::history(host, conversation.clone(), limit, cursor).await {
        Ok(p) => p,
        Err(e) => return from_host(e),
    };

    let items: Vec<Value> =
        page.items.into_iter().map(|item| history_item_to_json(&conversation, item)).collect();

    Response::ok(json!({
        "messages": items,
        "next_cursor": page.next_cursor,
        "kind": if is_group { "group" } else { "direct" },
    }))
}

pub(crate) async fn changes<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match req.params.get("conversation").and_then(Value::as_str) {
        Some(c) => c.to_string(),
        None => return Response::invalid_params("conversation is required"),
    };
    let after_seq = req.params.get("after_seq").and_then(Value::as_u64).unwrap_or(0);
    let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(200) as u32;

    let is_group = AppConversation::group_info(host, conversation.clone()).await.is_ok();

    let page = match AppConversation::changes(host, conversation, after_seq, limit).await {
        Ok(p) => p,
        Err(e) => return from_host(e),
    };

    let msgs: Vec<Value> = page.messages.iter().map(message_to_json).collect();
    Response::ok(json!({
        "messages": msgs,
        "last_seq": page.last_seq,
        "kind": if is_group { "group" } else { "direct" },
    }))
}

pub(crate) async fn delivery_status<H: AppHost>(host: &H, req: &Request) -> Response {
    let message_id = match req.params.get("message_id").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => return Response::invalid_params("message_id is required"),
    };
    match AppConversation::delivery_status(host, message_id).await {
        Ok(s) => Response::ok(json!({ "state": StoredState::from(s) })),
        Err(e) => from_host(e),
    }
}

pub(crate) async fn outbox<H: AppHost>(host: &H) -> Response {
    match AppConversation::outbox(host).await {
        Ok(msgs) => {
            let out: Vec<Value> = msgs
                .into_iter()
                .map(|m| {
                    json!({
                        "id": m.id,
                        "conversation": m.conversation,
                        "state": StoredState::from(m.state),
                        "last_error": m.last_error,
                    })
                })
                .collect();
            Response::ok(json!({ "outbox": out }))
        }
        Err(e) => from_host(e),
    }
}

pub(crate) async fn retry<H: AppHost>(host: &H, req: &Request) -> Response {
    let message_id = match req.params.get("message_id").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => return Response::invalid_params("message_id is required"),
    };
    match AppConversation::retry(host, message_id).await {
        Ok(()) => Response::ok(json!({ "retried": true })),
        Err(e) => from_host(e),
    }
}

const DELETE_NOTE: &str = "The local copy is removed and a deletion record kept. A request to \
                           delete it was sent to the other side; whether their client honours it \
                           is theirs to decide, and this cannot check. This installation's own \
                           message store still holds what it received.";
const DELETE_NOTE_NO_PEER: &str = "The local copy is removed and a deletion record kept. This is \
                                   a message you received; the other side's copy is theirs.";
const DELETE_NOTE_GROUP_ALONE: &str = "The local copy is removed and a deletion record kept. No \
                                       request was sent: nobody else in this group can receive \
                                       one from you now.";
const DELETE_NOTE_GROUP: &str = "The local copy is removed and a deletion record kept. A request \
                                 to delete it was sent to the other members; whether their \
                                 clients honour it is theirs to decide, and this cannot check. \
                                 Every member already holds the key this message was sent under.";

pub(crate) async fn delete_message<H: AppHost>(host: &H, req: &Request) -> Response {
    let message_id = match req.params.get("message_id").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => return Response::invalid_params("message_id is required"),
    };
    let ask_peer_req = req.params.get("ask_peer").and_then(Value::as_bool).unwrap_or(true);

    let msg = match AppConversation::get_message(host, message_id.clone()).await {
        Ok(m) => Some(m),
        Err(ConversationError::NotFound) => None,
        Err(e) => return from_host(e),
    };

    let (ask_peer, note) = if let Some(m) = &msg {
        let group_info = AppConversation::group_info(host, m.conversation.clone()).await.ok();
        if !m.outgoing {
            (false, DELETE_NOTE_NO_PEER)
        } else if let Some(g) = group_info {
            if !g.is_member || g.restored || g.members.len() <= 1 {
                (false, DELETE_NOTE_GROUP_ALONE)
            } else {
                (ask_peer_req, DELETE_NOTE_GROUP)
            }
        } else {
            (ask_peer_req, DELETE_NOTE)
        }
    } else {
        (ask_peer_req, DELETE_NOTE)
    };

    match AppConversation::delete_message(host, message_id.clone(), ask_peer).await {
        Ok(()) => Response::ok(json!({
            "deleted": message_id,
            "asked_peer": ask_peer,
            "note": note,
        })),
        Err(e) => from_host(e),
    }
}

pub(crate) async fn search<H: AppHost>(host: &H, req: &Request) -> Response {
    let query = match req.params.get("query").and_then(Value::as_str) {
        Some(q) if !q.is_empty() => q.to_string(),
        _ => return Response::invalid_params("query is required"),
    };
    let conversation = req.params.get("conversation").and_then(Value::as_str).map(str::to_string);
    let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(100) as u32;
    let kind_filter = req.params.get("kind").and_then(Value::as_str);

    let expected_kind = match kind_filter {
        Some("direct") => Some(ConversationKind::Direct),
        Some("group") => Some(ConversationKind::Group),
        Some(_) => return Response::invalid_params("unknown kind"),
        None => None,
    };

    let conv_kinds = if expected_kind.is_some() {
        match AppConversation::conversations(host).await {
            Ok(summaries) => {
                let map: HashMap<String, ConversationKind> =
                    summaries.into_iter().map(|s| (s.id, s.kind)).collect();
                Some(map)
            }
            Err(e) => return from_host(e),
        }
    } else {
        None
    };

    match AppConversation::search(host, query, conversation, limit).await {
        Ok(msgs) => {
            let out: Vec<Value> = msgs
                .into_iter()
                .filter(|m| !is_group_system_type(&m.content_type))
                .filter(|m| {
                    if let (Some(k), Some(map)) = (expected_kind, &conv_kinds) {
                        map.get(&m.conversation).copied() == Some(k)
                    } else {
                        true
                    }
                })
                .map(|m| message_to_json(&m))
                .collect();
            Response::ok(json!({ "matches": out }))
        }
        Err(e) => from_host(e),
    }
}

pub(crate) async fn transcript_digest<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match req
        .params
        .get("conversation")
        .or_else(|| req.params.get("group"))
        .and_then(Value::as_str)
    {
        Some(c) => c.to_string(),
        None => return Response::invalid_params("conversation is required"),
    };
    let transcript = match AppConversation::transcript_digest(host, conversation).await {
        Ok(t) => t,
        Err(e) => return from_host(e),
    };
    Response::ok(json!({
        "digest": transcript.digest,
        "rows": transcript.rows,
    }))
}

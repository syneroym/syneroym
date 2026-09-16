//! Conversation messaging operations: open, send, history, search, and
//! deletion.

use std::cmp::Reverse;

use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppConversation, AppDataLayer, AppHost,
    types::data_layer::{QueryOptions, RecordWriteValue},
};
use syneroym_roym_core::{
    clock,
    conversation::{
        ConversationRow, DELETION_REQUEST_CONTENT_TYPE, Direction, MessageRow, StoredState,
        deletion_request_body, encode_body, sort_key,
    },
    envelope::{Request, Response},
};

use super::{
    CONVERSATIONS, MESSAGES, ensure_conversations, ensure_messages,
    inbox::{host_last_error, host_message},
    load_conversation, load_message, own_conversation_address, person_did_for_address,
    profile_call, put_message,
};

async fn resolve_open_address<H: AppHost>(host: &H, req: &Request) -> Result<String, Response> {
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
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    let person_did = person_did_for_address(host, &address).await;
    if let Err(e) = upsert_conversation_open(host, &conversation_id, &address, person_did).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "conversation_id": conversation_id, "peer_address": address }))
}

/// Like `upsert_conversation` but does not bump `message_count` -- opening
/// a conversation is not a message.
async fn upsert_conversation_open<H: AppHost>(
    host: &H,
    conversation_id: &str,
    peer_address: &str,
    peer_person_did: Option<String>,
) -> Result<(), String> {
    ensure_conversations(host).await?;
    if load_conversation(host, conversation_id).await?.is_some() {
        return Ok(());
    }
    let now = clock::now_secs();
    let row = ConversationRow {
        id: conversation_id.to_string(),
        peer_address: peer_address.to_string(),
        peer_person_did,
        opened_at_secs: now,
        last_activity_ms: 0,
        message_count: 0,
    };
    AppDataLayer::put(
        host,
        CONVERSATIONS.to_string(),
        RecordWriteValue {
            id: conversation_id.to_string(),
            payload: serde_json::to_vec(&row).map_err(|e| e.to_string())?,
        },
    )
    .await
    .map_err(|e| e.to_string())
}

pub(crate) async fn list<H: AppHost>(host: &H, req: &Request) -> Response {
    if let Err(e) = ensure_conversations(host).await {
        return Response::internal_error(e);
    }
    let offset = req.params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
    let mut rows: Vec<ConversationRow> = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            CONVERSATIONS.to_string(),
            QueryOptions { filter: None, limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(row) = serde_json::from_slice::<ConversationRow>(&r.payload) {
                rows.push(row);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    rows.sort_by_key(|r| Reverse(r.last_activity_ms));
    let out: Vec<Value> = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .collect();
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
        req.params.get("content_type").and_then(Value::as_str).unwrap_or("text/plain").to_string();

    let message_id = match AppConversation::send(
        host,
        conversation.clone(),
        content_type.clone(),
        body.clone().into_bytes(),
    )
    .await
    {
        Ok(id) => id,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    // Read the state back rather than assuming it: the state this row is
    // born with is the host's answer, not this service's hope.
    let state = match AppConversation::delivery_status(host, message_id.clone()).await {
        Ok(s) => StoredState::from(s),
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };

    let now = clock::now_secs();
    let (body_encoding, stored_body) = encode_body(&content_type, body.as_bytes());
    // Take `author` and `sender_timestamp` from the host's own record of
    // the message it just enqueued, not from a local recomputation: the
    // peer stores the same two values, so both transcripts then compute
    // the same ADR-0013 sort key `(sender-timestamp, author, id)`. A
    // freshly sent message is `pending` and so in the outbox; it is only
    // absent if it reached `delivered` between `send` and this read (a
    // synthetic instantly-reachable peer) or the outbox read faulted.
    // Then `author` falls back to this installation's real conversation
    // address from `profile` -- still the value the peer stores, never the
    // old synthetic `self:` string -- and the timestamp to the local clock.
    let host_msg = host_message(host, &message_id).await;
    let author = match host_msg.as_ref() {
        Some(m) => m.author.clone(),
        None => own_conversation_address(host).await.unwrap_or_else(|| "self".to_string()),
    };
    let sender_timestamp_ms = host_msg.as_ref().map_or(now as i64 * 1000, |m| m.sender_timestamp);
    let row = MessageRow {
        id: message_id.clone(),
        conversation: conversation.clone(),
        author,
        direction: Direction::Outgoing,
        sender_timestamp_ms,
        content_type,
        body_encoding,
        body: Some(stored_body),
        state,
        last_error: None,
        deleted_at_secs: None,
        stored_at_secs: now,
    };
    if let Err(e) = put_message(host, &row).await {
        return Response::internal_error(e);
    }
    if let Err(e) = upsert_conversation_activity(host, &conversation, row.sender_timestamp_ms).await
    {
        return Response::internal_error(e);
    }

    Response::ok(json!({
        "message_id": message_id,
        "state": state,
        "sender_timestamp_ms": row.sender_timestamp_ms,
    }))
}

async fn upsert_conversation_activity<H: AppHost>(
    host: &H,
    conversation_id: &str,
    activity_ms: i64,
) -> Result<(), String> {
    if let Some(mut row) = load_conversation(host, conversation_id).await? {
        row.message_count += 1;
        row.last_activity_ms = row.last_activity_ms.max(activity_ms);
        AppDataLayer::put(
            host,
            CONVERSATIONS.to_string(),
            RecordWriteValue {
                id: conversation_id.to_string(),
                payload: serde_json::to_vec(&row).map_err(|e| e.to_string())?,
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn messages_of<H: AppHost>(host: &H, conversation: &str) -> Result<Vec<MessageRow>, String> {
    ensure_messages(host).await?;
    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            MESSAGES.to_string(),
            QueryOptions {
                filter: Some(json!({ "conversation": conversation }).to_string()),
                limit: Some(500),
                cursor: cursor.clone(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        for r in page.records {
            if let Ok(row) = serde_json::from_slice::<MessageRow>(&r.payload) {
                rows.push(row);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(rows)
}

pub(crate) async fn history<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match req.params.get("conversation").and_then(Value::as_str) {
        Some(c) => c.to_string(),
        None => return Response::invalid_params("conversation is required"),
    };
    let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;
    let offset = req.params.get("cursor").and_then(Value::as_u64).unwrap_or(0) as usize;

    let mut rows = match messages_of(host, &conversation).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    };

    // Reconcile: re-read the host's delivery-status for every row that is
    // not yet `Delivered` and not deleted, and persist what it read. A
    // `Delivered` row is terminal. A `Failed` row was told so explicitly
    // by an `on-delivery-state` notification, so a stale host read must
    // not walk it back to `pending` -- but a retry that succeeded while no
    // notification was listened for is real, so a `Failed` row does move
    // forward to `Delivered`. The cost is bounded by the number of
    // messages not yet delivered, not by history length.
    for row in rows.iter_mut() {
        if row.state == StoredState::Delivered || row.deleted_at_secs.is_some() {
            continue;
        }
        let Ok(live) = AppConversation::delivery_status(host, row.id.clone()).await else {
            continue;
        };
        let live = StoredState::from(live);
        if live == row.state {
            continue;
        }
        if row.state == StoredState::Failed && live != StoredState::Delivered {
            continue;
        }
        row.state = live;
        row.last_error =
            if live == StoredState::Failed { host_last_error(host, &row.id).await } else { None };
        let _ = put_message(host, row).await;
    }

    rows.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    let page: Vec<Value> = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .collect();
    Response::ok(json!({ "messages": page }))
}

pub(crate) async fn delivery_status<H: AppHost>(host: &H, req: &Request) -> Response {
    let message_id = match req.params.get("message_id").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => return Response::invalid_params("message_id is required"),
    };
    match AppConversation::delivery_status(host, message_id).await {
        Ok(s) => Response::ok(json!({ "state": StoredState::from(s) })),
        Err(e) => Response::internal_error(format!("{e:?}")),
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
        Err(e) => Response::internal_error(format!("{e:?}")),
    }
}

pub(crate) async fn retry<H: AppHost>(host: &H, req: &Request) -> Response {
    let message_id = match req.params.get("message_id").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => return Response::invalid_params("message_id is required"),
    };
    match AppConversation::retry(host, message_id).await {
        Ok(()) => Response::ok(json!({ "retried": true })),
        Err(e) => Response::internal_error(format!("{e:?}")),
    }
}

const DELETE_NOTE: &str = "The local copy is removed and a deletion record kept. A request to \
                           delete it was sent to the other side; whether their client honours it \
                           is theirs to decide, and this cannot check. This installation's own \
                           message store still holds what it received.";
const DELETE_NOTE_NO_PEER: &str = "The local copy is removed and a deletion record kept. This is \
                                   a message you received; the other side's copy is theirs.";

pub(crate) async fn delete_message<H: AppHost>(host: &H, req: &Request) -> Response {
    let message_id = match req.params.get("message_id").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => return Response::invalid_params("message_id is required"),
    };
    let ask_peer = req.params.get("ask_peer").and_then(Value::as_bool).unwrap_or(true);

    let Some(mut row) = (match load_message(host, &message_id).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    }) else {
        return Response::invalid_params("no such message");
    };

    let now = clock::now_secs();
    row.tombstone(now);
    if let Err(e) = put_message(host, &row).await {
        return Response::internal_error(e);
    }

    // `ask_peer` is meaningful only for a message this person authored:
    // asking somebody to delete what *they* sent is a different feature.
    let mut asked_peer = false;
    if row.direction == Direction::Outgoing && ask_peer {
        if let Err(e) = AppConversation::send(
            host,
            row.conversation.clone(),
            DELETION_REQUEST_CONTENT_TYPE.to_string(),
            deletion_request_body(&row.id),
        )
        .await
        {
            return Response::internal_error(format!("deletion request not queued: {e:?}"));
        }
        asked_peer = true;
    }

    let note = if row.direction == Direction::Outgoing { DELETE_NOTE } else { DELETE_NOTE_NO_PEER };
    Response::ok(json!({ "deleted": message_id, "asked_peer": asked_peer, "note": note }))
}

/// Escapes every regex metacharacter, so a person typing `(` is searching
/// for a bracket rather than writing a pattern.
fn escape_regex(query: &str) -> String {
    let mut out = String::with_capacity(query.len() * 2);
    for c in query.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub(crate) async fn search<H: AppHost>(host: &H, req: &Request) -> Response {
    let query = match req.params.get("query").and_then(Value::as_str) {
        Some(q) if !q.is_empty() => q.to_string(),
        _ => return Response::invalid_params("query is required"),
    };
    let conversation = req.params.get("conversation").and_then(Value::as_str);
    let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;

    if let Err(e) = ensure_messages(host).await {
        return Response::internal_error(e);
    }

    let mut filter = Map::new();
    filter.insert("body".to_string(), json!({ "$regex": escape_regex(&query) }));
    filter.insert("body_encoding".to_string(), json!("utf8"));
    if let Some(c) = conversation {
        filter.insert("conversation".to_string(), json!(c));
    }

    let mut matches: Vec<MessageRow> = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            MESSAGES.to_string(),
            QueryOptions {
                filter: Some(Value::Object(filter.clone()).to_string()),
                limit: Some(500),
                cursor: cursor.clone(),
            },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(row) = serde_json::from_slice::<MessageRow>(&r.payload)
                && row.deleted_at_secs.is_none()
            {
                matches.push(row);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    matches.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    let out: Vec<Value> = matches
        .into_iter()
        .take(limit)
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .collect();
    Response::ok(json!({ "matches": out }))
}

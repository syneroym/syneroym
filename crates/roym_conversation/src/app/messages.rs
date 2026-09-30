//! Conversation messaging operations: open, send, history, search, and
//! deletion.

use std::cmp::Reverse;

use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppConversation, AppDataLayer, AppHost,
    types::{conversation::ConversationError, data_layer::QueryOptions},
};
use syneroym_roym_core::{
    clock,
    conversation::{
        ConversationRow, ConversationRowKind, DELETION_REQUEST_CONTENT_TYPE, Direction, MessageRow,
        StoredState, deletion_request_body, encode_body,
        group::{
            CARDS_NOT_IN_GROUPS_MESSAGE, GROUP_PROFILE_CONTENT_TYPE, GROUP_REMOVED_NOTICE,
            GroupAdmission, MEMBERSHIP_EVENT_CONTENT_TYPE, is_group_system_type,
            transcript_digest as calculate_transcript_digest,
        },
        sort_key,
    },
    envelope::{Request, Response},
};

use super::{
    CONVERSATIONS, MESSAGES, ensure_conversations, ensure_messages, group, inbox::host_last_error,
    load_conversation, load_message, person_did_for_address, profile_call, put_conversation,
    put_message,
};

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
    if load_conversation(host, conversation_id).await?.is_some() {
        return Ok(());
    }
    let now = clock::now_secs();
    let row = ConversationRow {
        id: conversation_id.to_string(),
        kind: ConversationRowKind::Direct,
        peer_address: peer_address.to_string(),
        peer_person_did,
        opened_at_secs: now,
        last_activity_ms: 0,
        message_count: 0,
        group: None,
    };
    put_conversation(host, &row).await
}

pub(crate) async fn list<H: AppHost>(host: &H, req: &Request) -> Response {
    if let Err(e) = ensure_conversations(host).await {
        return Response::internal_error(e);
    }
    let offset = req.params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
    let kind_filter = req.params.get("kind").and_then(Value::as_str);
    let include_hidden = req.params.get("include_hidden").and_then(Value::as_bool).unwrap_or(false);

    if kind_filter != Some("direct") {
        group::adopt_new_groups(host).await;
    }

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
                if let Some(kind) = kind_filter {
                    let matches_kind = match kind {
                        "direct" => row.kind == ConversationRowKind::Direct,
                        "group" => row.kind == ConversationRowKind::Group,
                        _ => false,
                    };
                    if !matches_kind {
                        continue;
                    }
                }
                if row.kind == ConversationRowKind::Group && !include_hidden {
                    let is_shown = row
                        .group
                        .as_ref()
                        .is_some_and(|g| matches!(g.admission, GroupAdmission::Shown));
                    if !is_shown {
                        continue;
                    }
                }
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

pub(crate) async fn send_and_record<H: AppHost>(
    host: &H,
    conversation: &str,
    content_type: &str,
    body: &[u8],
) -> Result<MessageRow, Response> {
    let message_id = match AppConversation::send(
        host,
        conversation.to_string(),
        content_type.to_string(),
        body.to_vec(),
    )
    .await
    {
        Ok(id) => id,
        Err(ConversationError::InvalidArgument(m)) => return Err(Response::invalid_params(m)),
        Err(e) => return Err(Response::internal_error(format!("{e:?}"))),
    };

    let host_msg = match AppConversation::get_message(host, message_id.clone()).await {
        Ok(m) => m,
        Err(e) => return Err(Response::internal_error(format!("get_message: {e:?}"))),
    };

    let now = clock::now_secs();
    let (body_encoding, stored_body) = encode_body(content_type, body);
    let row = MessageRow {
        id: message_id,
        conversation: conversation.to_string(),
        author: host_msg.author,
        direction: Direction::Outgoing,
        sender_timestamp_ms: host_msg.sender_timestamp,
        content_type: content_type.to_string(),
        body_encoding,
        body: Some(stored_body),
        state: StoredState::from(host_msg.state),
        last_error: host_msg.last_error,
        deleted_at_secs: None,
        stored_at_secs: now,
    };
    if let Err(e) = put_message(host, &row).await {
        return Err(Response::internal_error(e));
    }
    if let Err(e) = upsert_conversation_activity(host, conversation, row.sender_timestamp_ms).await
    {
        return Err(Response::internal_error(e));
    }
    Ok(row)
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

    let row = match load_conversation(host, &conversation).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    };
    let is_group = row.as_ref().is_some_and(|r| r.kind == ConversationRowKind::Group);
    if content_type == MEMBERSHIP_EVENT_CONTENT_TYPE
        || (is_group
            && (content_type == "application/vnd.roym.card+json"
                || content_type == GROUP_PROFILE_CONTENT_TYPE))
    {
        return Response::invalid_params(if content_type == "application/vnd.roym.card+json" {
            CARDS_NOT_IN_GROUPS_MESSAGE
        } else {
            "this content type is reserved"
        });
    }
    if is_group {
        let info = match AppConversation::group_info(host, conversation.clone()).await {
            Ok(i) => i,
            Err(e) => return Response::internal_error(format!("{e:?}")),
        };
        if !info.is_member {
            return Response::invalid_params(GROUP_REMOVED_NOTICE);
        }
    }

    match send_and_record(host, &conversation, content_type, body.as_bytes()).await {
        Ok(msg) => Response::ok(json!({
            "message_id": msg.id,
            "state": msg.state,
            "sender_timestamp_ms": msg.sender_timestamp_ms,
        })),
        Err(resp) => resp,
    }
}

async fn upsert_conversation_activity<H: AppHost>(
    host: &H,
    conversation_id: &str,
    activity_ms: i64,
) -> Result<(), String> {
    if let Some(mut row) = load_conversation(host, conversation_id).await? {
        row.message_count += 1;
        row.last_activity_ms = row.last_activity_ms.max(activity_ms);
        put_conversation(host, &row).await?;
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

    let (kind_str, conv_row) = match load_conversation(host, &conversation).await {
        Ok(Some(r)) => {
            let k = match r.kind {
                ConversationRowKind::Direct => "direct",
                ConversationRowKind::Group => "group",
            };
            (k, Some(r))
        }
        Ok(None) => ("direct", None),
        Err(e) => return Response::internal_error(e),
    };
    if let Some(mut row) = conv_row
        && row.kind == ConversationRowKind::Group
        && let Ok(info) = AppConversation::group_info(host, conversation.clone()).await
        && let Ok(copied) = group::sync_membership_rows(host, &mut row, &info).await
        && copied > 0
    {
        let _ = put_conversation(host, &row).await;
    }

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
    Response::ok(json!({ "messages": page, "kind": kind_str }))
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
    match AppConversation::retry(host, message_id.clone()).await {
        Ok(()) => {
            mark_retried(host, &message_id).await;
            Response::ok(json!({ "retried": true }))
        }
        Err(e) => Response::internal_error(format!("{e:?}")),
    }
}

/// A failed row is never walked back to `pending` by a history read (a
/// stale host read must not undo a real failure), so the retry has to move
/// it itself. Otherwise the Hub shows `failed` for a message the host is
/// attempting again.
async fn mark_retried<H: AppHost>(host: &H, message_id: &str) {
    if let Ok(Some(mut row)) = load_message(host, message_id).await
        && row.state == StoredState::Failed
    {
        row.state = StoredState::Pending;
        row.last_error = None;
        let _ = put_message(host, &row).await;
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
    let ask_peer = req.params.get("ask_peer").and_then(Value::as_bool).unwrap_or(true);

    let Some(mut row) = (match load_message(host, &message_id).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    }) else {
        return Response::invalid_params("no such message");
    };

    if is_group_system_type(&row.content_type) {
        return Response::invalid_params("this row records a group change and cannot be deleted");
    }

    let now = clock::now_secs();
    row.tombstone(now);
    if let Err(e) = put_message(host, &row).await {
        return Response::internal_error(e);
    }

    let conv = load_conversation(host, &row.conversation).await.ok().flatten();
    let is_group = conv.as_ref().is_some_and(|c| c.kind == ConversationRowKind::Group);

    if row.direction == Direction::Incoming {
        return Response::ok(json!({
            "deleted": message_id,
            "asked_peer": false,
            "note": DELETE_NOTE_NO_PEER,
        }));
    }

    if !is_group {
        let mut asked_peer = false;
        if ask_peer {
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
        return Response::ok(json!({
            "deleted": message_id,
            "asked_peer": asked_peer,
            "note": DELETE_NOTE,
        }));
    }

    let info = match AppConversation::group_info(host, row.conversation.clone()).await {
        Ok(i) => i,
        Err(e) => return Response::internal_error(format!("{e:?}")),
    };
    if !info.is_member || info.members.len() <= 1 {
        return Response::ok(json!({
            "deleted": message_id,
            "asked_peer": false,
            "note": DELETE_NOTE_GROUP_ALONE,
        }));
    }

    let mut asked_peer = false;
    let mut send_error = None;
    if ask_peer {
        match AppConversation::send(
            host,
            row.conversation.clone(),
            DELETION_REQUEST_CONTENT_TYPE.to_string(),
            deletion_request_body(&row.id),
        )
        .await
        {
            Ok(_) => asked_peer = true,
            Err(e) => {
                send_error = Some(format!("{e:?}"));
            }
        }
    }
    let mut res = json!({
        "deleted": message_id,
        "asked_peer": asked_peer,
        "note": DELETE_NOTE_GROUP,
    });
    if let Some(err) = send_error {
        res["send_error"] = json!(err);
    }
    Response::ok(res)
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
    let kind_filter = req.params.get("kind").and_then(Value::as_str);

    if let Err(e) = ensure_messages(host).await {
        return Response::internal_error(e);
    }

    let mut filter = Map::new();
    filter.insert("body".to_string(), json!({ "$regex": escape_regex(&query) }));
    filter.insert("body_encoding".to_string(), json!("utf8"));
    filter.insert(
        "content_type".to_string(),
        json!({ "$nin": [MEMBERSHIP_EVENT_CONTENT_TYPE, GROUP_PROFILE_CONTENT_TYPE] }),
    );

    if let Some(kind) = kind_filter {
        let expected_kind = match kind {
            "direct" => ConversationRowKind::Direct,
            "group" => ConversationRowKind::Group,
            _ => return Response::invalid_params("unknown kind"),
        };
        let mut conv_ids = Vec::new();
        let mut conv_cursor = None;
        loop {
            let page = match AppDataLayer::query(
                host,
                CONVERSATIONS.to_string(),
                QueryOptions { filter: None, limit: Some(500), cursor: conv_cursor.clone() },
            )
            .await
            {
                Ok(p) => p,
                Err(e) => return Response::internal_error(e.to_string()),
            };
            for r in page.records {
                if let Ok(row) = serde_json::from_slice::<ConversationRow>(&r.payload)
                    && row.kind == expected_kind
                {
                    conv_ids.push(row.id);
                }
            }
            if page.next_cursor.is_none() || page.next_cursor == conv_cursor {
                break;
            }
            conv_cursor = page.next_cursor;
        }
        if conv_ids.is_empty() {
            return Response::ok(json!({ "matches": [] }));
        }
        if let Some(c) = conversation {
            if !conv_ids.contains(&c.to_string()) {
                return Response::ok(json!({ "matches": [] }));
            }
            filter.insert("conversation".to_string(), json!(c));
        } else {
            filter.insert("conversation".to_string(), json!({ "$in": conv_ids }));
        }
    } else if let Some(c) = conversation {
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

pub(crate) async fn transcript_digest<H: AppHost>(host: &H, req: &Request) -> Response {
    let conversation = match req.params.get("conversation").and_then(Value::as_str) {
        Some(c) => c.to_string(),
        None => return Response::invalid_params("conversation is required"),
    };
    let rows = match messages_of(host, &conversation).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    };
    let digest = match calculate_transcript_digest(&rows) {
        Ok(d) => d,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    Response::ok(json!({
        "digest": digest,
        "rows": rows.len(),
    }))
}

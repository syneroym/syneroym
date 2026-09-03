//! Conversation service application logic, target-independent.
//!
//! Two things live here: Roym's own copy of every message it sends and
//! receives (what export, search and delete act on), and the product's
//! inbox -- the enforcement point for the block list and the first-contact
//! rate limit.

use serde_json::{Value, json};
use syneroym_app_host::{
    AppConversation, AppDataLayer, AppHost,
    types::{
        conversation::{ConversationKind, DeliveryState, Message},
        data_layer::{
            CollectionSchema, IndexDefinition, IndexType, QueryOptions, RecordWriteValue,
        },
        proxy::CallTarget,
    },
};
use syneroym_roym_core::{
    admit,
    backup::{BUNDLE_VERSION, Bundle, BundleManifest, SECTION_CONVERSATIONS, SECTION_MESSAGES},
    clock,
    conversation::{
        ConversationRow, DELETION_REQUEST_CONTENT_TYPE, Direction, MessageRow, StoredState,
        deletion_request_body, encode_body, parse_deletion_request, sort_key,
    },
    envelope::{Request, Response},
    services, signing,
};

/// Bumped in this slice: the service gains its first state.
pub const SCHEMA_VERSION: u32 = 2;

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

async fn ensure_conversations<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(host, CONVERSATIONS, &[idx("last_activity_ms", IndexType::Numeric)]).await
}

async fn ensure_messages<H: AppHost>(host: &H) -> Result<(), String> {
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

async fn ensure_refused<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(host, REFUSED_MESSAGES, &[idx("at_secs", IndexType::Numeric)]).await
}

async fn put_message<H: AppHost>(host: &H, row: &MessageRow) -> Result<(), String> {
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

async fn load_message<H: AppHost>(host: &H, id: &str) -> Result<Option<MessageRow>, String> {
    ensure_messages(host).await?;
    let row = AppDataLayer::get(host, MESSAGES.to_string(), id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

async fn load_conversation<H: AppHost>(
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
async fn profile_call<H: AppHost>(
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

/// This peer's person DID as far as this product can say, from its own
/// contacts. `None` when no contact carries the address.
async fn person_did_for_address<H: AppHost>(host: &H, address: &str) -> Option<String> {
    let resp = profile_call(host, "contacts.list", json!({})).await.ok()?;
    let list = resp.result?;
    let rows = list.as_array()?;
    for row in rows {
        if row.get("conversation_address").and_then(Value::as_str) == Some(address) {
            return row.get("person_did").and_then(Value::as_str).map(str::to_string);
        }
    }
    None
}

async fn record_refused<H: AppHost>(
    host: &H,
    msg: &Message,
    reason: &str,
    now_secs: u64,
) -> Result<(), String> {
    ensure_refused(host).await?;
    let row = json!({
        "id": msg.id,
        "conversation": msg.conversation,
        "author": msg.author,
        "reason": reason,
        "at_secs": now_secs,
    });
    AppDataLayer::put(
        host,
        REFUSED_MESSAGES.to_string(),
        RecordWriteValue {
            id: msg.id.clone(),
            payload: serde_json::to_vec(&row).unwrap_or_default(),
        },
    )
    .await
    .map_err(|e| e.to_string())
}

/// Roym's inbox. Called from the guest `on-message` export on WASM and
/// from `ConversationSink::on_message` natively. Always returns `Ok`: the
/// host stored and acknowledged the message before this ran, so an `Err`
/// would make it log a delivery failure and, on WASM, retry -- turning a
/// deliberate product decision into a repeated apparent fault.
pub async fn on_message<H: AppHost>(host: &H, msg: Message) -> Result<(), String> {
    if let Err(e) = on_message_inner(host, &msg).await {
        // A storage fault is worth a trace, not an Err back to the host.
        log_inbox_error(&msg, &e);
    }
    Ok(())
}

fn log_inbox_error(msg: &Message, err: &str) {
    // No `tracing` dependency in this crate's wasm build; a stderr line is
    // enough and the native build's logger picks it up.
    eprintln!("roym conversation inbox: message {} not stored: {err}", msg.id);
}

async fn on_message_inner<H: AppHost>(host: &H, msg: &Message) -> Result<(), String> {
    let now = clock::now_secs();

    // The kind comes from the host's own summary, never guessed. A group
    // entry calls the same notifier; without this branch the first group
    // message would create a direct conversation whose peer is the author.
    let kind = AppConversation::conversations(host)
        .await
        .ok()
        .and_then(|cs| cs.into_iter().find(|c| c.id == msg.conversation).map(|c| c.kind))
        .unwrap_or(ConversationKind::Direct);
    if kind != ConversationKind::Direct {
        return record_refused(host, msg, "unsupported-kind", now).await;
    }

    let person_did = person_did_for_address(host, &msg.author).await;

    // Block is checked on every message: a person who blocks somebody
    // mid-conversation means it from that moment on.
    let block = profile_call(
        host,
        "block.check",
        json!({ "address": msg.author, "person_did": person_did }),
    )
    .await?;
    if block.result.as_ref().and_then(|v| v.get("blocked")).and_then(Value::as_bool) == Some(true) {
        return record_refused(host, msg, "blocked", now).await;
    }

    // The rate limit is a *first contact* limit, and calling the verb that
    // enforces it consumes a budget -- so it is consulted only when this
    // node holds no conversation with this peer yet.
    if load_conversation(host, &msg.conversation).await?.is_none() {
        let admit = profile_call(
            host,
            "contacts.admit-first-contact",
            json!({ "sender_address": msg.author, "sender_person_did": person_did }),
        )
        .await?;
        match admit.result.as_ref().and_then(|v| v.get("admission")).and_then(Value::as_str) {
            Some("allow") => {}
            Some("blocked") => return record_refused(host, msg, "blocked", now).await,
            _ => return record_refused(host, msg, "rate-limited", now).await,
        }
    }

    // A deletion request is not a message a person reads. It is honoured
    // only for a message the requester themselves authored here.
    if msg.content_type == DELETION_REQUEST_CONTENT_TYPE {
        if let Ok(target_id) = parse_deletion_request(&msg.body)
            && let Some(mut target) = load_message(host, &target_id).await?
            && target.conversation == msg.conversation
            && target.author == msg.author
        {
            target.tombstone(now);
            put_message(host, &target).await?;
        }
        return Ok(()); // never stored as a message either way
    }

    // Store it.
    upsert_conversation(
        host,
        &msg.conversation,
        &msg.author,
        person_did.clone(),
        msg.sender_timestamp,
    )
    .await?;
    let (body_encoding, body) = encode_body(&msg.content_type, &msg.body);
    let row = MessageRow {
        id: msg.id.clone(),
        conversation: msg.conversation.clone(),
        author: msg.author.clone(),
        direction: Direction::Incoming,
        sender_timestamp_ms: msg.sender_timestamp,
        content_type: msg.content_type.clone(),
        body_encoding,
        body: Some(body),
        state: StoredState::Delivered,
        last_error: msg.last_error.clone(),
        deleted_at_secs: None,
        stored_at_secs: now,
    };
    put_message(host, &row).await
}

async fn upsert_conversation<H: AppHost>(
    host: &H,
    conversation_id: &str,
    peer_address: &str,
    peer_person_did: Option<String>,
    activity_ms: i64,
) -> Result<(), String> {
    ensure_conversations(host).await?;
    let now = clock::now_secs();
    let existing = load_conversation(host, conversation_id).await?;
    let row = match existing {
        Some(mut r) => {
            r.message_count += 1;
            r.last_activity_ms = r.last_activity_ms.max(activity_ms);
            if r.peer_person_did.is_none() {
                r.peer_person_did = peer_person_did;
            }
            r
        }
        None => ConversationRow {
            id: conversation_id.to_string(),
            peer_address: peer_address.to_string(),
            peer_person_did,
            opened_at_secs: now,
            last_activity_ms: activity_ms,
            message_count: 1,
        },
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

/// Called on a delivery-state transition. Updates the row's state and
/// error, or does nothing if Roym holds no such row.
pub async fn on_delivery_state<H: AppHost>(
    host: &H,
    message_id: String,
    state: DeliveryState,
) -> Result<(), String> {
    let Some(mut row) = load_message(host, &message_id).await? else { return Ok(()) };
    row.state = StoredState::from(state);
    if row.state != StoredState::Failed {
        row.last_error = None;
    }
    put_message(host, &row).await
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
        "conversation.open" => open(host, &req).await,
        "conversation.list" => list(host, &req).await,
        "conversation.send" => send(host, &req).await,
        "conversation.history" => history(host, &req).await,
        "conversation.delivery-status" => delivery_status(host, &req).await,
        "conversation.outbox" => outbox(host).await,
        "conversation.retry" => retry(host, &req).await,
        "conversation.delete-message" => delete_message(host, &req).await,
        "conversation.search" => search(host, &req).await,
        "conversation.export" => export(host).await,
        "conversation.import" => import(host, &req).await,
        other => Response::method_not_found(other),
    }
}

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

async fn open<H: AppHost>(host: &H, req: &Request) -> Response {
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

async fn list<H: AppHost>(host: &H, req: &Request) -> Response {
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
    rows.sort_by_key(|r| std::cmp::Reverse(r.last_activity_ms));
    let out: Vec<Value> = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .collect();
    Response::ok(json!({ "conversations": out }))
}

async fn send<H: AppHost>(host: &H, req: &Request) -> Response {
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
    // The author of an outgoing message is this installation's own address;
    // the conversation row already carries the peer.
    let author = load_conversation(host, &conversation)
        .await
        .ok()
        .flatten()
        .map(|c| format!("self:{}", c.id))
        .unwrap_or_else(|| "self".to_string());
    let row = MessageRow {
        id: message_id.clone(),
        conversation: conversation.clone(),
        author,
        direction: Direction::Outgoing,
        sender_timestamp_ms: now as i64 * 1000,
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

    Response::ok(json!({ "message_id": message_id, "state": state }))
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

async fn history<H: AppHost>(host: &H, req: &Request) -> Response {
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

    // Reconcile: re-read the host's delivery-status for every row still
    // `Pending` and not deleted, and persist what it read. A `Delivered`
    // row is terminal; a `Failed` row was told so explicitly by an
    // `on-delivery-state` notification and must not be quietly walked
    // back to `pending` by a stale host read -- a retry that later
    // succeeds fires its own notification. So the cost is bounded by the
    // number of messages still in flight, not by history length.
    for row in rows.iter_mut() {
        if row.state == StoredState::Pending
            && row.deleted_at_secs.is_none()
            && let Ok(live) = AppConversation::delivery_status(host, row.id.clone()).await
        {
            let live = StoredState::from(live);
            if live != row.state {
                row.state = live;
                let _ = put_message(host, row).await;
            }
        }
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

async fn delivery_status<H: AppHost>(host: &H, req: &Request) -> Response {
    let message_id = match req.params.get("message_id").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => return Response::invalid_params("message_id is required"),
    };
    match AppConversation::delivery_status(host, message_id).await {
        Ok(s) => Response::ok(json!({ "state": StoredState::from(s) })),
        Err(e) => Response::internal_error(format!("{e:?}")),
    }
}

async fn outbox<H: AppHost>(host: &H) -> Response {
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

async fn retry<H: AppHost>(host: &H, req: &Request) -> Response {
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

async fn delete_message<H: AppHost>(host: &H, req: &Request) -> Response {
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

async fn search<H: AppHost>(host: &H, req: &Request) -> Response {
    let query = match req.params.get("query").and_then(Value::as_str) {
        Some(q) if !q.is_empty() => q.to_string(),
        _ => return Response::invalid_params("query is required"),
    };
    let conversation = req.params.get("conversation").and_then(Value::as_str);
    let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;

    if let Err(e) = ensure_messages(host).await {
        return Response::internal_error(e);
    }

    let mut filter = serde_json::Map::new();
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

async fn collect<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
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

async fn export<H: AppHost>(host: &H) -> Response {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let now = clock::now_secs();
    if let Err(e) = ensure_conversations(host).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_messages(host).await {
        return Response::internal_error(e);
    }
    let conversations = match collect(host, CONVERSATIONS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let messages = match collect(host, MESSAGES).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let sections = std::collections::BTreeMap::from([
        (SECTION_CONVERSATIONS.to_string(), conversations),
        (SECTION_MESSAGES.to_string(), messages),
    ]);
    let mut manifest_sections = std::collections::BTreeMap::new();
    for (k, v) in &sections {
        match Bundle::digest(SCHEMA_VERSION, v) {
            Ok(d) => {
                manifest_sections.insert(k.clone(), d);
            }
            Err(e) => return Response::internal_error(e.to_string()),
        }
    }
    let bundle = Bundle {
        manifest: BundleManifest {
            bundle_version: BUNDLE_VERSION,
            produced_at_secs: now,
            subject_did: owner,
            sections: manifest_sections,
        },
        sections,
    };
    match serde_json::to_value(&bundle) {
        Ok(v) => Response::ok(v),
        Err(e) => Response::internal_error(e.to_string()),
    }
}

async fn import<H: AppHost>(host: &H, req: &Request) -> Response {
    let bundle_val = match req.params.get("bundle").cloned().or_else(|| Some(req.params.clone())) {
        Some(v) => v,
        None => return Response::invalid_params("bundle is required"),
    };
    let bundle = match Bundle::from_json(&bundle_val.to_string()) {
        Ok(b) => b,
        Err(e) => return Response::invalid_params(format!("invalid bundle: {e}")),
    };
    if let Err(e) = bundle.check_integrity() {
        return Response::invalid_params(e.to_string());
    }
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if bundle.manifest.subject_did != owner {
        return Response::invalid_params(format!(
            "bundle belongs to '{}', this node holds '{}'",
            bundle.manifest.subject_did, owner
        ));
    }
    for (name, declared) in &bundle.manifest.sections {
        if declared.schema_version != SCHEMA_VERSION {
            return Response::invalid_params(format!(
                "section '{name}' has schema version {}, this node requires {SCHEMA_VERSION}",
                declared.schema_version
            ));
        }
    }

    let mut counts = serde_json::Map::new();
    for (name, records) in &bundle.sections {
        let collection = match name.as_str() {
            SECTION_CONVERSATIONS => CONVERSATIONS,
            SECTION_MESSAGES => MESSAGES,
            other => return Response::invalid_params(format!("unknown section '{other}'")),
        };
        if let Err(e) = ensure_coll(host, collection, &[]).await {
            return Response::internal_error(e);
        }
        let mut n = 0u64;
        for rec in records {
            let id = match rec.get("id").and_then(Value::as_str) {
                Some(i) => i.to_string(),
                None => return Response::invalid_params("record missing id"),
            };
            let payload_val = match rec.get("payload") {
                Some(p) => p.clone(),
                None => return Response::invalid_params("record missing payload"),
            };
            let bytes = match serde_json::to_vec(&payload_val) {
                Ok(b) => b,
                Err(e) => return Response::internal_error(e.to_string()),
            };
            if let Err(e) = AppDataLayer::put(
                host,
                collection.to_string(),
                RecordWriteValue { id, payload: bytes },
            )
            .await
            {
                return Response::internal_error(e.to_string());
            }
            n += 1;
        }
        counts.insert(collection.to_string(), json!(n));
    }
    Response::ok(json!({ "imported": counts }))
}

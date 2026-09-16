//! Inbound message delivery, validation, rate limiting, and delivery state.

use serde_json::{Value, json};
use syneroym_app_host::{
    AppConversation, AppDataLayer, AppHost,
    types::{
        conversation::{ConversationKind, DeliveryState, Message},
        data_layer::RecordWriteValue,
    },
};
use syneroym_roym_core::{
    clock,
    conversation::{
        ConversationRow, DELETION_REQUEST_CONTENT_TYPE, Direction, MessageRow, StoredState,
        encode_body, parse_deletion_request,
    },
};

use super::{
    CONVERSATIONS, REFUSED_MESSAGES, ensure_conversations, ensure_refused, load_conversation,
    load_message, person_did_for_address, profile_call, put_message,
};

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
/// from `ConversationSink::on_message` natively.
///
/// A deliberate product decision -- block, first-contact rate limit,
/// unsupported kind -- writes a `refused_messages` row and returns `Ok`:
/// there is nothing to retry. A storage fault or an unavailable `profile`
/// sibling returns `Err`, so the WASM host's retry and the native
/// notifier's warning both fire; without that the message is dropped with
/// only a `stderr` line and nothing ever repairs it (`conversation.history`
/// reads only Roym's own copy).
pub async fn on_message<H: AppHost>(host: &H, msg: Message) -> Result<(), String> {
    if let Err(e) = on_message_inner(host, &msg).await {
        log_inbox_error(&msg, &e);
        return Err(e);
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

    // Idempotent store: the WASM host retries `on-message` after a
    // transient fault (C5-2). A message already in Roym's copy must not be
    // stored or counted again -- this catches the common case, a retry
    // after a `profile` sibling was briefly unavailable, before any store
    // write ran. A fault strictly between `upsert_conversation` and
    // `put_message` can still double-count `message_count` on retry; that
    // narrower window is the C5-9(a) backlog row (unfenced count).
    if load_message(host, &msg.id).await?.is_some() {
        return Ok(());
    }

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
/// error, or does nothing if Roym holds no such row. The WIT
/// `delivery-state` carries no reason, so on a `failed` transition the
/// host's own reason is read back from its outbox, where the failed
/// message keeps its `last-error`.
pub async fn on_delivery_state<H: AppHost>(
    host: &H,
    message_id: String,
    state: DeliveryState,
) -> Result<(), String> {
    let Some(mut row) = load_message(host, &message_id).await? else { return Ok(()) };
    row.state = StoredState::from(state);
    match row.state {
        StoredState::Failed => {
            row.last_error = host_last_error(host, &message_id).await;
        }
        _ => row.last_error = None,
    }
    put_message(host, &row).await
}

/// The host's own record for a message still in flight, from its outbox.
/// A `delivered` message has left the outbox, so this returns `None` for
/// one -- callers only need it for `pending`/`failed` rows.
pub(crate) async fn host_message<H: AppHost>(host: &H, message_id: &str) -> Option<Message> {
    AppConversation::outbox(host).await.ok()?.into_iter().find(|m| m.id == message_id)
}

/// The host's own reason for a message's current state, from its outbox.
pub(crate) async fn host_last_error<H: AppHost>(host: &H, message_id: &str) -> Option<String> {
    host_message(host, message_id).await.and_then(|m| m.last_error)
}

//! The fixture's whole behaviour. Compiled unchanged into both builds; it
//! names no build-specific type and calls nothing but `syneroym-app-host`.

use core::fmt;

use serde_json::json;
use syneroym_app_host::{
    AppBlobReader, AppBlobWriter, AppHost,
    types::{
        conversation::{DeliveryState, Message},
        data_layer::{CollectionSchema, Mutation, QueryOptions, RecordWriteValue},
    },
};

const MESSAGES: &str = "messages";
const INBOX: &str = "inbox";
/// What `on_conversation_message` persists — never in-process state,
/// same rule `INBOX` follows.
const CONV_INBOX: &str = "conv_inbox";
/// What `on_conversation_state` persists.
const CONV_STATE_LOG: &str = "conv_state_log";
/// Dedicated to the mutation-shape verbs below (`patch`/`batch-mutate`/
/// `delete-many`/`drop-collection`) so they can seed, drop, and re-seed
/// freely without disturbing `MESSAGES`/`INBOX`'s own row counts, which the
/// messaging and `store-messages`/`read-messages` scenarios depend on.
const SCRATCH: &str = "scratch";

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Request {
    /// data-layer: ensure schema, write `count` rows, read them back.
    StoreMessages {
        count: u32,
    },
    /// data-layer: page through `messages` with an explicit limit.
    ReadMessages {
        limit: u32,
    },
    /// data-layer: a mutation the caller is not allowed to make
    /// (`execute-ddl` without `data-layer/admin`), to prove both builds deny
    /// identically.
    AdminDdl {
        sql: String,
    },
    /// data-layer: a read of an id that was never written.
    GetMissing {
        id: String,
    },
    /// blob-store one-shot round trip.
    PutBlob {
        body: String,
    },
    GetBlob {
        hash: String,
    },
    /// blob-store streaming round trip through the resources.
    StreamBlob {
        chunks: Vec<String>,
        read_chunk: u32,
    },
    /// messaging: subscribe, then publish to self; the delivery lands in
    /// `inbox` via `handle_message`/the shim's broker pump.
    SubscribeTopic {
        topic: String,
    },
    PublishTopic {
        topic: String,
        payload: String,
    },
    /// messaging: read what `on_message` stored. Never in-process state:
    /// every WASM invocation gets a fresh instance, so a static would not
    /// survive between a delivery and this read.
    ReadInbox,
    /// messaging: subscribe then immediately unsubscribe from the same
    /// topic.
    Unsubscribe {
        topic: String,
    },
    /// data-layer: write a row, then apply a JSON merge patch to it.
    Patch {
        id: String,
    },
    /// data-layer: two `put`s in one `batch-mutate` call.
    BatchMutate {
        id_a: String,
        id_b: String,
    },
    /// data-layer: write a row, then `delete-many` with an empty (match-all)
    /// filter.
    DeleteMany {
        id: String,
    },
    /// data-layer: drop `SCRATCH`, then recreate it so later scenarios that
    /// reuse it still find it there.
    DropCollection,
    /// blob-store: one-shot write, then delete it.
    DeleteBlob {
        body: String,
    },
    /// blob-store: open an upload, write to it, then abort instead of
    /// finishing. `abort`, like `finish`, is a WIT resource *method* (a
    /// borrowed handle) rather than the destructor (an owned one) -- the
    /// same distinction that made `HostBlobWriter::finish` easy to get
    /// wrong, so both need exercising on both builds, not just `finish`.
    AbortUpload {
        chunks: Vec<String>,
    },
    /// M06B slice B4: idempotent -- proves the id is stable across repeat
    /// calls.
    OpenConversation {
        peer_address: String,
    },
    /// M06B slice B4: returns the message id and its state (`pending`
    /// immediately -- `send` never touches the network).
    SendMessage {
        conversation: String,
        body: String,
    },
    ReadHistory {
        conversation: String,
        limit: u32,
    },
    DeliveryStatus {
        message: String,
    },
    /// M06B slice B4 (G2): the outbox surface.
    ReadOutbox,
    RetryMessage {
        message: String,
    },
    /// What `on_conversation_message` stored -- through `data-layer`, never
    /// in-process state (`D-B3-12`).
    ReadConversationInbox,
    /// What `on_conversation_state` stored, same rule.
    ReadStateLog,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Response {
    Ok(serde_json::Value),
    Err(String),
}

pub async fn run<H: AppHost>(host: &H, request: &str) -> Result<String, String> {
    let req: Request =
        serde_json::from_str(request).map_err(|e| format!("malformed request: {e}"))?; // the only WIT `Err`
    let response = match dispatch(host, req).await {
        Ok(v) => Response::Ok(v),
        Err(e) => Response::Err(e),
    };
    serde_json::to_string(&response).map_err(|e| e.to_string())
}

fn fmt_err<E: fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

/// Ensures `collection` exists, lazily, on first use -- this fixture has no
/// `init`/`migrate` lifecycle hook, since there is no native analogue of
/// "the host called `init` at deploy time" to keep in step with the WASM
/// side. `create_collection` is `CREATE TABLE IF NOT EXISTS` underneath, so
/// a repeat call is a no-op `Ok(())` on both builds, not an error to
/// special-case.
async fn ensure_collection<H: AppHost>(host: &H, collection: &str) -> Result<(), String> {
    host.create_collection(CollectionSchema { name: collection.to_string(), indexes: vec![] })
        .await
        .map_err(fmt_err)
}

async fn dispatch<H: AppHost>(host: &H, req: Request) -> Result<serde_json::Value, String> {
    match req {
        Request::StoreMessages { count } => {
            ensure_collection(host, MESSAGES).await?;
            for i in 0..count {
                host.put(
                    MESSAGES.into(),
                    RecordWriteValue {
                        id: format!("m{i}"),
                        payload: format!(r#"{{"seq":{i}}}"#).into_bytes(),
                    },
                )
                .await
                .map_err(fmt_err)?;
            }
            let page = host
                .query(
                    MESSAGES.into(),
                    QueryOptions { filter: None, limit: Some(count), cursor: None },
                )
                .await
                .map_err(fmt_err)?;
            Ok(json!({ "written": count, "read": page.records.len() }))
        }
        Request::ReadMessages { limit } => {
            ensure_collection(host, MESSAGES).await?;
            let page = host
                .query(
                    MESSAGES.into(),
                    QueryOptions { filter: None, limit: Some(limit), cursor: None },
                )
                .await
                .map_err(fmt_err)?;
            Ok(json!({ "read": page.records.len() }))
        }
        Request::AdminDdl { sql } => match host.execute_ddl(sql).await {
            Ok(()) => Ok(json!({ "admin-ddl": "allowed" })),
            Err(e) => Err(fmt_err(e)),
        },
        Request::GetMissing { id } => {
            ensure_collection(host, MESSAGES).await?;
            let found = host.get(MESSAGES.into(), id).await.map_err(fmt_err)?;
            Ok(json!({ "found": found.is_some() }))
        }
        Request::PutBlob { body } => {
            let hash = host.put_blob(body.clone().into_bytes()).await.map_err(fmt_err)?;
            Ok(json!({ "hash": hash, "bytes": body.len() }))
        }
        Request::GetBlob { hash } => {
            let bytes = host.get_blob(hash).await.map_err(fmt_err)?;
            Ok(json!({ "bytes": bytes.len(), "body": String::from_utf8_lossy(&bytes) }))
        }
        Request::StreamBlob { chunks, read_chunk } => {
            let mut w = host.open_upload().await.map_err(fmt_err)?;
            for c in &chunks {
                w.write(c.clone().into_bytes()).await.map_err(fmt_err)?;
            }
            let hash = w.finish().await.map_err(fmt_err)?; // consumes `w`
            let mut r = host.open_download(hash.clone(), 0).await.map_err(fmt_err)?;
            let mut out = Vec::new();
            loop {
                let part = r.read(read_chunk).await.map_err(fmt_err)?;
                if part.is_empty() {
                    break;
                }
                out.extend_from_slice(&part);
            }
            Ok(json!({ "hash": hash, "bytes": out.len(), "body": String::from_utf8_lossy(&out) }))
        }
        Request::SubscribeTopic { topic } => {
            host.subscribe(topic).await.map_err(fmt_err)?;
            Ok(json!({ "subscribed": true }))
        }
        Request::PublishTopic { topic, payload } => {
            host.publish(topic, payload.into_bytes()).await.map_err(fmt_err)?;
            Ok(json!({ "published": true }))
        }
        Request::ReadInbox => {
            ensure_collection(host, INBOX).await?;
            let page = host
                .query(INBOX.into(), QueryOptions { filter: None, limit: Some(100), cursor: None })
                .await
                .map_err(fmt_err)?;
            let entries: Vec<serde_json::Value> = page
                .records
                .into_iter()
                .filter_map(|r| serde_json::from_slice(&r.payload).ok())
                .collect();
            Ok(json!({ "entries": entries }))
        }
        Request::Unsubscribe { topic } => {
            host.subscribe(topic.clone()).await.map_err(fmt_err)?;
            host.unsubscribe(topic).await.map_err(fmt_err)?;
            Ok(json!({ "unsubscribed": true }))
        }
        Request::Patch { id } => {
            ensure_collection(host, SCRATCH).await?;
            host.put(
                SCRATCH.into(),
                RecordWriteValue { id: id.clone(), payload: b"{\"seq\":0}".to_vec() },
            )
            .await
            .map_err(fmt_err)?;
            host.patch(SCRATCH.into(), id.clone(), b"{\"patched\":true}".to_vec())
                .await
                .map_err(fmt_err)?;
            let after = host.get(SCRATCH.into(), id).await.map_err(fmt_err)?;
            Ok(json!({ "after": after.map(|r| String::from_utf8_lossy(&r.payload).into_owned()) }))
        }
        Request::BatchMutate { id_a, id_b } => {
            ensure_collection(host, SCRATCH).await?;
            host.batch_mutate(
                SCRATCH.into(),
                vec![
                    Mutation::Put(RecordWriteValue {
                        id: id_a.clone(),
                        payload: b"{\"n\":1}".to_vec(),
                    }),
                    Mutation::Put(RecordWriteValue {
                        id: id_b.clone(),
                        payload: b"{\"n\":2}".to_vec(),
                    }),
                ],
            )
            .await
            .map_err(fmt_err)?;
            let a_found = host.get(SCRATCH.into(), id_a).await.map_err(fmt_err)?.is_some();
            let b_found = host.get(SCRATCH.into(), id_b).await.map_err(fmt_err)?.is_some();
            Ok(json!({ "a_found": a_found, "b_found": b_found }))
        }
        Request::DeleteMany { id } => {
            ensure_collection(host, SCRATCH).await?;
            host.put(SCRATCH.into(), RecordWriteValue { id: id.clone(), payload: b"{}".to_vec() })
                .await
                .map_err(fmt_err)?;
            // An empty filter matches every row in the collection.
            let deleted = host.delete_many(SCRATCH.into(), String::new()).await.map_err(fmt_err)?;
            let still_present = host.get(SCRATCH.into(), id).await.map_err(fmt_err)?.is_some();
            Ok(json!({ "deleted": deleted, "still_present": still_present }))
        }
        Request::DropCollection => {
            ensure_collection(host, SCRATCH).await?;
            host.drop_collection(SCRATCH.into()).await.map_err(fmt_err)?;
            // Recreate immediately so later scenarios reusing `SCRATCH` (in
            // either request order) still find it.
            ensure_collection(host, SCRATCH).await?;
            Ok(json!({ "dropped": true }))
        }
        Request::DeleteBlob { body } => {
            let hash = host.put_blob(body.into_bytes()).await.map_err(fmt_err)?;
            host.delete_blob(hash.clone()).await.map_err(fmt_err)?;
            let still_gettable = host.get_blob(hash).await.is_ok();
            Ok(json!({ "deleted": true, "still_gettable": still_gettable }))
        }
        Request::AbortUpload { chunks } => {
            let mut w = host.open_upload().await.map_err(fmt_err)?;
            for c in &chunks {
                w.write(c.clone().into_bytes()).await.map_err(fmt_err)?;
            }
            w.abort().await; // consumes `w`
            Ok(json!({ "aborted": true }))
        }
        Request::OpenConversation { peer_address } => {
            let id = host.open_direct(peer_address).await.map_err(fmt_err)?;
            Ok(json!({ "conversation": id }))
        }
        Request::SendMessage { conversation, body } => {
            let id = host
                .send(conversation, "text/plain".to_string(), body.into_bytes())
                .await
                .map_err(fmt_err)?;
            Ok(json!({ "message": id }))
        }
        Request::ReadHistory { conversation, limit } => {
            let page = host.history(conversation, limit, None).await.map_err(fmt_err)?;
            Ok(json!({
                "messages": page.messages.iter().map(message_json).collect::<Vec<_>>(),
                "next-cursor": page.next_cursor,
            }))
        }
        Request::DeliveryStatus { message } => {
            let state = host.delivery_status(message).await.map_err(fmt_err)?;
            Ok(json!({ "state": delivery_state_str(state) }))
        }
        Request::ReadOutbox => {
            let outbox = host.outbox().await.map_err(fmt_err)?;
            Ok(json!({ "outbox": outbox.iter().map(message_json).collect::<Vec<_>>() }))
        }
        Request::RetryMessage { message } => {
            host.retry(message).await.map_err(fmt_err)?;
            Ok(json!({ "retried": true }))
        }
        Request::ReadConversationInbox => {
            ensure_collection(host, CONV_INBOX).await?;
            let page = host
                .query(
                    CONV_INBOX.into(),
                    QueryOptions { filter: None, limit: Some(1000), cursor: None },
                )
                .await
                .map_err(fmt_err)?;
            let entries: Vec<serde_json::Value> = page
                .records
                .into_iter()
                .filter_map(|r| serde_json::from_slice(&r.payload).ok())
                .collect();
            Ok(json!({ "entries": entries }))
        }
        Request::ReadStateLog => {
            ensure_collection(host, CONV_STATE_LOG).await?;
            let page = host
                .query(
                    CONV_STATE_LOG.into(),
                    QueryOptions { filter: None, limit: Some(1000), cursor: None },
                )
                .await
                .map_err(fmt_err)?;
            let entries: Vec<serde_json::Value> = page
                .records
                .into_iter()
                .filter_map(|r| serde_json::from_slice(&r.payload).ok())
                .collect();
            Ok(json!({ "entries": entries }))
        }
    }
}

fn delivery_state_str(s: DeliveryState) -> &'static str {
    match s {
        DeliveryState::Pending => "pending",
        DeliveryState::Delivered => "delivered",
        DeliveryState::Failed => "failed",
    }
}

fn message_json(m: &Message) -> serde_json::Value {
    json!({
        "id": m.id,
        "conversation": m.conversation,
        "author": m.author,
        "sender-timestamp": m.sender_timestamp,
        "content-type": m.content_type,
        "body": String::from_utf8_lossy(&m.body),
        "state": delivery_state_str(m.state),
        "verified": m.verified,
        "last-error": m.last_error,
    })
}

/// Called by both builds when a subscribed message arrives -- from the
/// exported `guest-api::handle-message` on WASM, from the shim's broker pump
/// natively. Persists through `data-layer`, never in process memory.
///
/// `topic` arrives fully namespaced (`svc/<service_id>/<topic>`) on both
/// builds, and is stored verbatim.
pub async fn on_message<H: AppHost>(
    host: &H,
    topic: String,
    payload: Vec<u8>,
) -> Result<(), String> {
    ensure_collection(host, INBOX).await?;
    let id = format!("{topic}:{}", inbox_entry_id(&payload));
    host.put(
        INBOX.into(),
        RecordWriteValue {
            id,
            payload: serde_json::to_vec(&json!({
                "topic": topic,
                "payload": String::from_utf8_lossy(&payload),
            }))
            .map_err(|e| e.to_string())?,
        },
    )
    .await
    .map_err(fmt_err)
}

/// Called by both builds when a durable conversation message arrives --
/// from the exported `guest-api::on-message` on WASM, from
/// `ConversationSink::on_message` natively. Persists through `data-layer`,
/// never in-process state (`D-B3-12`), keyed by the message's own id so a
/// redelivery overwrites rather than duplicates.
pub async fn on_conversation_message<H: AppHost>(host: &H, msg: Message) -> Result<(), String> {
    ensure_collection(host, CONV_INBOX).await?;
    host.put(
        CONV_INBOX.into(),
        RecordWriteValue {
            id: msg.id.clone(),
            payload: serde_json::to_vec(&message_json(&msg)).map_err(|e| e.to_string())?,
        },
    )
    .await
    .map_err(fmt_err)
}

/// Called by both builds on a delivery-state transition. Appends rather
/// than overwrites (keyed by message id + a monotonic-enough sequence
/// derived from the state name) so a test can observe the transition
/// sequence, not just the latest state.
pub async fn on_conversation_state<H: AppHost>(
    host: &H,
    message: String,
    state: DeliveryState,
) -> Result<(), String> {
    ensure_collection(host, CONV_STATE_LOG).await?;
    let state_str = delivery_state_str(state);
    host.put(
        CONV_STATE_LOG.into(),
        RecordWriteValue {
            id: format!("{message}:{state_str}"),
            payload: serde_json::to_vec(&json!({ "message": message, "state": state_str }))
                .map_err(|e| e.to_string())?,
        },
    )
    .await
    .map_err(fmt_err)
}

/// A short, stable id derived from the payload bytes -- avoids a hash
/// dependency for what is only ever a test fixture's own dedup key.
fn inbox_entry_id(payload: &[u8]) -> String {
    let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
    for b in payload {
        acc ^= u64::from(*b);
        acc = acc.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{acc:x}")
}

//! The fixture's whole behaviour. Compiled unchanged into both builds; it
//! names no build-specific type and calls nothing but `syneroym-app-host`.

use core::fmt;

use serde_json::json;
use syneroym_app_host::{
    AppAppConfig, AppBlobReader, AppBlobWriter, AppConversation, AppDataLayer, AppHost,
    AppInvocation, AppWebSocket,
    types::{
        conversation::{ConversationKind, DeliveryState, Message},
        data_layer::{CollectionSchema, Mutation, QueryOptions, RecordWriteValue},
        http::{CallerAuth, FrameKind, HttpRequest, HttpResponse},
        invocation::CallerOrigin,
        proxy::{CallOptions, CallTarget},
        signing::{Principal, RecordDraft},
    },
};
use syneroym_signed_record::{self as signed_record, Envelope, VerifyOptions};

mod callbacks;
mod dispatch;
mod http;
mod websocket;

pub use callbacks::{on_conversation_message, on_conversation_state, on_message};
pub use http::handle_http;
pub use websocket::{on_ws_close, on_ws_message, on_ws_open};

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
const WS_LOG: &str = "ws_log";
const HTTP_STORE: &str = "http_store";

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
    /// data-layer: call `create` twice with one shared id.
    CreateFence {
        id: String,
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
    /// finishing.
    AbortUpload {
        chunks: Vec<String>,
    },
    /// Proves the conversation id is stable across repeat calls.
    OpenConversation {
        peer_address: String,
    },
    /// Returns the message id and its state (pending immediately -- send never
    /// touches the network).
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
    /// The outbox surface.
    ReadOutbox,
    RetryMessage {
        message: String,
    },
    CreateGroup,
    AddMember {
        conversation: String,
        member_address: String,
    },
    RemoveMember {
        conversation: String,
        member_address: String,
    },
    Members {
        conversation: String,
    },
    MembershipHistory {
        conversation: String,
    },
    SyncNow {
        conversation: String,
    },
    ListConversations,
    /// What `on_conversation_message` stored -- through `data-layer`, never
    /// in-process state.
    ReadConversationInbox,
    /// What `on_conversation_state` stored, same rule.
    ReadStateLog,

    // ---- Proxy-call and config verbs ----
    ProxyCallSelf {
        service_id: String,
        interface: String,
        method: String,
        params: String,
    },
    ProxyCallDependency {
        name: String,
        interface: String,
        method: String,
        params: String,
    },
    ProxyCallUnboundDependency {
        name: String,
    },
    ProxyCallCrossServiceNative {
        target: String,
        interface: String,
        method: String,
        params: String,
    },
    ProxyEnqueue {
        name: String,
        idempotency_key: Option<String>,
    },
    ProxyEnqueueNoKey {
        name: String,
    },
    ProxyEnqueueEmptyKey {
        name: String,
    },
    ReadConfig {
        key: String,
    },
    ReadConfigSection {
        prefix: String,
    },
    RevealSecret {
        key: String,
    },
    WsSend {
        conn: String,
        body: String,
    },
    ReadWsLog,
    ReadHttpStore,

    // ---- Record signing verbs ----
    SignAsService {
        draft: RecordDraft,
    },
    SignAsDelegated {
        draft: RecordDraft,
        delegation_json: String,
    },
    SigningIdentity,
    VerifyRecord {
        signed_json: String,
        #[serde(alias = "now_secs")]
        now_secs: Option<u64>,
    },

    /// `syneroym:invocation`: reports the arm the host tells this call it
    /// arrived on. The whole point of the interface, and the shim's own
    /// rule is that a trait with only one of its two implementations
    /// exercised is how the native build becomes second-class.
    CallerOrigin,
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
    let response = match dispatch::dispatch(host, req).await {
        Ok(v) => Response::Ok(v),
        Err(e) => Response::Err(e),
    };
    serde_json::to_string(&response).map_err(|e| e.to_string())
}

fn fmt_err<E: fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

/// Ensures `collection` exists, lazily, on first use.
async fn ensure_collection<H: AppHost>(host: &H, collection: &str) -> Result<(), String> {
    host.create_collection(CollectionSchema { name: collection.to_string(), indexes: vec![] })
        .await
        .map_err(fmt_err)
}

fn delivery_state_str(state: DeliveryState) -> &'static str {
    match state {
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

/// A short, stable id derived from the payload bytes.
fn inbox_entry_id(payload: &[u8]) -> String {
    let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
    for b in payload {
        acc ^= u64::from(*b);
        acc = acc.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{acc:x}")
}

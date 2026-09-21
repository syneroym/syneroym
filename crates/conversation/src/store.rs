//! `conversation.db`: one file per service, on one shared connection with
//! its own `async_queue::Queue` — what makes the `send` transaction
//! atomic. Every `BLOB` column here is inside a DEK-opened database,
//! matching the rest of the tree's per-service stores.

use std::sync::{Arc, Mutex};

#[cfg(test)]
use anyhow::anyhow;
use rusqlite::Connection;
#[cfg(test)]
use rusqlite::params;
use syneroym_async_queue::Queue;
#[cfg(test)]
use syneroym_async_queue::QueueConfig;
use syneroym_rpc::{ConversationDeliveryState, ConversationKind, ConversationMessage};
use zeroize::Zeroizing;

use crate::dag::{EntryKind, MembershipPayload, WireEntry};

mod conversation;
mod dag_store;
mod message;
mod schema;
mod session;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("quota exceeded: pending messages cap reached")]
    PendingQuotaExceeded,
    #[error("quota exceeded: max messages per conversation reached")]
    MessageQuotaExceeded,
    #[error("quota exceeded: max dag entries per conversation reached")]
    DagEntryQuotaExceeded,
}

/// Per-conversation and per-service bounds, plus the clock/age
/// bounds `outbox.rs`/`transport.rs` apply. Converted from
/// `AppSandboxRole`'s `conversation_*` fields by the crate's caller
/// (`crates/substrate/src/runtime.rs`), not read from config directly here
/// — this crate does not depend on `syneroym-substrate`.
#[derive(Debug, Clone)]
pub struct ConversationConfig {
    pub max_body_bytes: u32,
    pub max_pending_per_conversation: u32,
    pub max_messages_per_conversation: u32,
    pub max_pending_age_secs: u64,
    pub max_clock_skew_secs: u64,
    pub prekey_requests_per_peer_per_hour: u32,
    pub conversation_group_sync_secs: u64,
    pub conversation_group_rekey_secs: u64,
    pub conversation_max_group_members: u32,
    pub conversation_max_dag_entries_per_conversation: u32,
    pub conversation_max_sync_entries_per_call: u32,
    pub conversation_relay_fanout: u32,
    pub conversation_sync_now_budget_ms: u64,
    pub conversation_background_sync_budget_ms: u64,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            max_body_bytes: 262_144,
            max_pending_per_conversation: 1_000,
            max_messages_per_conversation: 100_000,
            max_pending_age_secs: 2_592_000,
            max_clock_skew_secs: 86_400,
            prekey_requests_per_peer_per_hour: 20,
            conversation_group_sync_secs: 60,
            conversation_group_rekey_secs: 604_800,
            conversation_max_group_members: 256,
            conversation_max_dag_entries_per_conversation: 100_000,
            conversation_max_sync_entries_per_call: 64,
            conversation_relay_fanout: 3,
            conversation_sync_now_budget_ms: 3_000,
            conversation_background_sync_budget_ms: 160_000,
        }
    }
}

/// A row from `messages`, as `store.rs`'s own callers see it -- close to
/// but not identical to `syneroym_rpc::ConversationMessage` (this one
/// carries `signature`/`outgoing`, which are store-internal).
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: String,
    pub conversation_id: String,
    pub author: String,
    pub sender_timestamp_ms: i64,
    pub received_at_ms: i64,
    pub content_type: String,
    pub body: Vec<u8>,
    pub signature: [u8; 64],
    pub outgoing: bool,
    pub verified: bool,
    pub state: ConversationDeliveryState,
    pub last_error: Option<String>,
    pub system: bool,
    pub entry_id: Option<String>,
}

impl StoredMessage {
    #[must_use]
    pub fn into_wire(self) -> ConversationMessage {
        ConversationMessage {
            id: self.id,
            conversation: self.conversation_id,
            author: self.author,
            sender_timestamp: self.sender_timestamp_ms,
            received_at: self.received_at_ms,
            content_type: self.content_type,
            body: self.body,
            state: self.state,
            verified: self.verified,
            last_error: self.last_error,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConversationRow {
    pub id: String,
    pub kind: ConversationKind,
    pub peer_address: Option<String>,
    pub created_at_ms: i64,
    pub last_activity_ms: i64,
    pub owner_address: Option<String>,
    pub current_epoch: u64,
}

#[derive(Debug, Clone)]
pub struct HistoryPage {
    pub messages: Vec<StoredMessage>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredDagEntry {
    pub seq: i64,
    pub entry_id: String,
    pub conversation_id: String,
    pub author: String,
    pub sender_timestamp_ms: i64,
    pub epoch: u64,
    pub kind: EntryKind,
    pub header: Vec<u8>,
    pub ciphertext: Option<Vec<u8>>,
    pub nonce: Option<[u8; 12]>,
    pub payload: Option<MembershipPayload>,
    pub signature: [u8; 64],
    pub applied: bool,
    pub relay_pending: bool,
    pub parents: Vec<String>,
}

impl StoredDagEntry {
    #[must_use]
    pub fn into_wire(self) -> WireEntry {
        WireEntry {
            entry_id: self.entry_id,
            conversation_id: self.conversation_id,
            author: self.author,
            sender_timestamp_ms: self.sender_timestamp_ms,
            epoch: self.epoch,
            kind: self.kind,
            parents: self.parents,
            ciphertext: self.ciphertext,
            nonce: self.nonce,
            payload: self.payload,
            signature: self.signature,
        }
    }
}

/// A session row, as `crypto.rs` persists and reads it. `state` is opaque
/// to this module -- `crypto.rs`'s own encoding.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub peer_address: String,
    pub pinned_sig_key: [u8; 32],
    pub state: Vec<u8>,
}

/// This service's own long-term conversation keys: generated once, on
/// first use, and never derived from the service's ed25519 node identity.
#[derive(Debug, Clone)]
pub struct LocalIdentityRow {
    pub account_state: Zeroizing<Vec<u8>>,
    pub sig_secret: Zeroizing<Vec<u8>>,
}

pub struct ConversationStore {
    pub(super) conn: Arc<Mutex<Connection>>,
    pub(super) queue: Queue,
    pub(super) config: ConversationConfig,
}

impl std::fmt::Debug for ConversationStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationStore").finish_non_exhaustive()
    }
}

impl ConversationStore {
    #[must_use]
    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    #[must_use]
    pub fn config(&self) -> &ConversationConfig {
        &self.config
    }

    pub fn conn(&self) -> &std::sync::Mutex<Connection> {
        &self.conn
    }
}

/// The queue payload for one outgoing delivery -- shared between the
/// `send` write (this module) and the outbox worker's read (`outbox.rs`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct OutboxItem {
    pub message_id: String,
    pub peer_address: String,
    #[serde(default)]
    pub group: Option<String>,
}

#[must_use]
pub fn state_str(state: ConversationDeliveryState) -> &'static str {
    match state {
        ConversationDeliveryState::Pending => "pending",
        ConversationDeliveryState::Delivered => "delivered",
        ConversationDeliveryState::Failed => "failed",
    }
}

pub(super) fn state_from_str(s: &str) -> ConversationDeliveryState {
    match s {
        "delivered" => ConversationDeliveryState::Delivered,
        "failed" => ConversationDeliveryState::Failed,
        _ => ConversationDeliveryState::Pending,
    }
}

#[must_use]
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

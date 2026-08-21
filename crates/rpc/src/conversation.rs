//! Types and object-safe traits for `syneroym:conversation`.
//! Plain Rust here, not the WIT-generated shapes: this crate has no
//! `syneroym-wit-interfaces` dependency (no `wasmtime`), and both host
//! implementors -- `syneroym-sandbox-wasm`'s `HostState` (the WASM path) and
//! `syneroym-app-host-native`'s `NativeAppHost` (the native path) -- convert
//! between these and their own wire shape themselves, the same split
//! `data-layer`'s `Host` impl already draws.
//!
//! [`ConversationHost`] is held `Weak` by `HostState`/`AppSandboxEngine`
//! (the Slice-6B `Arc`-cycle reason: the only implementation,
//! `syneroym-conversation`'s `ConversationService`, is itself reached
//! through the engine it is wired into). [`ConversationNotifier`] is the
//! reverse direction, held `Weak` by `ConversationService`.

use std::fmt::Debug;

/// Mirrors `syneroym:conversation/conversation.conversation-error`.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ConversationError {
    #[error("permission denied")]
    PermissionDenied,
    #[error("not found")]
    NotFound,
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// A `send` never returns this -- `send` does not touch the network.
    #[error("unreachable: {0}")]
    Unreachable(String),
    #[error("quota exceeded")]
    QuotaExceeded,
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationDeliveryState {
    Pending,
    Delivered,
    Failed,
}

/// `Group` is reserved for the group slice (B5) and is never returned by
/// B4's own store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationKind {
    Direct,
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSummary {
    pub id: String,
    pub kind: ConversationKind,
    pub participants: Vec<String>,
    pub created_at: i64,
    pub last_activity_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessage {
    pub id: String,
    pub conversation: String,
    pub author: String,
    pub sender_timestamp: i64,
    pub received_at: i64,
    pub content_type: String,
    pub body: Vec<u8>,
    pub state: ConversationDeliveryState,
    pub verified: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationHistoryPage {
    pub messages: Vec<ConversationMessage>,
    pub next_cursor: Option<String>,
}

/// The guest-facing surface (`syneroym:conversation/conversation`, seven
/// functions) plus the two peer-facing transport verbs
/// (`prekey_bundle`/`peer_deliver`) reached only through the native-capability
/// `conversation` dispatch arm, never through this trait from a guest.
///
/// Every method is keyed by `service_id` (`HostState.component_id` /
/// `SynSvcNativeService.service_id`) -- never by anything a caller passes --
/// so one service can never reach another's conversation store (§5.3: "no
/// cross-service conversation access exists and no interface for one").
#[async_trait::async_trait]
pub trait ConversationHost: Send + Sync + Debug {
    async fn open_direct(
        &self,
        service_id: &str,
        peer_address: &str,
    ) -> Result<String, ConversationError>;

    async fn conversations(
        &self,
        service_id: &str,
    ) -> Result<Vec<ConversationSummary>, ConversationError>;

    /// Writes durably and returns `pending` immediately; never touches the
    /// network directly.
    async fn send(
        &self,
        service_id: &str,
        conversation: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<String, ConversationError>;

    async fn history(
        &self,
        service_id: &str,
        conversation: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<ConversationHistoryPage, ConversationError>;

    async fn delivery_status(
        &self,
        service_id: &str,
        message: &str,
    ) -> Result<ConversationDeliveryState, ConversationError>;

    async fn outbox(&self, service_id: &str)
    -> Result<Vec<ConversationMessage>, ConversationError>;

    async fn retry(&self, service_id: &str, message: &str) -> Result<(), ConversationError>;

    /// Peer-facing: serves this service's own X3DH prekey bundle to a
    /// verified requester, rate-limited per `requester_did`. The returned
    /// bytes are a JSON/serde-encoded `PrekeyBundle`; the transport layer
    /// on both ends agrees on the encoding, so this trait need not name it.
    async fn prekey_bundle(
        &self,
        service_id: &str,
        requester_did: &str,
    ) -> Result<Vec<u8>, ConversationError>;

    /// Peer-facing: receives one encrypted envelope from `requester_did`
    /// (the transport-verified owner Master DID, a coarse gate only).
    /// Returns the encoded `DeliveryAck` on success.
    async fn peer_deliver(
        &self,
        service_id: &str,
        requester_did: &str,
        envelope: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError>;
}

/// The host -> app direction (`syneroym:conversation/guest-api`), mirroring
/// `MessageSink`'s shape (B3 §13 item 3): the half with no automatic parity,
/// so both builds implement it explicitly.
#[async_trait::async_trait]
pub trait ConversationNotifier: Send + Sync + Debug {
    async fn notify_message(&self, service_id: &str, msg: ConversationMessage);
    async fn notify_delivery_state(
        &self,
        service_id: &str,
        message_id: String,
        state: ConversationDeliveryState,
    );
}

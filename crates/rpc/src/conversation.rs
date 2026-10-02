//! Types and object-safe traits for `syneroym:conversation`.
//! Plain Rust here, not the WIT-generated shapes: this crate has no
//! `syneroym-wit-interfaces` dependency (no `wasmtime`), and both host
//! implementors -- `syneroym-sandbox-wasm`'s `HostState` (the WASM path) and
//! `syneroym-app-host-native`'s `NativeAppHost` (the native path) -- convert
//! between these and their own wire shape themselves, the same split
//! `data-layer`'s `Host` impl already draws.
//!
//! [`ConversationHost`] is held `Weak` by `HostState`/`AppSandboxEngine`
//! (the `Arc`-cycle reason: the only implementation,
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

/// `Direct` is a fixed two-party pair. `Group` has its own membership DAG
/// on top of the message log (`syneroym-conversation`'s `group` module).
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
    pub message_count: u32,
    pub name: Option<String>,
    pub restored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMembershipEvent {
    pub entry: String,
    pub action: String,
    pub subject: String,
    pub epoch: u64,
    pub sender_timestamp: i64,
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
    pub outgoing: bool,
    pub deleted_at: Option<i64>,
    pub restored: bool,
    pub visible_seq: u64,
    pub refused: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropAnswer {
    pub reason: String,
    pub report: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    Accept,
    Hold(String),
    Drop(DropAnswer),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifyOutcome {
    Answered(Admission),
    NoHandler,
    NoAnswer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationNameEvent {
    pub entry: String,
    pub name: String,
    pub sender_timestamp: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationHistoryItem {
    Message(ConversationMessage),
    Membership(ConversationMembershipEvent),
    GroupName(ConversationNameEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationHistoryPage {
    pub items: Vec<ConversationHistoryItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationChangePage {
    pub messages: Vec<ConversationMessage>,
    pub last_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationExportChunk {
    pub data: Vec<u8>,
    pub next_cursor: Option<String>,
}

/// What the host knows about one group, in one call.
///
/// Mirrors `syneroym:conversation/conversation.group-info`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationGroupInfo {
    /// The group's owner, as a routing service id.
    pub owner: String,
    /// True when this service is the owner.
    pub is_owner: bool,
    /// True when this service is in the current member list.
    pub is_member: bool,
    /// Current members, sorted.
    pub members: Vec<String>,
    /// The newest epoch this substrate has seen named.
    pub epoch: u64,
    /// The newest epoch this substrate holds a key for.
    pub key_epoch: u64,
    /// When this substrate stored the key for `key_epoch`, Unix milliseconds.
    pub key_stored_at: i64,
    pub name: Option<String>,
    pub restored: bool,
}

/// The guest-facing surface (`syneroym:conversation/conversation`) plus
/// peer-facing transport verbs reached through the native-capability
/// `conversation` dispatch arm.
///
/// Every method is keyed by `service_id`.
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

    async fn create_group(&self, service_id: &str) -> Result<String, ConversationError>;

    async fn add_member(
        &self,
        service_id: &str,
        conversation: &str,
        member_address: &str,
    ) -> Result<(), ConversationError>;

    async fn remove_member(
        &self,
        service_id: &str,
        conversation: &str,
        member_address: &str,
    ) -> Result<(), ConversationError>;

    async fn members(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<Vec<String>, ConversationError>;

    async fn sync_now(&self, service_id: &str, conversation: &str)
    -> Result<(), ConversationError>;

    /// Returns owner, membership, epoch, and key state for a group.
    async fn group_info(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<ConversationGroupInfo, ConversationError>;

    /// Returns one stored message by id.
    async fn get_message(
        &self,
        service_id: &str,
        message: &str,
    ) -> Result<ConversationMessage, ConversationError>;

    async fn delete_message(
        &self,
        service_id: &str,
        message: &str,
        ask_others: bool,
    ) -> Result<(), ConversationError>;

    async fn readmit(
        &self,
        service_id: &str,
        conversation: &str,
        reasons: Vec<String>,
    ) -> Result<u32, ConversationError>;

    async fn changes(
        &self,
        service_id: &str,
        conversation: &str,
        after_seq: u64,
        limit: u32,
    ) -> Result<ConversationChangePage, ConversationError>;

    async fn search(
        &self,
        service_id: &str,
        query: &str,
        conversation: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ConversationMessage>, ConversationError>;

    async fn set_group_name(
        &self,
        service_id: &str,
        conversation: &str,
        name: &str,
    ) -> Result<(), ConversationError>;

    async fn transcript_digest(
        &self,
        service_id: &str,
        conversation: &str,
    ) -> Result<String, ConversationError>;

    async fn export_history(
        &self,
        service_id: &str,
        cursor: Option<String>,
    ) -> Result<ConversationExportChunk, ConversationError>;

    async fn import_history(
        &self,
        service_id: &str,
        data: Vec<u8>,
    ) -> Result<u32, ConversationError>;

    /// Peer-facing: accepts DAG entries pushed by another member.
    async fn group_push(
        &self,
        service_id: &str,
        requester_did: &str,
        request: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError>;

    /// Peer-facing: serves entries this substrate holds past the
    /// requester's cursor.
    async fn group_sync(
        &self,
        service_id: &str,
        requester_did: &str,
        request: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError>;

    /// Peer-facing: serves this service's own X3DH prekey bundle to a
    /// verified requester.
    async fn prekey_bundle(
        &self,
        service_id: &str,
        requester_did: &str,
    ) -> Result<Vec<u8>, ConversationError>;

    /// Peer-facing: receives one encrypted envelope from `requester_did`.
    async fn peer_deliver(
        &self,
        service_id: &str,
        requester_did: &str,
        envelope: Vec<u8>,
    ) -> Result<Vec<u8>, ConversationError>;
}

/// The host -> app direction (`syneroym:conversation/guest-api`).
#[async_trait::async_trait]
pub trait ConversationNotifier: Send + Sync + Debug {
    async fn notify_message(&self, service_id: &str, msg: ConversationMessage) -> NotifyOutcome;
    async fn notify_delivery_state(
        &self,
        service_id: &str,
        message_id: String,
        state: ConversationDeliveryState,
    );
}

//! What a mailbox and its callers send each other (ADR-0026): the method
//! names, the request and answer shapes, and the error codes. The sealed
//! message and the receipt themselves are in `syneroym-mailbox-format`.
//!
//! No type here refuses unknown fields: a mailbox and its callers may run
//! different versions.

use serde::{Deserialize, Serialize};
use syneroym_mailbox_format::{DEDICATED_MAX_TTL_SECS, DepositId, Receipt, SealedBytes};

use crate::{
    dht_registry::SignedEndpointInfo,
    local_registry::{self, NATIVE_CAPABILITY_INTERFACES},
    util,
};

pub const MAILBOX_INTERFACE: &str = "mailbox";
pub const PROTOCOL_VERSION: u32 = 1;

pub const METHOD_INFO: &str = "info";
pub const METHOD_REGISTER: &str = "register";
pub const METHOD_UNREGISTER: &str = "unregister";
pub const METHOD_DEPOSIT: &str = "deposit";
pub const METHOD_CLAIM: &str = "claim";
pub const METHOD_ACK: &str = "ack";
pub const METHOD_STATUS: &str = "status";

/// How long a sender keeps trying a message to a receiver with a mailbox,
/// direct and by deposit. The receiver fence for such a service lasts at
/// least this long, plus the clock skew allowance.
pub const MAILBOX_SENDER_MAX_AGE_SECS: u64 = 30 * 24 * 3600;
// The receiver fence window must not grow with a passport.
const _: () = assert!(DEDICATED_MAX_TTL_SECS <= MAILBOX_SENDER_MAX_AGE_SECS);

/// Largest accepted child token, the same bound as a relay registration.
pub const MAX_CHILD_TOKEN_BYTES: usize = 16 * 1024;

/// A message claimed this many times without an ack is closed.
pub const MAX_CLAIMS: u32 = 5;
pub const MAX_CLAIM_BATCH: u32 = 32;
pub const MAX_CLAIM_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ACK_BATCH: usize = 32;
pub const MAX_STATUS_BATCH: usize = 100;
pub const MAX_RECEIPT_JSON_BYTES: usize = 4 * 1024;

pub const MAX_RECORD_JSON_BYTES: usize = 4 * 1024;

/// Conversation calls a sender may always deposit. Every service with a
/// mailbox accepts them; they arrive on the native `conversation` interface,
/// which no manifest declares.
pub const ALWAYS_ELIGIBLE: [(&str, &str); 2] =
    [("conversation", "deliver"), ("conversation", "group-push")];

/// The longest `mailbox_methods` list a service may declare.
pub const MAX_MAILBOX_METHODS: usize = 64;

/// Whether `interface` belongs to the node or the platform, so no service
/// may list it in `mailbox_methods`. Matched by name and by short hash,
/// like `local_registry::is_node_level_interface`.
#[must_use]
pub fn is_reserved_mailbox_interface(interface: &str) -> bool {
    local_registry::is_node_level_interface(interface)
        || NATIVE_CAPABILITY_INTERFACES
            .iter()
            .any(|name| *name == interface || util::short_hash(name) == interface)
}

/// The mailbox a hosted service pulls from, as its signed record names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedMailbox {
    /// The node DID of the mailbox host.
    pub mailbox: String,
    /// The longest time-to-live of this service's queue. The open limit
    /// until the receiver has registered and learned the real one.
    pub max_ttl_secs: u64,
    /// The guest methods this service accepts from its mailbox, as
    /// `<interface>#<method>`, besides `ALWAYS_ELIGIBLE`. Checked at deploy.
    #[serde(default)]
    pub methods: Vec<String>,
}

impl HostedMailbox {
    /// Whether a call to `interface` (canonical name) and `method` may be
    /// delivered from the mailbox. An entry may name the interface by its
    /// short hash.
    #[must_use]
    pub fn accepts(&self, interface: &str, method: &str) -> bool {
        if ALWAYS_ELIGIBLE.contains(&(interface, method)) {
            return true;
        }
        let hash = util::short_hash(interface);
        self.methods
            .iter()
            .filter_map(|entry| entry.split_once('#'))
            .any(|(i, m)| m == method && (i == interface || i == hash))
    }
}

/// What one pull pass did (`security.mailbox-pull-now`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullSummary {
    /// Services with a mailbox that the pass looked at.
    pub services: u32,
    /// Services skipped or stopped in this pass (no certificate, no key, no
    /// record, a certificate or a register the mailbox refused, a full
    /// mailbox, not started yet after a restart).
    pub blocked: u32,
    /// Services whose queue the mailbox accepted at the last register.
    pub registered: u32,
    pub delivered: u32,
    pub rejected: u32,
    /// Messages left for a later claim (a temporary local failure).
    pub not_acked: u32,
}

/// JSON-RPC error codes. Each answer also carries `data.kind`, the
/// matching `ErrorKind` text.
pub mod codes {
    pub const UNKNOWN_DESTINATION: i32 = -32040;
    pub const TOO_LARGE: i32 = -32041;
    pub const QUEUE_FULL: i32 = -32042;
    pub const RATE_LIMITED: i32 = -32043;
    pub const MAILBOX_FULL: i32 = -32044;
    pub const NOT_AN_INSTANCE: i32 = -32045;
    pub const STALE_INSTANCE: i32 = -32046;
    pub const BAD_RECORD: i32 = -32047;
    pub const DUPLICATE_DEPOSIT: i32 = -32048;
    pub const UNAUTHENTICATED: i32 = -32049;
    /// Also carries `data.max_ttl_secs`, the limit of the queue.
    pub const TTL_TOO_LONG: i32 = -32050;
    /// The child token of `register` was refused. `data.code` is the
    /// passport refusal code (`expired`, `denied`, `limit_reached`, ...).
    pub const PASSPORT_REFUSED: i32 = -32051;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueLimits {
    pub max_bytes: u64,
    pub max_messages: u32,
    pub max_ttl_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfoAnswer {
    pub version: u32,
    /// The node DID of this mailbox.
    pub mailbox: String,
    pub limits: QueueLimits,
    pub max_sealed_bytes: usize,
    pub receipt_keep_secs: u64,
    pub visibility_secs: u64,
    pub queue_lifetime_secs: u64,
    pub max_claims: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterParams {
    pub record: SignedEndpointInfo,
    /// The owner's child token for this member master (JSON of a capability
    /// token whose one proof is the mailbox passport). Raises the queue's
    /// limits by the passport's facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterAnswer {
    pub limits: QueueLimits,
    pub queue_ends_at_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnregisterAnswer {
    pub removed: bool,
}

// `deposit` takes a `syneroym_mailbox_format::Deposit` as its params and
// answers a `Receipt::Stored`.

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimParams {
    #[serde(default)]
    pub max: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimedMessage {
    /// The depositor, as the mailbox saw it on the transport.
    pub sender: String,
    pub deposit_id: DepositId,
    pub sealed: SealedBytes,
    pub deposited_at_secs: u64,
    pub expires_at_secs: u64,
    pub claim_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimAnswer {
    pub messages: Vec<ClaimedMessage>,
    pub visible_again_at_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckEntry {
    pub sender: String,
    pub deposit_id: DepositId,
    pub receipt: Receipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckParams {
    pub acks: Vec<AckEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AckOutcome {
    Stored,
    NotFound,
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckAnswer {
    pub results: Vec<AckOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusParams {
    pub deposit_ids: Vec<DepositId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEntry {
    pub deposit_id: DepositId,
    pub receipt: Receipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusAnswer {
    pub receipts: Vec<StatusEntry>,
}

#[cfg(test)]
mod tests;

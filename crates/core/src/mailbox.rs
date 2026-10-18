//! What a mailbox and its callers send each other (ADR-0026): the method
//! names, the request and answer shapes, and the error codes. The sealed
//! message and the receipt themselves are in `syneroym-mailbox-format`.
//!
//! No type here refuses unknown fields: a mailbox and its callers may run
//! different versions.

use serde::{Deserialize, Serialize};
use syneroym_mailbox_format::{DepositId, Receipt, SealedBytes};

use crate::dht_registry::SignedEndpointInfo;

pub const MAILBOX_INTERFACE: &str = "mailbox";
pub const PROTOCOL_VERSION: u32 = 1;

pub const METHOD_INFO: &str = "info";
pub const METHOD_REGISTER: &str = "register";
pub const METHOD_UNREGISTER: &str = "unregister";
pub const METHOD_DEPOSIT: &str = "deposit";
pub const METHOD_CLAIM: &str = "claim";
pub const METHOD_ACK: &str = "ack";
pub const METHOD_STATUS: &str = "status";

/// A message claimed this many times without an ack is closed.
pub const MAX_CLAIMS: u32 = 5;
pub const MAX_CLAIM_BATCH: u32 = 32;
pub const MAX_CLAIM_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ACK_BATCH: usize = 32;
pub const MAX_STATUS_BATCH: usize = 100;
pub const MAX_RECEIPT_JSON_BYTES: usize = 4 * 1024;
pub const MAX_RECORD_JSON_BYTES: usize = 4 * 1024;

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

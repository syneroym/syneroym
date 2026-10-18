//! `[roles.mailbox]`: this node holds sealed messages for services that
//! cannot be reached (ADR-0026). Values are the limits of an open mailbox.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MailboxRole {
    /// SQLite file under `storage.db_dir`.
    pub db_name: String,
    /// Maximum bytes held across all messages in one queue.
    pub max_queue_bytes: u64,
    /// Maximum number of messages held in one queue.
    pub max_queue_messages: u32,
    /// Maximum time-to-live for a message, in seconds.
    pub max_ttl_secs: u64,
    /// How long receipts are kept after message completion or expiry, in
    /// seconds.
    pub receipt_keep_secs: u64,
    /// How long a claimed message is hidden from other claims, in seconds.
    pub visibility_secs: u64,
    /// How long an idle queue is kept without a register or claim, in seconds.
    pub queue_lifetime_secs: u64,
    /// Maximum bytes held across all queues on this node.
    pub max_total_bytes: u64,
    /// Maximum number of queues on this node.
    pub max_queues: u32,
    /// Maximum deposits per minute from one sender DID.
    pub deposits_per_minute_per_sender: u32,
    /// Maximum deposits per minute across all senders on this node.
    pub deposits_per_minute_total: u32,
    /// Interval between background sweeps, in seconds.
    pub sweep_interval_secs: u64,
    /// Interval between republishing stored service records, in seconds.
    pub republish_interval_secs: u64,
}

impl Default for MailboxRole {
    fn default() -> Self {
        Self {
            db_name: "mailbox.db".to_string(),
            max_queue_bytes: 5 * 1024 * 1024,
            max_queue_messages: 500,
            max_ttl_secs: 7 * 24 * 3600,
            receipt_keep_secs: 14 * 24 * 3600,
            visibility_secs: 300,
            queue_lifetime_secs: 30 * 24 * 3600,
            max_total_bytes: 1024 * 1024 * 1024,
            max_queues: 10_000,
            deposits_per_minute_per_sender: 60,
            deposits_per_minute_total: 6_000,
            sweep_interval_secs: 60,
            republish_interval_secs: 3_600,
        }
    }
}

//! Mailbox limit validation from role configuration.

use anyhow::bail;
use syneroym_core::{config::MailboxRole, mailbox::QueueLimits};
use syneroym_mailbox_format::{MAX_SEALED_BYTES, OPEN_MAX_TTL_SECS};

/// The limits of this mailbox, checked once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub queue: QueueLimits,
    pub receipt_keep_secs: u64,
    pub visibility_secs: u64,
    pub queue_lifetime_secs: u64,
    pub max_total_bytes: u64,
    pub max_queues: u32,
}

impl Limits {
    pub fn from_role(role: &MailboxRole) -> anyhow::Result<Self> {
        if role.db_name.is_empty()
            || role.db_name.contains('/')
            || role.db_name.contains('\\')
            || role.db_name.contains("..")
        {
            bail!("[roles.mailbox] db_name must not be empty or contain '/', '\\', or '..'");
        }
        if role.max_queue_bytes == 0 {
            bail!("[roles.mailbox] max_queue_bytes must be greater than 0");
        }
        if role.max_queue_messages == 0 {
            bail!("[roles.mailbox] max_queue_messages must be greater than 0");
        }
        if role.max_ttl_secs == 0 {
            bail!("[roles.mailbox] max_ttl_secs must be greater than 0");
        }
        if role.receipt_keep_secs == 0 {
            bail!("[roles.mailbox] receipt_keep_secs must be greater than 0");
        }
        if role.visibility_secs == 0 {
            bail!("[roles.mailbox] visibility_secs must be greater than 0");
        }
        if role.queue_lifetime_secs == 0 {
            bail!("[roles.mailbox] queue_lifetime_secs must be greater than 0");
        }
        if role.max_total_bytes == 0 {
            bail!("[roles.mailbox] max_total_bytes must be greater than 0");
        }
        if role.max_queues == 0 {
            bail!("[roles.mailbox] max_queues must be greater than 0");
        }
        if role.deposits_per_minute_per_sender == 0 {
            bail!("[roles.mailbox] deposits_per_minute_per_sender must be greater than 0");
        }
        if role.deposits_per_minute_total == 0 {
            bail!("[roles.mailbox] deposits_per_minute_total must be greater than 0");
        }
        if role.register_per_minute_per_master == 0 {
            bail!("[roles.mailbox] register_per_minute_per_master must be greater than 0");
        }
        if role.register_per_minute_total == 0 {
            bail!("[roles.mailbox] register_per_minute_total must be greater than 0");
        }
        if role.receiver_calls_per_minute_per_master == 0 {
            bail!("[roles.mailbox] receiver_calls_per_minute_per_master must be greater than 0");
        }
        if role.receiver_calls_per_minute_total == 0 {
            bail!("[roles.mailbox] receiver_calls_per_minute_total must be greater than 0");
        }
        if role.sweep_interval_secs == 0 {
            bail!("[roles.mailbox] sweep_interval_secs must be greater than 0");
        }
        if role.republish_interval_secs == 0 {
            bail!("[roles.mailbox] republish_interval_secs must be greater than 0");
        }
        if role.max_ttl_secs > OPEN_MAX_TTL_SECS {
            bail!(
                "[roles.mailbox] max_ttl_secs ({}) cannot exceed open maximum \
                 ({OPEN_MAX_TTL_SECS} s)",
                role.max_ttl_secs
            );
        }
        if role.max_queue_bytes < MAX_SEALED_BYTES as u64 {
            bail!(
                "[roles.mailbox] max_queue_bytes ({}) must be at least MAX_SEALED_BYTES \
                 ({MAX_SEALED_BYTES} bytes)",
                role.max_queue_bytes
            );
        }
        if role.max_total_bytes < role.max_queue_bytes {
            bail!(
                "[roles.mailbox] max_total_bytes ({}) must be at least max_queue_bytes ({})",
                role.max_total_bytes,
                role.max_queue_bytes
            );
        }
        let min_receiver_total = (role.max_queues as u64).saturating_mul(2);
        if (role.receiver_calls_per_minute_total as u64) < min_receiver_total {
            bail!(
                "[roles.mailbox] receiver_calls_per_minute_total ({}) must be at least 2 × \
                 max_queues ({min_receiver_total})",
                role.receiver_calls_per_minute_total
            );
        }

        Ok(Self {
            queue: QueueLimits {
                max_bytes: role.max_queue_bytes,
                max_messages: role.max_queue_messages,
                max_ttl_secs: role.max_ttl_secs,
            },
            receipt_keep_secs: role.receipt_keep_secs,
            visibility_secs: role.visibility_secs,
            queue_lifetime_secs: role.queue_lifetime_secs,
            max_total_bytes: role.max_total_bytes,
            max_queues: role.max_queues,
        })
    }
}

#[cfg(test)]
mod tests;

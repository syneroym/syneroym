//! Mailbox errors and conversions to RPC errors.

use serde_json::json;
use syneroym_core::mailbox::codes;
use syneroym_rpc::RpcError;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum MailboxError {
    #[error("the caller has no verified identity")]
    Unauthenticated,
    #[error("the caller is not a service instance: {0}")]
    NotAnInstance(String),
    #[error("this mailbox has no queue for {0}")]
    UnknownDestination(String),
    #[error("sealed message is {bytes} bytes, over the {max}-byte maximum")]
    TooLarge { bytes: usize, max: usize },
    #[error("the queue of {0} is full")]
    QueueFull(String),
    #[error("too many deposits; try again later")]
    RateLimited,
    #[error("this mailbox is full")]
    MailboxFull,
    #[error("a newer instance of {0} holds this queue")]
    StaleInstance(String),
    #[error("the service record is not accepted: {0}")]
    BadRecord(String),
    #[error("deposit id {0} was already used by this sender")]
    DuplicateDeposit(String),
    #[error("time-to-live {ttl_secs} s is over this queue's {max_secs} s maximum")]
    TtlTooLong { ttl_secs: u64, max_secs: u64 },
    #[error("invalid parameters: {0}")]
    InvalidParams(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl MailboxError {
    /// The `data.kind` text, snake_case of the variant name.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::NotAnInstance(_) => "not_an_instance",
            Self::UnknownDestination(_) => "unknown_destination",
            Self::TooLarge { .. } => "too_large",
            Self::QueueFull(_) => "queue_full",
            Self::RateLimited => "rate_limited",
            Self::MailboxFull => "mailbox_full",
            Self::StaleInstance(_) => "stale_instance",
            Self::BadRecord(_) => "bad_record",
            Self::DuplicateDeposit(_) => "duplicate_deposit",
            Self::TtlTooLong { .. } => "ttl_too_long",
            Self::InvalidParams(_) => "invalid_params",
            Self::Internal(_) => "internal",
        }
    }
}

impl From<MailboxError> for RpcError {
    fn from(e: MailboxError) -> Self {
        match e {
            MailboxError::InvalidParams(msg) => RpcError::InvalidParams(msg),
            MailboxError::Internal(msg) => RpcError::InternalError(msg),
            MailboxError::TtlTooLong { max_secs, .. } => {
                let code = codes::TTL_TOO_LONG;
                let data = json!({
                    "kind": "ttl_too_long",
                    "max_ttl_secs": max_secs,
                });
                RpcError::Custom(code, "ttl_too_long".to_string(), Some(data))
            }
            MailboxError::Unauthenticated => {
                let code = codes::UNAUTHENTICATED;
                let data = json!({ "kind": "unauthenticated" });
                RpcError::Custom(code, "unauthenticated".to_string(), Some(data))
            }
            MailboxError::NotAnInstance(ref msg) => {
                let code = codes::NOT_AN_INSTANCE;
                let data = json!({ "kind": "not_an_instance" });
                RpcError::Custom(code, msg.clone(), Some(data))
            }
            MailboxError::UnknownDestination(ref d) => {
                let code = codes::UNKNOWN_DESTINATION;
                let data = json!({ "kind": "unknown_destination" });
                RpcError::Custom(code, format!("unknown destination {d}"), Some(data))
            }
            MailboxError::TooLarge { bytes, max } => {
                let code = codes::TOO_LARGE;
                let data = json!({ "kind": "too_large" });
                RpcError::Custom(
                    code,
                    format!("payload is {bytes} bytes; max is {max}"),
                    Some(data),
                )
            }
            MailboxError::QueueFull(ref q) => {
                let code = codes::QUEUE_FULL;
                let data = json!({ "kind": "queue_full" });
                RpcError::Custom(code, format!("queue is full for {q}"), Some(data))
            }
            MailboxError::RateLimited => {
                let code = codes::RATE_LIMITED;
                let data = json!({ "kind": "rate_limited" });
                RpcError::Custom(code, "rate limit exceeded".to_string(), Some(data))
            }
            MailboxError::MailboxFull => {
                let code = codes::MAILBOX_FULL;
                let data = json!({ "kind": "mailbox_full" });
                RpcError::Custom(code, "mailbox storage is full".to_string(), Some(data))
            }
            MailboxError::StaleInstance(ref d) => {
                let code = codes::STALE_INSTANCE;
                let data = json!({ "kind": "stale_instance" });
                RpcError::Custom(code, format!("instance certificate is stale for {d}"), Some(data))
            }
            MailboxError::BadRecord(ref msg) => {
                let code = codes::BAD_RECORD;
                let data = json!({ "kind": "bad_record" });
                RpcError::Custom(code, msg.clone(), Some(data))
            }
            MailboxError::DuplicateDeposit(ref id) => {
                let code = codes::DUPLICATE_DEPOSIT;
                let data = json!({ "kind": "duplicate_deposit" });
                RpcError::Custom(code, format!("deposit {id} already exists"), Some(data))
            }
        }
    }
}

impl From<rusqlite::Error> for MailboxError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

#[cfg(test)]
mod tests;

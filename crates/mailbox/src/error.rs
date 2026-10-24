//! Mailbox errors and conversions to RPC errors.

use serde_json::json;
use syneroym_core::mailbox::codes;
use syneroym_passport::RegisterError;
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
    #[error("the passport is refused ({code}): {reason}")]
    PassportRefused { code: &'static str, reason: String },
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
            Self::PassportRefused { .. } => "passport_refused",
            Self::InvalidParams(_) => "invalid_params",
            Self::Internal(_) => "internal",
        }
    }
}

fn code_of(e: &MailboxError) -> i32 {
    match e {
        MailboxError::Unauthenticated => codes::UNAUTHENTICATED,
        MailboxError::NotAnInstance(_) => codes::NOT_AN_INSTANCE,
        MailboxError::UnknownDestination(_) => codes::UNKNOWN_DESTINATION,
        MailboxError::TooLarge { .. } => codes::TOO_LARGE,
        MailboxError::QueueFull(_) => codes::QUEUE_FULL,
        MailboxError::RateLimited => codes::RATE_LIMITED,
        MailboxError::MailboxFull => codes::MAILBOX_FULL,
        MailboxError::StaleInstance(_) => codes::STALE_INSTANCE,
        MailboxError::BadRecord(_) => codes::BAD_RECORD,
        MailboxError::DuplicateDeposit(_) => codes::DUPLICATE_DEPOSIT,
        MailboxError::TtlTooLong { .. } => codes::TTL_TOO_LONG,
        MailboxError::PassportRefused { .. } => codes::PASSPORT_REFUSED,
        MailboxError::InvalidParams(_) => -32602,
        MailboxError::Internal(_) => -32603,
    }
}

impl From<MailboxError> for RpcError {
    fn from(e: MailboxError) -> Self {
        match e {
            MailboxError::InvalidParams(msg) => RpcError::InvalidParams(msg),
            MailboxError::Internal(msg) => RpcError::InternalError(msg),
            MailboxError::TtlTooLong { max_secs, .. } => {
                let code = code_of(&e);
                let data = json!({
                    "kind": e.kind(),
                    "max_ttl_secs": max_secs,
                });
                RpcError::Custom(code, e.to_string(), Some(data))
            }
            MailboxError::PassportRefused { code: refusal, .. } => {
                let code = code_of(&e);
                let data = json!({
                    "kind": e.kind(),
                    "code": refusal,
                });
                RpcError::Custom(code, e.to_string(), Some(data))
            }
            other => {
                let code = code_of(&other);
                let data = json!({ "kind": other.kind() });
                RpcError::Custom(code, other.to_string(), Some(data))
            }
        }
    }
}

impl From<RegisterError> for MailboxError {
    /// `RegisterError` is not `Clone`, so the variant keeps its code and text.
    fn from(e: RegisterError) -> Self {
        match e {
            RegisterError::Store(msg) => Self::Internal(msg),
            other => Self::PassportRefused { code: other.code(), reason: other.to_string() },
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

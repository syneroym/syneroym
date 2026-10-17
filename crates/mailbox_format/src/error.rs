//! Error types of the crate. No message here contains message content.

use crate::{message::RefusedMessage, receipt::RejectCode};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SealError {
    #[error("time-to-live is zero")]
    ZeroTtl,
    #[error("destination is not a did:key: {0}")]
    NotADidKey(String),
    #[error("the instance key does not match the certificate")]
    SignerMismatch,
    #[error("the sender certificate is not valid now: {0}")]
    BadCertificate(String),
    #[error("sealed message is {bytes} bytes, over the {max}-byte maximum")]
    TooLarge { bytes: usize, max: usize },
    #[error("no random bytes: {0}")]
    Random(String),
    #[error("json: {0}")]
    Json(String),
    #[error("sealing failed: {0}")]
    Crypto(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OpenError {
    // The first four are found before the message signature matches the key
    // of its certificate, so the sender is not proven: the ack is unsigned.
    #[error("sealed message is {bytes} bytes, over the {max}-byte maximum")]
    TooLarge { bytes: usize, max: usize },
    #[error("the sealed message does not open with this key")]
    Undecryptable,
    #[error("the opened message is malformed: {0}")]
    Malformed(String),
    #[error("the message signature does not match the sender certificate")]
    BadSignature,
    // Signed by the key its certificate names, but the certificate or the
    // send time failed. The ack is signed only for a sender-caused code.
    #[error("refused ({}): {}", .0.code.as_str(), .0.detail)]
    Refused(Box<RefusedMessage>),
}

impl OpenError {
    /// The code for the ack. Use `Receipt::unsigned_rejection` with it for
    /// every variant except `Refused`, whose `receipt` method decides.
    #[must_use]
    pub fn reject_code(&self) -> RejectCode {
        match self {
            Self::TooLarge { .. } => RejectCode::TooLarge,
            Self::Undecryptable => RejectCode::Undecryptable,
            Self::Malformed(_) => RejectCode::Malformed,
            Self::BadSignature => RejectCode::BadSignature,
            Self::Refused(refused) => refused.code,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReceiptError {
    #[error("the receipt carries no receiver signature")]
    Unsigned,
    /// A signed rejection with a code that is not sender-caused, or unknown.
    #[error("a rejection with this code is not signed or final")]
    NotSignable,
    #[error("the receipt is signed for another service")]
    WrongReceiver,
    #[error("the receipt signature does not match the sent message")]
    BadSignature,
    #[error("the receiver certificate is not valid: {0}")]
    BadCertificate(String),
    #[error("the receipt time is in the future")]
    FromFuture,
    #[error("the receipt is older than the message it answers")]
    BeforeSend,
    #[error("the instance key does not match the certificate")]
    SignerMismatch,
    #[error("json: {0}")]
    Json(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DepositError {
    #[error("destination is not a did:key: {0}")]
    NotADidKey(String),
    #[error("time-to-live is zero")]
    ZeroTtl,
    #[error("time-to-live {ttl_secs} s is over the {max_secs} s maximum")]
    TtlTooLong { ttl_secs: u64, max_secs: u64 },
    #[error("sealed message is {bytes} bytes, over the {max}-byte maximum")]
    TooLarge { bytes: usize, max: usize },
    #[error("sealed message is {bytes} bytes, under the {min}-byte minimum")]
    TooSmall { bytes: usize, min: usize },
}

#[cfg(test)]
mod tests;

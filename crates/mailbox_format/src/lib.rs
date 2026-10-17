#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! The formats a service mailbox, a sender and a receiver share (ADR-0026):
//! the mailbox key pair of a service member, the deposit (an outer part the
//! mailbox reads and an inner part sealed for the receiver), and the receipt
//! the sender reads back.
//!
//! Every function here is pure: the caller passes the time and does any
//! network lookup. Revocation of a sender's instance key is not checked
//! here; the receiver checks it against the master's anchor after `open`.

mod crypto;
mod deposit;
mod error;
mod keys;
mod message;
mod receipt;

pub use deposit::{Deposit, DepositId, DepositIdParseError, SealedBytes, SealedBytesParseError};
pub use error::{DepositError, OpenError, ReceiptError, SealError};
pub use keys::{KeyParseError, MailboxPublicKey, MailboxSecretKey};
pub use message::{
    MessageContent, MessageHash, MessageHashParseError, OpenPolicy, OpenedMessage, Recipient,
    RefusedMessage, Sealed, Sender, SentDeposit, open, seal,
};
pub use receipt::{Receipt, ReceiverProof, RejectCode, SignedOutcome};

/// The HPKE `info` input: domain separation for every sealed message.
pub const HPKE_INFO: &[u8] = b"syneroym-mailbox-v1";
/// Domain tag of the sender's signature over a message.
pub const MESSAGE_DOMAIN: &str = "syneroym-mailbox-message-v1";
/// Domain tag of the receiver's signature over a receipt.
pub const RECEIPT_DOMAIN: &str = "syneroym-mailbox-receipt-v1";
/// Largest sealed message, in raw bytes before base64.
pub const MAX_SEALED_BYTES: usize = 64 * 1024;
/// Longest time-to-live of an open mailbox. A dedicated mailbox may allow
/// more; the receiver then passes its own limit in `OpenPolicy`.
pub const OPEN_MAX_TTL_SECS: u64 = 7 * 24 * 3600;
/// Clock difference allowed between two nodes, the same as Conversation.
pub const MAX_CLOCK_SKEW_SECS: u64 = 24 * 3600;

#[cfg(test)]
mod test_support;

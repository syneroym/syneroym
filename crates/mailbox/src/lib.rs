#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! The service mailbox (ADR-0026): a native service that holds sealed
//! messages for services that cannot be reached, and gives them to the
//! newest instance of the receiver. It reads only the outer part of a
//! message; the inner part is sealed for the receiver.

mod caller;
mod error;
mod limits;
mod record;
mod service;
mod store;
mod worker;

pub use error::MailboxError;
pub use limits::Limits;
pub use service::MailboxService;
pub use worker::MailboxWorker;

#[cfg(test)]
mod test_support;

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Cryptographic identity management for Syneroym nodes.

pub mod delegation;
pub mod substrate;

#[cfg(feature = "backup")]
pub mod backup;

mod document;
mod keys;

pub use delegation::DelegationCertificate;
pub use document::IdentityDoc;
pub use keys::{Identity, lock_memory};

//! Cryptographic identity management for Syneroym nodes.

pub mod substrate;

mod document;
mod keys;

pub use document::IdentityDoc;
pub use keys::Identity;

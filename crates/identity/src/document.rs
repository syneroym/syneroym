//! Decentralized Identity Documents (DIDs)
//!
//! Defines standard `IdentityDoc` models and metadata representation
//! for nodes in the Syneroym ecosystem.

use serde::{Deserialize, Serialize};

/// A serialized representation of the node's public identity document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityDoc {
    pub id: String,
    pub pubkey_hex: String,
    pub created_at: u64,
}

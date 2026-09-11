//! Community Registry Client and Types
//!
//! Provides structures and client methods for registering, querying, and
//! resolving service/substrate endpoints in the Syneroym community registry.

pub mod client;
pub mod master_anchor;
pub mod types;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use std::time;

#[cfg(test)]
pub(crate) use bytes::Bytes;
#[cfg(test)]
pub(crate) use client::extract_verified_endpoint_from_packet;
pub use client::*;
pub use master_anchor::*;
#[cfg(test)]
pub(crate) use pkarr::{
    Keypair, PublicKey, SignedPacket, Timestamp,
    dns::{
        CLASS, Name, ResourceRecord,
        rdata::{RData, TXT},
    },
};
#[cfg(test)]
pub(crate) use syneroym_identity::substrate;
pub use types::*;

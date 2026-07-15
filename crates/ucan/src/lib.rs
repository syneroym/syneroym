#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! UCAN capability model (ADR-0015): resource/ability/capability types, the
//! signed `CapabilityToken` delegation chain and its verification, and the
//! verified `SessionContext` a chain resolves into.

mod capability;
mod normalize;
mod session;
mod token;

pub use capability::{Ability, Capability, ResourceUri};
pub use normalize::{AuthNormalizer, DidKeyNormalizer};
pub use session::SessionContext;
pub use token::{CapabilityToken, ChainVerifyOpts, verify_chain};

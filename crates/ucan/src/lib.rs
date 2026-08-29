#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! UCAN capability model (ADR-0015): resource/ability/capability types, the
//! signed `CapabilityToken` delegation chain and its verification, and the
//! verified `SessionContext` a chain resolves into.

mod capability;
mod session;
mod session_token;
mod token;

pub use capability::{Ability, Capability, ResourceUri};
pub use session::SessionContext;
pub use session_token::{
    AUTH_METHOD_DELEGATED_KEY, AUTH_METHOD_FIXED, AUTH_METHOD_LOCAL, CLAIM_AUTH_METHOD,
    CLAIM_DELEGATION_EXPIRES_AT_SECS, SessionToken, SessionTokenClaims,
};
pub use token::{CapabilityToken, ChainVerifyOpts, verify_chain};

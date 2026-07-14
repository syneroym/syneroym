#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! UCAN capability model (ADR-0015): resource/ability/capability types and
//! the verified `SessionContext` they resolve into.
//!
//! `CapabilityToken`, `issue`, and `verify_chain` — the signed delegation
//! chain and its verification — are deferred to Slice B1; this crate ships
//! only the capability type model and pure entailment/attenuation logic for
//! B0.

mod capability;
mod session;

pub use capability::{Ability, Capability, ResourceUri};
pub use session::SessionContext;

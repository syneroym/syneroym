#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Operator passports (ADR-0015 A10): a signed, time-limited token that
//! admits a user to an operator's service (a home relay or a dedicated
//! mailbox). This crate holds the parts both uses share: issuing, the
//! registration checks, the challenge store, the lease table with its deny
//! list, and the request limiter. A service adds only its own HTTP or RPC
//! surface and how it enforces a lease.

use std::time::{SystemTime, UNIX_EPOCH};

mod chain;
mod challenge_store;
mod error;
mod issue;
mod lease_store;
mod limiter;
mod passport_use;
mod presented;
mod proof;
mod registrar;
pub mod wire;

pub use challenge_store::{CHALLENGE_LIFE_SECS, ChallengeStore, MAX_CHALLENGES};
pub use error::RegisterError;
pub use issue::{PassportId, child_token, issue_passport};
pub use lease_store::{Lease, LeaseStore};
pub use limiter::{REQUESTS_PER_MINUTE, RequestLimiter};
pub use passport_use::{MAILBOX_USE, PassportUse, RELAY_HOME, facts};
pub use presented::{PresentedToken, inspect_child_token};
pub use proof::{
    CHALLENGE_DOMAIN, Challenge, Proof, SignerBinding, presentation_hash, sign_challenge,
};
pub use registrar::{CheckedRegistration, Registrar};
pub use syneroym_ucan::CapabilityToken;

/// Unix seconds from the wall clock, for the `now_secs` argument of every
/// call in this crate. Saturates to 0 before the epoch instead of failing.
#[must_use]
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod test_support;

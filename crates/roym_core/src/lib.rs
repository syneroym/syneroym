#![cfg_attr(
    any(test, feature = "test-host"),
    allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
//! `syneroym-roym-core`: shared service constants, envelopes, method router,
//! card definitions, data-layer paging, and dual-build helpers for the Roym
//! SynApp.

pub mod admit;
pub mod area;
pub mod backup;
pub mod card;
pub mod clock;
pub mod conversation;
pub mod deal;
pub mod directory;
pub mod disclosure;
pub mod dual_build;
pub mod envelope;
pub mod extension;
pub mod fulfilment;
pub mod lang;
pub mod listing;
pub mod membership;
pub mod method;
pub mod money;
pub mod paging;
pub mod payment;
pub mod person;
pub mod presentation;
pub mod progress;
pub mod record;
pub mod router;
pub mod safety;
pub mod services;
pub mod signing;
pub mod step;
pub mod taxonomy;
#[cfg(any(test, feature = "test-host"))]
pub mod testing;
pub mod transaction;
pub mod verdict;

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `syneroym-roym-core`: shared service constants, envelopes, method router,
//! card definitions, and dual-build helpers for the Roym SynApp.

pub mod backup;
pub mod card;
pub mod clock;
pub mod dual_build;
pub mod envelope;
pub mod person;
pub mod record;
pub mod router;
pub mod safety;
pub mod services;
pub mod signing;

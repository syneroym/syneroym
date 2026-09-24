#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration suite driving the Roym SynApp through both builds -- the real
//! `wasm32-wasip2` components via `AppSandboxEngine`, and the same sources
//! linked in via `syneroym-app-host-native` -- and asserting the results
//! are identical across all scenarios.

#[path = "dual_build_parity/fixtures.rs"]
pub(crate) mod fixtures;
#[path = "dual_build_parity/helpers.rs"]
pub(crate) mod helpers;

#[path = "dual_build_parity/booking.rs"]
mod booking;
#[path = "dual_build_parity/bundles.rs"]
mod bundles;
#[path = "dual_build_parity/catalog.rs"]
mod catalog;
#[path = "dual_build_parity/conversation.rs"]
mod conversation;
#[path = "dual_build_parity/directory.rs"]
mod directory;
#[path = "dual_build_parity/fulfilment.rs"]
mod fulfilment;
#[path = "dual_build_parity/payment.rs"]
mod payment;
#[path = "dual_build_parity/profile.rs"]
mod profile;
#[path = "dual_build_parity/transaction.rs"]
mod transaction;
#[path = "dual_build_parity/transaction_cards.rs"]
mod transaction_cards;
#[path = "dual_build_parity/wire_origin.rs"]
mod wire_origin;

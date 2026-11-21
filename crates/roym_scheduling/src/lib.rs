#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `syneroym-roym-scheduling`: the scheduling extension's service. It holds
//! the availability slots and the reservations that the deal core takes on
//! them.

pub mod api;
pub mod app;
#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;

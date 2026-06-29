#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! This module generates Rust types from the `control-plane.wit` interface.

pub mod control_plane;

#[cfg(not(target_arch = "wasm32"))]
pub mod host;

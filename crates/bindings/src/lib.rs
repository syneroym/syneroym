#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! This module generates Rust types from the `control-plane.wit` interface.

pub mod control_plane;
pub mod host;

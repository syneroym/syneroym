#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! This module generates Rust types from the WIT interfaces including
//! control-plane, data-layer, and vault.

pub mod app_config;
pub mod control_plane;
pub mod data_layer;
pub mod vault;

#[cfg(not(target_arch = "wasm32"))]
pub mod host;

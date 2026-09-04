#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! This module generates Rust types from the WIT interfaces including
//! control-plane, data-layer, and vault.

#[cfg(feature = "app-config")]
pub mod app_config;
#[cfg(feature = "blob-store")]
pub mod blob_store;
#[cfg(feature = "control-plane")]
pub mod control_plane;
#[cfg(feature = "conversation")]
pub mod conversation;
#[cfg(feature = "data-layer")]
pub mod data_layer;
#[cfg(feature = "http")]
pub mod http_guest;
#[cfg(feature = "invocation")]
pub mod invocation;
#[cfg(feature = "messaging")]
pub mod messaging;
#[cfg(feature = "proxy")]
pub mod proxy;
#[cfg(feature = "signing")]
pub mod signing;
#[cfg(feature = "supervisor")]
pub mod supervisor;
#[cfg(feature = "vault")]
pub mod vault;

#[cfg(not(target_arch = "wasm32"))]
pub mod host;

#[cfg(not(target_arch = "wasm32"))]
pub mod http_host;

#[cfg(not(target_arch = "wasm32"))]
pub mod conversation_host;

#[cfg(not(target_arch = "wasm32"))]
pub mod signing_host;

#[cfg(not(target_arch = "wasm32"))]
pub mod invocation_host;

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `syneroym-roym-conversation`: conversation service skeleton.

pub mod app;
#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! `syneroym-test-dual-build-fixture`: one source tree, built two ways --
//! as a `wasm32-wasip2` component and linked into `syneroym-substrate`.
//! `app.rs` is the fixture's entire behaviour, target-independent; `guest.rs`
//! and `native.rs` are the two build-specific wirings around it.

pub mod app;
#[cfg(target_arch = "wasm32")]
mod guest;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;

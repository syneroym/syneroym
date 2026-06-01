#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Podman container execution engine
//!
//! Handles lifecycle of Podman containers using std::process::Command.

pub mod engine;
pub use engine::ContainerEngine;

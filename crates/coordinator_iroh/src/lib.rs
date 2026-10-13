#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Iroh transport coordinator component.

pub mod config;
pub mod coordinator;
pub mod info_endpoint;
pub mod registration;

pub use coordinator::CoordinatorIroh;

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Iroh transport coordinator component.

mod config;
mod coordinator;
pub mod info_endpoint;

pub use coordinator::CoordinatorIroh;
pub use info_endpoint::CoordinatorInfo;

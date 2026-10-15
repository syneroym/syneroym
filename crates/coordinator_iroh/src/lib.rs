#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Iroh transport coordinator component.

mod config;
mod coordinator;
mod info_endpoint;
mod passport;
mod registration;

pub use coordinator::CoordinatorIroh;

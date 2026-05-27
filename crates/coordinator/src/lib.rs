#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Coordinator that helps peers within ecosystem discover a channel to communicate and often help relay data.

mod coordinator;

pub use coordinator::EcosystemCoordinator;

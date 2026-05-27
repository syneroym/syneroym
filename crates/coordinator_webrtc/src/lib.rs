#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! WebRTC transport coordinator component.

pub mod bootstrap;
mod coordinator;
pub mod signalling;

pub use coordinator::CoordinatorWebRtc;

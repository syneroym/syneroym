#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Application sandbox engine for isolating user applications.

pub mod conversions;
mod engine;
mod stream;

pub use engine::{
    AppSandboxEngine, HostState, MessagingContext, StreamRequestOutcome, WasmResourceQuota,
};
pub use stream::{GuestStreamCursor, GuestStreamSink, StreamContext, StreamRegistry};

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Application sandbox engine for isolating user applications.

pub mod conversions;
mod engine;
mod host_capabilities;
mod stream;

pub use engine::{AppSandboxEngine, StreamRequestOutcome, WasmResourceQuota};
pub use host_capabilities::{HostState, MessagingContext, empty_service_proxy};
pub use stream::{GuestStreamCursor, GuestStreamSink, StreamContext, StreamRegistry};

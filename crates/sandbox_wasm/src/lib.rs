#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Application sandbox engine for isolating user applications.

pub mod conversions;
mod engine;
mod host_capabilities;
mod http;
mod stream;

pub use engine::{
    AppSandboxEngine, FrameKind, GuestHttpFailure, GuestHttpOutcome, StreamRequestOutcome,
    WasmResourceQuota, WebSocketReceiver, WebSocketSender,
};
pub use host_capabilities::{HostState, InvocationOrigin, MessagingContext, empty_service_proxy};
pub use stream::{GuestStreamCursor, GuestStreamSink, StreamContext, StreamRegistry};

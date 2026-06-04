//! Native service execution bridge
//!
//! Provides the core abstraction and types for in-process, native Rust
//! services, permitting local request dispatching within the substrate.

use std::fmt::Debug;

use serde_json::Value;

use crate::RpcResult;

/// Represents a parsed and validated request ready for dispatch to a native
/// service.
#[derive(Debug)]
pub struct NativeInvocation {
    pub interface: String,
    pub method: String,
    pub params: Value,
}

/// Represents a response from a native service.
#[derive(Debug)]
pub struct NativeResponse {
    pub payload: Value,
}

#[async_trait::async_trait]
pub trait NativeService: Send + Sync + Debug {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse>;
}

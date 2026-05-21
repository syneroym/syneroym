//! Native wRPC client & server bridge
//!
//! Adapts native wRPC traits to async stream readers/writers, permitting
//! wRPC calls over raw TCP/IP or Unix Socket streams.

use serde_json::Value;
use std::fmt::Debug;

use crate::RpcResult;

/// Represents a parsed and validated request ready for dispatch to a native service.
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

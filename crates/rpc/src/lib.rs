#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Syneroym RPC framing library
//!
//! Core module for RPC protocol compilation, providing framing,
//! serialization, type conversions, and transport adapters.

use std::result;

mod converter;
mod dispatch_registry;
pub mod framing;
mod native;
mod proxy;
mod types;

pub use converter::JsonRpcConverter;
pub use dispatch_registry::{NativeDispatchRegistry, WeakNativeDispatchRegistry};
pub use native::{
    AuthLevel, CallerContext, CallerProof, NativeInvocation, NativeResponse, NativeService,
};
pub use proxy::{
    CallOrigin, DEFAULT_PROXY_CALL_TIMEOUT, PROXY_TRANSPORT_RPC_CODE, ProxyError, ProxyProtocol,
    ProxyRequest, ServiceProxy, UNSUPPORTED_PROTOCOL_RPC_CODE, UNSUPPORTED_TARGET_RPC_CODE,
};
use serde_json::Value;
pub use syneroym_ucan::{
    Ability, Capability, CapabilityToken, ChainVerifyOpts, ResourceUri, SessionContext,
    verify_chain,
};
use thiserror::Error;
pub use types::{
    JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse, MESSAGING_MESSAGE_METHOD,
    MessagingNotification,
};

#[derive(Error, Debug)]
pub enum RpcError {
    #[error("Method not found: {0}")]
    MethodNotFound(String),
    #[error("Invalid parameters: {0}")]
    InvalidParams(String),
    #[error("Internal error: {0}")]
    InternalError(String),
    #[error("{1}")]
    Custom(i32, String, Option<Value>),
}

impl RpcError {
    #[must_use]
    pub const fn code(&self) -> i32 {
        match self {
            Self::MethodNotFound(_) => -32601,
            Self::InvalidParams(_) => -32602,
            Self::InternalError(_) => -32603,
            Self::Custom(code, _, _) => *code,
        }
    }

    #[must_use]
    pub fn data(&self) -> Option<Value> {
        match self {
            Self::Custom(_, _, data) => data.clone(),
            _ => None,
        }
    }
}

pub type RpcResult<T> = result::Result<T, RpcError>;

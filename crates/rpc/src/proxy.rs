//! Transport-agnostic service-proxy contract (M04A Slice A1).
//!
//! [`ServiceProxy`] is the Universal Proxy's outbound-call interface: a typed
//! `(service, interface, method, params)` call routed to a local native
//! service, a local WASM component, or a remote node over Iroh QUIC +
//! JSON-RPC. `syneroym-router`'s `ProxyRouter` is the only implementation;
//! this crate only defines the contract, so both `router` (the impl) and
//! `sandbox-wasm` (the guest-facing host function, which needs the trait
//! object without depending on `router`) can share it.

use std::{fmt::Debug, time::Duration};

use serde_json::Value;

use crate::CallerContext;

/// Reserved wire tag (A.5/A.7): only `JsonRpcV1` exists in M4. A future wRPC
/// wire adds a variant here plus a `RemoteHop` impl in `syneroym-router` --
/// no other type changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProxyProtocol {
    #[default]
    JsonRpcV1,
}

impl ProxyProtocol {
    pub const JSON_RPC_V1_TAG: &'static str = "json-rpc/v1";

    /// `None`/`"json-rpc/v1"` decode to [`Self::JsonRpcV1`]; anything else is
    /// `Err(tag)` -- the caller turns that into
    /// [`ProxyError::UnsupportedProtocol`] (the minimal `[LFC-VER]` behavior
    /// kept from the deferred protocol-negotiation slice, A.7).
    pub fn parse(tag: Option<&str>) -> Result<Self, String> {
        match tag {
            None | Some(Self::JSON_RPC_V1_TAG) => Ok(Self::JsonRpcV1),
            Some(other) => Err(other.to_string()),
        }
    }
}

/// Who originated a proxy call. **Host-set, never guest-settable**: the guest
/// host function (`syneroym-sandbox-wasm`'s `proxy::Host::call`) always
/// constructs `Guest`, so a component cannot claim `Native` to slip past the
/// guest native-capability gate (`ProxyRouter::check_native_capability_gate`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallOrigin {
    /// A WASM component calling out through `syneroym:proxy`. `service_id` is
    /// the guest's own raw component id -- not the `"system:"`-prefixed
    /// synthetic DID `CallerContext::service_system` puts in `caller_did`.
    Guest { service_id: String },
    /// Substrate-internal: the FDAE policy engine's relationship-proof fetch
    /// (M04B B3), control-plane internals, tests. Not subject to the guest
    /// native-capability gate -- enforcement for these lives at the
    /// data-owning node (ADR-0016 §6; M04B "enforce at the data-owning
    /// node").
    Native,
}

/// A cross-service call. Locally constructed; `caller` is **never**
/// wire-serialized (ADR-0016 §6) -- only `caller.proof` (signed material)
/// crosses a hop, and the destination re-verifies it and builds a fresh
/// `CallerContext`.
#[derive(Clone, Debug)]
pub struct ProxyRequest {
    pub target_service: String,
    pub interface: String,
    pub method: String,
    pub params: Value,
    pub caller: CallerContext,
    pub origin: CallOrigin,
    pub protocol: ProxyProtocol,
    /// Retry eligibility. Transport failures are retried with backoff only
    /// when `true`; a callee-returned error is never retried. Failed-after-
    /// retries fails directly -- no queueing (DLQ is M5).
    pub idempotent: bool,
    /// Per-call deadline. `None` uses [`DEFAULT_PROXY_CALL_TIMEOUT`].
    pub timeout: Option<Duration>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("unknown service: {0}")]
    ServiceNotFound(String),
    #[error("unsupported protocol '{0}' (this node speaks json-rpc/v1)")]
    UnsupportedProtocol(String),
    #[error("target endpoint kind is not callable over the proxy: {0}")]
    UnsupportedTarget(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("transport error: {0}")]
    Transport(String),
    /// The callee answered with a JSON-RPC error -- definitive, never
    /// retried.
    #[error("callee error {code}: {message}")]
    Callee { code: i32, message: String, data: Option<Value> },
    #[error("timed out after {0:?}")]
    Timeout(Duration),
    #[error("internal proxy error: {0}")]
    Internal(String),
}

impl ProxyError {
    /// JSON-RPC code for surfacing this over a wire/HTTP boundary.
    #[must_use]
    pub fn code(&self) -> i32 {
        match self {
            Self::ServiceNotFound(_) => -32601,
            Self::UnsupportedProtocol(_) => UNSUPPORTED_PROTOCOL_RPC_CODE,
            Self::UnsupportedTarget(_) => UNSUPPORTED_TARGET_RPC_CODE,
            Self::PermissionDenied(_) => -32010, // same shape as data-layer denial
            Self::Transport(_) | Self::Timeout(_) => PROXY_TRANSPORT_RPC_CODE,
            Self::Callee { code, .. } => *code,
            Self::Internal(_) => -32603,
        }
    }
}

/// Reserved JSON-RPC error code for a caller declaring a protocol scheme this
/// node does not speak (the minimal `[LFC-VER]` behavior kept from the
/// deferred protocol-negotiation slice, A.7).
pub const UNSUPPORTED_PROTOCOL_RPC_CODE: i32 = -32091;
/// Reserved JSON-RPC error code for a proxy transport failure (connect
/// failure, malformed response, or exhausted retries).
pub const PROXY_TRANSPORT_RPC_CODE: i32 = -32092;
/// Reserved JSON-RPC error code for a proxy target endpoint kind that isn't
/// callable over the proxy (e.g. a TCP/Podman passthrough target -- Flag F4).
pub const UNSUPPORTED_TARGET_RPC_CODE: i32 = -32093;
/// Default per-call deadline when [`ProxyRequest::timeout`] is `None`.
pub const DEFAULT_PROXY_CALL_TIMEOUT: Duration = Duration::from_secs(30);

#[async_trait::async_trait]
pub trait ServiceProxy: Send + Sync + Debug {
    /// Returns the callee's JSON-RPC `result` value on success.
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_parse_accepts_none_and_the_reserved_tag() {
        assert_eq!(ProxyProtocol::parse(None), Ok(ProxyProtocol::JsonRpcV1));
        assert_eq!(ProxyProtocol::parse(Some("json-rpc/v1")), Ok(ProxyProtocol::JsonRpcV1));
    }

    #[test]
    fn protocol_parse_rejects_unknown_tag() {
        assert_eq!(ProxyProtocol::parse(Some("wrpc")), Err("wrpc".to_string()));
    }

    #[test]
    fn error_code_mapping() {
        assert_eq!(ProxyError::ServiceNotFound("x".into()).code(), -32601);
        assert_eq!(ProxyError::UnsupportedProtocol("x".into()).code(), -32091);
        assert_eq!(ProxyError::UnsupportedTarget("x".into()).code(), -32093);
        assert_eq!(ProxyError::PermissionDenied("x".into()).code(), -32010);
        assert_eq!(ProxyError::Transport("x".into()).code(), -32092);
        assert_eq!(ProxyError::Timeout(Duration::from_secs(1)).code(), -32092);
        assert_eq!(
            ProxyError::Callee { code: -32010, message: "x".into(), data: None }.code(),
            -32010
        );
        assert_eq!(ProxyError::Internal("x".into()).code(), -32603);
    }
}

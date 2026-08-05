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

/// Reserved wire tag: only `JsonRpcV1` exists today. A future wRPC wire
/// adds a variant here plus a `RemoteHop` impl in `syneroym-router` -- no
/// other type changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProxyProtocol {
    #[default]
    JsonRpcV1,
}

impl ProxyProtocol {
    pub const JSON_RPC_V1_TAG: &'static str = "json-rpc/v1";

    /// `None`/`"json-rpc/v1"` decode to [`Self::JsonRpcV1`]; anything else
    /// is `Err(tag)` -- the caller turns that into
    /// [`ProxyError::UnsupportedProtocol`]. Protocol negotiation does not
    /// exist: a node either speaks the tag or refuses it.
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
    Native {
        /// The deployed service this call is made *on behalf of*, when there
        /// is one -- the FDAE relationship-proof fetch, which must travel as
        /// that service rather than as the node. `None` for node-level
        /// internals and tests.
        service_id: Option<String>,
    },
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
    /// Retry eligibility asserted by the caller. Transport failures are
    /// retried with backoff only when `true` (or when
    /// [`Self::idempotency_key`] is set, which is a strictly stronger
    /// fence); a callee-returned error is never retried.
    pub idempotent: bool,
    /// The receiver-side fence for at-least-once delivery (ADR-0023 §4).
    /// Present means the receiving node records the call's first outcome
    /// under `(caller, key)` and answers any duplicate from that record,
    /// which is what makes a retry -- and a dead letter's later replay --
    /// safe. Absent means the call has no fence, so it is never queued for
    /// redelivery and never written to a dead-letter table: the caller is
    /// alive and holding the error, and there is nothing safe to replay.
    pub idempotency_key: Option<String>,
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

/// What a queued call is addressed to, stored as the caller *named* it.
///
/// A dependency is deliberately not resolved before storage: resolution
/// happens host-side on every attempt precisely so a caller can never
/// snapshot the resolved DID past a re-pushed binding (ADR-0021 §2), and a
/// queued call that stored the resolved DID would snapshot exactly that --
/// for hours, which is longer than any caller could manage on its own.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum QueuedTarget {
    /// A declared dependency name, re-resolved at each delivery attempt.
    Dependency(String),
    /// A DID (or registry alias) the caller named directly.
    Service(String),
}

/// One durable, fire-and-forget call, as it waits in a service's outbox.
///
/// Carries everything a delivery attempt needs long after the calling
/// component instance that produced it is gone: `app_instance_id` because
/// dependency resolution is scoped to an app instance and the host reads it
/// from the live instance rather than from the caller, and
/// `caller_service_id` because the identity the call travels under has to be
/// rebuilt identically to the live path or authorization at the receiver
/// would silently diverge between the two.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct QueuedCall {
    pub app_instance_id: Option<String>,
    pub caller_service_id: String,
    pub target: QueuedTarget,
    pub routing_key: Option<String>,
    pub interface: String,
    pub method: String,
    pub params: Value,
    /// Always present: a queued call with no fence could not be retried
    /// safely, let alone replayed out of a dead-letter table, so
    /// `enqueue` refuses one outright rather than storing it.
    pub idempotency_key: String,
    /// [`ProxyProtocol`]'s wire tag, stored as text so the payload does not
    /// depend on a Rust enum's shape.
    pub protocol: Option<String>,
    pub timeout_ms: Option<u64>,
}

/// One queued call, as an operator listing shows it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct QueuedCallInfo {
    pub id: u64,
    pub idempotency_key: String,
    pub attempts: u32,
}

/// One terminally failed queued call, as an operator listing shows it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeadLetterInfo {
    pub id: u64,
    pub idempotency_key: String,
    pub attempts: u32,
    pub last_error: String,
    pub created_at: i64,
}

/// Read-and-replay access to a service's durable proxy queues, for the
/// operator verbs.
///
/// Defined here rather than on the outbox itself because the control plane
/// exposes those verbs and the router owns the queues, and the dependency
/// runs router -> control-plane. Same shape and same reason as
/// [`ServiceProxy`]: the contract lives in the crate both sides already
/// share.
#[async_trait::async_trait]
pub trait ProxyQueueInspector: Send + Sync + Debug {
    async fn queued_calls(&self, service_id: &str) -> Result<Vec<QueuedCallInfo>, String>;
    async fn dead_letters(&self, service_id: &str) -> Result<Vec<DeadLetterInfo>, String>;
    /// Re-enqueues a dead letter. Never executes inline.
    async fn replay_dead_letter(&self, service_id: &str, id: u64) -> Result<(), String>;
}

#[async_trait::async_trait]
pub trait ServiceProxy: Send + Sync + Debug {
    /// Returns the callee's JSON-RPC `result` value on success.
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError>;

    /// Hands a call to the calling service's durable outbox: delivery
    /// survives an unreachable target and a process restart, and the caller
    /// never sees the outcome.
    ///
    /// Try-then-queue: a reachable target costs one call and zero queue
    /// writes, and only a transport failure puts the item on disk. The
    /// default body refuses -- a proxy with no per-service storage behind
    /// it has nowhere to keep the item, and pretending otherwise would drop
    /// a call the caller believes is durable.
    async fn enqueue(&self, call: QueuedCall) -> Result<(), ProxyError> {
        let _ = call;
        Err(ProxyError::Internal("this proxy has no durable outbox behind it".to_string()))
    }
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

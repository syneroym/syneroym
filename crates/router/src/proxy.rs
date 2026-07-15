//! Universal Proxy dispatch (M04A Slice A1): a transport-agnostic outbound
//! [`ServiceProxy`] implementation. Routes a typed `(service, interface,
//! method, params)` call to a local native service, a local WASM component,
//! or a remote node over Iroh QUIC + JSON-RPC, with retry/backoff hook
//! points. The trait itself lives in `syneroym-rpc`; `ProxyRouter` is its
//! only implementation.

use std::{
    fmt::{self, Debug, Formatter},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use iroh::{Endpoint, EndpointAddr};
use serde_json::Value;
use syneroym_core::{
    config::RetryPolicy,
    dht_registry::RegistryClient,
    local_registry::{EndpointRegistry, NATIVE_CAPABILITY_INTERFACES, SubstrateEndpoint},
    retry, util,
};
use syneroym_identity::{DelegationCertificate, Identity};
use syneroym_rpc::{
    CallOrigin, DEFAULT_PROXY_CALL_TIMEOUT, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcResponse,
    NativeInvocation, ProxyError, ProxyProtocol, ProxyRequest, RpcError, ServiceProxy,
    WeakNativeDispatchRegistry, framing,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::time;
use tracing::warn;

use crate::{net_iroh, preamble::RoutePreamble};

/// One wire's worth of "send this JSON-RPC request to that node and read the
/// response". The transport-agnostic seam a future wRPC wire (A.5) slots
/// into: a second impl plus a second `ProxyProtocol` variant, nothing else.
#[async_trait::async_trait]
pub trait RemoteHop: Send + Sync + Debug {
    async fn call(
        &self,
        addr: &EndpointAddr,
        preamble: &RoutePreamble,
        request: &JsonRpcRequest,
        timeout: Duration,
    ) -> Result<Value, ProxyError>;
}

/// [`RemoteHop`] over a live Iroh QUIC connection. `endpoint` is `None` on a
/// WebRTC-only node (no Iroh interface configured) -- every remote hop then
/// fails with a typed transport error rather than panicking.
pub struct IrohHop {
    endpoint: Option<Endpoint>,
    /// Connection-establishment retries only. Forced to a single attempt
    /// (`max_attempts: 1`) regardless of what the caller passes in: the
    /// call-level retry loop in [`ProxyRouter::invoke_remote`] already
    /// retries the whole call (connect + request), so letting
    /// `connect_with_retry` retry underneath it too would multiply worst-
    /// case attempts to `max_attempts²` for an unreachable peer.
    connect_retry_policy: RetryPolicy,
}

impl Debug for IrohHop {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("IrohHop").field("has_endpoint", &self.endpoint.is_some()).finish()
    }
}

impl IrohHop {
    #[must_use]
    pub fn new(endpoint: Option<Endpoint>, retry_policy: RetryPolicy) -> Self {
        Self { endpoint, connect_retry_policy: RetryPolicy { max_attempts: 1, ..retry_policy } }
    }
}

fn transport_err(e: impl std::fmt::Display) -> ProxyError {
    ProxyError::Transport(e.to_string())
}

#[async_trait::async_trait]
impl RemoteHop for IrohHop {
    async fn call(
        &self,
        addr: &EndpointAddr,
        preamble: &RoutePreamble,
        request: &JsonRpcRequest,
        timeout: Duration,
    ) -> Result<Value, ProxyError> {
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or_else(|| ProxyError::Transport("no Iroh endpoint configured".to_string()))?;

        let body = serde_json::to_vec(request)
            .map_err(|e| ProxyError::Internal(format!("failed to serialize request: {e}")))?;

        // The whole attempt -- connect, open the bi-stream, write the
        // preamble and request, and read the response -- sits under one
        // deadline, not just the final read: a peer that accepts the QUIC
        // connection but stalls before accepting the stream (or stops
        // reading so send-side flow control blocks) must not hang past
        // `timeout`.
        let frame = time::timeout(timeout, async {
            let conn = net_iroh::connect_with_retry(
                endpoint,
                addr.clone(),
                crate::SYNEROYM_ALPN,
                &self.connect_retry_policy,
            )
            .await
            .map_err(transport_err)?;

            let (mut send, mut recv) = conn.open_bi().await.map_err(transport_err)?;
            send.write_all(preamble.to_preamble_line().as_bytes()).await.map_err(transport_err)?;
            framing::write_frame(&mut send, &body).await.map_err(transport_err)?;
            send.finish().map_err(transport_err)?;

            framing::read_frame(&mut recv).await.map_err(transport_err)
        })
        .await
        .map_err(|_| ProxyError::Timeout(timeout))??;
        if frame.is_empty() {
            return Err(ProxyError::Transport("empty response frame".to_string()));
        }

        // Success or error envelope -- a JSON-RPC error is a *definitive*
        // answer, never a transport failure. `JsonRpcResponse::result` is a
        // required field, so an error-shaped frame (no `result`) fails this
        // parse and falls through to the error-envelope parse below.
        if let Ok(ok) = serde_json::from_slice::<JsonRpcResponse>(&frame) {
            return Ok(ok.result);
        }
        let err: JsonRpcErrorResponse = serde_json::from_slice(&frame)
            .map_err(|e| ProxyError::Transport(format!("malformed response: {e}")))?;
        Err(ProxyError::Callee {
            code: err.error.code,
            message: err.error.message,
            data: err.error.data,
        })
    }
}

/// The Universal Proxy's outbound router (M04A Slice A1). Holds `Weak`
/// handles into the engine/dispatch-registry it routes to, and the
/// registry/registry-client it uses to resolve targets -- see the module doc
/// comment on ownership direction (`RouteHandlerInner` is the strong owner;
/// `AppSandboxEngine` only ever holds a `Weak<dyn ServiceProxy>` back, to
/// avoid the `RouteHandlerInner -> ProxyRouter -> AppSandboxEngine ->
/// ProxyRouter` reference cycle that hung graceful shutdown in Slice 6B).
pub struct ProxyRouter {
    registry: EndpointRegistry,
    registry_client: Arc<RegistryClient>,
    native_dispatch: WeakNativeDispatchRegistry,
    app_sandbox_engine: Weak<AppSandboxEngine>,
    hop: Arc<dyn RemoteHop>,
    node_identity: Arc<Identity>,
    retry_policy: RetryPolicy,
}

impl Debug for ProxyRouter {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyRouter").finish_non_exhaustive()
    }
}

impl ProxyRouter {
    #[must_use]
    pub fn new(
        registry: EndpointRegistry,
        registry_client: Arc<RegistryClient>,
        native_dispatch: WeakNativeDispatchRegistry,
        app_sandbox_engine: Weak<AppSandboxEngine>,
        hop: Arc<dyn RemoteHop>,
        node_identity: Arc<Identity>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            registry,
            registry_client,
            native_dispatch,
            app_sandbox_engine,
            hop,
            node_identity,
            retry_policy,
        }
    }

    /// `data-layer`, `vault`, `app-config`, `blob-store`, `messaging`,
    /// `http-native` -- the reserved names every deployed service
    /// auto-registers (`syneroym_core::local_registry::
    /// NATIVE_CAPABILITY_INTERFACES`).
    ///
    /// TODO(M04B/FDAE): this is an interim, coarse, fail-closed gate -- "a
    /// guest may only reach its **own** service's native capabilities
    /// through the proxy." M04B replaces it with real per-caller/per-row
    /// policy evaluated against `caller.session` at the data-owning node, at
    /// which point a guest-originated cross-service `data-layer` read
    /// becomes expressible (and filtered), not refused outright. Do not
    /// widen this gate before that policy exists.
    ///
    /// Applies to `CallOrigin::Guest` only. A substrate-internal
    /// (`CallOrigin::Native`) call to another service's `data-layer` is
    /// exactly what M04B's Slice B3 relationship-proof fetch is, so gating
    /// it here would foreclose the scenario A1 is explicitly supposed to
    /// co-design for. Native-origin calls are authorized at the
    /// **data-owning node** -- the destination re-verifies the forwarded
    /// proof (`invoke_remote`) and, once M04B lands, runs the FDAE policy
    /// inside the callee's own `data-layer` dispatch.
    fn check_native_capability_gate(&self, req: &ProxyRequest) -> Result<(), ProxyError> {
        let CallOrigin::Guest { service_id } = &req.origin else { return Ok(()) };

        // `req.interface` may be the literal name or `EndpointRegistry`'s
        // short-hash of it (`local_registry::short_hash` is an unsalted
        // SHA-256 prefix -- guest-computable, and `lookup` canonicalizes it
        // right back to the literal name for dispatch). Matching only the
        // literal string here let a guest bypass this gate entirely by
        // passing the hash instead of the name.
        let is_native_capability = NATIVE_CAPABILITY_INTERFACES
            .iter()
            .any(|name| *name == req.interface || util::short_hash(name) == req.interface);
        if !is_native_capability {
            return Ok(());
        }

        // Compare the guest's **raw** component_id against the target. NOT
        // `caller.caller_did`: `CallerContext::service_system` sets that to
        // `"system:<service_id>"`, which can never equal a plain service
        // id -- using it would reject a component's calls to its own
        // service too.
        if service_id == &req.target_service {
            return Ok(());
        }

        Err(ProxyError::PermissionDenied(format!(
            "component '{service_id}' may not reach native capability '{}' on service '{}' \
             through the proxy (cross-service native-capability policy is FDAE/M04B)",
            req.interface, req.target_service
        )))
    }

    /// Local-node dispatch: the endpoint registry is authoritative for
    /// services hosted on this node -- also the `<5ms p99` same-node path
    /// (in-process dispatch, no wire round trip).
    async fn invoke_local(
        &self,
        req: &ProxyRequest,
        endpoint: SubstrateEndpoint,
        canonical_iface: String,
    ) -> Result<Value, ProxyError> {
        let call_timeout = req.timeout.unwrap_or(DEFAULT_PROXY_CALL_TIMEOUT);
        match endpoint {
            SubstrateEndpoint::NativeHostChannel { service_id } => {
                let table = self.native_dispatch.upgrade().ok_or_else(|| {
                    ProxyError::Internal("native dispatch registry gone".to_string())
                })?;
                let svc = table
                    .get(&service_id)
                    .as_deref()
                    .cloned()
                    .ok_or_else(|| ProxyError::ServiceNotFound(service_id.clone()))?;
                let invocation = NativeInvocation {
                    interface: canonical_iface,
                    method: req.method.clone(),
                    params: req.params.clone(),
                    caller: req.caller.clone(),
                };
                time::timeout(call_timeout, svc.dispatch(invocation))
                    .await
                    .map_err(|_| ProxyError::Timeout(call_timeout))?
                    .map(|r| r.payload)
                    .map_err(|e: RpcError| ProxyError::Callee {
                        code: e.code(),
                        message: e.to_string(),
                        data: e.data(),
                    })
            }
            // Identity threading through a proxied WASM call is "the callee
            // acts as itself" (`execute_wasm_json` builds its own
            // `service_system`/`local_elevated` caller internally, per B0's
            // shape) -- the proxy caller's own identity does not currently
            // reach a WASM callee's host state. Not an oversight: a
            // caller-scoped guest identity is an FDAE/M04B concern.
            //
            // Known limitation, same boundary: any error from
            // `execute_wasm_json` -- including a callee's own typed
            // `result::err` -- collapses to `Callee{ code: -32603 }` below.
            // The structured `E` doesn't survive the WIT<->JSON boundary
            // here, so a caller can't distinguish a business rejection from
            // a host crash. Acceptable for A1; a component-to-component
            // error channel that can carry typed errors is a follow-up.
            SubstrateEndpoint::WasmChannel { service_id } => {
                let engine = self.app_sandbox_engine.upgrade().ok_or_else(|| {
                    ProxyError::Internal("sandbox engine unavailable".to_string())
                })?;
                let request = JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    method: req.method.clone(),
                    params: req.params.clone(),
                    id: Some(Value::from(1)),
                };
                time::timeout(
                    call_timeout,
                    engine.execute_wasm_json(&service_id, &canonical_iface, &request),
                )
                .await
                .map_err(|_| ProxyError::Timeout(call_timeout))?
                .map_err(|e| ProxyError::Callee {
                    code: -32603,
                    message: e.to_string(),
                    data: None,
                })
            }
            other @ (SubstrateEndpoint::TcpHostPort { .. }
            | SubstrateEndpoint::PodmanSocket { .. }) => {
                Err(ProxyError::UnsupportedTarget(format!("{other:?}")))
            }
        }
    }

    /// Resolves `req.target_service`'s Iroh address via the community
    /// registry / DHT and dispatches the call over [`RemoteHop`].
    async fn invoke_remote(&self, req: &ProxyRequest) -> Result<Value, ProxyError> {
        let addr = net_iroh::resolve_iroh_addr(&self.registry_client, &req.target_service)
            .await
            .map_err(|_| ProxyError::ServiceNotFound(req.target_service.clone()))?;
        self.invoke_remote_at(&addr, req).await
    }

    /// The retry loop and preamble construction, split out from
    /// [`Self::invoke_remote`] so unit tests can drive it against a
    /// pre-resolved (synthetic) address without a live registry/DHT.
    async fn invoke_remote_at(
        &self,
        addr: &EndpointAddr,
        req: &ProxyRequest,
    ) -> Result<Value, ProxyError> {
        // Identity: forward the caller's *signed proof* verbatim when it has
        // one (ADR-0016 §6 -- the destination re-verifies with
        // `verify_preamble` and builds a fresh `CallerContext`); otherwise
        // present this node's own identity -- but only for a genuine
        // substrate-internal (`CallOrigin::Native`) call. A guest never
        // carries a proof (`CallerContext::service_system`'s `proof` is
        // always `None`; B1/UCAN delegation is what will give it one), so
        // presenting the node's own key on its behalf would launder the
        // guest's call as this node's real, potentially privileged DID at
        // the destination. Leaving `pubkey` unset instead makes the
        // destination treat it as anonymous, which the native-dispatch arm
        // already rejects and non-native paths already tolerate. Capabilities
        // never cross either way.
        let mut preamble = RoutePreamble::binary_json_rpc(&req.target_service, &req.interface);
        match &req.caller.proof {
            Some(proof) => {
                preamble.pubkey = Some(proof.pubkey_hex.clone());
                preamble.delegation = proof
                    .delegation_json
                    .as_deref()
                    .and_then(|json| DelegationCertificate::from_json(json).ok());
            }
            None if matches!(req.origin, CallOrigin::Native) => {
                preamble.pubkey = Some(hex::encode(self.node_identity.public_key().to_bytes()));
            }
            None => {}
        }

        let json_rpc_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: req.method.clone(),
            params: req.params.clone(),
            id: Some(Value::from(1)),
        };
        let call_timeout = req.timeout.unwrap_or(DEFAULT_PROXY_CALL_TIMEOUT);

        // Retry loop. Only *transport* failures are retried, and only when
        // the caller declared the call idempotent. A callee-returned error
        // is never retried. Failed-after-retries fails directly -- no DLQ
        // (M5).
        let attempts: u8 = if req.idempotent { self.retry_policy.max_attempts.max(1) } else { 1 };
        let mut backoff = self.retry_policy.initial_backoff_ms;
        let mut attempt: u8 = 1;
        loop {
            match self.hop.call(addr, &preamble, &json_rpc_request, call_timeout).await {
                Ok(v) => return Ok(v),
                Err(e)
                    if attempt >= attempts
                        || !matches!(e, ProxyError::Transport(_) | ProxyError::Timeout(_)) =>
                {
                    return Err(e);
                }
                Err(e) => {
                    warn!(attempt, max = attempts, error = %e, "proxy call failed; retrying");
                    metrics::counter!("substrate.proxy.retries").increment(1);
                    time::sleep(Duration::from_millis(retry::calculate_jittered_backoff(backoff)))
                        .await;
                    backoff = ((backoff as f64 * self.retry_policy.backoff_multiplier) as u64)
                        .min(self.retry_policy.max_backoff_ms);
                    attempt += 1;
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl ServiceProxy for ProxyRouter {
    async fn invoke(&self, req: ProxyRequest) -> Result<Value, ProxyError> {
        // Protocol gate: the minimal `[LFC-VER]` behavior kept from the
        // deferred protocol-negotiation slice (A.7). `ProxyProtocol` has
        // exactly one variant today, so this is a no-op in practice; it
        // stays as the seam a future wRPC variant plugs into.
        if req.protocol != ProxyProtocol::JsonRpcV1 {
            return Err(ProxyError::UnsupportedProtocol(format!("{:?}", req.protocol)));
        }

        // Capability gate: a WASM guest must not reach another service's
        // native capabilities through the proxy.
        self.check_native_capability_gate(&req)?;

        metrics::counter!("substrate.proxy.calls").increment(1);
        let started = Instant::now();

        // Local first: the endpoint registry is authoritative for services
        // hosted on this node (this is also the <5ms same-node path).
        let outcome = match self.registry.lookup(&req.target_service, &req.interface) {
            Some((endpoint, canonical_iface)) => {
                self.invoke_local(&req, endpoint, canonical_iface).await
            }
            None => self.invoke_remote(&req).await,
        };

        metrics::histogram!("substrate.proxy.duration_ms")
            .record(started.elapsed().as_secs_f64() * 1000.0);
        if outcome.is_err() {
            metrics::counter!("substrate.proxy.errors").increment(1);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use dashmap::DashMap;
    use iroh::SecretKey;
    use syneroym_core::storage::MockStorage;
    use syneroym_rpc::{
        AuthLevel, CallerContext, NativeDispatchRegistry, NativeResponse, NativeService, RpcResult,
        SessionContext,
    };

    use super::*;

    fn test_caller(did: &str) -> CallerContext {
        CallerContext {
            caller_did: did.to_string(),
            app_instance: None,
            session: SessionContext::default(),
            auth: AuthLevel::Delegated,
            proof: None,
        }
    }

    fn base_request(target_service: &str, interface: &str) -> ProxyRequest {
        ProxyRequest {
            target_service: target_service.to_string(),
            interface: interface.to_string(),
            method: "get".to_string(),
            params: Value::Null,
            caller: test_caller("did:key:zTestCaller"),
            origin: CallOrigin::Native,
            protocol: ProxyProtocol::JsonRpcV1,
            idempotent: false,
            timeout: Some(Duration::from_secs(1)),
        }
    }

    fn synthetic_addr() -> EndpointAddr {
        let node_id = SecretKey::generate(&mut rand::rng()).public();
        EndpointAddr::new(node_id)
    }

    fn empty_registry() -> EndpointRegistry {
        EndpointRegistry::new_mock(Arc::new(MockStorage::new()))
    }

    fn empty_registry_client() -> Arc<RegistryClient> {
        Arc::new(RegistryClient::new(false, None))
    }

    fn test_router(hop: Arc<dyn RemoteHop>, registry: EndpointRegistry) -> ProxyRouter {
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop,
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy {
                max_attempts: 3,
                initial_backoff_ms: 1,
                backoff_multiplier: 2.0,
                max_backoff_ms: 5,
            },
        )
    }

    #[derive(Debug, Default)]
    struct RecordingNativeService {
        invoked: AtomicUsize,
        last_caller_did: Mutex<Option<String>>,
    }

    #[async_trait::async_trait]
    impl NativeService for RecordingNativeService {
        async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
            self.invoked.fetch_add(1, Ordering::SeqCst);
            *self.last_caller_did.lock().unwrap() = Some(invocation.caller.caller_did.clone());
            Ok(NativeResponse { payload: Value::String("ok".to_string()) })
        }
    }

    #[derive(Debug, Clone)]
    enum MockOutcome {
        Success(Value),
        Transport,
        Callee { code: i32, message: String },
    }

    #[derive(Debug, Default)]
    struct MockHop {
        calls: AtomicUsize,
        last_preamble: Mutex<Option<RoutePreamble>>,
        outcomes: Mutex<std::collections::VecDeque<MockOutcome>>,
    }

    impl MockHop {
        fn with_outcomes(outcomes: Vec<MockOutcome>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                last_preamble: Mutex::new(None),
                outcomes: Mutex::new(outcomes.into()),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl RemoteHop for MockHop {
        async fn call(
            &self,
            _addr: &EndpointAddr,
            preamble: &RoutePreamble,
            _request: &JsonRpcRequest,
            _timeout: Duration,
        ) -> Result<Value, ProxyError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_preamble.lock().unwrap() = Some(preamble.clone());
            match self.outcomes.lock().unwrap().pop_front() {
                Some(MockOutcome::Success(v)) => Ok(v),
                Some(MockOutcome::Transport) | None => {
                    Err(ProxyError::Transport("mock transport failure".to_string()))
                }
                Some(MockOutcome::Callee { code, message }) => {
                    Err(ProxyError::Callee { code, message, data: None })
                }
            }
        }
    }

    // -- local native dispatch -------------------------------------------

    #[tokio::test]
    async fn invoke_local_native_reaches_registered_service_with_caller_identity() {
        let registry = empty_registry();
        registry
            .register(
                "svc-a".to_string(),
                "data-layer".to_string(),
                SubstrateEndpoint::NativeHostChannel { service_id: "svc-a".to_string() },
            )
            .await
            .unwrap();

        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let service = Arc::new(RecordingNativeService::default());
        native_dispatch.insert("svc-a".to_string(), service.clone() as Arc<dyn NativeService>);

        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            Arc::new(MockHop::default()),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy::default(),
        );

        let mut req = base_request("svc-a", "data-layer");
        req.caller = test_caller("did:key:zCallerOne");
        let result = router.invoke(req).await.unwrap();
        assert_eq!(result, Value::String("ok".to_string()));
        assert_eq!(service.invoked.load(Ordering::SeqCst), 1);
        assert_eq!(service.last_caller_did.lock().unwrap().as_deref(), Some("did:key:zCallerOne"));
    }

    // -- unknown service ---------------------------------------------------

    #[tokio::test]
    async fn unknown_service_is_service_not_found_and_hop_never_called() {
        let hop = Arc::new(MockHop::default());
        let router = test_router(hop.clone(), empty_registry());

        let result = router.invoke(base_request("no-such-service", "greet")).await;
        assert!(matches!(result, Err(ProxyError::ServiceNotFound(_))));
        assert_eq!(hop.call_count(), 0);
    }

    // -- native capability gate (§5.3) -------------------------------------

    #[tokio::test]
    async fn guest_cross_service_native_capability_is_denied_and_never_dispatched() {
        let registry = empty_registry();
        registry
            .register(
                "svc-b".to_string(),
                "data-layer".to_string(),
                SubstrateEndpoint::NativeHostChannel { service_id: "svc-b".to_string() },
            )
            .await
            .unwrap();
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let service = Arc::new(RecordingNativeService::default());
        native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            Arc::new(MockHop::default()),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy::default(),
        );

        let mut req = base_request("svc-b", "data-layer");
        req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
        let result = router.invoke(req).await;
        assert!(matches!(result, Err(ProxyError::PermissionDenied(_))));
        assert_eq!(service.invoked.load(Ordering::SeqCst), 0);
    }

    /// A guest that requests the interface by its `short_hash` (what
    /// `EndpointRegistry::lookup` also accepts and canonicalizes back to the
    /// literal name) must be denied exactly like the literal-name request
    /// above -- `short_hash` is an unsalted SHA-256 prefix, so it's
    /// guest-computable and must not bypass the gate.
    #[tokio::test]
    async fn guest_cross_service_native_capability_is_denied_via_short_hash_too() {
        let registry = empty_registry();
        registry
            .register(
                "svc-b".to_string(),
                "data-layer".to_string(),
                SubstrateEndpoint::NativeHostChannel { service_id: "svc-b".to_string() },
            )
            .await
            .unwrap();
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let service = Arc::new(RecordingNativeService::default());
        native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            Arc::new(MockHop::default()),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy::default(),
        );

        let mut req = base_request("svc-b", &util::short_hash("data-layer"));
        req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
        let result = router.invoke(req).await;
        assert!(matches!(result, Err(ProxyError::PermissionDenied(_))));
        assert_eq!(service.invoked.load(Ordering::SeqCst), 0);
    }

    /// The case that would fail against a `caller_did`-based comparison
    /// instead of the guest's raw `component_id` -- `service_system` puts
    /// `"system:svc-a"` in `caller_did`, which would never equal a plain
    /// service id.
    #[tokio::test]
    async fn guest_reaching_its_own_native_capability_is_allowed() {
        let registry = empty_registry();
        registry
            .register(
                "svc-a".to_string(),
                "data-layer".to_string(),
                SubstrateEndpoint::NativeHostChannel { service_id: "svc-a".to_string() },
            )
            .await
            .unwrap();
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let service = Arc::new(RecordingNativeService::default());
        native_dispatch.insert("svc-a".to_string(), service.clone() as Arc<dyn NativeService>);

        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            Arc::new(MockHop::default()),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy::default(),
        );

        let mut req = base_request("svc-a", "data-layer");
        req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
        req.caller = CallerContext::service_system("svc-a");
        let result = router.invoke(req).await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(service.invoked.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn guest_reaching_a_non_native_interface_on_another_service_is_allowed() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let router = test_router(hop.clone(), empty_registry());

        let mut req = base_request("svc-b", "some-app-interface");
        req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
        let result = router.invoke_remote_at(&synthetic_addr(), &req).await;
        assert!(result.is_ok());
    }

    /// The M04B B3 relationship-proof-fetch shape -- a guard against a
    /// future tightening of the gate silently re-breaking B3.
    #[tokio::test]
    async fn native_origin_cross_service_data_layer_call_is_allowed_by_the_gate() {
        let registry = empty_registry();
        registry
            .register(
                "svc-b".to_string(),
                "data-layer".to_string(),
                SubstrateEndpoint::NativeHostChannel { service_id: "svc-b".to_string() },
            )
            .await
            .unwrap();
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let service = Arc::new(RecordingNativeService::default());
        native_dispatch.insert("svc-b".to_string(), service.clone() as Arc<dyn NativeService>);

        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            Arc::new(MockHop::default()),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy::default(),
        );

        let mut req = base_request("svc-b", "data-layer");
        req.origin = CallOrigin::Native;
        let result = router.invoke(req).await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(service.invoked.load(Ordering::SeqCst), 1);
    }

    // -- remote dispatch: retry -------------------------------------------

    #[tokio::test]
    async fn idempotent_call_retries_transport_failures_up_to_max_attempts() {
        let hop = Arc::new(MockHop::with_outcomes(vec![
            MockOutcome::Transport,
            MockOutcome::Transport,
            MockOutcome::Transport,
        ]));
        let router = test_router(hop.clone(), empty_registry());

        let mut req = base_request("remote-svc", "greet");
        req.idempotent = true;
        let result = router.invoke_remote_at(&synthetic_addr(), &req).await;
        assert!(matches!(result, Err(ProxyError::Transport(_))));
        assert_eq!(hop.call_count(), 3, "must retry up to max_attempts (3) for an idempotent call");
    }

    #[tokio::test]
    async fn non_idempotent_call_never_retries_transport_failures() {
        let hop =
            Arc::new(MockHop::with_outcomes(vec![MockOutcome::Transport, MockOutcome::Transport]));
        let router = test_router(hop.clone(), empty_registry());

        let req = base_request("remote-svc", "greet"); // idempotent: false (default)
        let result = router.invoke_remote_at(&synthetic_addr(), &req).await;
        assert!(matches!(result, Err(ProxyError::Transport(_))));
        assert_eq!(hop.call_count(), 1, "a non-idempotent call must never be retried");
    }

    #[tokio::test]
    async fn callee_error_is_never_retried_even_when_idempotent() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Callee {
            code: -32010,
            message: "denied".to_string(),
        }]));
        let router = test_router(hop.clone(), empty_registry());

        let mut req = base_request("remote-svc", "greet");
        req.idempotent = true;
        let result = router.invoke_remote_at(&synthetic_addr(), &req).await;
        assert!(matches!(result, Err(ProxyError::Callee { code: -32010, .. })));
        assert_eq!(hop.call_count(), 1, "a definitive callee error must never be retried");
    }

    #[tokio::test]
    async fn idempotent_call_stops_retrying_once_it_succeeds() {
        let hop = Arc::new(MockHop::with_outcomes(vec![
            MockOutcome::Transport,
            MockOutcome::Success(Value::String("recovered".to_string())),
        ]));
        let router = test_router(hop.clone(), empty_registry());

        let mut req = base_request("remote-svc", "greet");
        req.idempotent = true;
        let result = router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();
        assert_eq!(result, Value::String("recovered".to_string()));
        assert_eq!(hop.call_count(), 2);
    }

    // -- remote dispatch: proof forwarding ---------------------------------

    #[tokio::test]
    async fn caller_with_proof_forwards_it_verbatim_on_the_outbound_preamble() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let router = test_router(hop.clone(), empty_registry());

        let mut req = base_request("remote-svc", "greet");
        req.caller.proof = Some(syneroym_rpc::CallerProof {
            pubkey_hex: "deadbeef".to_string(),
            delegation_json: None,
        });
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(preamble.pubkey.as_deref(), Some("deadbeef"));
    }

    #[tokio::test]
    async fn caller_without_proof_presents_the_nodes_own_identity() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let identity = Arc::new(Identity::generate().unwrap());
        let expected_pubkey = hex::encode(identity.public_key().to_bytes());
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let router = ProxyRouter::new(
            empty_registry(),
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop.clone(),
            identity,
            RetryPolicy::default(),
        );

        let req = base_request("remote-svc", "greet"); // caller.proof: None
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(preamble.pubkey.as_deref(), Some(expected_pubkey.as_str()));
    }

    /// A guest never carries a proof (`CallerContext::service_system`), so
    /// unlike the `CallOrigin::Native` case above, a cross-node guest call
    /// must not launder itself as the node's own identity -- that would let
    /// the destination attribute the call to a real, potentially privileged
    /// DID (e.g. its `admin_ucan_root`) with no marker that a guest
    /// originated it.
    #[tokio::test]
    async fn guest_without_proof_forwards_as_anonymous_not_node_identity() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let identity = Arc::new(Identity::generate().unwrap());
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let router = ProxyRouter::new(
            empty_registry(),
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop.clone(),
            identity,
            RetryPolicy::default(),
        );

        let mut req = base_request("remote-svc", "greet"); // caller.proof: None
        req.origin = CallOrigin::Guest { service_id: "guest-component".to_string() };
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(preamble.pubkey, None);
    }
}

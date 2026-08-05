//! Universal Proxy dispatch (M04A Slice A1): a transport-agnostic outbound
//! [`ServiceProxy`] implementation. Routes a typed `(service, interface,
//! method, params)` call to a local native service, a local WASM component,
//! or a remote node over Iroh QUIC + JSON-RPC, with retry/backoff hook
//! points. The trait itself lives in `syneroym-rpc`; `ProxyRouter` is its
//! only implementation.

use std::{
    collections::BTreeSet,
    fmt::{self, Debug, Formatter},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use iroh::{Endpoint, EndpointAddr};
use serde_json::Value;
use syneroym_async_queue::{FailOutcome, Queue};
use syneroym_core::{
    config::RetryPolicy,
    dht_registry::RegistryClient,
    local_registry::{
        EndpointRegistry, NATIVE_CAPABILITY_INTERFACES, NODE_NATIVE_INTERFACES, SubstrateEndpoint,
    },
    retry, util,
};
use syneroym_identity::{DelegationCertificate, Identity};
use syneroym_rpc::{
    CallOrigin, CallerContext, DEFAULT_PROXY_CALL_TIMEOUT, JsonRpcErrorResponse, JsonRpcRequest,
    JsonRpcResponse, NativeInvocation, ProxyError, ProxyProtocol, ProxyRequest, QueuedCall,
    RpcError, ServiceProxy, WeakNativeDispatchRegistry, framing,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::time;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::{
    call_dedup::{self, CallDedupGuard, GuardOutcome},
    net_iroh,
    preamble::RoutePreamble,
    proxy_outbox::{self, Disposition, ProxyOutbox},
};

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
    /// The receiver-side idempotency fence for calls that land on *this*
    /// node. `None` on a node with no storage provider at all (a
    /// coordinator), which hosts no deployed services and therefore has
    /// nowhere to remember a key -- a keyed call there is refused rather
    /// than executed unfenced.
    ///
    /// Attached after construction rather than taken as an eighth
    /// constructor argument: the router already takes seven, and every one
    /// of its test and bench call sites would otherwise have to pass a
    /// `None` that says nothing. "This node can fence" is a property of the
    /// deployment, so it reads better as something a node either has or
    /// does not.
    dedup_guard: Option<Arc<CallDedupGuard>>,
    /// The durable outbox behind `enqueue`, one queue per calling service.
    /// `None` on a node with no per-service storage, where there is
    /// nowhere to keep an item and no guest to produce one.
    outbox: Option<Arc<ProxyOutbox>>,
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
            dedup_guard: None,
            outbox: None,
        }
    }

    /// Gives this router the fence it applies to keyed calls arriving for
    /// a service on this node.
    #[must_use]
    pub fn with_dedup_guard(mut self, guard: Arc<CallDedupGuard>) -> Self {
        self.dedup_guard = Some(guard);
        self
    }

    /// Gives this router the durable outbox behind `enqueue`.
    #[must_use]
    pub fn with_outbox(mut self, outbox: Arc<ProxyOutbox>) -> Self {
        self.outbox = Some(outbox);
        self
    }

    #[must_use]
    pub fn outbox(&self) -> Option<&Arc<ProxyOutbox>> {
        self.outbox.as_ref()
    }

    /// Rebuilds the live cross-service call a stored item describes.
    ///
    /// The identity has to match what `proxy::Host::call` builds for the
    /// same cross-service call exactly -- `service_system(caller)` plus a
    /// guest origin -- or authorization at the receiver would silently
    /// differ between the immediate attempt and every later one.
    fn request_from(&self, call: &QueuedCall, target_service: String) -> ProxyRequest {
        ProxyRequest {
            target_service,
            interface: call.interface.clone(),
            method: call.method.clone(),
            params: call.params.clone(),
            caller: CallerContext::service_system(&call.caller_service_id),
            origin: CallOrigin::Guest { service_id: call.caller_service_id.clone() },
            protocol: ProxyProtocol::parse(call.protocol.as_deref())
                .unwrap_or(ProxyProtocol::JsonRpcV1),
            idempotent: true,
            idempotency_key: Some(call.idempotency_key.clone()),
            timeout: call.timeout_ms.map(Duration::from_millis),
        }
    }

    /// Writes a dead letter for a failed call that carried a fence.
    ///
    /// An **unkeyed** call writes nothing, and that is the rule rather
    /// than an omission: its caller is alive and holding the error, so
    /// this is not silent loss, and there would be nothing safe to replay
    /// -- a replayable dead letter for a call with no fence *is* a second
    /// delivery of an unfenced call.
    ///
    /// The recorded target is the DID this attempt actually resolved to,
    /// not the dependency name: this row describes one specific attempt an
    /// operator may choose to repeat, and the resolution already happened
    /// before the request existed.
    async fn record_failed_call(&self, req: &ProxyRequest, error: &ProxyError) {
        let (Some(outbox), Some(key)) = (&self.outbox, req.idempotency_key.as_deref()) else {
            return;
        };
        let CallOrigin::Guest { service_id } = &req.origin else { return };
        // A queued item that failed is dead-lettered by the worker, which
        // owns its own retry history; this path is only for the caller
        // that is still holding the error.
        let call = QueuedCall {
            app_instance_id: None,
            caller_service_id: service_id.clone(),
            target: syneroym_rpc::QueuedTarget::Service(req.target_service.clone()),
            routing_key: None,
            interface: req.interface.clone(),
            method: req.method.clone(),
            params: req.params.clone(),
            idempotency_key: key.to_string(),
            protocol: None,
            timeout_ms: req.timeout.map(|t| t.as_millis() as u64),
        };
        if let Err(e) = outbox.record_dead_letter(&call, &error.to_string()).await {
            warn!(error = %e, "could not record a dead letter for a failed keyed call");
        }
    }

    /// Delivers one queued item: re-resolve, then invoke. Split out so the
    /// worker and the immediate try-then-queue attempt cannot drift apart.
    async fn deliver_queued(&self, call: &QueuedCall) -> Result<Value, ProxyError> {
        let outbox = self
            .outbox
            .as_ref()
            .ok_or_else(|| ProxyError::Internal("no durable outbox on this node".to_string()))?;
        // Re-resolved on every attempt, never stored: a binding re-pushed
        // while the item waited has to take effect (ADR-0021 §2).
        let target = outbox.resolve_target(call)?;
        self.invoke_inner(&self.request_from(call, target)).await
    }

    /// [`Self::invoke_local`] under the receiver-side fence.
    ///
    /// A call with no key runs straight through, touching no store: that
    /// is every call on the hot path today, and it is what keeps the
    /// fence off the existing call budget.
    async fn invoke_local_guarded(
        &self,
        req: &ProxyRequest,
        endpoint: SubstrateEndpoint,
        canonical_iface: String,
    ) -> Result<Value, ProxyError> {
        let Some(guard) = &self.dedup_guard else {
            if req.idempotency_key.is_some() {
                return Err(ProxyError::Internal(
                    "this node keeps no per-service storage, so it cannot honour an idempotency \
                     key"
                    .to_string(),
                ));
            }
            return self.invoke_local(req, endpoint, canonical_iface).await;
        };
        match guard
            .begin(
                &req.target_service,
                &req.interface,
                Some(&req.caller.caller_did),
                req.idempotency_key.as_deref(),
            )
            .await
        {
            GuardOutcome::Refuse(e) => Err(e),
            GuardOutcome::Answer(outcome) => call_dedup::replay_as_result(outcome),
            GuardOutcome::Execute(claim) => {
                let outcome = self.invoke_local(req, endpoint, canonical_iface).await;
                if let Some(claim) = claim {
                    claim.settle(&outcome).await;
                }
                outcome
            }
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
        let matches_interface =
            |name: &&str| *name == req.interface || util::short_hash(name) == req.interface;

        // `orchestrator`/`security` are node-level, not service-scoped, so
        // the same-service exemption below (which only makes sense for an
        // interface a service can itself hold) does not apply -- denied
        // outright, for any target. Since ADR-0020 §1, a guest whose own
        // service holds an installed instance certificate presents a
        // *verified* identity on outbound calls (see `invoke_remote_at`);
        // without this, that verified identity would reach these two
        // node-owned interfaces exactly like a legitimate native caller --
        // a WASM guest was never able to present a verified identity to a
        // native interface at all before that certificate mechanism
        // existed. Both interfaces are gated now
        // (`orchestrator` since the deploy-grant admission gate, `security` on
        // `substrate/admin`, `control_plane::service`), but this
        // outright denial stays: a deployed service's own instance identity
        // should never reach node-owner-only interfaces at all, gated or
        // not.
        if NODE_NATIVE_INTERFACES.iter().any(matches_interface) {
            return Err(ProxyError::PermissionDenied(format!(
                "component '{service_id}' may not reach node-level interface '{}' through the \
                 proxy",
                req.interface
            )));
        }

        let is_native_capability = NATIVE_CAPABILITY_INTERFACES.iter().any(matches_interface);
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
            // acts as itself": `caller: None` below always synthesizes
            // `service_system` (D-B3-4/D-04-02-h; `execute_wasm_json`'s own
            // doc comment). This is a *different* question from the two
            // D-04-02-h ingresses B3.5-fdae closed (a router-verified
            // caller's own identity reaching its own guest's reads, direct
            // or via self-proxy) -- this is one guest delegating to a
            // *different* service's guest-exported interface through the
            // proxy, which would need real caller-delegation (B1/UCAN,
            // not yet built) to forward safely. Not an oversight.
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
                    idempotency_key: req.idempotency_key.clone(),
                };
                time::timeout(
                    call_timeout,
                    engine.execute_wasm_json(&service_id, &canonical_iface, &request, None),
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
        // one and the call is genuinely substrate-internal (ADR-0016 §6 --
        // the destination re-verifies with `verify_preamble` and builds a
        // fresh `CallerContext`); otherwise present this node's own identity
        // -- again only for `CallOrigin::Native`. A guest is never allowed to
        // present the *caller's* proof or the *node's* key remotely, even one
        // it legitimately carries today (Slice B3.5-fdae forwards a
        // self-proxy caller's real `CallerContext`, proof included, whenever
        // the target is the guest's own service -- `check_native_capability_
        // gate`'s same-service check only restricts *native-capability*
        // interfaces, so an ordinary interface the local registry doesn't
        // happen to have registered still falls through to this remote path
        // with that proof attached). Presenting either of those on a guest's
        // behalf would let the guest steer its own freely-chosen `(interface,
        // method, params)` onto the wire under a real, potentially
        // privileged identity -- exactly the laundering this function's
        // `CallOrigin::Guest` branch exists to prevent.
        //
        // What a guest *may* present is its own service's certified instance
        // key (ADR-0020 §1): that grants no privilege the guest didn't
        // already have as itself, since the guest still chooses the call --
        // only the identity it travels under changes, from anonymous to the
        // member master this substrate derived and was certified for. `None`
        // for either the certificate or the recorded owner (a service
        // deployed before an owner/certificate existed) falls back to
        // presenting nothing, unchanged from before: the destination treats
        // it as anonymous, which the native-dispatch arm already rejects and
        // non-native paths already tolerate. Capabilities never cross either
        // way.
        let mut preamble = RoutePreamble::binary_json_rpc(&req.target_service, &req.interface);
        match (&req.caller.proof, &req.origin) {
            (Some(proof), CallOrigin::Native { .. }) => {
                preamble.pubkey = Some(proof.pubkey_hex.clone());
                preamble.delegation = proof
                    .delegation_json
                    .as_deref()
                    .and_then(|json| DelegationCertificate::from_json(json).ok());
            }
            // A0 built this for the guest-origin arm; the same reasoning
            // applies to a substrate-internal call made on a service's
            // behalf. Only the no-proof case: the arm above forwards the
            // original caller's chain verbatim, which is what lets the
            // destination re-derive `subject_did`/`anchor_did` and authorize
            // the real caller (D-B3-9). Presenting the service's identity
            // here instead would silently change who the destination thinks
            // is asking.
            (None, CallOrigin::Native { service_id: Some(sid) }) => {
                if let Some(cert) = self.registry.instance_cert(sid)
                    && !cert.is_expired()
                    && let Some(owner) = self.registry.owner_of(sid)
                {
                    let instance = self.node_identity.derive_service_identity(&owner, sid);
                    preamble.pubkey = Some(hex::encode(instance.public_key().to_bytes()));
                    preamble.delegation = Some(cert);
                } else {
                    preamble.pubkey = Some(hex::encode(self.node_identity.public_key().to_bytes()));
                }
            }
            (None, CallOrigin::Native { service_id: None }) => {
                preamble.pubkey = Some(hex::encode(self.node_identity.public_key().to_bytes()));
            }
            (_, CallOrigin::Guest { service_id }) => {
                // An expired certificate is worse than none: the
                // destination hard-rejects any connection whose delegation
                // fails to verify (`route_handler/io.rs`), where a `None`
                // pubkey instead falls back to anonymous -- which non-
                // native-dispatch destinations already tolerate. Presenting
                // it anyway would turn a missed renewal into an outage for
                // passthrough/relay calls that never cared about identity
                // before this certificate mechanism existed.
                if let Some(cert) = self.registry.instance_cert(service_id)
                    && !cert.is_expired()
                    && let Some(owner) = self.registry.owner_of(service_id)
                {
                    let instance = self.node_identity.derive_service_identity(&owner, service_id);
                    preamble.pubkey = Some(hex::encode(instance.public_key().to_bytes()));
                    preamble.delegation = Some(cert);
                }
            }
        }

        let json_rpc_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: req.method.clone(),
            params: req.params.clone(),
            id: Some(Value::from(1)),
            idempotency_key: req.idempotency_key.clone(),
        };
        let call_timeout = req.timeout.unwrap_or(DEFAULT_PROXY_CALL_TIMEOUT);

        // Retry loop. Only *transport* failures are retried; a
        // callee-returned error is a definitive answer and is never
        // retried. Retry-eligible means the caller declared the call
        // idempotent, or supplied an idempotency key -- a key is a
        // strictly stronger fence than the caller's own assertion, since
        // the receiver enforces it.
        //
        // Exhausting the budget always fails the caller directly. It
        // additionally writes a dead letter *only* when the call carried a
        // key (`record_failed_call`): a dead letter exists to be replayed,
        // and replaying a call with no fence would be a second delivery of
        // something nothing can deduplicate.
        let retry_eligible = req.idempotent || req.idempotency_key.is_some();
        let attempts: u8 = if retry_eligible { self.retry_policy.max_attempts.max(1) } else { 1 };
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

impl ProxyRouter {
    /// The dispatch itself, with no dead-letter side effect.
    ///
    /// Split from the trait method so the two callers that own their own
    /// failure record -- the outbox worker, which dead-letters through the
    /// queue's retry history, and `enqueue`'s immediate attempt, which
    /// either queues the item or hands the error back -- cannot also
    /// produce a second row through `record_failed_call`.
    async fn invoke_inner(&self, req: &ProxyRequest) -> Result<Value, ProxyError> {
        // Protocol gate: the minimal `[LFC-VER]` behavior kept from the
        // deferred protocol-negotiation slice (A.7). `ProxyProtocol` has
        // exactly one variant today, so this is a no-op in practice; it
        // stays as the seam a future wRPC variant plugs into.
        if req.protocol != ProxyProtocol::JsonRpcV1 {
            return Err(ProxyError::UnsupportedProtocol(format!("{:?}", req.protocol)));
        }

        // Capability gate: a WASM guest must not reach another service's
        // native capabilities through the proxy.
        self.check_native_capability_gate(req)?;

        metrics::counter!("substrate.proxy.calls").increment(1);
        let started = Instant::now();

        // Local first: the endpoint registry is authoritative for services
        // hosted on this node (this is also the <5ms same-node path).
        let outcome = match self.registry.lookup(&req.target_service, &req.interface) {
            Some((endpoint, canonical_iface)) => {
                self.invoke_local_guarded(req, endpoint, canonical_iface).await
            }
            None => self.invoke_remote(req).await,
        };

        metrics::histogram!("substrate.proxy.duration_ms")
            .record(started.elapsed().as_secs_f64() * 1000.0);
        if outcome.is_err() {
            metrics::counter!("substrate.proxy.errors").increment(1);
        }
        outcome
    }

    /// Try-then-queue. A reachable target costs one call and zero queue
    /// writes; only a retryable failure puts the item on disk.
    ///
    /// The three refusals all happen before anything is written, and all
    /// for the same reason: each names a condition every later delivery
    /// attempt would hit too, so failing now is the same answer given
    /// hours sooner, to a caller that is still alive to read it.
    async fn enqueue_call(&self, call: QueuedCall) -> Result<(), ProxyError> {
        let Some(outbox) = &self.outbox else {
            return Err(ProxyError::Internal(
                "this node keeps no per-service storage, so it has no durable outbox".to_string(),
            ));
        };

        // A node-level interface keeps no record to fence a key with, so
        // it could never honour the guarantee a queued call depends on.
        if call_dedup::is_node_level_interface(&call.interface) {
            return Err(ProxyError::PermissionDenied(format!(
                "node-level interface '{}' cannot be reached through the durable outbox: it keeps \
                 no record to deduplicate a redelivery against",
                call.interface
            )));
        }

        // Without an unexpired instance certificate every delivery attempt
        // would present as anonymous, and the receiver refuses a keyed
        // call it cannot scope to a caller. Queuing it would mean ten
        // hours of retries ending in a dead letter, to say the same "no".
        let certified = self
            .registry
            .instance_cert(&call.caller_service_id)
            .is_some_and(|cert| !cert.is_expired());
        if !certified {
            return Err(ProxyError::PermissionDenied(format!(
                "service '{}' holds no unexpired instance certificate, so a queued call from it \
                 would be refused as anonymous at every delivery attempt",
                call.caller_service_id
            )));
        }

        let target = outbox.resolve_target(&call)?;

        // The one caller identity that cannot be rebuilt at delivery is a
        // guest's own forwarded caller, which is exactly what a self-call
        // uses. It is also local and by definition running, so durability
        // buys it nothing.
        if target == call.caller_service_id {
            return Err(ProxyError::UnsupportedTarget(format!(
                "service '{}' cannot enqueue a call to itself: it is local and already running, \
                 so there is nothing for a durable queue to survive",
                call.caller_service_id
            )));
        }

        match self.invoke_inner(&self.request_from(&call, target)).await {
            Ok(_) => Ok(()),
            Err(e) if proxy_outbox::disposition_of(&e) == Disposition::Retry => {
                warn!(error = %e, "proxy enqueue could not deliver now; queueing");
                outbox.store(&call).await
            }
            // Terminal on the very first attempt: the caller is still here
            // to be told, which is a better answer than a dead letter it
            // cannot read.
            Err(e) => Err(e),
        }
    }
}

#[async_trait::async_trait]
impl ServiceProxy for ProxyRouter {
    /// Dispatches a call, and -- when it fails and carried a fence --
    /// records it for an operator. See [`Self::record_failed_call`] for
    /// why an unkeyed failure writes nothing.
    async fn invoke(&self, req: ProxyRequest) -> Result<Value, ProxyError> {
        let outcome = self.invoke_inner(&req).await;
        if let Err(error) = &outcome {
            self.record_failed_call(&req, error).await;
        }
        outcome
    }

    async fn enqueue(&self, call: QueuedCall) -> Result<(), ProxyError> {
        ProxyRouter::enqueue_call(self, call).await
    }
}

impl ProxyRouter {
    /// One pass over every outbox this node currently has open, plus every
    /// deployed service whose queue file already exists on disk -- the
    /// second half is what lets a restart pick up items written before it.
    ///
    /// Returns how many items it settled, for tests and metrics.
    pub async fn drain_outboxes_once(&self) -> usize {
        let Some(outbox) = &self.outbox else { return 0 };
        let deployed: BTreeSet<String> = self
            .registry
            .get_all_endpoints()
            .into_iter()
            .map(|(service_id, _, _)| service_id)
            .collect();

        let mut services: BTreeSet<String> = outbox.open_services().into_iter().collect();
        for service_id in &deployed {
            if outbox.queue_file_exists(service_id) {
                services.insert(service_id.clone());
            }
        }

        let mut settled = 0;
        for service_id in services {
            let Ok(queue) = outbox.queue_for(&service_id).await else { continue };
            // Nothing removes a service's data directory on undeploy, so
            // an outbox can outlive the service that wrote it. Its items
            // are completed silently: delivering would resurrect intent an
            // operator withdrew, and dead-lettering would raise noise
            // about a service nobody is going to act on.
            let still_deployed = deployed.contains(&service_id);
            settled += self.drain_one_outbox(&queue, still_deployed).await;
        }
        settled
    }

    async fn drain_one_outbox(&self, queue: &Queue, still_deployed: bool) -> usize {
        let now = proxy_outbox::now_ms();
        let claimed = {
            let queue = queue.clone();
            match tokio::task::spawn_blocking(move || {
                queue.claim_due(now, proxy_outbox::CLAIM_LIMIT_PER_TICK)
            })
            .await
            {
                Ok(Ok(items)) => items,
                _ => return 0,
            }
        };

        let mut settled = 0;
        for item in claimed {
            let queue = queue.clone();
            if !still_deployed {
                let _ = tokio::task::spawn_blocking(move || queue.complete(item.id)).await;
                settled += 1;
                continue;
            }
            // A claim that never resolves through `fail`/`complete` -- a
            // panic, a crash, a shutdown caught mid-delivery -- is bounded
            // by this rather than by the attempt count, which only `fail`
            // advances.
            if item.claim_count > u32::from(queue.max_attempts()) {
                let _ = tokio::task::spawn_blocking(move || {
                    queue.fail(item.id, now, "claimed repeatedly without ever completing", true)
                })
                .await;
                settled += 1;
                continue;
            }
            let Ok(call) = serde_json::from_slice::<QueuedCall>(&item.payload) else {
                let _ = tokio::task::spawn_blocking(move || {
                    queue.fail(item.id, now, "queued payload is unreadable", true)
                })
                .await;
                settled += 1;
                continue;
            };
            settled += 1;
            match self.deliver_queued(&call).await {
                Ok(_) => {
                    metrics::counter!("substrate.proxy.outbox.delivered").increment(1);
                    let _ = tokio::task::spawn_blocking(move || queue.complete(item.id)).await;
                }
                Err(e) => {
                    proxy_outbox::log_delivery_failure(&call.idempotency_key, &e);
                    let terminal = proxy_outbox::disposition_of(&e) == Disposition::Terminal;
                    let message = e.to_string();
                    let outcome = tokio::task::spawn_blocking(move || {
                        queue.fail(item.id, now, &message, terminal)
                    })
                    .await;
                    if let Ok(Ok(FailOutcome::DeadLettered { .. })) = outcome {
                        metrics::counter!("substrate.proxy.outbox.dead_lettered").increment(1);
                    }
                }
            }
        }
        settled
    }

    /// The outbox worker's resident loop.
    ///
    /// Cancellation is raced *into* a delivery, not merely against the
    /// next tick: a shutdown that had to wait out an in-flight call to an
    /// unreachable peer would wait the full call budget. The abandoned
    /// item stays on disk, still claimed, and returns after its visibility
    /// timeout -- nothing is lost by not draining.
    pub async fn run_outbox_worker(self: Arc<Self>, tick: Duration, cancel: CancellationToken) {
        if self.outbox.is_none() {
            return;
        }
        let mut ticker = time::interval(tick);
        ticker.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                _ = ticker.tick() => {}
            }
            tokio::select! {
                () = cancel.cancelled() => return,
                settled = self.drain_outboxes_once() => {
                    if settled > 0 {
                        debug!(settled, "proxy outbox worker settled queued calls");
                    }
                }
            }
        }
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
    use syneroym_async_queue::QueueConfig;
    use syneroym_core::{dht_registry::MasterAnchorPayload, storage::MockStorage};
    use syneroym_identity::{delegation::SCOPE_SERVICE_INSTANCE, substrate};
    use syneroym_rpc::{
        AuthLevel, CallerContext, CallerProof, NativeDispatchRegistry, NativeResponse,
        NativeService, QueuedTarget, RpcResult, SessionContext,
    };

    use super::*;
    use crate::{HandshakeVerifier, MasterAnchorResolver, proxy_outbox::ProxyOutbox};

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
            origin: CallOrigin::Native { service_id: None },
            protocol: ProxyProtocol::JsonRpcV1,
            idempotency_key: None,
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

    /// A node whose registry holds `svc-a` as a native endpoint backed by
    /// `service`, with a dedup guard over a real (unencrypted) per-service
    /// store. Unencrypted deliberately: the fence's behavior is the
    /// subject here, and the SQLCipher half is pinned in the queue crate.
    struct GuardedNode {
        router: ProxyRouter,
        service: Arc<RecordingNativeService>,
        _native_dispatch: NativeDispatchRegistry,
        _dir: tempfile::TempDir,
    }

    async fn guarded_node(with_store: bool) -> GuardedNode {
        use syneroym_async_queue::DedupConfig;
        use syneroym_data_db::SqliteStorageProvider;
        use syneroym_data_keystore::KeyStore;

        let registry = empty_registry();
        registry
            .register(
                "svc-a".to_string(),
                "greeter".to_string(),
                SubstrateEndpoint::NativeHostChannel { service_id: "svc-a".to_string() },
            )
            .await
            .unwrap();
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let service = Arc::new(RecordingNativeService::default());
        native_dispatch.insert("svc-a".to_string(), service.clone() as Arc<dyn NativeService>);

        let dir = tempfile::tempdir().unwrap();
        let service_dir = dir.path().join("services").join("svc-a");
        std::fs::create_dir_all(&service_dir).unwrap();
        std::fs::write(service_dir.join("state.db"), b"").unwrap();

        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            Arc::new(MockHop::default()),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy::default(),
        );
        let router = if with_store {
            let provider = Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
            router.with_dedup_guard(Arc::new(crate::CallDedupGuard::new(
                provider,
                Arc::new(KeyStore::new()),
                DedupConfig {
                    ttl_ms: 600_000,
                    claim_window_ms: 60_000,
                    max_rows: 100,
                    max_result_bytes: 64 * 1024,
                },
            )))
        } else {
            router
        };
        GuardedNode { router, service, _native_dispatch: native_dispatch, _dir: dir }
    }

    // -- the durable outbox ------------------------------------------------

    /// A node that can enqueue: a certified calling service, a real
    /// per-service store, and a resolver whose bindings a test can change
    /// between attempts.
    struct OutboxNode {
        router: Arc<ProxyRouter>,
        registry: EndpointRegistry,
        resolver: Arc<syneroym_app_orchestration::LogicalResolver>,
        target: Arc<RecordingNativeService>,
        outbox: Arc<ProxyOutbox>,
        _native_dispatch: NativeDispatchRegistry,
        dir: tempfile::TempDir,
    }

    const CALLER: &str = "did:key:zCaller";

    /// `target_reachable` decides whether the immediate attempt succeeds:
    /// a registered native endpoint answers, while a WASM endpoint with no
    /// engine behind it fails with the retryable "sandbox engine
    /// unavailable" -- a shutdown-window state, which is exactly the shape
    /// that must queue rather than fail the caller.
    async fn outbox_node(target_reachable: bool, max_attempts: u8) -> OutboxNode {
        use syneroym_app_orchestration::{
            AppInstanceId, LogicalResolver, LogicalServiceName, ServiceId, StaticInventory,
            TopologyEntry, TopologyEpoch, TopologyMode,
        };
        use syneroym_data_db::SqliteStorageProvider;
        use syneroym_data_keystore::KeyStore;

        let registry = empty_registry();
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let target = Arc::new(RecordingNativeService::default());
        native_dispatch
            .insert("did:key:zTarget".to_string(), target.clone() as Arc<dyn NativeService>);
        registry
            .register(
                "did:key:zTarget".to_string(),
                "greeter".to_string(),
                if target_reachable {
                    SubstrateEndpoint::NativeHostChannel {
                        service_id: "did:key:zTarget".to_string(),
                    }
                } else {
                    SubstrateEndpoint::WasmChannel { service_id: "did:key:zTarget".to_string() }
                },
            )
            .await
            .unwrap();

        // The calling service is itself deployed on this node: the worker
        // only drains services the endpoint registry still knows, so
        // without this every queued item would look orphaned.
        registry
            .register(
                CALLER.to_string(),
                "caller-iface".to_string(),
                SubstrateEndpoint::WasmChannel { service_id: CALLER.to_string() },
            )
            .await
            .unwrap();

        // The calling service holds an unexpired instance certificate, so
        // `enqueue`'s certificate refusal does not fire.
        let node_identity = Arc::new(Identity::generate().unwrap());
        let owner = "did:key:zOwner".to_string();
        registry.set_owner(CALLER.to_string(), owner.clone()).await.unwrap();
        let instance = node_identity.derive_service_identity(&owner, CALLER);
        let cert = DelegationCertificate::issue(
            &Identity::generate().unwrap(),
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert(CALLER.to_string(), cert).await.unwrap();

        let dir = tempfile::tempdir().unwrap();
        for service in [CALLER, "did:key:zTarget"] {
            let service_dir = dir.path().join("services").join(service);
            std::fs::create_dir_all(&service_dir).unwrap();
            std::fs::write(service_dir.join("state.db"), b"").unwrap();
        }

        let inventory = Arc::new(StaticInventory::new());
        let resolver = Arc::new(LogicalResolver::new(inventory));
        resolver.register(
            AppInstanceId::new("app-1"),
            LogicalServiceName::new("backend"),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![ServiceId::new("did:key:zTarget")],
                sharding_strategy: None,
                epoch: TopologyEpoch::default(),
                // No caching, so a test that re-registers a binding sees
                // the change on the very next resolution.
                cache_ttl: Duration::ZERO,
            },
        );

        let provider = Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
        let guard_provider = provider.clone();
        let config = QueueConfig {
            retry: RetryPolicy {
                max_attempts,
                initial_backoff_ms: 1,
                backoff_multiplier: 2.0,
                max_backoff_ms: 4,
            },
            visibility_timeout_ms: 0,
            dlq_max_rows: 100,
        };
        let outbox = Arc::new(ProxyOutbox::new(
            provider,
            Arc::new(KeyStore::new()),
            resolver.clone(),
            config,
        ));
        let router = Arc::new(
            ProxyRouter::new(
                registry.clone(),
                empty_registry_client(),
                Arc::downgrade(&native_dispatch),
                Weak::new(),
                Arc::new(MockHop::default()),
                node_identity,
                RetryPolicy { max_attempts: 1, ..RetryPolicy::default() },
            )
            .with_dedup_guard(Arc::new(crate::CallDedupGuard::new(
                guard_provider,
                Arc::new(KeyStore::new()),
                syneroym_async_queue::DedupConfig {
                    ttl_ms: 600_000,
                    claim_window_ms: 60_000,
                    max_rows: 100,
                    max_result_bytes: 64 * 1024,
                },
            )))
            .with_outbox(outbox.clone()),
        );
        OutboxNode {
            router,
            registry,
            resolver,
            target,
            outbox,
            _native_dispatch: native_dispatch,
            dir,
        }
    }

    fn queued_call(target: QueuedTarget, key: &str) -> QueuedCall {
        QueuedCall {
            app_instance_id: Some("app-1".to_string()),
            caller_service_id: CALLER.to_string(),
            target,
            routing_key: None,
            interface: "greeter".to_string(),
            method: "greet".to_string(),
            params: Value::Null,
            idempotency_key: key.to_string(),
            protocol: None,
            timeout_ms: Some(500),
        }
    }

    impl OutboxNode {
        async fn queued(&self) -> Vec<syneroym_async_queue::QueueItem> {
            self.outbox.queue_for(CALLER).await.unwrap().all().unwrap()
        }

        async fn dead_letters(&self) -> Vec<syneroym_async_queue::DeadLetter> {
            self.outbox.queue_for(CALLER).await.unwrap().dead_letters().unwrap()
        }

        fn queue_file_exists(&self) -> bool {
            self.dir.path().join("services").join(CALLER).join("async.db").exists()
        }
    }

    /// The happy path: a reachable target costs one call and zero queue
    /// writes. Asserted as "untouched" -- no queue file is created at all.
    #[tokio::test]
    async fn an_enqueue_to_a_reachable_target_delivers_synchronously_and_never_touches_the_queue() {
        let node = outbox_node(true, 3).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();
        assert_eq!(node.target.invoked.load(Ordering::SeqCst), 1);
        assert!(!node.queue_file_exists(), "a delivered call must not create an outbox");
    }

    #[tokio::test]
    async fn an_enqueue_to_an_unreachable_target_lands_in_that_services_own_outbox() {
        let node = outbox_node(false, 3).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();
        let queued = node.queued().await;
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].queue_key, "k1", "the queue key is the idempotency key");
    }

    /// The same failure B1's review found the hard way, prevented here by
    /// construction: the queue key is the idempotency key, so a caller
    /// re-enqueueing the same logical operation gets one item, not two.
    #[tokio::test]
    async fn a_second_enqueue_for_the_same_key_while_one_is_pending_is_a_no_op() {
        let node = outbox_node(false, 3).await;
        for _ in 0..3 {
            node.router
                .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
                .await
                .unwrap();
        }
        assert_eq!(node.queued().await.len(), 1);
    }

    /// Without a certificate every delivery attempt would present as
    /// anonymous and be refused, so this fails at the call rather than ten
    /// hours later at the dead-letter table.
    #[tokio::test]
    async fn an_enqueue_from_a_service_with_no_unexpired_certificate_is_refused() {
        let node = outbox_node(true, 3).await;
        node.registry.remove_instance_cert(CALLER).await.unwrap();
        let result = node
            .router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await;
        assert!(matches!(result, Err(ProxyError::PermissionDenied(_))), "got {result:?}");
        assert!(!node.queue_file_exists());
    }

    /// The one caller identity that cannot be rebuilt at delivery, and a
    /// target that is local and running anyway.
    #[tokio::test]
    async fn an_enqueue_to_the_calling_service_itself_is_refused() {
        let node = outbox_node(true, 3).await;
        let result =
            node.router.enqueue(queued_call(QueuedTarget::Service(CALLER.to_string()), "k1")).await;
        assert!(matches!(result, Err(ProxyError::UnsupportedTarget(_))), "got {result:?}");
        assert!(!node.queue_file_exists());
    }

    #[tokio::test]
    async fn an_enqueue_to_a_node_level_interface_is_refused() {
        let node = outbox_node(true, 3).await;
        let mut call = queued_call(QueuedTarget::Service("did:key:zNode".into()), "k1");
        call.interface = "orchestrator".to_string();
        let result = node.router.enqueue(call).await;
        assert!(matches!(result, Err(ProxyError::PermissionDenied(_))), "got {result:?}");
    }

    /// A queued delivery must reach the receiver under the identity the
    /// live cross-service path builds, or authorization would silently
    /// differ between the immediate attempt and every later one.
    #[tokio::test]
    async fn a_delivered_call_carries_the_same_caller_identity_the_live_path_builds() {
        let node = outbox_node(true, 3).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();
        let immediate = node.target.last_caller_did.lock().unwrap().clone();
        assert_eq!(immediate.as_deref(), Some("system:did:key:zCaller"));

        // And the worker's own rebuild agrees with it.
        let queued = queued_call(QueuedTarget::Dependency("backend".into()), "k2");
        let rebuilt = node.router.request_from(&queued, "did:key:zTarget".to_string());
        assert_eq!(rebuilt.caller.caller_did, "system:did:key:zCaller");
        assert_eq!(rebuilt.origin, CallOrigin::Guest { service_id: CALLER.to_string() });
        assert_eq!(rebuilt.idempotency_key.as_deref(), Some("k2"));
        assert!(rebuilt.idempotent, "a keyed call is always retry-eligible");
    }

    /// The entire reason the payload stores intent: a binding re-pushed
    /// while the item waited has to take effect on the next attempt.
    #[tokio::test]
    async fn a_queued_call_resolves_its_dependency_again_at_delivery() {
        use syneroym_app_orchestration::{
            AppInstanceId, LogicalServiceName, ServiceId, TopologyEntry, TopologyEpoch,
            TopologyMode,
        };
        let node = outbox_node(false, 3).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();
        assert_eq!(node.queued().await.len(), 1);

        // Re-point the dependency at a member that actually answers, the
        // way a re-pushed binding would. It is a deployed service too, so
        // the receiver-side fence can open a store for it.
        let moved_dir = node.dir.path().join("services").join("did:key:zMoved");
        std::fs::create_dir_all(&moved_dir).unwrap();
        std::fs::write(moved_dir.join("state.db"), b"").unwrap();
        let reachable = Arc::new(RecordingNativeService::default());
        node._native_dispatch
            .insert("did:key:zMoved".to_string(), reachable.clone() as Arc<dyn NativeService>);
        node.registry
            .register(
                "did:key:zMoved".to_string(),
                "greeter".to_string(),
                SubstrateEndpoint::NativeHostChannel { service_id: "did:key:zMoved".to_string() },
            )
            .await
            .unwrap();
        node.resolver.register(
            AppInstanceId::new("app-1"),
            LogicalServiceName::new("backend"),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![ServiceId::new("did:key:zMoved")],
                sharding_strategy: None,
                epoch: TopologyEpoch::default(),
                cache_ttl: Duration::ZERO,
            },
        );

        node.router.drain_outboxes_once().await;
        assert_eq!(
            reachable.invoked.load(Ordering::SeqCst),
            1,
            "the re-pushed binding must take effect at delivery"
        );
        assert!(node.queued().await.is_empty(), "a delivered item leaves the outbox");
    }

    /// Failure-matrix row 9, raised by the worker itself before `invoke`
    /// rather than as a proxy error.
    #[tokio::test]
    async fn a_queued_call_whose_dependency_no_longer_resolves_is_terminal() {
        use syneroym_app_orchestration::{
            AppInstanceId, LogicalServiceName, TopologyEntry, TopologyEpoch, TopologyMode,
        };
        let node = outbox_node(false, 50).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();

        // The binding goes away entirely.
        node.resolver.register(
            AppInstanceId::new("app-1"),
            LogicalServiceName::new("backend"),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![],
                sharding_strategy: None,
                epoch: TopologyEpoch::default(),
                cache_ttl: Duration::ZERO,
            },
        );

        node.router.drain_outboxes_once().await;
        let dead = node.dead_letters().await;
        assert_eq!(dead.len(), 1, "an unresolvable dependency is terminal, not retried to budget");
        assert!(node.queued().await.is_empty());
    }

    /// A callee error has no reader on this path, so recording it is the
    /// only way it is ever seen -- the reverse of the synchronous rule.
    #[tokio::test]
    async fn a_callee_error_on_a_queued_item_dead_letters_rather_than_completing() {
        let node = outbox_node(false, 50).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();
        // Make the target answer definitively rather than being absent.
        node.registry
            .register(
                "did:key:zTarget".to_string(),
                "greeter".to_string(),
                SubstrateEndpoint::NativeHostChannel { service_id: "did:key:zTarget".to_string() },
            )
            .await
            .unwrap();
        node.target.fail_with.store(true, Ordering::SeqCst);

        node.router.drain_outboxes_once().await;
        assert_eq!(node.dead_letters().await.len(), 1);
        assert!(node.queued().await.is_empty());
    }

    /// The retryable case: the item stays, with an attempt spent.
    #[tokio::test]
    async fn a_transport_failure_retries_on_the_configured_schedule() {
        let node = outbox_node(false, 50).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();
        node.router.drain_outboxes_once().await;

        let queued = node.queued().await;
        assert_eq!(queued.len(), 1, "a retryable failure keeps the item");
        assert_eq!(queued[0].attempts, 1, "and spends exactly one attempt");
        assert!(node.dead_letters().await.is_empty());
    }

    /// The poison-pill ceiling: a delivery that never resolves through
    /// `fail`/`complete` still consumes a bounded number of claims.
    #[tokio::test]
    async fn an_item_whose_claim_count_reaches_the_budget_dead_letters() {
        let node = outbox_node(false, 2).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();
        let queue = node.outbox.queue_for(CALLER).await.unwrap();
        // Simulate claims that never resolved -- a crashed worker.
        for _ in 0..3 {
            queue.claim_due(proxy_outbox::now_ms(), 10).unwrap();
        }
        node.router.drain_outboxes_once().await;
        assert_eq!(
            node.dead_letters().await.len(),
            1,
            "a poison pill must not be handed out forever"
        );
    }

    /// Nothing removes a service's data directory on undeploy, so an
    /// outbox can outlive its service. Delivering would resurrect intent
    /// an operator withdrew; dead-lettering would raise noise nobody will
    /// act on.
    #[tokio::test]
    async fn an_item_for_an_undeployed_service_is_completed_not_delivered() {
        let node = outbox_node(false, 50).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();
        assert_eq!(node.queued().await.len(), 1);

        // The calling service is undeployed: its endpoints go away.
        node.registry.remove(CALLER, "greeter").await.ok();
        for (interface, _) in node.registry.lookup_by_service(CALLER) {
            node.registry.remove(CALLER, &interface).await.ok();
        }

        node.router.drain_outboxes_once().await;
        assert!(node.queued().await.is_empty(), "the item must be completed");
        assert!(
            node.dead_letters().await.is_empty(),
            "and silently -- not raised as a dead letter for a service nobody will act on"
        );
        assert_eq!(node.target.invoked.load(Ordering::SeqCst), 0, "and never delivered");
    }

    /// Cancellation must interrupt a delivery that genuinely never
    /// resolves, not merely win a race against the next tick -- a worker
    /// waiting out a call to an unreachable peer is exactly the case this
    /// queue exists for, so shutdown must not wait for it.
    #[tokio::test]
    async fn shutdown_abandons_an_in_flight_delivery_rather_than_draining() {
        let node = outbox_node(false, 50).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();

        let cancel = CancellationToken::new();
        let worker = {
            let router = node.router.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                router.run_outbox_worker(Duration::from_millis(5), cancel).await;
            })
        };
        // Let the worker get into its loop, then cancel.
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel.cancel();

        let stopped = tokio::time::timeout(Duration::from_secs(2), worker).await;
        assert!(
            stopped.is_ok(),
            "the worker must stop promptly on cancellation rather than draining its queue"
        );
    }

    // -- the dead-letter tier ----------------------------------------------

    /// A guest-origin call that failed for good, as the synchronous tier
    /// produces it: unreachable target, so the retry budget is exhausted.
    fn failing_guest_request(key: Option<&str>) -> ProxyRequest {
        let mut req = base_request("did:key:zTarget", "greeter");
        req.origin = CallOrigin::Guest { service_id: CALLER.to_string() };
        req.caller = CallerContext::service_system(CALLER);
        req.idempotency_key = key.map(str::to_string);
        req
    }

    /// D-B2-1's first tier, and the assertion that keeps the whole rule
    /// coherent: with no fence there is nothing safe to replay, so there
    /// is no row. The caller is alive and holding the error -- this is not
    /// silent loss.
    #[tokio::test]
    async fn an_unkeyed_call_that_exhausts_its_retries_writes_no_dead_letter() {
        let node = outbox_node(false, 50).await;
        let result = node.router.invoke(failing_guest_request(None)).await;
        assert!(result.is_err(), "the call must still fail to its caller");
        assert!(
            !node.queue_file_exists(),
            "an unfenced failure must leave no replayable record behind"
        );
    }

    /// The second tier: the row is *additional*, never a substitute for
    /// the caller's own error.
    #[tokio::test]
    async fn a_keyed_call_that_exhausts_its_retries_writes_a_dead_letter_and_still_returns_its_error()
     {
        let node = outbox_node(false, 50).await;
        let result = node.router.invoke(failing_guest_request(Some("k1"))).await;
        assert!(result.is_err(), "the caller must still get its error");

        let dead = node.dead_letters().await;
        assert_eq!(dead.len(), 1, "a keyed failure must also be recorded for an operator");
        assert_eq!(dead[0].queue_key, "k1");
        assert!(node.queued().await.is_empty(), "and must not linger in the outbox");
    }

    /// One permanently broken recipient must not be able to evict every
    /// other conversation's dead letters, which is what scoping the cap by
    /// target buys.
    #[tokio::test]
    async fn the_dlq_cap_is_scoped_per_target() {
        let node = outbox_node(false, 50).await;
        let queue = node.outbox.queue_for(CALLER).await.unwrap();

        // Two targets, and a cap that the first one alone would blow past.
        for i in 0..4 {
            let call =
                queued_call(QueuedTarget::Service("did:key:zNoisy".into()), &format!("n{i}"));
            node.outbox.record_dead_letter(&call, "unreachable").await.unwrap();
        }
        let quiet = queued_call(QueuedTarget::Service("did:key:zQuiet".into()), "q0");
        node.outbox.record_dead_letter(&quiet, "unreachable").await.unwrap();

        let keys: Vec<String> =
            queue.dead_letters().unwrap().into_iter().map(|d| d.queue_key).collect();
        assert!(
            keys.contains(&"q0".to_string()),
            "the quiet target's dead letter must survive the noisy one's overflow, got {keys:?}"
        );
    }

    fn keyed_request(key: &str) -> ProxyRequest {
        let mut req = base_request("svc-a", "greeter");
        req.caller = CallerContext::service_system("svc-caller");
        req.idempotency_key = Some(key.to_string());
        req
    }

    /// The `invoke_local` entry point. Together with the
    /// `dispatch_json_rpc_once` case, this is what makes "one guard, both
    /// entry points" a fact rather than an intention.
    #[tokio::test]
    async fn a_local_call_is_deduplicated_by_the_proxy() {
        let node = guarded_node(true).await;
        let first = node.router.invoke(keyed_request("k1")).await.unwrap();
        assert_eq!(first, Value::String("ok".to_string()));
        assert_eq!(node.service.invoked.load(Ordering::SeqCst), 1);

        let repeat = node.router.invoke(keyed_request("k1")).await.unwrap();
        assert_eq!(repeat, first, "the duplicate must get the first call's own result");
        assert_eq!(
            node.service.invoked.load(Ordering::SeqCst),
            1,
            "the target must not run a second time"
        );
    }

    /// A different key is a different call, so the target does run again
    /// -- the fence must not swallow genuinely distinct work.
    #[tokio::test]
    async fn a_different_key_is_a_different_call() {
        let node = guarded_node(true).await;
        node.router.invoke(keyed_request("k1")).await.unwrap();
        node.router.invoke(keyed_request("k2")).await.unwrap();
        assert_eq!(node.service.invoked.load(Ordering::SeqCst), 2);
    }

    /// An unkeyed call is untouched by any of this, including on a node
    /// that has a store: it executes every time, exactly as before.
    #[tokio::test]
    async fn an_unkeyed_call_is_never_deduplicated() {
        let node = guarded_node(true).await;
        let mut req = base_request("svc-a", "greeter");
        req.caller = CallerContext::service_system("svc-caller");
        node.router.invoke(req.clone()).await.unwrap();
        node.router.invoke(req).await.unwrap();
        assert_eq!(node.service.invoked.load(Ordering::SeqCst), 2);
    }

    /// Coordinator mode: no storage provider at all, so there is nowhere
    /// to remember a key. Refused rather than executed unfenced.
    #[tokio::test]
    async fn a_keyed_call_is_refused_on_a_node_with_no_storage_provider() {
        let node = guarded_node(false).await;
        let result = node.router.invoke(keyed_request("k1")).await;
        assert!(matches!(result, Err(ProxyError::Internal(_))), "got {result:?}");
        assert_eq!(node.service.invoked.load(Ordering::SeqCst), 0);
    }

    /// A denial or an unknown service happens before the target runs, so
    /// it must leave no claim behind -- otherwise a corrected retry is
    /// blocked for the whole claim window for something that never ran.
    #[tokio::test]
    async fn a_failure_before_the_target_ran_leaves_no_claim_behind() {
        let node = guarded_node(true).await;

        // A guest reaching another service's native capability is denied
        // by the capability gate, which runs before any dispatch.
        let mut denied = keyed_request("k1");
        denied.interface = "data-layer".to_string();
        denied.target_service = "svc-a".to_string();
        denied.origin = CallOrigin::Guest { service_id: "svc-caller".to_string() };
        assert!(matches!(node.router.invoke(denied).await, Err(ProxyError::PermissionDenied(_))));

        // The same key, corrected, must still be free to run.
        let corrected = node.router.invoke(keyed_request("k1")).await;
        assert!(corrected.is_ok(), "a corrected retry must not be blocked: {corrected:?}");
        assert_eq!(node.service.invoked.load(Ordering::SeqCst), 1);
    }

    /// The invariant that makes the local `system:<id>` and the remote DID
    /// namespaces safe to keep disjoint: whether a target is local or
    /// remote never changes between attempts, so one caller reaches one
    /// target under one identity every time.
    #[tokio::test]
    async fn one_caller_reaches_one_target_under_one_identity_on_every_attempt() {
        let node = guarded_node(true).await;
        node.router.invoke(keyed_request("k1")).await.unwrap();
        let first_identity = node.service.last_caller_did.lock().unwrap().clone();

        // A second, differently-keyed call from the same caller to the
        // same target arrives under the same identity.
        node.router.invoke(keyed_request("k2")).await.unwrap();
        assert_eq!(*node.service.last_caller_did.lock().unwrap(), first_identity);
        assert_eq!(first_identity.as_deref(), Some("system:svc-caller"));
    }

    #[derive(Debug, Default)]
    struct RecordingNativeService {
        invoked: AtomicUsize,
        last_caller_did: Mutex<Option<String>>,
        /// Makes the target answer definitively rather than being absent,
        /// so the queued path's callee-error classification can be driven.
        fail_with: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    impl NativeService for RecordingNativeService {
        async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
            self.invoked.fetch_add(1, Ordering::SeqCst);
            *self.last_caller_did.lock().unwrap() = Some(invocation.caller.caller_did.clone());
            if self.fail_with.load(Ordering::SeqCst) {
                return Err(RpcError::InternalError("the target says no".to_string()));
            }
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

    /// A0-01: `orchestrator`/`security` are node-level (registered under the
    /// node's own DID, not any deployed service's), so unlike the
    /// `NATIVE_CAPABILITY_INTERFACES` gate above there is no same-service
    /// exemption -- a guest whose own service is the node's own DID (which
    /// cannot legitimately happen, but a guest freely chooses
    /// `target_service`) must still be denied. Guards against a guest whose
    /// service holds an installed instance certificate (ADR-0020 §1) walking
    /// its now-verified identity into `orchestrator` (gated since the
    /// deploy-grant admission gate) or `security` (also gated on
    /// `substrate/admin` now) -- neither of which a guest could reach at all
    /// before that certificate mechanism existed, since a guest-origin call
    /// always presented anonymous.
    #[tokio::test]
    async fn guest_cannot_reach_node_level_orchestrator_or_security_through_the_proxy() {
        let registry = empty_registry();
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let service = Arc::new(RecordingNativeService::default());
        native_dispatch.insert("node-did".to_string(), service.clone() as Arc<dyn NativeService>);

        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            Arc::new(MockHop::default()),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy::default(),
        );

        for interface in ["orchestrator", "security"] {
            let mut req = base_request("node-did", interface);
            req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
            let result = router.invoke(req).await;
            assert!(
                matches!(result, Err(ProxyError::PermissionDenied(_))),
                "interface '{interface}' must be denied, got {result:?}"
            );
        }
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
        req.origin = CallOrigin::Native { service_id: None };
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
        req.caller.proof =
            Some(CallerProof { pubkey_hex: "deadbeef".to_string(), delegation_json: None });
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

    /// Slice B3.5-fdae's self-proxy branch (`host_capabilities.rs`) forwards
    /// a `CallOrigin::Guest` request that legitimately carries the real
    /// caller's proof when the target is the guest's own service. If that
    /// request ever falls through `invoke`'s local-registry lookup (an
    /// interface the local `EndpointRegistry` hasn't got, even for the
    /// guest's raw own `component_id` -- `check_native_capability_gate`
    /// only restricts *native-capability* interfaces cross-service, not
    /// this fallback), it must not present that proof, or this node's own
    /// identity, to a remote destination the guest fully chose the
    /// `(interface, method, params)` for. Same invariant as
    /// `guest_without_proof_forwards_as_anonymous_not_node_identity`, now
    /// pinned for a guest caller that *does* carry a proof.
    #[tokio::test]
    async fn guest_with_proof_still_forwards_as_anonymous_not_the_real_proof() {
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

        let mut req = base_request("guest-component", "some-unregistered-iface");
        req.caller.proof =
            Some(CallerProof { pubkey_hex: "deadbeef".to_string(), delegation_json: None });
        req.origin = CallOrigin::Guest { service_id: "guest-component".to_string() };
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(
            preamble.pubkey, None,
            "a guest-origin call must never present a proof (its own or the node's) to a remote \
             destination, even when `req.caller.proof` is `Some` -- otherwise a guest could \
             launder a real caller's identity onto the wire by steering a self-proxy call onto an \
             interface the local registry misses"
        );
    }

    struct EmptyAnchorResolver;
    #[async_trait::async_trait]
    impl MasterAnchorResolver for EmptyAnchorResolver {
        async fn resolve_master_anchor(
            &self,
            _master_id: &str,
        ) -> Result<MasterAnchorPayload, anyhow::Error> {
            Ok(MasterAnchorPayload::default())
        }
    }

    /// The slice's core claim: a service holding an installed instance
    /// certificate makes a guest-origin remote call under its own member
    /// master, not anonymous and not the node's identity -- and the
    /// destination's handshake (fed the exact preamble this router
    /// constructs) resolves that master, matching `HandshakeVerifier`'s
    /// contract end to end.
    #[tokio::test]
    async fn a_guest_call_travels_under_its_services_member_master_not_the_node_identity() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let node_identity = Arc::new(Identity::generate().unwrap());
        let registry = empty_registry();

        let owner_did = "did:key:zMemberOwner".to_string();
        let service_id = "guest-with-cert".to_string();
        registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

        let member_master = Identity::generate().unwrap();
        let member_master_did = substrate::derive_did_key(&member_master.public_key());
        let instance = node_identity.derive_service_identity(&owner_did, &service_id);
        let cert = DelegationCertificate::issue(
            &member_master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop.clone(),
            node_identity,
            RetryPolicy::default(),
        );

        let mut req = base_request("remote-svc", "greet");
        req.origin = CallOrigin::Guest { service_id: service_id.clone() };
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(
            preamble.pubkey.as_deref(),
            Some(hex::encode(instance.public_key().to_bytes()).as_str())
        );
        assert!(preamble.delegation.is_some());

        let verified = HandshakeVerifier::verify_preamble(&preamble, &EmptyAnchorResolver)
            .await
            .expect("the destination's handshake must admit a service-instance certificate");
        assert_eq!(verified.master_did, member_master_did);
    }

    /// The unchanged path (the migration guarantee): a service with no
    /// installed certificate presents nothing, exactly like before this arm
    /// existed.
    #[tokio::test]
    async fn a_guest_call_from_a_service_without_a_certificate_is_still_anonymous() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let registry = empty_registry();
        registry.set_owner("no-cert-svc".to_string(), "did:key:zOwner".to_string()).await.unwrap();

        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop.clone(),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy::default(),
        );

        let mut req = base_request("remote-svc", "greet");
        req.origin = CallOrigin::Guest { service_id: "no-cert-svc".to_string() };
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(preamble.pubkey, None);
        assert!(preamble.delegation.is_none());
    }

    /// A0-07: an already-expired installed certificate must fall back to
    /// anonymous exactly like no certificate at all -- not get attached and
    /// then hard-rejected at the destination (`route_handler/io.rs` rejects
    /// any connection whose delegation fails to verify), which would turn a
    /// missed renewal into an outage for passthrough/relay calls that
    /// tolerated anonymous before this certificate mechanism existed.
    #[tokio::test]
    async fn a_guest_call_from_a_service_with_an_expired_certificate_is_anonymous_not_rejected() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let node_identity = Arc::new(Identity::generate().unwrap());
        let registry = empty_registry();

        let owner_did = "did:key:zMemberOwner".to_string();
        let service_id = "expired-cert-svc".to_string();
        registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

        let member_master = Identity::generate().unwrap();
        let instance = node_identity.derive_service_identity(&owner_did, &service_id);
        let cert = DelegationCertificate::issue(
            &member_master,
            instance.public_key(),
            0,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        assert!(cert.is_expired());
        registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop.clone(),
            node_identity,
            RetryPolicy::default(),
        );

        let mut req = base_request("remote-svc", "greet");
        req.origin = CallOrigin::Guest { service_id: service_id.clone() };
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(preamble.pubkey, None);
        assert!(preamble.delegation.is_none());
    }

    /// A2's transport half: a substrate-internal call made on a deployed
    /// service's behalf (the FDAE relationship-proof fetch) presents that
    /// service's certified instance key, not the node's -- the same
    /// reasoning A0 applied to the guest-origin arm, now at the
    /// `(None, Native { service_id: Some(_) })` site.
    #[tokio::test]
    async fn a_native_origin_call_on_a_services_behalf_presents_that_services_instance_key() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let node_identity = Arc::new(Identity::generate().unwrap());
        let registry = empty_registry();

        let owner_did = "did:key:zMemberOwner".to_string();
        let service_id = "hr-svc".to_string();
        registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

        let member_master = Identity::generate().unwrap();
        let member_master_did = substrate::derive_did_key(&member_master.public_key());
        let instance = node_identity.derive_service_identity(&owner_did, &service_id);
        let cert = DelegationCertificate::issue(
            &member_master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop.clone(),
            node_identity,
            RetryPolicy::default(),
        );

        let mut req = base_request("remote-svc", "data-layer");
        req.caller.proof = None;
        req.origin = CallOrigin::Native { service_id: Some(service_id.clone()) };
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(
            preamble.pubkey.as_deref(),
            Some(hex::encode(instance.public_key().to_bytes()).as_str())
        );
        assert!(preamble.delegation.is_some());

        let verified = HandshakeVerifier::verify_preamble(&preamble, &EmptyAnchorResolver)
            .await
            .expect("the destination's handshake must admit a service-instance certificate");
        assert_eq!(verified.master_did, member_master_did);
    }

    /// D-B3-9's guard: when the caller already carries a signed proof (a
    /// forwarded chain), a `Native` call must forward that proof verbatim
    /// -- never substitute the service's own identity -- so the destination
    /// can re-derive `subject_did`/`anchor_did` from the real chain. This is
    /// what keeps FDAE's cross-service fetch authorizing the *real* caller
    /// rather than the relaying service.
    #[tokio::test]
    async fn a_native_origin_call_with_a_caller_proof_still_forwards_the_proof_verbatim() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let node_identity = Arc::new(Identity::generate().unwrap());
        let registry = empty_registry();

        let owner_did = "did:key:zMemberOwner".to_string();
        let service_id = "hr-svc".to_string();
        registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();
        let member_master = Identity::generate().unwrap();
        let instance = node_identity.derive_service_identity(&owner_did, &service_id);
        let cert = DelegationCertificate::issue(
            &member_master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop.clone(),
            node_identity,
            RetryPolicy::default(),
        );

        let forwarded_pubkey_hex = "aabbccdd".to_string();
        let mut req = base_request("remote-svc", "data-layer");
        req.caller.proof =
            Some(CallerProof { pubkey_hex: forwarded_pubkey_hex.clone(), delegation_json: None });
        req.origin = CallOrigin::Native { service_id: Some(service_id.clone()) };
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(
            preamble.pubkey.as_deref(),
            Some(forwarded_pubkey_hex.as_str()),
            "an already-present caller proof must be forwarded verbatim, never replaced by the \
             relaying service's own certified identity"
        );
    }

    /// A native-origin call made on nobody's behalf (`service_id: None`,
    /// substrate-internal tooling, tests) keeps presenting the node's own
    /// identity, exactly as before this arm existed.
    #[tokio::test]
    async fn a_native_origin_call_with_no_service_id_still_presents_the_node_identity() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let node_identity = Arc::new(Identity::generate().unwrap());
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let router = ProxyRouter::new(
            empty_registry(),
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop.clone(),
            node_identity.clone(),
            RetryPolicy::default(),
        );

        let mut req = base_request("remote-svc", "data-layer");
        req.caller.proof = None;
        req.origin = CallOrigin::Native { service_id: None };
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(
            preamble.pubkey.as_deref(),
            Some(hex::encode(node_identity.public_key().to_bytes()).as_str()),
            "a node-level native call (no service_id) must still present the node's own identity"
        );
        assert!(preamble.delegation.is_none());
    }

    /// The same fallback A0 gave the guest arm: an installed-but-expired
    /// certificate must fall back to the node identity, not anonymous -- a
    /// substrate-internal call has always presented *something*, and
    /// dropping to anonymous would break native-dispatch destinations that
    /// reject an anonymous caller outright.
    #[tokio::test]
    async fn a_native_origin_call_with_an_expired_certificate_falls_back_to_the_node_identity() {
        let hop = Arc::new(MockHop::with_outcomes(vec![MockOutcome::Success(Value::Null)]));
        let node_identity = Arc::new(Identity::generate().unwrap());
        let registry = empty_registry();

        let owner_did = "did:key:zMemberOwner".to_string();
        let service_id = "expired-cert-native-svc".to_string();
        registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();

        let member_master = Identity::generate().unwrap();
        let instance = node_identity.derive_service_identity(&owner_did, &service_id);
        let cert = DelegationCertificate::issue(
            &member_master,
            instance.public_key(),
            0,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        assert!(cert.is_expired());
        registry.set_instance_cert(service_id.clone(), cert).await.unwrap();

        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Arc::downgrade(&native_dispatch),
            Weak::new(),
            hop.clone(),
            node_identity.clone(),
            RetryPolicy::default(),
        );

        let mut req = base_request("remote-svc", "data-layer");
        req.caller.proof = None;
        req.origin = CallOrigin::Native { service_id: Some(service_id.clone()) };
        router.invoke_remote_at(&synthetic_addr(), &req).await.unwrap();

        let preamble = hop.last_preamble.lock().unwrap().clone().unwrap();
        assert_eq!(
            preamble.pubkey.as_deref(),
            Some(hex::encode(node_identity.public_key().to_bytes()).as_str()),
            "an expired certificate must fall back to the node identity, not anonymous -- a \
             native-origin call has always presented something"
        );
        assert!(preamble.delegation.is_none());
    }
}

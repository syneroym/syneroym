//! Universal Proxy dispatch (M04A Slice A1): a transport-agnostic outbound
//! [`ServiceProxy`] implementation. Routes a typed `(service, interface,
//! method, params)` call to a local native service, a local WASM component,
//! or a remote node over Iroh QUIC + JSON-RPC, with retry/backoff hook
//! points. The trait itself lives in `syneroym-rpc`; `ProxyRouter` is its
//! only implementation.

use std::{
    collections::BTreeSet,
    fmt::{self, Debug, Formatter},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

use iroh::{Endpoint, EndpointAddr};
use serde_json::Value;
use syneroym_app_orchestration::saga_undo_name;
use syneroym_async_queue::{
    CALL_ALREADY_RUNNING_RPC_CODE, CALL_RESULT_NOT_RETAINED_RPC_CODE, CompensationOutcome,
    FailOutcome, MAX_SAGA_PAYLOAD_BYTES, MIN_STEP_CALL_BUDGET_MS, Queue, SagaHead,
    SagaInfo as QueueSagaInfo, SagaLog, StepIntent, StepRow,
};
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
    CallOrigin, CallerContext, DEFAULT_PROXY_CALL_TIMEOUT, DeadLetterInfo, JsonRpcErrorResponse,
    JsonRpcRequest, JsonRpcResponse, NativeInvocation, ProxyError, ProxyProtocol,
    ProxyQueueInspector, ProxyRequest, QueuedCall, QueuedCallInfo, QueuedTarget, RpcError,
    SERVICE_NOT_FOUND_RPC_CODE, SagaBegin, SagaInfo, SagaState as RpcSagaState, SagaStepRequest,
    ServiceProxy, WeakNativeDispatchRegistry, framing,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::{task, time};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::{
    call_dedup::{self, CallDedupGuard, GuardOutcome},
    net_iroh,
    preamble::RoutePreamble,
    proxy_outbox::{self, Disposition, ProxyOutbox},
    saga::SagaStore,
};

/// Whether `error` came from the call actually reaching its target, as
/// opposed to being the receiver-side fence's own answer or a refusal
/// raised before anything was attempted.
///
/// Only the former is worth a dead letter: a dead letter exists to be
/// replayed, and replaying a refusal just re-earns the refusal.
fn target_produced(error: &ProxyError) -> bool {
    match error {
        ProxyError::Callee { code, .. } => {
            *code != CALL_ALREADY_RUNNING_RPC_CODE && *code != CALL_RESULT_NOT_RETAINED_RPC_CODE
        }
        ProxyError::Transport(_) | ProxyError::Timeout(_) => true,
        // A target that is not found *is* worth a row, and this is the
        // one place the two classifiers have to be read together. The
        // queued path treats not-found as retryable, because a node
        // republishes its endpoint record before its services finish
        // coming up, so the answer is often "not yet" rather than "no".
        // The same reasoning applies here: the synchronous caller has
        // exhausted its own budget against a target that may simply have
        // been mid-restart, which is exactly a call worth being able to
        // replay later. Excluding it would silently drop the second tier
        // of the dead-letter rule for the most transient failure there is.
        ProxyError::ServiceNotFound(_) => true,
        // Raised instead of a dispatch, and settled: a denied gate, an
        // unusable target kind, a protocol this node does not speak, or a
        // store it could not open. Replaying any of these re-earns the
        // same refusal.
        ProxyError::PermissionDenied(_)
        | ProxyError::UnsupportedTarget(_)
        | ProxyError::UnsupportedProtocol(_)
        | ProxyError::Internal(_) => false,
    }
}

/// How long `enqueue`'s immediate try-then-queue attempt may take before
/// the item is simply queued instead.
///
/// Deliberately well under the sandbox's own `dispatch_epoch_timeout_secs`
/// (5s by default): a guest calling a fire-and-forget verb must get an
/// answer promptly whatever the target is doing, and anything this probe
/// would have waited longer to learn is something the outbox worker will
/// find out on its own schedule.
const ENQUEUE_PROBE_BUDGET: Duration = Duration::from_secs(2);

/// Bounds one saga undo's own call attempt. `compensate_next_step` sends up
/// to `SAGA_SWEEP_LIMIT` undos per service, sequentially, from
/// `run_async_worker` -- the same task that drains every service's outbox.
/// Left to `DEFAULT_PROXY_CALL_TIMEOUT`'s 30s default, one saga stuck on an
/// unreachable provider could hold that shared loop, and therefore every
/// other service's delivery, for minutes. A saga is not a probe -- it is a
/// real delivery attempt on its own retry schedule -- so this is longer
/// than `ENQUEUE_PROBE_BUDGET`, just not unbounded.
const SAGA_UNDO_CALL_BUDGET: Duration = Duration::from_secs(5);

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
    /// The durable saga step log behind `syneroym:proxy/saga`, one log per
    /// driving service. Same `None` reasoning as `outbox`.
    sagas: Option<Arc<SagaStore>>,
    /// Services the saga sweep saw absent from the registry on the
    /// *previous* tick -- one tick's grace before their saga log is
    /// dropped. Undeploy removes a service's endpoints one interface at a
    /// time, so a service with real open sagas can be transiently absent
    /// from `get_all_endpoints()` for a single tick during a clean
    /// redeploy. Unlike the outbox's undeployed-service branch, which only
    /// completes not-yet-delivered intent, dropping a saga log destroys
    /// the only record a compensation would ever need -- one miss must
    /// not be enough to do that.
    saga_undeploy_candidates: Mutex<BTreeSet<String>>,
    /// `outbox` and `sagas` bundled behind one [`ProxyQueueInspector`]
    /// handle, rebuilt whenever either changes. `ProxyRouter`
    /// itself is this bundle's one strong owner -- the control plane's
    /// `proxy_queues: OnceLock<Weak<dyn ProxyQueueInspector>>` downgrades
    /// from it, the same way it already downgrades from `outbox` alone
    /// today, so the `Weak` stays valid for as long as this router does.
    proxy_state: Option<Arc<ProxyState>>,
}

/// The two durable per-service stores the operator verbs read, behind one
/// handle: a service's durable proxy state is one question to an operator,
/// not two.
pub struct ProxyState {
    outbox: Arc<ProxyOutbox>,
    sagas: Arc<SagaStore>,
}

impl std::fmt::Debug for ProxyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyState").finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl ProxyQueueInspector for ProxyState {
    async fn queued_calls(&self, service_id: &str) -> Result<Vec<QueuedCallInfo>, String> {
        self.outbox.queued_calls(service_id).await
    }

    async fn dead_letters(&self, service_id: &str) -> Result<Vec<DeadLetterInfo>, String> {
        self.outbox.dead_letters(service_id).await
    }

    async fn replay_dead_letter(&self, service_id: &str, id: u64) -> Result<(), String> {
        self.outbox.replay_dead_letter(service_id, id).await
    }

    async fn sagas(&self, service_id: &str) -> Result<Vec<SagaInfo>, String> {
        let Some(log) = self.sagas.existing_log_for(service_id).await.map_err(|e| e.to_string())?
        else {
            return Ok(Vec::new());
        };
        let items = task::spawn_blocking(move || log.list())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        Ok(items.into_iter().map(rpc_saga_info_from).collect())
    }

    async fn rearm_saga(&self, service_id: &str, saga_id: &str) -> Result<(), String> {
        let Some(log) = self.sagas.existing_log_for(service_id).await.map_err(|e| e.to_string())?
        else {
            return Err(format!("service '{service_id}' has no durable saga log"));
        };
        let now = proxy_outbox::now_ms();
        let id = saga_id.to_string();
        let rearmed = task::spawn_blocking(move || log.rearm(&id, now))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        if rearmed {
            Ok(())
        } else {
            Err(format!(
                "saga '{saga_id}' is not failed -- only a failed saga can be re-armed to \
                 compensate"
            ))
        }
    }
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
            sagas: None,
            saga_undeploy_candidates: Mutex::new(BTreeSet::new()),
            proxy_state: None,
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
        self.rebuild_proxy_state();
        self
    }

    #[must_use]
    pub fn outbox(&self) -> Option<&Arc<ProxyOutbox>> {
        self.outbox.as_ref()
    }

    /// Gives this router the durable saga step log behind
    /// `syneroym:proxy/saga`.
    #[must_use]
    pub fn with_sagas(mut self, sagas: Arc<SagaStore>) -> Self {
        self.sagas = Some(sagas);
        self.rebuild_proxy_state();
        self
    }

    #[must_use]
    pub fn sagas(&self) -> Option<&Arc<SagaStore>> {
        self.sagas.as_ref()
    }

    /// Rebuilds the `outbox`+`sagas` bundle once both are present, so
    /// [`Self::proxy_state`] always reflects the router's own current
    /// handles.
    fn rebuild_proxy_state(&mut self) {
        if let (Some(outbox), Some(sagas)) = (&self.outbox, &self.sagas) {
            self.proxy_state =
                Some(Arc::new(ProxyState { outbox: outbox.clone(), sagas: sagas.clone() }));
        }
    }

    /// The `outbox`+`sagas` bundle the control plane's operator verbs read
    /// through, once this router has both. `None` until then -- a node
    /// with only one of the two (a test harness, most often) has no
    /// combined view to offer.
    #[must_use]
    pub fn proxy_state(&self) -> Option<&Arc<ProxyState>> {
        self.proxy_state.as_ref()
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

        // Only a failure the *target* produced earns a row. The fence's
        // own answers are not delivery failures at all:
        //
        // - "already running here" means the call is succeeding on another task right
        //   now, so a dead letter for it would be indistinguishable from a genuine
        //   exhausted delivery and would never be cleared when the real call finished;
        // - "already ran, result too large to retain" is a delivery;
        // - a fail-closed refusal (no store, anonymous caller, node-level target) means
        //   nothing was attempted, so there is nothing to replay -- and replaying would
        //   hit the same refusal.
        if !target_produced(error) {
            return;
        }

        // A queued item that failed is dead-lettered by the worker, which
        // owns its own retry history; this path is only for the caller
        // that is still holding the error.
        let call = QueuedCall {
            app_instance_id: None,
            caller_service_id: service_id.clone(),
            target: QueuedTarget::Service(req.target_service.clone()),
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
    /// Returns `Err` only when there is nothing to deliver *to* -- the
    /// stored dependency name no longer resolves to any member.
    ///
    /// Kept separate from the delivery itself because the two failures are
    /// not the same kind. "This name is bound to nobody" is settled: no
    /// number of retries invents a member, so it is terminal. "I could not
    /// reach the member it is bound to" is not settled at all, and is
    /// handled by the ordinary retry classification.
    ///
    /// Re-resolved on every attempt and never stored, so a binding
    /// re-pushed while the item waited takes effect (ADR-0021 §2).
    fn resolve_queued_target(&self, call: &QueuedCall) -> Result<String, ProxyError> {
        self.outbox
            .as_ref()
            .ok_or_else(|| ProxyError::Internal("no durable outbox on this node".to_string()))?
            .resolve_target(call)
    }

    async fn deliver_queued(&self, call: &QueuedCall, target: String) -> Result<Value, ProxyError> {
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
        // Keyed on the *resolved* endpoint's service id, not the id the
        // caller addressed. The wire entry point keys on the resolved one
        // too, and for a native channel registered under a different id
        // than it is addressed by the two disagree -- which would make
        // "one guard, both entry points" one guard reading two different
        // keys.
        let store_owner = match &endpoint {
            SubstrateEndpoint::NativeHostChannel { service_id }
            | SubstrateEndpoint::WasmChannel { service_id } => service_id.clone(),
            _ => req.target_service.clone(),
        };
        match guard
            .begin(
                &store_owner,
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

        // An empty interface is D-S3-15's convenience for a caller that
        // cannot know a remote service's interface names -- a gateway or
        // coordinator resolving an external hostname. A WASM guest is
        // never in that position: it always names the interface it wants.
        // Refused here, before `registry.lookup` gets a chance to resolve
        // it to "the one app-declared interface" of whatever
        // `target_service` names (finding A4) -- `matches_interface`
        // below can never match `""` against a real interface name, so
        // nothing past this point would otherwise have stopped it.
        if req.interface.is_empty() {
            return Err(ProxyError::PermissionDenied(format!(
                "component '{service_id}' must name an interface; the proxy does not resolve an \
                 empty interface for a guest call"
            )));
        }

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

        // The immediate attempt is a *probe*, not a delivery, so it runs
        // under its own tight bound rather than the call's full budget.
        //
        // Without this the guest waits out a doomed connect and every
        // retry underneath it -- comfortably past the sandbox's own
        // `dispatch_epoch_timeout_secs` (5s), which interrupts the guest
        // mid-call and turns a successful "accepted for delivery" into a
        // trap. That is the opposite of what a fire-and-forget verb owes
        // its caller, and the outbox already *is* the retry mechanism, so
        // there is nothing to gain by waiting longer here.
        let probe = time::timeout(
            ENQUEUE_PROBE_BUDGET,
            self.invoke_inner(&self.request_from(&call, target)),
        )
        .await;
        match probe {
            Ok(Ok(_)) => Ok(()),
            // The probe ran out of its own budget: nothing is known about
            // the target, which is exactly the retryable case.
            Err(_) => {
                warn!("proxy enqueue probe timed out; queueing");
                outbox.store(&call).await
            }
            // Matched on the enum rather than compared against one
            // variant, so a future third disposition is a compile error
            // here instead of silently falling through to "terminal" --
            // which is exactly how `Delivered` slipped past this arm when
            // it was added for the worker.
            Ok(Err(e)) => match proxy_outbox::disposition_of(&e) {
                // The receiver already ran this call and could not hand
                // its result back. Reporting that to the guest as a
                // failure would be the same confusion the queued path was
                // fixed for, one layer up: the call succeeded.
                Disposition::Delivered => Ok(()),
                Disposition::Retry => {
                    warn!(error = %e, "proxy enqueue could not deliver now; queueing");
                    outbox.store(&call).await
                }
                // Terminal on the very first attempt: the caller is still
                // here to be told, which is a better answer than a dead
                // letter it cannot read.
                Disposition::Terminal => Err(e),
            },
        }
    }
}

fn no_saga_store_error() -> ProxyError {
    ProxyError::Internal(
        "this node keeps no per-service storage, so it has no durable saga log".to_string(),
    )
}

fn parse_rpc_saga_state(state: &str) -> RpcSagaState {
    match state {
        "open" => RpcSagaState::Open,
        "compensating" => RpcSagaState::Compensating,
        "compensated" => RpcSagaState::Compensated,
        // A stored state this router does not recognise is a schema-level
        // bug, not a value an operator should ever act on differently from
        // an ordinary failure -- `SagaLog` itself already validates every
        // state it reads back, so this arm is unreachable in practice.
        _ => RpcSagaState::Failed,
    }
}

fn rpc_saga_info_from(info: QueueSagaInfo) -> SagaInfo {
    SagaInfo {
        saga_id: info.saga_id,
        name: info.name,
        state: parse_rpc_saga_state(&info.state),
        steps: info.steps,
        compensated_steps: info.compensated_steps,
        created_at: info.created_at,
        deadline_at: info.deadline_at,
        last_error: info.last_error,
    }
}

/// Builds the live cross-service call one saga step's own forward call
/// sends -- byte-identical in shape to `request_from`'s reasoning for a
/// queued call: the identity has to match what `proxy::Host::call` would
/// build for the same call, or authorization at the receiver would
/// silently diverge between a live step and one replayed later.
fn request_from_step(
    req: &SagaStepRequest,
    target_service: String,
    timeout_ms: u64,
) -> ProxyRequest {
    ProxyRequest {
        target_service,
        interface: req.interface.clone(),
        method: req.method.clone(),
        params: req.params.clone(),
        caller: CallerContext::service_system(&req.caller_service_id),
        origin: CallOrigin::Guest { service_id: req.caller_service_id.clone() },
        protocol: ProxyProtocol::parse(req.protocol.as_deref()).unwrap_or(ProxyProtocol::JsonRpcV1),
        idempotent: req.idempotency_key.is_some(),
        idempotency_key: req.idempotency_key.clone(),
        timeout: Some(Duration::from_millis(timeout_ms)),
    }
}

impl ProxyRouter {
    /// Opens a saga and returns its host-minted id (never guest-chosen).
    /// Refuses up front on the same two grounds `enqueue` does, for the
    /// identical reason: every undo this saga may later send travels under
    /// the caller's own identity, so a caller with no unexpired instance
    /// certificate would have every one of them refused as anonymous.
    async fn saga_begin_impl(&self, req: SagaBegin) -> Result<String, ProxyError> {
        let Some(store) = &self.sagas else { return Err(no_saga_store_error()) };

        let cert = self
            .registry
            .instance_cert(&req.caller_service_id)
            .filter(|c| !c.is_expired())
            .ok_or_else(|| {
                ProxyError::PermissionDenied(format!(
                    "service '{}' holds no unexpired instance certificate, so every undo this \
                     saga may later send would be refused as anonymous",
                    req.caller_service_id
                ))
            })?;

        let config = store.config().clone();
        let deadline_ms = match req.deadline_secs {
            None => config.default_deadline_ms,
            Some(secs) => {
                let ms = i64::try_from(secs.saturating_mul(1000)).unwrap_or(i64::MAX);
                if ms > config.max_deadline_ms {
                    return Err(ProxyError::PermissionDenied(format!(
                        "requested saga deadline {secs}s exceeds this node's ceiling of {}s",
                        config.max_deadline_ms / 1000
                    )));
                }
                ms
            }
        };

        let now = proxy_outbox::now_ms();
        // `deadline_ms` above is a duration; `sagas.deadline_at` is the
        // absolute instant `SagaLog::abandoned` compares against `now`.
        let deadline_at = now.saturating_add(deadline_ms);

        // Not a refusal: a *managed* instance's certificate is renewed on
        // every supervisor pass, so its own current expiry cannot decide
        // whether a long deadline is sound. An unmanaged instance's can,
        // and this is the only signal the host has for it.
        let cert_expires_ms =
            i64::try_from(cert.expires_at_secs.saturating_mul(1000)).unwrap_or(i64::MAX);
        if deadline_at > cert_expires_ms {
            warn!(
                caller = %req.caller_service_id,
                deadline_at,
                cert_expires_at = cert_expires_ms,
                "saga deadline outlives the caller's current instance certificate; a service \
                 whose certificate is not renewed before then cannot compensate past its expiry"
            );
        }

        let saga_id = uuid::Uuid::new_v4().to_string();
        let log = store.log_for(&req.caller_service_id).await?;
        let id = saga_id.clone();
        let name = req.name.clone();
        let app_instance_id = req.app_instance_id.clone();
        task::spawn_blocking(move || {
            log.begin(&id, &name, app_instance_id.as_deref(), deadline_at, now)
        })
        .await
        .map_err(|e| ProxyError::Internal(format!("saga begin task failed: {e}")))?
        .map_err(|e| ProxyError::Internal(e.to_string()))?;
        metrics::counter!("substrate.proxy.saga.opened").increment(1);
        Ok(saga_id)
    }

    /// Takes one forward step: records its intent, dispatches the call, and
    /// records the outcome. Refuses the same two node/self targets
    /// `enqueue` refuses, and for the same reason: a node-level target's
    /// undo can never be fenced, and a self-target's undo cannot be
    /// rebuilt under the right caller identity.
    async fn saga_step_impl(&self, req: SagaStepRequest) -> Result<Value, ProxyError> {
        let started = Instant::now();
        let Some(store) = &self.sagas else { return Err(no_saga_store_error()) };

        if call_dedup::is_node_level_interface(&req.interface) {
            return Err(ProxyError::PermissionDenied(format!(
                "node-level interface '{}' cannot take part in a saga: its compensation could \
                 never be fenced, so the first undo would fail the whole saga",
                req.interface
            )));
        }

        let log = store.log_for(&req.caller_service_id).await?;
        let target = store.resolve_step_target(
            req.app_instance_id.as_deref(),
            &req.target,
            req.routing_key.as_deref(),
        )?;

        if target == req.caller_service_id {
            return Err(ProxyError::UnsupportedTarget(format!(
                "service '{}' cannot make a saga step against itself: its own compensation would \
                 run under a different caller identity than the step did",
                req.caller_service_id
            )));
        }

        let target_json = serde_json::to_string(&req.target)
            .map_err(|e| ProxyError::Internal(format!("saga step target unencodable: {e}")))?;
        let params_bytes = serde_json::to_vec(&req.params)
            .map_err(|e| ProxyError::Internal(format!("saga step params unencodable: {e}")))?;
        if params_bytes.len() > MAX_SAGA_PAYLOAD_BYTES {
            return Err(ProxyError::Internal(format!(
                "saga step params exceed the {MAX_SAGA_PAYLOAD_BYTES} byte limit"
            )));
        }
        let intent = StepIntent {
            target: target_json,
            routing_key: req.routing_key.clone(),
            interface: req.interface.clone(),
            method: req.method.clone(),
            params: params_bytes,
        };

        let now = proxy_outbox::now_ms();
        let saga_id = req.saga_id.clone();
        let log_for_write = log.clone();
        let idx =
            task::spawn_blocking(move || log_for_write.record_step_intent(&saga_id, &intent, now))
                .await
                .map_err(|e| ProxyError::Internal(format!("saga step task failed: {e}")))?
                .map_err(|e| ProxyError::Internal(e.to_string()))?;

        // Budget: the guest's own `timeout_ms` when it named one, otherwise
        // what is *left* of this node's step budget after this function's
        // own bookkeeping so far (the log open plus the intent write) --
        // one second inside the guest's epoch, not the proxy's 30s default,
        // and minus the bookkeeping rather than ignoring it.
        let budget_ms = req.timeout_ms.unwrap_or_else(|| {
            step_call_budget_ms(store.config().step_timeout_ms, started.elapsed())
        });

        // `invoke`, not `invoke_inner`: a keyed step still earns the
        // guest proxy outbox's own dead letter on failure, an operator
        // surface this reuses rather than duplicates.
        let outcome = self.invoke(request_from_step(&req, target, budget_ms)).await;

        let saga_id = req.saga_id.clone();
        let now = proxy_outbox::now_ms();
        match &outcome {
            Ok(value) => {
                let result_bytes = serde_json::to_vec(value).ok();
                let _ = task::spawn_blocking(move || {
                    log.record_step_outcome(&saga_id, idx, result_bytes.as_deref(), None, now)
                })
                .await;
            }
            Err(e) => {
                let error_text = e.to_string();
                let _ = task::spawn_blocking(move || {
                    log.record_step_outcome(&saga_id, idx, None, Some(&error_text), now)
                })
                .await;
            }
        }
        outcome
    }

    /// The workflow reached its goal: drops the log. Refuses on anything
    /// but `open`.
    async fn saga_commit_impl(&self, service_id: &str, saga_id: &str) -> Result<(), ProxyError> {
        let Some(store) = &self.sagas else { return Err(no_saga_store_error()) };
        let log = store.log_for(service_id).await?;
        let id = saga_id.to_string();
        task::spawn_blocking(move || log.commit(&id))
            .await
            .map_err(|e| ProxyError::Internal(format!("saga commit task failed: {e}")))?
            .map_err(|e| ProxyError::Internal(e.to_string()))?;
        metrics::counter!("substrate.proxy.saga.committed").increment(1);
        Ok(())
    }

    /// Marks the saga for compensation. Returns immediately -- the walk
    /// runs on the async worker's next tick, never inline. Idempotent:
    /// asking an already-compensating saga to compensate again is a no-op,
    /// not an error.
    async fn saga_compensate_impl(
        &self,
        service_id: &str,
        saga_id: &str,
    ) -> Result<(), ProxyError> {
        let Some(store) = &self.sagas else { return Err(no_saga_store_error()) };
        let log = store.log_for(service_id).await?;
        let now = proxy_outbox::now_ms();
        let id = saga_id.to_string();
        let log_for_mark = log.clone();
        let transitioned = task::spawn_blocking(move || log_for_mark.mark_compensating(&id, now))
            .await
            .map_err(|e| ProxyError::Internal(format!("saga compensate task failed: {e}")))?
            .map_err(|e| ProxyError::Internal(e.to_string()))?;
        if transitioned {
            return Ok(());
        }
        let id = saga_id.to_string();
        let info = task::spawn_blocking(move || log.status(&id))
            .await
            .map_err(|e| ProxyError::Internal(format!("saga status task failed: {e}")))?
            .map_err(|e| ProxyError::Internal(e.to_string()))?;
        match info {
            None => Err(ProxyError::Internal(format!("unknown saga {saga_id}"))),
            Some(info) if info.state == "compensating" => Ok(()),
            Some(info) => Err(ProxyError::Internal(format!(
                "saga {saga_id} is {}; only an open saga can be asked to compensate",
                info.state
            ))),
        }
    }

    async fn saga_status_impl(
        &self,
        service_id: &str,
        saga_id: &str,
    ) -> Result<SagaInfo, ProxyError> {
        let Some(store) = &self.sagas else { return Err(no_saga_store_error()) };
        let Some(log) = store.existing_log_for(service_id).await? else {
            return Err(ProxyError::Internal(format!("unknown saga {saga_id}")));
        };
        let id = saga_id.to_string();
        let info = task::spawn_blocking(move || log.status(&id))
            .await
            .map_err(|e| ProxyError::Internal(format!("saga status task failed: {e}")))?
            .map_err(|e| ProxyError::Internal(e.to_string()))?
            .ok_or_else(|| ProxyError::Internal(format!("unknown saga {saga_id}")))?;
        Ok(rpc_saga_info_from(info))
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

    async fn saga_begin(&self, req: SagaBegin) -> Result<String, ProxyError> {
        ProxyRouter::saga_begin_impl(self, req).await
    }

    async fn saga_step(&self, req: SagaStepRequest) -> Result<Value, ProxyError> {
        ProxyRouter::saga_step_impl(self, req).await
    }

    async fn saga_commit(&self, service_id: &str, saga_id: &str) -> Result<(), ProxyError> {
        ProxyRouter::saga_commit_impl(self, service_id, saga_id).await
    }

    async fn saga_compensate(&self, service_id: &str, saga_id: &str) -> Result<(), ProxyError> {
        ProxyRouter::saga_compensate_impl(self, service_id, saga_id).await
    }

    async fn saga_status(&self, service_id: &str, saga_id: &str) -> Result<SagaInfo, ProxyError> {
        ProxyRouter::saga_status_impl(self, service_id, saga_id).await
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
            // A name bound to nobody is terminal on its own terms: this is
            // a failure to have a target at all, not a failed delivery, so
            // it does not go through the retry classification.
            let target = match self.resolve_queued_target(&call) {
                Ok(target) => target,
                Err(e) => {
                    proxy_outbox::log_delivery_failure(&call.idempotency_key, &e);
                    let message = e.to_string();
                    let outcome = tokio::task::spawn_blocking(move || {
                        queue.fail(item.id, now, &message, true)
                    })
                    .await;
                    if let Ok(Ok(FailOutcome::DeadLettered { .. })) = outcome {
                        metrics::counter!("substrate.proxy.outbox.dead_lettered").increment(1);
                    }
                    continue;
                }
            };
            match self.deliver_queued(&call, target).await {
                Ok(_) => {
                    metrics::counter!("substrate.proxy.outbox.delivered").increment(1);
                    let _ = tokio::task::spawn_blocking(move || queue.complete(item.id)).await;
                }
                Err(e) => match proxy_outbox::disposition_of(&e) {
                    // The receiver already ran this item and could not
                    // hand back its result. That is a delivery reported
                    // through the error channel, so the item is done.
                    Disposition::Delivered => {
                        metrics::counter!("substrate.proxy.outbox.delivered").increment(1);
                        let _ = tokio::task::spawn_blocking(move || queue.complete(item.id)).await;
                    }
                    disposition => {
                        proxy_outbox::log_delivery_failure(&call.idempotency_key, &e);
                        let terminal = disposition == Disposition::Terminal;
                        let message = e.to_string();
                        let outcome = tokio::task::spawn_blocking(move || {
                            queue.fail(item.id, now, &message, terminal)
                        })
                        .await;
                        if let Ok(Ok(FailOutcome::DeadLettered { .. })) = outcome {
                            metrics::counter!("substrate.proxy.outbox.dead_lettered").increment(1);
                        }
                    }
                },
            }
        }
        settled
    }

    /// The resident loop: drains outboxes and sweeps saga logs, then races
    /// cancellation into both -- see `run_async_worker`'s own doc for why
    /// it carries a name that says both, not just the first.
    pub async fn run_async_worker(self: Arc<Self>, tick: Duration, cancel: CancellationToken) {
        if self.outbox.is_none() && self.sagas.is_none() {
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
                (outbox_settled, saga_settled) = async {
                    let a = self.drain_outboxes_once().await;
                    let b = self.sweep_sagas_once().await;
                    (a, b)
                } => {
                    if outbox_settled > 0 || saga_settled > 0 {
                        debug!(
                            outbox_settled,
                            saga_settled,
                            "async worker settled queued calls and saga undos"
                        );
                    }
                }
            }
        }
    }

    /// Locks `saga_undeploy_candidates`, recovering rather than panicking
    /// if a prior panic poisoned it: the set holds nothing but service ids
    /// pending a second absent tick, so an inconsistent view after a panic
    /// costs at most one extra tick of delay before a real undeploy is
    /// confirmed, never a lost or duplicated drop.
    fn saga_undeploy_candidates_lock(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.saga_undeploy_candidates.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// One pass over every saga log this node has open, plus every deployed
    /// service whose log file already exists -- the second half is what
    /// lets a restart pick up a saga written before it. Mirrors
    /// `drain_outboxes_once`'s shape, but not its undeployed-service rule
    /// verbatim: the outbox only completes not-yet-delivered intent on one
    /// absent tick, which is recoverable; dropping a saga log is not, so
    /// this sweep requires the service to be absent on two consecutive
    /// ticks before it drops anything (`saga_undeploy_candidates`).
    ///
    /// Returns how many sagas it settled (a step undone, or a saga finished
    /// or failed), for tests and metrics.
    pub async fn sweep_sagas_once(&self) -> usize {
        let Some(store) = &self.sagas else { return 0 };
        let deployed: BTreeSet<String> = self
            .registry
            .get_all_endpoints()
            .into_iter()
            .map(|(service_id, _, _)| service_id)
            .collect();

        let mut services: BTreeSet<String> = store.open_services().into_iter().collect();
        for service_id in &deployed {
            if store.log_file_exists(service_id) {
                services.insert(service_id.clone());
            }
        }

        let now = proxy_outbox::now_ms();
        let mut settled = 0;
        for service_id in services {
            // Not `else { continue }` silently: a locked vault
            // makes every open fail with `KekRequired`, which is the state
            // of every substrate after a restart until an operator injects
            // the KEK -- silence there means a node that compensates
            // nothing and says nothing about it.
            let log = match store.log_for(&service_id).await {
                Ok(log) => log,
                Err(e) => {
                    warn!(service_id = %service_id, error = %e, "cannot open saga log this sweep");
                    continue;
                }
            };

            if !deployed.contains(&service_id) {
                // A single absent tick is not enough: undeploy removes a
                // service's endpoints one interface at a time, so a
                // redeploy can leave a service transiently missing from
                // `get_all_endpoints()` for exactly one tick. Only a
                // *second* consecutive absence drops the log.
                let confirmed = !self.saga_undeploy_candidates_lock().insert(service_id.clone());
                if !confirmed {
                    debug!(
                        service_id = %service_id,
                        "service absent from the registry this tick; sagas kept pending a second consecutive absence"
                    );
                    continue;
                }

                // Nothing removes a service's data directory on undeploy.
                // Its sagas are dropped rather than compensated: the
                // operator withdrew the whole service, and sending undos on
                // behalf of something that no longer exists is the mirror
                // of the outbox's own "delivering would resurrect intent an
                // operator withdrew".
                let dropped = {
                    let log = log.clone();
                    task::spawn_blocking(move || log.drop_all_for_undeployed()).await
                };
                self.saga_undeploy_candidates_lock().remove(&service_id);
                match dropped {
                    Ok(Ok(())) => {
                        info!(service_id = %service_id, "dropped sagas for an undeployed service");
                    }
                    Ok(Err(e)) => {
                        warn!(service_id = %service_id, error = %e, "could not drop sagas for an undeployed service");
                    }
                    Err(e) => {
                        warn!(service_id = %service_id, error = %e, "saga drop task failed");
                    }
                }
                continue;
            }

            // Deployed again (or still): clear any stale absence marker so
            // a later real undeploy needs its own two consecutive ticks
            // rather than firing on the first one because of a mark left
            // over from an earlier redeploy.
            self.saga_undeploy_candidates_lock().remove(&service_id);

            // The crash case: an open saga past its deadline starts
            // walking back. Nothing else can notice, because a guest does
            // not exist between calls.
            let abandoned = {
                let log = log.clone();
                task::spawn_blocking(move || log.abandoned(now, SAGA_SWEEP_LIMIT)).await
            };
            if let Ok(Ok(heads)) = abandoned {
                for head in heads {
                    let saga_id = head.saga_id.clone();
                    let started = {
                        let log = log.clone();
                        task::spawn_blocking(move || log.mark_compensating(&saga_id, now)).await
                    };
                    if matches!(started, Ok(Ok(true))) {
                        warn!(
                            saga = %head.saga_id,
                            service_id = %service_id,
                            "saga passed its deadline; compensating"
                        );
                    }
                }
            }

            let due = {
                let log = log.clone();
                task::spawn_blocking(move || log.due_compensations(now, SAGA_SWEEP_LIMIT)).await
            };
            if let Ok(Ok(heads)) = due {
                for head in heads {
                    settled += self.compensate_next_step(&service_id, &log, &head).await;
                }
            }
        }
        settled
    }

    /// One undo. Deliberately one per saga per tick: the walk is ordered,
    /// so a step that fails must not be overtaken by the step below it,
    /// and a saga with a slow provider must not hold the tick against
    /// every other saga.
    async fn compensate_next_step(
        &self,
        service_id: &str,
        log: &SagaLog,
        head: &SagaHead,
    ) -> usize {
        let now = proxy_outbox::now_ms();
        let saga_id = head.saga_id.clone();
        let step = {
            let log = log.clone();
            task::spawn_blocking(move || log.next_uncompensated_step(&saga_id)).await
        };
        let Ok(Ok(step)) = step else { return 0 };
        let Some(step) = step else {
            // Nothing left to compensate: done.
            let saga_id = head.saga_id.clone();
            let log = log.clone();
            let _ = task::spawn_blocking(move || log.finish_compensation(&saga_id, now)).await;
            metrics::counter!("substrate.proxy.saga.compensated").increment(1);
            return 1;
        };

        // Before dispatch, not after: a crash inside the call must cost an
        // attempt, or a poison step is retried forever. Safe only because
        // the idempotency key below lets the receiver answer a duplicate
        // from its own record.
        let idx = step.idx;
        let saga_id = head.saga_id.clone();
        let attempts = {
            let log = log.clone();
            task::spawn_blocking(move || log.begin_undo_attempt(&saga_id, idx, now)).await
        };
        let Ok(Ok(attempts)) = attempts else { return 0 };
        if attempts > u32::from(log.max_attempts()) {
            let saga_id = head.saga_id.clone();
            let log = log.clone();
            let _ = task::spawn_blocking(move || {
                log.fail_compensation(
                    &saga_id,
                    idx,
                    now,
                    "undo attempted repeatedly without ever completing",
                    true,
                )
            })
            .await;
            metrics::counter!("substrate.proxy.saga.failed").increment(1);
            return 1;
        }

        let Some(sagas) = &self.sagas else { return 0 };
        let target_value: QueuedTarget = match serde_json::from_str(&step.target) {
            Ok(t) => t,
            Err(e) => {
                self.fail_terminal_saga_step(
                    log,
                    &head.saga_id,
                    idx,
                    now,
                    &format!("stored saga step target is unreadable: {e}"),
                )
                .await;
                return 1;
            }
        };
        // A dependency name bound to nobody is not a failed delivery, it is
        // having nothing to deliver to -- terminal on its own terms, the
        // same split `resolve_queued_target` makes for the outbox.
        let target = match sagas.resolve_step_target(
            head.app_instance_id.as_deref(),
            &target_value,
            step.routing_key.as_deref(),
        ) {
            Ok(t) => t,
            Err(e) => {
                self.fail_terminal_saga_step(log, &head.saga_id, idx, now, &e.to_string()).await;
                return 1;
            }
        };

        let params_value: Value = serde_json::from_slice(&step.params).unwrap_or(Value::Null);
        let result_value: Option<Value> =
            step.result.as_deref().and_then(|bytes| serde_json::from_slice(bytes).ok());
        let params = merge_forward_result(&params_value, result_value.as_ref());

        let req = ProxyRequest {
            target_service: target,
            interface: step.interface.clone(),
            method: saga_undo_name(&step.method),
            params,
            caller: CallerContext::service_system(service_id),
            origin: CallOrigin::Guest { service_id: service_id.to_string() },
            protocol: ProxyProtocol::JsonRpcV1,
            idempotent: true,
            idempotency_key: Some(format!("saga:{}:{}", head.saga_id, idx)),
            timeout: Some(SAGA_UNDO_CALL_BUDGET),
        };

        // `invoke_inner`, not `invoke`: an undo has no live caller holding
        // the error, so writing a second, un-repliable proxy dead letter
        // for it would only add noise -- the saga's own `fail_compensation`
        // is this failure's operator surface.
        match self.invoke_inner(&req).await {
            Ok(_) => {
                metrics::counter!("substrate.proxy.saga.undo_delivered").increment(1);
                let saga_id = head.saga_id.clone();
                let log = log.clone();
                let _ = task::spawn_blocking(move || log.mark_step_compensated(&saga_id, idx, now))
                    .await;
            }
            Err(e) => match proxy_outbox::disposition_of(&e) {
                // The receiver already ran this undo and could not hand
                // back a result. That is a delivery, not a failure.
                Disposition::Delivered => {
                    let saga_id = head.saga_id.clone();
                    let log = log.clone();
                    let _ =
                        task::spawn_blocking(move || log.mark_step_compensated(&saga_id, idx, now))
                            .await;
                }
                Disposition::Retry => {
                    self.fail_saga_step(
                        log,
                        &head.saga_id,
                        idx,
                        now,
                        &explain_undo_error(&e, &step),
                    )
                    .await;
                }
                Disposition::Terminal => {
                    self.fail_terminal_saga_step(
                        log,
                        &head.saga_id,
                        idx,
                        now,
                        &explain_undo_error(&e, &step),
                    )
                    .await;
                }
            },
        }
        1
    }

    /// Records a retryable undo failure. `Disposition::Retry` does not mean
    /// the saga stays `compensating`: this attempt may have been the one
    /// that exhausted the budget, and that case must count toward
    /// `substrate.proxy.saga.failed` exactly as a terminal disposition does
    /// -- an operator watching that counter cannot see the difference.
    async fn fail_saga_step(&self, log: &SagaLog, saga_id: &str, idx: u32, now: i64, error: &str) {
        let log = log.clone();
        let saga_id = saga_id.to_string();
        let error = error.to_string();
        let outcome =
            task::spawn_blocking(move || log.fail_compensation(&saga_id, idx, now, &error, false))
                .await;
        if matches!(outcome, Ok(Ok(CompensationOutcome::Failed))) {
            metrics::counter!("substrate.proxy.saga.failed").increment(1);
        }
    }

    /// Records an undo failure that can never succeed. `terminal = true`
    /// always exhausts the saga regardless of attempts remaining, so this
    /// always counts toward `substrate.proxy.saga.failed`.
    async fn fail_terminal_saga_step(
        &self,
        log: &SagaLog,
        saga_id: &str,
        idx: u32,
        now: i64,
        error: &str,
    ) {
        let log = log.clone();
        let saga_id = saga_id.to_string();
        let error = error.to_string();
        let _ =
            task::spawn_blocking(move || log.fail_compensation(&saga_id, idx, now, &error, true))
                .await;
        metrics::counter!("substrate.proxy.saga.failed").increment(1);
    }
}

/// How many sagas one sweep tick may act on for one service -- mirrors
/// `proxy_outbox::CLAIM_LIMIT_PER_TICK`'s reasoning: one service with many
/// due sagas must not spend the whole node's tick budget on its own
/// backlog.
const SAGA_SWEEP_LIMIT: u32 = 16;

/// The step budget's margin rule, as a pure function so the subtraction
/// itself is unit-testable without a real clock: a step's own call budget
/// is what is
/// *left* of `step_timeout_ms` after `bookkeeping` (the log open plus the
/// intent write) was spent, never `step_timeout_ms` plus that time. A slow
/// cold open shortens the call the guest's epoch has room for; it does not
/// borrow against the epoch.
fn step_call_budget_ms(step_timeout_ms: u64, bookkeeping: Duration) -> u64 {
    step_timeout_ms.saturating_sub(bookkeeping.as_millis() as u64).max(MIN_STEP_CALL_BUDGET_MS)
}

/// The three-case rule for merging a forward call's own result into its
/// undo's parameters: an object gains a `forward-result` member, an array
/// gains a trailing element, and `null`/absent (or any other scalar) is
/// wrapped into `{"forward-result": ...}`. A forward call that produced no
/// result (`result` is `None`) sends no `forward-result` at all, which
/// binds to `none` for an `option<string>` parameter -- so an undo written
/// as "ensure this is not in effect" works even for a step whose own
/// result was never recorded.
fn merge_forward_result(params: &Value, result: Option<&Value>) -> Value {
    let Some(result) = result else { return params.clone() };
    match params {
        Value::Object(map) => {
            let mut map = map.clone();
            map.insert("forward-result".to_string(), result.clone());
            Value::Object(map)
        }
        Value::Array(items) => {
            let mut items = items.clone();
            items.push(result.clone());
            Value::Array(items)
        }
        _ => {
            let mut map = serde_json::Map::new();
            map.insert("forward-result".to_string(), result.clone());
            Value::Object(map)
        }
    }
}

/// What the deploy gate deliberately does not catch surfaces here
/// instead: a callee error carrying the shared "not found" wire code
/// (`SERVICE_NOT_FOUND_RPC_CODE`, reused for both "no such service" and "no
/// such method on a service that exists") is rewritten to name the
/// convention explicitly, so an operator reading `sagas`' `last-error`
/// learns what to fix rather than staring at a bare JSON-RPC code.
fn explain_undo_error(error: &ProxyError, step: &StepRow) -> String {
    let base = error.to_string();
    let looks_like_a_missing_export =
        matches!(error, ProxyError::Callee { code, .. } if *code == SERVICE_NOT_FOUND_RPC_CODE);
    if looks_like_a_missing_export {
        format!(
            "{base}; target does not export '{}' on '{}': a saga participant must export \
             saga-undo-<method> for every operation a step calls",
            saga_undo_name(&step.method),
            step.interface
        )
    } else {
        base
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
        NativeService, QueuedTarget, RpcResult, SERVICE_NOT_FOUND_RPC_CODE, SessionContext,
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

        let guard_registry = registry.clone();
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
                guard_registry,
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
        provider: Arc<syneroym_data_db::SqliteStorageProvider>,
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
            TopologyEntry, TopologyEpoch, TopologyKey, TopologyMode,
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
            TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![ServiceId::new("did:key:zTarget")],
                sharding_strategy: None,
                epoch: TopologyEpoch::default(),
                // No caching, so a test that re-registers a binding sees
                // the change on the very next resolution.
                cache_ttl: Duration::ZERO,
                not_after: None,
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
            max_pending_rows: syneroym_async_queue::DEFAULT_MAX_PENDING_ROWS,
        };
        let outbox = Arc::new(ProxyOutbox::new(
            provider.clone(),
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
                registry.clone(),
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
            provider,
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

    /// A guest re-enqueueing a key the receiver already ran -- and whose
    /// result was too large to retain -- must be told the call succeeded,
    /// not handed an error. The fence answers through the error channel
    /// because there is no value to return; that is a delivery, and the
    /// synchronous probe has to read it the same way the worker does.
    #[tokio::test]
    async fn an_enqueue_whose_target_already_ran_it_reports_success() {
        let node = outbox_node(true, 50).await;

        // Stand in for the receiver's answer: the fence reports "already
        // ran here, result not retained" through a callee error.
        let already_ran = ProxyError::Callee {
            code: syneroym_async_queue::CALL_RESULT_NOT_RETAINED_RPC_CODE,
            message: "this call already ran here".to_string(),
            data: None,
        };
        assert_eq!(
            proxy_outbox::disposition_of(&already_ran),
            Disposition::Delivered,
            "precondition: this is the code the receiver answers a duplicate with"
        );

        node.target.answer_with.lock().unwrap().replace(already_ran);
        let outcome = node
            .router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await;
        assert!(
            outcome.is_ok(),
            "a call the receiver already ran must report success to the guest, got {outcome:?}"
        );
        assert!(!node.queue_file_exists(), "and must not be queued for another delivery attempt");
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
            TopologyKey, TopologyMode,
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
            TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![ServiceId::new("did:key:zMoved")],
                sharding_strategy: None,
                epoch: TopologyEpoch::default(),
                cache_ttl: Duration::ZERO,
                not_after: None,
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
            AppInstanceId, LogicalServiceName, TopologyEntry, TopologyEpoch, TopologyKey,
            TopologyMode,
        };
        let node = outbox_node(false, 50).await;
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();

        // The binding goes away entirely.
        node.resolver.register(
            TopologyKey::local(AppInstanceId::new("app-1"), LogicalServiceName::new("backend")),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![],
                sharding_strategy: None,
                epoch: TopologyEpoch::default(),
                cache_ttl: Duration::ZERO,
                not_after: None,
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
        let dead = node.dead_letters().await;
        assert_eq!(dead.len(), 1);
        assert!(node.queued().await.is_empty());
        // The target refused on its own terms. Pinned explicitly so this
        // test cannot quietly become vacuous: "service not found" is
        // retried rather than dead-lettered, so a scenario that drifted
        // onto that code would assert nothing.
        assert!(
            !dead[0].last_error.contains(&SERVICE_NOT_FOUND_RPC_CODE.to_string()),
            "this case must exercise a genuine callee refusal, not the retried not-found code: {}",
            dead[0].last_error
        );
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
        let node = outbox_node(true, 50).await;

        // Written *straight into the queue*, not through `enqueue`. The
        // probe would claim this key on the way past, and a claim left in
        // flight makes the worker's own delivery hit the fence's
        // "already running here" before it ever reaches the target -- so
        // there would be no in-flight delivery to abandon, structurally,
        // however the timing fell. Bypassing the probe also leaves the
        // target's invocation count at zero, so the wait below can only be
        // satisfied by the worker.
        // A long per-call budget on purpose. With the fixture's usual
        // 500 ms the "blocked" dispatch resolves on its own via the call
        // timeout, the drain returns, and the test passes whether or not
        // cancellation is raced into the delivery -- which is precisely
        // how this test was vacuous twice. At 30 s the only way the worker
        // returns inside the assertion below is by being interrupted.
        let mut item = queued_call(QueuedTarget::Dependency("backend".into()), "worker-only");
        item.timeout_ms = Some(30_000);
        node.outbox.store(&item).await.unwrap();
        assert_eq!(node.queued().await.len(), 1);
        assert_eq!(
            node.target.invoked.load(Ordering::SeqCst),
            0,
            "nothing may have reached the target before the worker starts"
        );

        // The target blocks and is never released during the test, so any
        // delivery the worker starts genuinely never resolves on its own.
        let release = Arc::new(tokio::sync::Notify::new());
        *node.target.hold.lock().unwrap() = Some(release.clone());

        let cancel = CancellationToken::new();
        let worker = {
            let router = node.router.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                router.run_async_worker(Duration::from_millis(5), cancel).await;
            })
        };

        let entered =
            wait_for(Duration::from_secs(5), || node.target.invoked.load(Ordering::SeqCst) >= 1)
                .await;
        assert!(
            entered,
            "the worker never reached the target, so this test would prove nothing about \
             abandoning a delivery"
        );

        // The load-bearing assertion. The delivery in flight right now
        // cannot finish -- nothing will notify `release` before the
        // assertion below. So the worker can only return if cancellation
        // is raced *into* the delivery; a worker that merely checks
        // cancellation between ticks stays inside `drain_outboxes_once`
        // forever and this times out.
        cancel.cancel();
        let stopped = tokio::time::timeout(Duration::from_secs(2), worker).await;
        assert!(
            stopped.is_ok(),
            "shutdown must interrupt a delivery that never resolves, not wait it out"
        );

        // Nothing was lost by not draining: the abandoned item is still
        // on the outbox, and its visibility timeout returns it to a later
        // worker.
        release.notify_waiters();
        assert_eq!(node.queued().await.len(), 1, "the abandoned item must still be on the outbox");
    }

    /// Polls until `check` holds or the budget runs out.
    async fn wait_for<F: FnMut() -> bool>(budget: Duration, mut check: F) -> bool {
        let deadline = std::time::Instant::now() + budget;
        while std::time::Instant::now() < deadline {
            if check() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    // -- the dead-letter tier ----------------------------------------------

    /// A guest-origin call that failed for good, as the synchronous tier
    /// produces it. The target is registered and *answers* -- a refusal it
    /// produced itself -- because only a failure the target produced earns
    /// a dead letter: this node's own refusals have nothing to replay.
    /// A call that runs out of its own deadline against a target that
    /// never answers -- a transport-class failure, not a callee one.
    ///
    /// This is the closest a unit fixture gets to "the budget ran out":
    /// the retry *loop* itself lives on the remote path, which needs a
    /// resolvable address this harness has no registry client for. What
    /// matters for the tier rule below is that the failure is one the
    /// caller is left holding and that a retry could plausibly have
    /// fixed, which a definitive callee refusal is not.
    fn timing_out_guest_request(key: Option<&str>) -> ProxyRequest {
        let mut req = base_request("did:key:zTarget", "greeter");
        req.origin = CallOrigin::Guest { service_id: CALLER.to_string() };
        req.caller = CallerContext::service_system(CALLER);
        req.idempotency_key = key.map(str::to_string);
        req.idempotent = true;
        req.timeout = Some(Duration::from_millis(100));
        req
    }

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
    ///
    /// Driven through a real transport-class failure (the call runs out
    /// of its deadline against a target that never answers), not a callee
    /// refusal -- a callee error is never retried, so a pair built on one
    /// would be testing something other than what these names say.
    #[tokio::test]
    async fn an_unkeyed_call_that_fails_at_the_transport_writes_no_dead_letter() {
        let node = outbox_node(true, 50).await;
        let release = Arc::new(tokio::sync::Notify::new());
        *node.target.hold.lock().unwrap() = Some(release.clone());
        let result = node.router.invoke(timing_out_guest_request(None)).await;
        assert!(
            matches!(result, Err(ProxyError::Transport(_)) | Err(ProxyError::Timeout(_))),
            "the call must have failed at the transport, got {result:?}"
        );
        assert!(
            !node.queue_file_exists(),
            "an unfenced failure must leave no replayable record behind"
        );
        release.notify_waiters();
    }

    /// The second tier: the row is *additional*, never a substitute for
    /// the caller's own error. Same real transport failure as its twin.
    #[tokio::test]
    async fn a_keyed_call_that_fails_at_the_transport_writes_a_dead_letter_and_still_returns_its_error()
     {
        let node = outbox_node(true, 50).await;
        let release = Arc::new(tokio::sync::Notify::new());
        *node.target.hold.lock().unwrap() = Some(release.clone());
        let result = node.router.invoke(timing_out_guest_request(Some("k1"))).await;
        assert!(
            matches!(result, Err(ProxyError::Transport(_)) | Err(ProxyError::Timeout(_))),
            "the caller must still get its own transport error, got {result:?}"
        );
        release.notify_waiters();

        let dead = node.dead_letters().await;
        assert_eq!(dead.len(), 1, "a keyed failure must also be recorded for an operator");
        assert_eq!(dead[0].queue_key, "k1");
        assert!(node.queued().await.is_empty(), "and must not linger in the outbox");
    }

    /// A keyed call the *target itself* refused. Distinct from exhaustion
    /// above: a callee error is never retried, so nothing is exhausted --
    /// but it is still an answer the target produced, so it still earns a
    /// row an operator can see.
    #[tokio::test]
    async fn a_keyed_call_a_target_refuses_writes_a_dead_letter_without_retrying() {
        let node = outbox_node(true, 50).await;
        node.target.fail_with.store(true, Ordering::SeqCst);
        let result = node.router.invoke(failing_guest_request(Some("k1"))).await;
        assert!(matches!(result, Err(ProxyError::Callee { .. })), "got {result:?}");
        assert_eq!(
            node.target.invoked.load(Ordering::SeqCst),
            1,
            "a callee error is definitive and must not be retried"
        );
        assert_eq!(node.dead_letters().await.len(), 1);
    }

    /// The fence's own answers are not delivery failures, so none of them
    /// may leave an operator-visible dead letter. "Already running here"
    /// is the sharp case: the call is succeeding on another task right
    /// now, and a row for it would be indistinguishable from a genuine
    /// exhausted delivery and never cleared.
    #[test]
    fn the_fences_own_answers_never_earn_a_dead_letter() {
        for code in [
            syneroym_async_queue::CALL_ALREADY_RUNNING_RPC_CODE,
            syneroym_async_queue::CALL_RESULT_NOT_RETAINED_RPC_CODE,
        ] {
            let error = ProxyError::Callee { code, message: "fence".to_string(), data: None };
            assert!(!target_produced(&error), "code {code} must not earn a dead letter");
        }
        // Nor may a refusal raised before anything was attempted:
        // replaying it just re-earns the refusal.
        assert!(!target_produced(&ProxyError::PermissionDenied("no store".to_string())));
        assert!(!target_produced(&ProxyError::Internal("no storage provider".to_string())));

        // A not-found target is worth a row, and the two classifiers must
        // agree about that: the queued path retries it as "not yet", so
        // the synchronous path must not silently drop it.
        let not_found = ProxyError::ServiceNotFound("mid-restart".to_string());
        assert!(target_produced(&not_found));
        assert_eq!(
            proxy_outbox::disposition_of(&not_found),
            Disposition::Retry,
            "the two classifiers must not disagree about one error"
        );

        // And a real answer from the target does.
        assert!(target_produced(&ProxyError::Callee {
            code: -32010,
            message: "denied by the target".to_string(),
            data: None,
        }));
        assert!(target_produced(&ProxyError::Transport("peer went away".to_string())));
    }

    /// Failure-matrix row 12 says queue growth is bounded. The
    /// dead-letter table was; the *pending* outbox was not, so a guest
    /// aimed at an unreachable target could hold unbounded rows for the
    /// whole attempt budget. It refuses rather than evicting: a pending
    /// item is work somebody still expects to happen.
    #[tokio::test]
    async fn a_full_outbox_refuses_further_enqueues_rather_than_evicting() {
        let node = outbox_node(false, 50).await;
        let mut cfg = node.outbox.config().clone();
        cfg.max_pending_rows = 2;
        let tight = Arc::new(ProxyOutbox::new(
            node.provider.clone(),
            Arc::new(syneroym_data_keystore::KeyStore::new()),
            node.resolver.clone(),
            cfg,
        ));
        for i in 0..2 {
            tight
                .store(&queued_call(
                    QueuedTarget::Service("did:key:zTarget".into()),
                    &format!("k{i}"),
                ))
                .await
                .unwrap();
        }
        let refused =
            tight.store(&queued_call(QueuedTarget::Service("did:key:zTarget".into()), "k2")).await;
        assert!(
            matches!(refused, Err(ProxyError::UnsupportedTarget(_))),
            "a full outbox must refuse, got {refused:?}"
        );
        assert_eq!(
            tight.queue_for(CALLER).await.unwrap().all().unwrap().len(),
            2,
            "and must not have evicted anything to make room"
        );
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

    /// The property that makes replay safe at all, and the reason a dead
    /// letter needs a key to exist: replaying a call the target already
    /// ran must not run it twice. Drives a replay against a target that
    /// already executed the original.
    #[tokio::test]
    async fn a_replayed_call_is_deduplicated_at_the_receiver_if_the_first_one_landed() {
        let node = outbox_node(true, 50).await;

        // The original lands.
        node.router
            .enqueue(queued_call(QueuedTarget::Dependency("backend".into()), "k1"))
            .await
            .unwrap();
        assert_eq!(node.target.invoked.load(Ordering::SeqCst), 1);

        // An operator finds a dead letter for the same logical operation
        // and replays it -- the situation replay exists for, where it is
        // not knowable whether the first attempt landed.
        let call = queued_call(QueuedTarget::Dependency("backend".into()), "k1");
        node.outbox.record_dead_letter(&call, "looked unreachable").await.unwrap();
        let dead = node.outbox.dead_letters(CALLER).await.unwrap();
        assert_eq!(dead.len(), 1);
        node.outbox.replay_dead_letter(CALLER, dead[0].id).await.unwrap();

        node.router.drain_outboxes_once().await;
        assert_eq!(
            node.target.invoked.load(Ordering::SeqCst),
            1,
            "the receiver's record of the first call must stop the replay re-executing it"
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
        /// When set, `dispatch` blocks until this is notified -- a
        /// delivery that genuinely never resolves, which is the only way
        /// to test that shutdown interrupts one.
        hold: Mutex<Option<Arc<tokio::sync::Notify>>>,
        /// When set, `dispatch` answers with this exact error, so a test
        /// can drive a specific reserved code the receiver would produce.
        answer_with: Mutex<Option<ProxyError>>,
        /// The `(interface, method, params)` of the most recent dispatch --
        /// lets a saga test confirm the walk actually called
        /// `saga-undo-<method>`, not the forward method again.
        last_invocation: Mutex<Option<(String, String, Value)>>,
    }

    #[async_trait::async_trait]
    impl NativeService for RecordingNativeService {
        async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
            self.invoked.fetch_add(1, Ordering::SeqCst);
            *self.last_caller_did.lock().unwrap() = Some(invocation.caller.caller_did.clone());
            *self.last_invocation.lock().unwrap() = Some((
                invocation.interface.clone(),
                invocation.method.clone(),
                invocation.params.clone(),
            ));
            let hold = self.hold.lock().unwrap().clone();
            if let Some(hold) = hold {
                hold.notified().await;
            }
            if let Some(answer) = self.answer_with.lock().unwrap().as_ref() {
                let (code, message) = match answer {
                    ProxyError::Callee { code, message, .. } => (*code, message.clone()),
                    other => (-32603, other.to_string()),
                };
                return Err(RpcError::Custom(code, message, None));
            }
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

    /// Finding A4: D-S3-15's empty-interface convenience ("the destination
    /// resolves the caller's one app-declared interface") is for an
    /// external caller that cannot know a service's interface names -- the
    /// gateway or coordinator resolving a hostname. A WASM guest always
    /// names the interface it wants, so an empty one must be denied before
    /// `registry.lookup` gets a chance to resolve it -- the target
    /// registers exactly one app-declared interface here, so a resolve
    /// would otherwise have succeeded.
    #[tokio::test]
    async fn guest_with_an_empty_interface_is_denied_before_resolution() {
        let registry = empty_registry();
        registry
            .register(
                "svc-b".to_string(),
                "default".to_string(),
                SubstrateEndpoint::WasmChannel { service_id: "svc-b".to_string() },
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

        let mut req = base_request("svc-b", "");
        req.origin = CallOrigin::Guest { service_id: "svc-a".to_string() };
        let result = router.invoke(req).await;
        assert!(matches!(result, Err(ProxyError::PermissionDenied(_))), "{result:?}");
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

    #[derive(Debug)]
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

    // -- sagas ---------------------------------------------------------

    /// A node that can drive a saga: a certified calling service, a real
    /// per-service saga log, and a reachable target -- the same shape
    /// `outbox_node` builds for `enqueue`, since a saga step is `invoke`
    /// plus a log write over the identical wiring.
    struct SagaNode {
        router: Arc<ProxyRouter>,
        registry: EndpointRegistry,
        target: Arc<RecordingNativeService>,
        sagas: Arc<SagaStore>,
        dedup_guard: Arc<crate::CallDedupGuard>,
        _native_dispatch: NativeDispatchRegistry,
        _dir: tempfile::TempDir,
    }

    const SAGA_CALLER: &str = "did:key:zSagaCaller";
    const SAGA_TARGET: &str = "did:key:zSagaTarget";

    fn saga_config(dispatch_epoch_timeout_secs: u64) -> syneroym_async_queue::SagaConfig {
        syneroym_async_queue::SagaConfig::from(&syneroym_core::config::AppSandboxRole {
            dispatch_epoch_timeout_secs,
            ..syneroym_core::config::AppSandboxRole::default()
        })
    }

    async fn saga_node(dispatch_epoch_timeout_secs: u64) -> SagaNode {
        use syneroym_data_db::SqliteStorageProvider;
        use syneroym_data_keystore::KeyStore;

        let registry = empty_registry();
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let target = Arc::new(RecordingNativeService::default());
        native_dispatch.insert(SAGA_TARGET.to_string(), target.clone() as Arc<dyn NativeService>);
        registry
            .register(
                SAGA_TARGET.to_string(),
                "saga-participant".to_string(),
                SubstrateEndpoint::NativeHostChannel { service_id: SAGA_TARGET.to_string() },
            )
            .await
            .unwrap();
        registry
            .register(
                SAGA_CALLER.to_string(),
                "saga-driver".to_string(),
                SubstrateEndpoint::WasmChannel { service_id: SAGA_CALLER.to_string() },
            )
            .await
            .unwrap();

        let node_identity = Arc::new(Identity::generate().unwrap());
        let owner = "did:key:zSagaOwner".to_string();
        registry.set_owner(SAGA_CALLER.to_string(), owner.clone()).await.unwrap();
        let instance = node_identity.derive_service_identity(&owner, SAGA_CALLER);
        let cert = DelegationCertificate::issue(
            &Identity::generate().unwrap(),
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        registry.set_instance_cert(SAGA_CALLER.to_string(), cert).await.unwrap();

        let dir = tempfile::tempdir().unwrap();
        for service in [SAGA_CALLER, SAGA_TARGET] {
            let service_dir = dir.path().join("services").join(service);
            std::fs::create_dir_all(&service_dir).unwrap();
            std::fs::write(service_dir.join("state.db"), b"").unwrap();
        }

        let provider = Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
        let resolver = syneroym_app_orchestration::empty_resolver();
        let sagas = Arc::new(SagaStore::new(
            provider.clone(),
            Arc::new(KeyStore::new()),
            resolver,
            saga_config(dispatch_epoch_timeout_secs),
        ));
        // Every undo the walk sends is keyed, so the receiver needs a real
        // fence behind it -- with no dedup guard configured,
        // `invoke_local_guarded` refuses any keyed call outright.
        let dedup_guard = Arc::new(crate::CallDedupGuard::new(
            provider,
            Arc::new(KeyStore::new()),
            registry.clone(),
            syneroym_async_queue::DedupConfig {
                ttl_ms: 600_000,
                claim_window_ms: 60_000,
                max_rows: 100,
                max_result_bytes: 64 * 1024,
            },
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
            .with_dedup_guard(dedup_guard.clone())
            .with_sagas(sagas.clone()),
        );
        SagaNode {
            router,
            registry,
            target,
            sagas,
            dedup_guard,
            _native_dispatch: native_dispatch,
            _dir: dir,
        }
    }

    fn saga_step_request(saga_id: &str, target: &str) -> SagaStepRequest {
        SagaStepRequest {
            caller_service_id: SAGA_CALLER.to_string(),
            app_instance_id: None,
            saga_id: saga_id.to_string(),
            target: QueuedTarget::Service(target.to_string()),
            routing_key: None,
            interface: "saga-participant".to_string(),
            method: "reserve".to_string(),
            params: Value::Null,
            idempotency_key: None,
            protocol: None,
            timeout_ms: None,
        }
    }

    async fn begun_saga(node: &SagaNode) -> String {
        node.router
            .saga_begin(SagaBegin {
                caller_service_id: SAGA_CALLER.to_string(),
                app_instance_id: None,
                name: "wf".to_string(),
                deadline_secs: None,
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_step_records_its_intent_before_the_call_lands() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;

        node.router.saga_step(saga_step_request(&saga_id, SAGA_TARGET)).await.unwrap();

        let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
        let step = log.next_uncompensated_step(&saga_id).unwrap().unwrap();
        assert_eq!(step.idx, 0);
        assert_eq!(step.method, "reserve");
        assert_eq!(node.target.invoked.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_step_whose_call_fails_is_recorded_as_failed_and_still_returns_its_error() {
        let node = saga_node(5).await;
        node.target.fail_with.store(true, Ordering::SeqCst);
        let saga_id = begun_saga(&node).await;

        let result = node.router.saga_step(saga_step_request(&saga_id, SAGA_TARGET)).await;
        assert!(result.is_err(), "the caller must still see the failure");

        let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
        assert!(
            log.next_uncompensated_step(&saga_id).unwrap().is_none(),
            "a failed step is never compensated"
        );
    }

    #[tokio::test]
    async fn a_step_on_an_unknown_saga_is_refused() {
        let node = saga_node(5).await;
        let result = node.router.saga_step(saga_step_request("no-such-saga", SAGA_TARGET)).await;
        assert!(result.is_err(), "unexpected success");
    }

    #[tokio::test]
    async fn a_step_against_a_node_level_interface_is_refused() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;

        for interface in ["orchestrator", "security", "supervisor"] {
            let mut req = saga_step_request(&saga_id, SAGA_TARGET);
            req.interface = interface.to_string();
            let result = node.router.saga_step(req).await;
            assert!(
                matches!(result, Err(ProxyError::PermissionDenied(_))),
                "interface '{interface}' must be refused, got {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_step_against_the_callers_own_service_is_refused() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;

        let result = node.router.saga_step(saga_step_request(&saga_id, SAGA_CALLER)).await;
        assert!(
            matches!(result, Err(ProxyError::UnsupportedTarget(_))),
            "a self-target must be refused, got {result:?}"
        );
    }

    #[tokio::test]
    async fn a_step_with_no_timeout_takes_a_budget_inside_the_guests_epoch_not_the_proxys_default()
    {
        // The node's epoch is 5s, so the derived step budget is 4s -- well
        // under `DEFAULT_PROXY_CALL_TIMEOUT` (30s). A target that blocks
        // past 4s but under 30s must still time out, which only holds if
        // the derived budget -- not the proxy default -- was applied.
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        node.target.hold.lock().unwrap().replace(Arc::new(tokio::sync::Notify::new()));

        let started = Instant::now();
        let result = node.router.saga_step(saga_step_request(&saga_id, SAGA_TARGET)).await;
        assert!(matches!(result, Err(ProxyError::Timeout(_))), "got {result:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the step must not wait out the proxy's own 30s default"
        );
    }

    #[tokio::test]
    async fn a_step_with_an_explicit_timeout_keeps_it() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        let mut req = saga_step_request(&saga_id, SAGA_TARGET);
        req.timeout_ms = Some(50);
        node.target.hold.lock().unwrap().replace(Arc::new(tokio::sync::Notify::new()));

        let started = Instant::now();
        let result = node.router.saga_step(req).await;
        assert!(matches!(result, Err(ProxyError::Timeout(_))), "got {result:?}");
        assert!(started.elapsed() < Duration::from_secs(2), "the explicit 50ms budget must apply");
    }

    #[tokio::test]
    async fn begin_is_refused_for_a_service_with_no_unexpired_instance_certificate() {
        let node = saga_node(5).await;
        node.registry.remove_instance_cert(SAGA_CALLER).await.unwrap();

        let result = node
            .router
            .saga_begin(SagaBegin {
                caller_service_id: SAGA_CALLER.to_string(),
                app_instance_id: None,
                name: "wf".to_string(),
                deadline_secs: None,
            })
            .await;
        assert!(
            matches!(result, Err(ProxyError::PermissionDenied(_))),
            "an uncertified caller must be refused, got {result:?}"
        );
    }

    /// A managed instance's certificate is renewed on every supervisor
    /// pass, so its own current expiry cannot decide whether a long
    /// deadline is sound -- `begin` warns rather than refusing.
    #[tokio::test]
    async fn begin_warns_but_proceeds_when_the_deadline_outlives_the_certificate() {
        let node = saga_node(5).await;
        let cert = DelegationCertificate::issue(
            &Identity::generate().unwrap(),
            Identity::generate().unwrap().public_key(),
            10,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        node.registry.set_instance_cert(SAGA_CALLER.to_string(), cert).await.unwrap();

        let result = node
            .router
            .saga_begin(SagaBegin {
                caller_service_id: SAGA_CALLER.to_string(),
                app_instance_id: None,
                name: "wf".to_string(),
                deadline_secs: Some(60),
            })
            .await;
        assert!(
            result.is_ok(),
            "a deadline outliving the certificate must warn, not refuse: {result:?}"
        );
    }

    #[tokio::test]
    async fn begin_is_refused_above_the_deadline_ceiling() {
        let node = saga_node(5).await;
        let result = node
            .router
            .saga_begin(SagaBegin {
                caller_service_id: SAGA_CALLER.to_string(),
                app_instance_id: None,
                name: "wf".to_string(),
                deadline_secs: Some(999_999_999),
            })
            .await;
        assert!(
            matches!(result, Err(ProxyError::PermissionDenied(_))),
            "a deadline above the ceiling must be refused rather than clamped, got {result:?}"
        );
    }

    #[tokio::test]
    async fn a_deadline_of_none_takes_the_node_default() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        let info = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
        let expected = saga_config(5).default_deadline_ms;
        assert_eq!(info.deadline_at - info.created_at, expected);
    }

    #[tokio::test]
    async fn commit_drops_the_log_so_a_later_compensate_is_refused() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;

        node.router.saga_commit(SAGA_CALLER, &saga_id).await.unwrap();

        let status = node.router.saga_status(SAGA_CALLER, &saga_id).await;
        assert!(status.is_err(), "a committed saga must be gone");
        let compensate = node.router.saga_compensate(SAGA_CALLER, &saga_id).await;
        assert!(compensate.is_err(), "compensate must be refused after commit");
    }

    #[tokio::test]
    async fn a_saga_id_is_minted_by_the_host_and_is_unique_per_begin() {
        let node = saga_node(5).await;
        let a = begun_saga(&node).await;
        let b = begun_saga(&node).await;
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn two_concurrent_begins_through_a_cold_cache_share_one_log_handle() {
        let node = saga_node(5).await;
        let router_a = node.router.clone();
        let router_b = node.router.clone();
        let (a, b) = tokio::join!(
            router_a.saga_begin(SagaBegin {
                caller_service_id: SAGA_CALLER.to_string(),
                app_instance_id: None,
                name: "wf-a".to_string(),
                deadline_secs: None,
            }),
            router_b.saga_begin(SagaBegin {
                caller_service_id: SAGA_CALLER.to_string(),
                app_instance_id: None,
                name: "wf-b".to_string(),
                deadline_secs: None,
            }),
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a, b);
        // Both sagas are visible through the same log -- if two separate
        // connections had been opened, one write could be invisible to a
        // read through the other handle.
        assert!(node.router.saga_status(SAGA_CALLER, &a).await.is_ok());
        assert!(node.router.saga_status(SAGA_CALLER, &b).await.is_ok());
    }

    // -- merging the forward result into an undo's parameters -----------

    #[test]
    fn merge_forward_result_adds_a_member_to_an_object_and_an_element_to_an_array() {
        let object = serde_json::json!({"item": "a"});
        let merged = merge_forward_result(&object, Some(&Value::String("id-1".to_string())));
        assert_eq!(merged, serde_json::json!({"item": "a", "forward-result": "id-1"}));

        let array = serde_json::json!(["a", 1]);
        let merged = merge_forward_result(&array, Some(&Value::String("id-1".to_string())));
        assert_eq!(merged, serde_json::json!(["a", 1, "id-1"]));
    }

    #[test]
    fn merge_forward_result_makes_an_object_when_the_forward_params_were_null() {
        let merged = merge_forward_result(&Value::Null, Some(&Value::String("id-1".to_string())));
        assert_eq!(merged, serde_json::json!({"forward-result": "id-1"}));
    }

    #[test]
    fn a_forward_call_that_returned_nothing_sends_no_forward_result() {
        let object = serde_json::json!({"item": "a"});
        let merged = merge_forward_result(&object, None);
        assert_eq!(merged, object);
    }

    // -- bookkeeping shortens the step budget ----------------------------

    #[test]
    fn bookkeeping_before_the_call_shortens_the_step_budget_rather_than_extending_the_epoch() {
        // The whole rule in one number: a slower bookkeeping phase (the log
        // open plus the intent write) must leave the call *less* time, not
        // the same amount tacked on top of it -- otherwise a guest's own
        // epoch could be overrun by exactly the bookkeeping cost this
        // subtraction exists to protect against.
        let fast = step_call_budget_ms(4_000, Duration::from_millis(10));
        let slow = step_call_budget_ms(4_000, Duration::from_millis(1_500));
        assert_eq!(fast, 3_990);
        assert_eq!(slow, 2_500);
        assert!(
            slow < fast,
            "more bookkeeping time must leave a smaller call budget, not a larger one"
        );
    }

    #[test]
    fn the_step_budget_floors_at_the_minimum_rather_than_going_negative() {
        // Bookkeeping that ate the whole epoch (a very cold open, or a tiny
        // configured epoch) must not produce a zero or negative budget --
        // `saturating_sub` alone would floor at zero, which is not a call
        // budget, it is an instant refusal.
        let budget = step_call_budget_ms(4_000, Duration::from_secs(10));
        assert_eq!(budget, syneroym_async_queue::MIN_STEP_CALL_BUDGET_MS);
    }

    // -- the reverse walk -------------------------------------------------

    async fn add_step(node: &SagaNode, saga_id: &str, item: &str) {
        let mut req = saga_step_request(saga_id, SAGA_TARGET);
        req.params = serde_json::json!({"item": item});
        node.router.saga_step(req).await.unwrap();
    }

    #[tokio::test]
    async fn the_walk_undoes_the_newest_step_first() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        add_step(&node, &saga_id, "a").await;
        add_step(&node, &saga_id, "b").await;
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        node.router.sweep_sagas_once().await;
        let (_, method, params) = node.target.last_invocation.lock().unwrap().clone().unwrap();
        assert_eq!(method, "saga-undo-reserve");
        // The forward call's own result ("ok") is merged in as
        // `forward-result`.
        assert_eq!(params, serde_json::json!({"item": "b", "forward-result": "ok"}));

        node.router.sweep_sagas_once().await;
        let (_, method, params) = node.target.last_invocation.lock().unwrap().clone().unwrap();
        assert_eq!(method, "saga-undo-reserve");
        assert_eq!(params, serde_json::json!({"item": "a", "forward-result": "ok"}));

        node.router.sweep_sagas_once().await;
        let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
        assert_eq!(
            status.state,
            RpcSagaState::Compensated,
            "a finished compensation drops the step log but keeps a terminal row an operator can \
             still see"
        );
    }

    #[tokio::test]
    async fn the_walk_undoes_a_pending_step_too() {
        // Never records an outcome for the step -- the crash-mid-call case
        // the walk must compensate anyway.
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        let log = node.sagas.log_for(SAGA_CALLER).await.unwrap();
        log.record_step_intent(
            &saga_id,
            &syneroym_async_queue::StepIntent {
                target: serde_json::to_string(&QueuedTarget::Service(SAGA_TARGET.to_string()))
                    .unwrap(),
                routing_key: None,
                interface: "saga-participant".to_string(),
                method: "reserve".to_string(),
                params: b"{}".to_vec(),
            },
            proxy_outbox::now_ms(),
        )
        .unwrap();
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        node.router.sweep_sagas_once().await;
        assert_eq!(
            node.target.invoked.load(Ordering::SeqCst),
            1,
            "the pending step was undone too"
        );
    }

    #[tokio::test]
    async fn a_late_arriving_forward_result_does_not_revert_an_already_compensated_step() {
        // SAGA-02: intent-before-call means a step row can sit `pending`
        // for the whole forward call. If a deadline (or a concurrent
        // `compensate`) starts the walk before that call returns, and the
        // walk reaches and compensates the step first, the forward call's
        // own late-arriving result must not overwrite `compensated` back
        // to `done` -- that would cost a spurious second undo and make
        // `compensated_steps` count backwards. Reproduced directly against
        // the log rather than by racing two real tasks: the intent is
        // recorded exactly as if the forward call were still in flight
        // (no outcome recorded), which is the same state a real in-flight
        // call leaves it in.
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        let log = node.sagas.log_for(SAGA_CALLER).await.unwrap();
        log.record_step_intent(
            &saga_id,
            &syneroym_async_queue::StepIntent {
                target: serde_json::to_string(&QueuedTarget::Service(SAGA_TARGET.to_string()))
                    .unwrap(),
                routing_key: None,
                interface: "saga-participant".to_string(),
                method: "reserve".to_string(),
                params: b"{}".to_vec(),
            },
            proxy_outbox::now_ms(),
        )
        .unwrap();
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        // One sweep: the pending step is undone and marked `compensated`,
        // but the saga itself is not yet finished (that needs a sweep that
        // finds nothing left).
        node.router.sweep_sagas_once().await;
        assert_eq!(
            node.target.invoked.load(Ordering::SeqCst),
            1,
            "the pending step's undo must have run"
        );

        // The forward call's own result finally arrives, after the walk
        // already decided this step's fate.
        log.record_step_outcome(&saga_id, 0, Some(b"late-result"), None, proxy_outbox::now_ms())
            .unwrap();

        // If the guard above did nothing, this write just reset the step
        // back to `done`, and this second sweep would find it again and
        // send a second undo.
        node.router.sweep_sagas_once().await;
        assert_eq!(
            node.target.invoked.load(Ordering::SeqCst),
            1,
            "a late-arriving forward result must not cause a second undo to be sent"
        );
        let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
        assert_eq!(status.state, RpcSagaState::Compensated);
    }

    #[tokio::test]
    async fn an_undo_carries_the_saga_and_step_as_its_idempotency_key() {
        // Every undo's idempotency key is host-minted as
        // `saga:<saga-id>:<idx>`, never guest-set -- this is what makes
        // incrementing attempts before dispatch safe, since a re-dispatch
        // after a crash mid-undo is fenced by the receiver's own record
        // under exactly this key.
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        add_step(&node, &saga_id, "a").await;
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        node.router.sweep_sagas_once().await;
        // 2, not 1: `add_step`'s own forward call already reached the
        // target once, and the undo is the second.
        assert_eq!(node.target.invoked.load(Ordering::SeqCst), 2);

        let caller = format!("system:{SAGA_CALLER}");
        let key = format!("saga:{saga_id}:0");
        assert!(
            node.dedup_guard.debug_has_settled_key(SAGA_TARGET, &caller, &key).await,
            "the undo must have fenced under exactly 'saga:<saga-id>:<idx>', got no settled \
             record for that key"
        );
    }

    #[tokio::test]
    async fn a_dependency_bound_to_nobody_fails_the_compensation_without_retrying() {
        // Row 9's saga counterpart, and the regression test for the
        // bug this review round found: a target that no longer resolves
        // to anybody is "nothing to deliver to", not a failed delivery --
        // it must fail the saga on the very first sweep, never spend the
        // retry budget (`fail_terminal_saga_step`, not `fail_saga_step`).
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        let log = node.sagas.log_for(SAGA_CALLER).await.unwrap();
        log.record_step_intent(
            &saga_id,
            &syneroym_async_queue::StepIntent {
                target: serde_json::to_string(&QueuedTarget::Dependency("gone".to_string()))
                    .unwrap(),
                routing_key: None,
                interface: "saga-participant".to_string(),
                method: "reserve".to_string(),
                params: b"{}".to_vec(),
            },
            proxy_outbox::now_ms(),
        )
        .unwrap();
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        node.router.sweep_sagas_once().await;
        let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
        assert_eq!(
            status.state,
            RpcSagaState::Failed,
            "a target bound to nobody must fail the saga at once, not stay compensating"
        );
        assert_eq!(
            node.target.invoked.load(Ordering::SeqCst),
            0,
            "there was nowhere to deliver to, so nothing should ever have been dispatched"
        );
    }

    #[tokio::test]
    async fn an_unopenable_saga_log_settles_nothing_and_loses_nothing() {
        // A locked vault (`KekRequired`, the state of every substrate
        // after a restart until an operator injects the KEK) must not
        // make the sweep act as if the service had no sagas at all.
        // Proven behaviourally: the saga survives the locked sweep
        // untouched, and a fresh sweep against the same file picks it
        // straight back up once the KEK is injected.
        use syneroym_data_db::SqliteStorageProvider;
        use syneroym_data_keystore::KeyStore;

        let dir = tempfile::tempdir().unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([7u8; 32]).unwrap();
        // Real encryption, unlike every other saga test in this module --
        // the locked-vault condition only exists in that mode.
        let provider = Arc::new(SqliteStorageProvider::new(dir.path(), true).unwrap());
        let resolver = syneroym_app_orchestration::empty_resolver();

        let sagas1 = Arc::new(SagaStore::new(
            provider.clone(),
            key_store.clone(),
            resolver.clone(),
            saga_config(5),
        ));
        let log1 = sagas1.log_for(SAGA_CALLER).await.unwrap();
        let now = proxy_outbox::now_ms();
        log1.begin("locked-saga", "wf", None, now + 60_000, now).unwrap();

        key_store.clear_kek();

        // A fresh `SagaStore` over the same file with a cold cache --
        // exactly what a restart looks like.
        let sagas2 = Arc::new(SagaStore::new(
            provider.clone(),
            key_store.clone(),
            resolver.clone(),
            saga_config(5),
        ));
        let registry = empty_registry();
        registry
            .register(
                SAGA_CALLER.to_string(),
                "saga-driver".to_string(),
                SubstrateEndpoint::WasmChannel { service_id: SAGA_CALLER.to_string() },
            )
            .await
            .unwrap();
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
        let router = Arc::new(
            ProxyRouter::new(
                registry,
                empty_registry_client(),
                Arc::downgrade(&native_dispatch),
                Weak::new(),
                Arc::new(MockHop::default()),
                Arc::new(Identity::generate().unwrap()),
                RetryPolicy { max_attempts: 1, ..RetryPolicy::default() },
            )
            .with_sagas(sagas2.clone()),
        );

        let settled = router.sweep_sagas_once().await;
        assert_eq!(settled, 0, "a locked vault must settle nothing, not error out or panic");

        key_store.inject_kek([7u8; 32]).unwrap();
        let log_after_unlock = sagas2.log_for(SAGA_CALLER).await.unwrap();
        assert!(
            log_after_unlock.status("locked-saga").unwrap().is_some(),
            "the saga written before the lock must still be there once it is unlocked"
        );
    }

    #[tokio::test]
    async fn cancellation_interrupts_an_in_flight_undo_and_leaves_the_saga_compensating() {
        // The same property
        // `shutdown_abandons_an_in_flight_delivery_rather_than_draining` proves
        // for the outbox, for the saga sweep: worker shutdown must interrupt an
        // undo that never resolves, not wait out its own
        // budget (`SAGA_UNDO_CALL_BUDGET`). Left otherwise, one
        // unreachable participant could hold node shutdown hostage.
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        add_step(&node, &saga_id, "a").await;
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        // The target blocks and is never released during the test, so any
        // undo the worker starts genuinely never resolves on its own.
        let release = Arc::new(tokio::sync::Notify::new());
        *node.target.hold.lock().unwrap() = Some(release.clone());

        let cancel = CancellationToken::new();
        let worker = {
            let router = node.router.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                router.run_async_worker(Duration::from_millis(5), cancel).await;
            })
        };

        let entered =
            wait_for(Duration::from_secs(5), || node.target.invoked.load(Ordering::SeqCst) >= 1)
                .await;
        assert!(
            entered,
            "the worker never reached the target, so this test would prove nothing about \
             interrupting an in-flight undo"
        );

        // The load-bearing assertion: the undo in flight right now cannot
        // finish on its own, so the worker can only return if cancellation
        // is raced *into* the delivery.
        cancel.cancel();
        let stopped = tokio::time::timeout(Duration::from_secs(2), worker).await;
        assert!(
            stopped.is_ok(),
            "shutdown must interrupt an undo that never resolves, not wait it out"
        );

        release.notify_waiters();
        let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
        assert_eq!(
            status.state,
            RpcSagaState::Compensating,
            "an abandoned undo must leave the saga compensating, not silently finished or failed"
        );
    }

    #[tokio::test]
    async fn a_retryable_undo_failure_schedules_a_backoff_and_keeps_the_saga_compensating() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        add_step(&node, &saga_id, "a").await;
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        // Re-register the target as a WASM channel with no sandbox engine
        // behind this router (`Weak::new()`) -- "sandbox engine
        // unavailable" is a `ProxyError::Internal`, which `disposition_of`
        // classifies `Retry` (a shutdown-window state, not a settled
        // refusal). A definitive `Callee` error -- what `fail_with` would
        // produce -- is terminal by default and does not exercise this
        // path.
        node.registry
            .register(
                SAGA_TARGET.to_string(),
                "saga-participant".to_string(),
                SubstrateEndpoint::WasmChannel { service_id: SAGA_TARGET.to_string() },
            )
            .await
            .unwrap();

        node.router.sweep_sagas_once().await;
        let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
        assert_eq!(
            status.state,
            RpcSagaState::Compensating,
            "a retryable failure keeps compensating"
        );
    }

    #[tokio::test]
    async fn a_terminal_undo_failure_fails_the_saga_immediately() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        add_step(&node, &saga_id, "a").await;
        *node.target.answer_with.lock().unwrap() =
            Some(ProxyError::Callee { code: -32010, message: "denied".to_string(), data: None });
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        node.router.sweep_sagas_once().await;
        let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
        assert_eq!(status.state, RpcSagaState::Failed, "a terminal failure fails the saga at once");
    }

    #[tokio::test]
    async fn an_undo_the_receiver_had_already_run_counts_as_compensated() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        add_step(&node, &saga_id, "a").await;
        // "Already ran, result too large to retain" -- a delivery reported
        // through the error channel, per `disposition_of`'s `Delivered`
        // case (B2's N1). `CALL_ALREADY_RUNNING_RPC_CODE` is a *different*
        // code (still in flight right now) and classifies `Retry`, not
        // `Delivered`.
        *node.target.answer_with.lock().unwrap() = Some(ProxyError::Callee {
            code: syneroym_async_queue::CALL_RESULT_NOT_RETAINED_RPC_CODE,
            message: "already ran; result too large to retain".to_string(),
            data: None,
        });
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        node.router.sweep_sagas_once().await;
        node.router.sweep_sagas_once().await;
        let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
        assert_eq!(
            status.state,
            RpcSagaState::Compensated,
            "the receiver's already-ran answer must count as compensated, not retried forever"
        );
    }

    #[tokio::test]
    async fn a_missing_compensation_is_recorded_with_an_error_that_names_the_convention() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        add_step(&node, &saga_id, "a").await;
        *node.target.answer_with.lock().unwrap() = Some(ProxyError::Callee {
            code: syneroym_rpc::SERVICE_NOT_FOUND_RPC_CODE,
            message: "Method not found: saga-undo-reserve".to_string(),
            data: None,
        });
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        // `SERVICE_NOT_FOUND_RPC_CODE` classifies as retryable, so this is
        // recorded as a `Retry` outcome, not `Failed` -- but the rewritten
        // message must already name the convention on this very first
        // attempt, not only once the saga eventually fails.
        node.router.sweep_sagas_once().await;
        let status = node.router.saga_status(SAGA_CALLER, &saga_id).await.unwrap();
        assert!(
            status.last_error.as_deref().is_some_and(|e| e.contains("saga-undo-reserve")),
            "expected the recorded error to name the convention, got {:?}",
            status.last_error
        );
    }

    #[tokio::test]
    async fn a_saga_past_its_deadline_starts_compensating_without_the_guest() {
        let node = saga_node(5).await;
        let saga_id = node
            .router
            .saga_begin(SagaBegin {
                caller_service_id: SAGA_CALLER.to_string(),
                app_instance_id: None,
                name: "wf".to_string(),
                deadline_secs: Some(1),
            })
            .await
            .unwrap();
        add_step(&node, &saga_id, "a").await;

        // No guest-driven `compensate` call at all -- the deadline alone
        // must start the walk.
        let past_deadline = proxy_outbox::now_ms() + 2_000;
        let log = node.sagas.log_for(SAGA_CALLER).await.unwrap();
        for head in log.abandoned(past_deadline, 10).unwrap() {
            log.mark_compensating(&head.saga_id, past_deadline).unwrap();
        }

        node.router.sweep_sagas_once().await;
        assert_eq!(node.target.invoked.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_undeployed_services_sagas_are_dropped_rather_than_compensated() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        add_step(&node, &saga_id, "a").await;
        // `add_step`'s own forward call already reached the target once.
        let invoked_before = node.target.invoked.load(Ordering::SeqCst);
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        node.registry.remove_instance_cert(SAGA_CALLER).await.unwrap();
        node.registry.remove(SAGA_CALLER, "saga-driver").await.unwrap();

        // Two consecutive absent ticks, not one: a redeploy can leave a
        // service briefly missing from the registry for a single tick, and
        // one miss must not be enough to destroy its saga log (SAGA-03).
        node.router.sweep_sagas_once().await;
        let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
        assert!(
            !log.list().unwrap().is_empty(),
            "one absent tick alone must not drop the saga -- it could be a redeploy in flight"
        );

        node.router.sweep_sagas_once().await;
        assert_eq!(
            node.target.invoked.load(Ordering::SeqCst),
            invoked_before,
            "an undeployed service's saga must never send an undo"
        );
        let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
        assert!(log.list().unwrap().is_empty(), "its sagas must be dropped, not left compensating");
    }

    #[tokio::test]
    async fn a_service_that_reappears_between_ticks_keeps_its_sagas() {
        let node = saga_node(5).await;
        let saga_id = begun_saga(&node).await;
        add_step(&node, &saga_id, "a").await;
        node.router.saga_compensate(SAGA_CALLER, &saga_id).await.unwrap();

        let endpoint = node.registry.lookup_by_service(SAGA_CALLER)[0].1.clone();
        node.registry.remove_instance_cert(SAGA_CALLER).await.unwrap();
        node.registry.remove(SAGA_CALLER, "saga-driver").await.unwrap();
        node.router.sweep_sagas_once().await;

        // The service comes back before a second consecutive absent tick.
        node.registry
            .register(SAGA_CALLER.to_string(), "saga-driver".to_string(), endpoint)
            .await
            .unwrap();
        node.router.sweep_sagas_once().await;

        // A later real undeploy must still need its own two consecutive
        // ticks, not fire immediately off a stale mark from the first one.
        node.registry.remove(SAGA_CALLER, "saga-driver").await.unwrap();
        node.router.sweep_sagas_once().await;
        let log = node.sagas.existing_log_for(SAGA_CALLER).await.unwrap().unwrap();
        assert!(
            !log.list().unwrap().is_empty(),
            "the absence mark must have been cleared when the service reappeared"
        );
    }

    /// The control plane downgrades `ProxyState` into a
    /// `Weak<dyn ProxyQueueInspector>` it holds in a `OnceLock`. That
    /// `Weak` must stay valid for as long as the router does -- which only
    /// holds if `ProxyRouter` itself is the bundle's one strong owner. A
    /// wrapper built and downgraded with no long-lived owner would drop the
    /// moment the constructing function returned, and every operator verb
    /// would then answer "this node keeps no durable proxy state".
    #[tokio::test]
    async fn the_operator_verbs_still_answer_after_the_router_is_the_only_owner_left() {
        use syneroym_data_db::SqliteStorageProvider;
        use syneroym_data_keystore::KeyStore;

        let node = saga_node(5).await;
        let dir = tempfile::tempdir().unwrap();
        let provider = Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
        let outbox = Arc::new(ProxyOutbox::new(
            provider,
            Arc::new(KeyStore::new()),
            syneroym_app_orchestration::empty_resolver(),
            QueueConfig {
                retry: RetryPolicy::default(),
                visibility_timeout_ms: 60_000,
                dlq_max_rows: 100,
                max_pending_rows: syneroym_async_queue::DEFAULT_MAX_PENDING_ROWS,
            },
        ));

        let router = ProxyRouter::new(
            node.registry.clone(),
            empty_registry_client(),
            Weak::new(),
            Weak::new(),
            Arc::new(MockHop::default()),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy::default(),
        )
        .with_outbox(outbox)
        .with_sagas(node.sagas.clone());
        let router = Arc::new(router);

        // Mirrors `route_handler.rs`'s own wiring exactly: downgrade from a
        // local binding, then let that binding go out of scope.
        let weak: Weak<dyn ProxyQueueInspector> = {
            let state = router.proxy_state().expect("both outbox and sagas are wired").clone();
            Arc::downgrade(&state) as Weak<dyn ProxyQueueInspector>
        };

        assert!(
            weak.upgrade().is_some(),
            "the router's own Arc<ProxyState> must be what keeps this Weak alive"
        );
    }

    #[test]
    fn check_native_capability_gate_refuses_cross_service_signing_call() {
        let registry =
            EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));
        let router = ProxyRouter::new(
            registry,
            empty_registry_client(),
            Weak::new(),
            Weak::new(),
            Arc::new(MockHop::default()),
            Arc::new(Identity::generate().unwrap()),
            RetryPolicy::default(),
        );

        let mut self_req = base_request("svc1", "signing");
        self_req.origin = CallOrigin::Guest { service_id: "svc1".to_string() };
        self_req.method = "sign-record".to_string();
        assert!(router.check_native_capability_gate(&self_req).is_ok());

        let mut cross_req = base_request("svc2", "signing");
        cross_req.origin = CallOrigin::Guest { service_id: "svc1".to_string() };
        cross_req.method = "sign-record".to_string();
        let err = router.check_native_capability_gate(&cross_req).unwrap_err();
        assert!(matches!(err, ProxyError::PermissionDenied(msg) if msg.contains("signing")));
    }
}

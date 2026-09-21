//! Universal Proxy dispatch: a transport-agnostic outbound
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

mod hop;
mod outbox_forwarding;
mod router;
mod saga_dispatch;
mod state;

#[cfg(test)]
mod tests;

pub use hop::*;
#[cfg(test)]
use saga_dispatch::{merge_forward_result, step_call_budget_ms};
pub use state::*;

/// avoid the `RouteHandlerInner -> ProxyRouter -> AppSandboxEngine ->
/// ProxyRouter` reference cycle that once hung graceful shutdown).
/// The Universal Proxy's outbound router. Holds `Weak`
/// handles into the engine/dispatch-registry it routes to, and the
/// registry/registry-client it uses to resolve targets -- see the module doc
/// comment on ownership direction (`RouteHandlerInner` is the strong owner;
/// `AppSandboxEngine` only ever holds a `Weak<dyn ServiceProxy>` back, to
/// avoid the `RouteHandlerInner -> ProxyRouter -> AppSandboxEngine ->
/// ProxyRouter` reference cycle that once hung graceful shutdown).
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
}

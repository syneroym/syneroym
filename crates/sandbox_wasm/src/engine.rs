//! WASM execution engine based on Wasmtime
//!
//! Sets up the sandboxed environment with strict CPU/memory quotas,
//! registers host capabilities, and runs WASM component binaries.

use std::{
    fmt::{self, Debug, Formatter},
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use syneroym_app_host::types::http::FrameKind;
use syneroym_app_host::types::http::{HttpRequest, HttpResponse};
use syneroym_app_orchestration::LogicalResolver;
use syneroym_chunk_transfer::{self as chunk_transfer, ChunkSink};
use syneroym_core::{
    config::SubstrateConfig,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    record_signer::NodeRecordSigner,
    streaming::StreamDirection,
};
use syneroym_data_blob::traits::BlobProvider;
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::Policy;
use syneroym_mqtt_broker::{MqttBroker, SubscriptionHandle};
use syneroym_rpc::{
    AbacAuthContext, AbacError, AuthLevel, CallerContext, CandidateRow, ConversationDeliveryState,
    JsonRpcRequest, RowAuthorizer, RowDecision, ServiceProxy, WebSocketSenders,
};
pub use syneroym_rpc::{WebSocketReceiver, WebSocketSender};
use syneroym_wit_interfaces::{
    control_plane::exports::syneroym::control_plane::orchestrator::{
        ArtifactSource, DeployManifest, ServiceType,
    },
    conversation_host::syneroym::conversation::conversation,
    host::syneroym::{
        app_config::app_config,
        blob_store::blob_store,
        data_layer::store,
        host::context,
        messaging::host_api,
        proxy::{proxy, saga},
        vault::vault,
    },
    http_host::syneroym::http::websocket,
    invocation_host::syneroym::invocation::invocation,
    signing_host::syneroym::signing::signing,
};
use tokio::{
    fs as tokio_fs,
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, Semaphore, oneshot},
    task, time,
};
use tracing::{debug, error, info, warn};
use wasmtime::{
    Cache, CacheConfig, Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, Store,
    Trap,
    component::{
        Component, Func, HasSelf, Instance, InstancePre, Linker, Val, types::ComponentItem,
    },
};
use wasmtime_wasi::p2;

use crate::{
    conversions,
    host_capabilities::{self, HostState, InvocationOrigin, MessagingContext},
    http,
    stream::{self, GuestStreamCursor, GuestStreamSink, StreamContext, StreamRegistry},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmResourceQuota {
    pub max_instructions: Option<u64>,
    pub max_memory_bytes: Option<u64>,
}

/// Engine: Passive code module that wraps low-level OS operations
/// to spin up Wasmtime or Podman instances.
#[allow(clippy::type_complexity)]
pub struct AppSandboxEngine {
    pub(crate) blobs_dir: PathBuf,
    pub(crate) engine: Engine,
    pub(crate) linker: Linker<HostState>,
    // Cache of pre-linked instances for fast instantiation
    pub(crate) components: DashMap<String, (InstancePre<HostState>, Option<WasmResourceQuota>)>,
    /// Resolved-policy cache, keyed by `service_id`, next to `components`.
    /// The value is itself an `Option` so that *"resolved: this service has
    /// no policy"* -- the common case -- is cached too, instead of
    /// re-querying `substrate.db` per invocation (ADR-0017).
    /// `parse_and_validate` compiles the embedded JSON Schema and
    /// re-validates on every call, which would put schema compilation on the
    /// hot path of every guest invocation if this weren't cached. Evicted on
    /// `stop_wasm` and `compile_and_cache_wasm` so a re-deploy re-resolves
    /// rather than serving the previous policy.
    pub(crate) fdae_policies: DashMap<String, Option<Arc<Policy>>>,
    /// Per-service generation counter, bumped by every `fdae_policies`
    /// eviction (`stop_wasm`, `compile_and_cache_wasm`). `resolve_fdae_policy`
    /// captures it before its (possibly slow, cross-await) storage read and
    /// compares after: if an eviction raced the read, the read is stale and
    /// must not be cached. Without this, a redeploy's eviction can fire
    /// against a key that isn't cached yet (the racing load hasn't inserted
    /// it), and the racing load then inserts its *old* result afterward --
    /// resurrecting a policy the redeploy should have replaced, indefinitely
    /// (until the next `stop_wasm`/redeploy). See `resolve_fdae_policy`.
    ///
    /// Two known, accepted residuals, both narrower than the race above:
    /// (1) the generation comparison and the `fdae_policies` insert in
    /// `resolve_fdae_policy` are still two separate `DashMap` operations
    /// (no `await` between them, unlike the wide window this counter
    /// closes), so an eviction landing in that narrow gap is still
    /// silently undone; and (2) entries here are only ever inserted or
    /// bumped, never removed (`stop_wasm` evicts `fdae_policies` but not
    /// this map), so the map grows by one entry per distinct `service_id`
    /// this process has ever seen, for the process's lifetime. Neither has
    /// an observed impact -- closing (1) fully would need the two maps
    /// merged behind one lock (a real redesign for a race narrower than
    /// the one already closed), and (2) is bounded by service churn, not
    /// request volume.
    pub(crate) fdae_policy_generation: DashMap<String, u64>,
    pub(crate) default_max_instructions: Option<u64>,
    pub(crate) default_max_memory_bytes: Option<u64>,
    pub(crate) _shutdown_tx: Option<oneshot::Sender<()>>,
    pub key_store: Arc<KeyStore>,
    pub storage_provider: Arc<dyn StorageProvider>,
    pub blob_provider: Arc<dyn BlobProvider>,
    pub messaging_broker: Arc<MqttBroker>,
    /// Set once, immediately after the engine is wrapped in an `Arc` by its
    /// owner (see module docs on [`MessagingContext`]). Lets a live
    /// `subscribe()` call's forwarding task reach back into the engine to
    /// invoke `deliver_message` long after the `Store` that made the call
    /// is gone.
    pub self_weak: OnceLock<Weak<AppSandboxEngine>>,
    /// Set once at the composition root, immediately after the engine and
    /// the `ProxyRouter` (`syneroym-router`) are both
    /// constructed. `Weak`, not `Arc`: the proxy holds a
    /// `Weak<AppSandboxEngine>` back (its local-WASM-target dispatch path),
    /// and two strong refs would be an uncollectable cycle (the same class
    /// that once hung graceful shutdown).
    pub service_proxy: OnceLock<Weak<dyn ServiceProxy>>,
    /// The Conversation service. Same two-phase `Weak`
    /// wiring as `service_proxy`, for the same cycle reason:
    /// `ConversationService` and this engine are constructed independently
    /// at the composition root and wired to each other afterward.
    pub conversation: OnceLock<Weak<dyn syneroym_rpc::ConversationHost>>,
    /// The node's record signer (`syneroym:signing`).
    pub record_signer: OnceLock<Arc<NodeRecordSigner>>,
    /// Live guest-delivery subscriptions, keyed `(service_id,
    /// namespaced_topic)`. Dropping an entry unsubscribes from the broker
    /// (see `SubscriptionHandle::drop`).
    pub(crate) subscriptions: DashMap<(String, String), SubscriptionHandle>,
    /// `register-stream-protocol` (ADR-0014) writes into this
    /// same registry the router reads from, giving restart-replay and
    /// undeploy-cleanup for free -- see ADR-0014 "Where Registration Lives".
    pub(crate) endpoint_registry: EndpointRegistry,
    /// ADR-0021 §2: resolves a guest-declared dependency name to a
    /// member master DID, host-side. One `LogicalResolver` per substrate,
    /// shared with `ControlPlaneService`'s write side (`runtime.rs`'s
    /// composition root) -- read and write share the same cache so a
    /// binding write's eviction is visible here immediately.
    pub(crate) logical_resolver: Arc<LogicalResolver>,
    /// Per-service open-stream-instance task tracking; see `StreamRegistry`.
    pub(crate) stream_registry: StreamRegistry,
    pub(crate) max_concurrent_streams_per_service: u32,
    /// Bounds how many stream instances may be open across
    /// *all* services at once. Each open stream holds a pooled component
    /// instance for its whole lifetime (`open_stream_instance`), competing
    /// for the same engine-wide `total_component_instances` pool
    /// (`build_wasm_engine`) as every short-lived RPC/message-delivery call
    /// across every deployed service -- `max_concurrent_streams_per_service`
    /// alone only bounds one service's contribution, not the aggregate
    /// across services. Acquiring a permit here before opening a stream
    /// instance (see `run_stream_protocol_request`) keeps
    /// `STREAM_INSTANCE_POOL_HEADROOM` pool slots always available for
    /// ordinary calls, instead of letting streams silently starve them.
    pub(crate) stream_instance_permits: Arc<Semaphore>,
    /// Pool slots the stage-4 ABAC after-step (`authorize_rows`) may hold
    /// concurrently, out of `max_concurrent_instances`. A stage-4-active
    /// read holds *two* instances at once for the
    /// after-step's duration: its own live dispatch instance (uncounted
    /// here -- it isn't gated by any semaphore today) plus this throw-away
    /// one, which nothing budgeted for before this fix. With no cap, a
    /// handful of concurrent stage-4 reads could exhaust the whole
    /// wasmtime pool -- a hard `PoolConcurrencyLimitError` at instantiation,
    /// not a wait -- which every caller (`AbacError::Unavailable`) then
    /// mapped to an empty-but-successful page, indistinguishable from
    /// "nothing matched". Acquiring a permit here turns that into bounded
    /// queuing instead: a caller waiting past `FDAE_ABAC_TIMEOUT` surfaces
    /// as `AbacError::BudgetExceeded`, which ingress code now maps to a
    /// distinguishable resource-exhausted error, never to
    /// "authorized, zero rows". Fixed at half of
    /// `STREAM_INSTANCE_POOL_HEADROOM` (see where it's
    /// computed in `init` for the full accounting), *not* scaled by
    /// `max_concurrent_instances`: `stream_instance_permits`' own budget
    /// formula predates this fix and is asserted exactly by an existing
    /// test at a small pool size, so this reservation had to fit inside
    /// the headroom that formula already carves out for "ordinary calls",
    /// rather than growing the pool's overall reservation and shrinking
    /// what streams get. Raising stage-4 throughput today means raising
    /// this fixed budget (and `STREAM_INSTANCE_POOL_HEADROOM` alongside
    /// it) in code, not raising `max_concurrent_instances` alone.
    pub(crate) abac_instance_permits: Arc<Semaphore>,
    /// Pool slots a health sweep's `rpc` probes may hold concurrently, out
    /// of the `STREAM_INSTANCE_POOL_HEADROOM` slots already reserved for
    /// short-lived ordinary calls generally, found while measuring the
    /// health-poll-cost budget. Before this, nothing
    /// bounded how many `rpc` probes could instantiate a component at
    /// once: `status_impl`'s own concurrent fan-out sends every
    /// target's probe at the same time, and once that count exceeds this
    /// engine's `total_component_instances` pool, wasmtime rejects the
    /// excess outright -- a real deploy reporting `ProbeFailing` with a
    /// resource-exhaustion message, not an unhealthy service. Acquiring a
    /// permit here turns that into bounded queuing instead: a wasm-heavy
    /// instance's sweep degrades to serialized-but-correct probing rather
    /// than spurious failures. Scoped to probes specifically, not every
    /// ordinary call -- the broader "nothing gates ordinary calls against
    /// the pool at all" question is unrelated to this budget and out of
    /// scope here.
    pub(crate) probe_instance_permits: Arc<Semaphore>,
    /// Epoch-tick budget for an ordinary dispatch call (RPC/proxy
    /// invocation, message delivery, one streaming chunk) -- see
    /// `AppSandboxRole::dispatch_epoch_timeout_secs`.
    pub(crate) dispatch_epoch_ticks: u64,
    /// Epoch-tick budget for a component's `init()`/`migrate()` lifecycle
    /// hook -- see `AppSandboxRole::lifecycle_hook_epoch_timeout_secs`.
    pub(crate) lifecycle_hook_epoch_ticks: u64,
    /// Epoch-tick budget for one stage-4 ABAC after-step invocation -- see
    /// `AppSandboxRole::abac_epoch_timeout_secs`.
    pub(crate) abac_epoch_ticks: u64,
    /// Fuel ceiling for one stage-4 ABAC after-step invocation -- see
    /// `AppSandboxRole::abac_max_instructions`.
    pub(crate) abac_max_instructions: u64,
    /// Total component instantiations this engine has performed, process-
    /// lifetime. Alongside the pre-existing
    /// `substrate.wasm.instantiation_ms` histogram (a duration, not a
    /// count) -- a test needs an in-process value it can read
    /// a **delta** across (an absolute count is meaningless on its own:
    /// deploy-time lifecycle hooks instantiate the component too, so a test
    /// asserting on an absolute value would be coupled to unrelated
    /// deploy-time behaviour).
    pub(crate) instantiations: AtomicU64,
    /// Pool slots this node will let *guest HTTP* requests hold
    /// concurrently, per service. Unlike an RPC client, one
    /// browser page issues six or more parallel requests, and exhausting
    /// wasmtime's pool is a hard `PoolConcurrencyLimitError` at
    /// instantiation rather than a wait -- so without this, a single page
    /// load turns into 500s and can also drain the headroom
    /// `stream_instance_permits` reserves for ordinary calls. Bounded
    /// queuing instead, with a 503 past the wait.
    ///
    /// Entries are removed by `forget_guest_http_permits` on undeploy,
    /// matching `unsubscribe_all`/`abort_streams` -- every other
    /// per-service map here has an explicit teardown, and a map that only
    /// ever grows is a leak however small.
    pub(crate) guest_http_permits: Arc<DashMap<String, Arc<Semaphore>>>,
    /// Snapshot of `AppSandboxRole::max_concurrent_guest_http_per_service`,
    /// the size each per-service semaphore above is created at.
    pub(crate) max_concurrent_guest_http_per_service: u32,
    /// Registry for routing guest unicast sends back to active WebSocket
    /// connections.
    pub websocket_senders: OnceLock<Arc<syneroym_rpc::WebSocketSenders>>,

    /// Pool slots this node will let active WebSocket connections hold
    /// concurrently.
    pub(crate) guest_websocket_permits: Arc<DashMap<String, Arc<Semaphore>>>,
    pub(crate) max_concurrent_websockets_per_service: u32,
    pub(crate) max_sse_subscribers_per_service: u32,
}

/// Per-instantiation differences from an ordinary dispatch call. Bundled
/// into one struct rather than more positional parameters on
/// `build_store_and_instantiate` -- today's sole non-default use is the
/// stage-4 after-step (`authorize_rows`), which needs both fields at once.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InstanceOptions {
    /// Overrides the service's own quota-derived fuel. `None` keeps it.
    pub(crate) fuel_override: Option<u64>,
    pub(crate) read_only: bool,
    /// Where this call entered the node. `Local` for every path except the
    /// router's inbound wire dispatch.
    pub(crate) invocation_origin: InvocationOrigin,
}

impl InstanceOptions {
    /// Router ingress -- guest HTTP, a raw stream, a websocket frame. The
    /// call came off the wire, so `invocation.caller()` must report
    /// `verified`/`anonymous`, never `internal`.
    pub(crate) fn from_wire() -> Self {
        Self { invocation_origin: InvocationOrigin::Wire, ..Self::default() }
    }
}

/// Pool slots reserved out of `max_concurrent_instances` for short-lived
/// RPC/message-delivery calls; the remainder is the budget
/// `stream_instance_permits` hands out to long-lived stream instances. See
/// that field's doc comment for the cross-service DoS this prevents.
const STREAM_INSTANCE_POOL_HEADROOM: u32 = 2;

/// How long a guest HTTP request waits for its service's admission permit
/// before the router answers 503. Short on purpose: a
/// browser that waited longer than this has already given the user a
/// stalled page, and a fast, honest "busy, retry" beats a slow success.
const GUEST_HTTP_ADMISSION_TIMEOUT: Duration = Duration::from_secs(2);

/// How often the epoch ticker (spawned in `init`) advances Wasmtime's global
/// epoch. `Store::set_epoch_deadline` counts in ticks of this interval, not
/// seconds directly -- see [`ticks_for_secs`].
const EPOCH_TICK_MS: u64 = 100;

/// Converts an operator-facing timeout in seconds
/// (`AppSandboxRole::dispatch_epoch_timeout_secs` /
/// `lifecycle_hook_epoch_timeout_secs`) into the tick count
/// `Store::set_epoch_deadline` expects, given the `EPOCH_TICK_MS` ticker.
const fn ticks_for_secs(secs: u64) -> u64 {
    (secs * 1000) / EPOCH_TICK_MS
}

/// Tracks the `substrate.wasm.active_instances` gauge for the lifetime of
/// one guest call. Hoisted to module scope from
/// its original home inside `execute_wasm_vals` so
/// `handle_guest_http_request` can reuse it too -- every other
/// guest-invoking path records this metric, and the guest HTTP path is no
/// exception.
struct ActiveInstanceGuard;
impl ActiveInstanceGuard {
    pub(crate) fn new() -> Self {
        metrics::gauge!("substrate.wasm.active_instances").increment(1.0);
        Self
    }
}
impl Drop for ActiveInstanceGuard {
    fn drop(&mut self) {
        metrics::gauge!("substrate.wasm.active_instances").decrement(1.0);
    }
}

/// How a `Func::call_async` failure should be read. One
/// definition, shared by `execute_wasm_vals`, `authorize_rows`, and
/// `handle_guest_http_request` -- previously two hand-rolled, independently
/// drifting copies of the same taxonomy, one of which had no memory-fault
/// arm at all. Each consumer maps these variants to its own error type,
/// preserving that consumer's pre-refactor behaviour exactly (see the two
/// call sites for the pinned mapping, including the deliberately
/// unchanged gaps).
pub(crate) enum CallFailure {
    OutOfFuel,
    MemoryFault,
    Deadline,
    Other,
}

/// Classifies a `Func::call_async` error by inspecting the Wasmtime trap
/// type first, then falling back to matching known substrings in the root
/// cause -- the same two-step approach both pre-refactor copies used,
/// unified into one place. Order matters: fuel is checked before memory,
/// which is checked before an epoch deadline, matching both original
/// implementations' precedence.
pub(crate) fn classify_call_failure(e: &wasmtime::Error) -> CallFailure {
    if let Some(Trap::OutOfFuel) = e.downcast_ref::<Trap>() {
        return CallFailure::OutOfFuel;
    }
    let err_str = e.root_cause().to_string();
    if err_str.contains("all fuel consumed") || err_str.contains("out of fuel") {
        return CallFailure::OutOfFuel;
    }
    if err_str.contains("exceeded its memory limits") || err_str.contains("MemoryFault") {
        return CallFailure::MemoryFault;
    }
    if err_str.contains("epoch") || err_str.contains("deadline") {
        return CallFailure::Deadline;
    }
    CallFailure::Other
}

impl Debug for AppSandboxEngine {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppSandboxEngine")
            .field("blobs_dir", &self.blobs_dir)
            .field("components_len", &self.components.len())
            .finish()
    }
}

impl AppSandboxEngine {
    /// Helper to extract a WASM function and its result length. When
    /// `interface_name` is `Some`, looks up `method_name` nested inside that
    /// named interface's exported instance (the shape of ordinary `interface`
    /// exports). When `None`, looks up `method_name` directly as a root-level
    /// component export -- the shape of a WIT world's own `export foo: func`
    /// declarations, such as the `data-layer-guest` world's `init`/`migrate`.
    pub fn get_wasm_func(
        store: &mut Store<HostState>,
        instance: &Instance,
        interface_name: Option<&str>,
        method_name: &str,
    ) -> Result<(Func, usize, ComponentItem)> {
        let (item, func_idx) = match interface_name {
            Some(interface_name) => {
                let (_, instance_idx) = instance
                    .get_export(&mut *store, None, interface_name)
                    .ok_or_else(|| anyhow::anyhow!("Interface '{interface_name}' not found"))?;
                instance.get_export(&mut *store, Some(&instance_idx), method_name).ok_or_else(
                    || {
                        anyhow::anyhow!(
                            "Method '{method_name}' not found in interface '{interface_name}'"
                        )
                    },
                )?
            }
            None => instance
                .get_export(&mut *store, None, method_name)
                .ok_or_else(|| anyhow::anyhow!("Root export '{method_name}' not found"))?,
        };

        let func = instance
            .get_func(&mut *store, func_idx)
            .ok_or_else(|| anyhow::anyhow!("Method is not a function"))?;

        let results_len = match &item {
            ComponentItem::ComponentFunc(f) => f.results().len(),
            _ => 0,
        };

        Ok((func, results_len, item))
    }

    /// Extracts the failure message from a guest function's `result<_,
    /// string>` return value, if it returned `Err`. Shared by
    /// `invoke_lifecycle_hook` and `deliver_message`, which both call
    /// guest exports returning this shape and only care about the
    /// failure message.
    pub(crate) fn wasm_result_err(results: &[Val]) -> Option<&str> {
        if let Some(Val::Result(Err(Some(boxed)))) = results.first()
            && let Val::String(msg) = boxed.as_ref()
        {
            Some(msg.as_str())
        } else {
            None
        }
    }

    /// The well-known guest export for the stage-4 ABAC after-step
    /// (ADR-0017 §7, `wit/data-layer/authorizer.wit`). Deliberately not part
    /// of the `host-environment` world -- a component only needs to
    /// implement it when a deployed policy opts in.
    const AUTHORIZER_INTERFACE: &str = "syneroym:data-layer/authorizer@0.1.0";

    /// Total component instantiations this engine has performed,
    /// process-lifetime. Tests assert on a **delta**
    /// measured around the request under test, never this absolute value --
    /// deploy-time lifecycle hooks instantiate the component too.
    #[must_use]
    pub fn instantiations(&self) -> u64 {
        self.instantiations.load(Ordering::Relaxed)
    }

    /// Helper shared by `prepare_wasm_execution` and `invoke_lifecycle_hook`:
    /// looks up the pre-linked component, resolves its resource quotas,
    /// builds a fresh `HostState`/`Store`, and instantiates it.
    pub(crate) async fn build_store_and_instantiate(
        &self,
        service_id: &str,
        caller: CallerContext,
        epoch_deadline_ticks: u64,
        opts: InstanceOptions,
    ) -> Result<(Store<HostState>, Instance, Option<u64>)> {
        // Look up the pre-linked component instance
        let (instance_pre, quota) = {
            let entry = self
                .components
                .get(service_id)
                .ok_or_else(|| anyhow::anyhow!("Component not found for service {service_id}"))?;
            entry.value().clone()
        };
        debug!("looked up pre-linked component");

        // Resolve quotas
        let max_instructions =
            quota.as_ref().and_then(|q| q.max_instructions).or(self.default_max_instructions);

        let max_memory_bytes = quota
            .as_ref()
            .and_then(|q| q.max_memory_bytes)
            .or(self.default_max_memory_bytes)
            .map(|m| m as usize);

        let config_generation =
            match self.storage_provider.get_latest_config_generation(service_id).await {
                Ok(Some((g, _))) => g,
                Ok(None) => 0,
                Err(e) => {
                    error!("Failed to fetch config generation for {}: {}", service_id, e);
                    0
                }
            };

        // Create host state
        let messaging = MessagingContext {
            broker: self.messaging_broker.clone(),
            engine: self.self_weak.get().cloned().unwrap_or_default(),
        };
        let streaming = StreamContext {
            registry: self.endpoint_registry.clone(),
            engine: self.self_weak.get().cloned().unwrap_or_default(),
        };
        let service_proxy = self
            .service_proxy
            .get()
            .cloned()
            .unwrap_or_else(host_capabilities::empty_service_proxy);
        // `self_weak` is set once by the composition root immediately after
        // this engine is wrapped in an `Arc`, and `AppSandboxEngine` is the
        // sole `RowAuthorizer` implementation -- unsized coercion turns the
        // concrete `Weak<AppSandboxEngine>` into `Weak<dyn RowAuthorizer>` at
        // this `let`'s type annotation, same as `Arc<T> -> Arc<dyn Trait>`.
        let row_authorizer: Weak<dyn RowAuthorizer> = if let Some(w) = self.self_weak.get() {
            w.clone()
        } else {
            syneroym_rpc::empty_row_authorizer()
        };
        let fdae_policy = self.resolve_fdae_policy(service_id).await;
        // ADR-0021 §2: the host supplies `app_instance_id` from its own
        // records, never from the guest -- a guest that could name its own
        // app instance could address an arbitrary one.
        let app_instance_id =
            self.endpoint_registry.app_context_of(service_id).map(|(instance, _name)| instance);
        let conversation = self
            .conversation
            .get()
            .cloned()
            .unwrap_or_else(host_capabilities::empty_conversation_host);
        let host_state = HostState::new(
            service_id.to_string(),
            max_memory_bytes,
            self.key_store.clone(),
            self.storage_provider.clone(),
            self.blob_provider.clone(),
            caller,
            config_generation,
            messaging,
            streaming,
            service_proxy,
            fdae_policy,
            opts.read_only,
            row_authorizer,
            app_instance_id,
            self.logical_resolver.clone(),
        )
        .with_conversation(conversation)
        .with_websocket_senders(self.websocket_senders())
        .with_record_signer(self.record_signer.get().cloned())
        .with_invocation_origin(opts.invocation_origin);

        debug!("created wasi ctx and host state");

        // Create a new store
        let mut store = Store::new(&self.engine, host_state);

        store.limiter(|state| state);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(epoch_deadline_ticks);

        if let Some(instructions) = opts.fuel_override.or(max_instructions) {
            store.set_fuel(instructions)?;
        }

        let inst_start = Instant::now();
        let instance = instance_pre.instantiate_async(&mut store).await?;
        metrics::histogram!("substrate.wasm.instantiation_ms")
            .record(inst_start.elapsed().as_secs_f64() * 1000.0);
        metrics::counter!("substrate.wasm.instantiations_total").increment(1);
        self.instantiations.fetch_add(1, Ordering::Relaxed);

        debug!("instantiated store and instance");

        Ok((store, instance, max_instructions))
    }

    /// Helper to prepare WASM execution context and extract function
    ///
    /// `caller`, when `Some`, is the real caller this invocation carries
    /// through into `HostState.caller`; `None`
    /// preserves the prior synthesized-`service_system` behavior (an
    /// unauthenticated connection, or a test/dev-harness call via
    /// [`Self::execute_wasm`]).
    pub(crate) async fn prepare_wasm_execution(
        &self,
        service_id: &str,
        interface_name: &str,
        method_name: &str,
        caller: Option<CallerContext>,
        origin: InvocationOrigin,
    ) -> Result<(Store<HostState>, Func, usize, ComponentItem)> {
        // This is the ordinary dispatch path -- reached from wire-originated
        // JSON-RPC (`dispatch.rs`) and guest-to-guest proxy calls, both of
        // which let the caller pick `method_name` freely. It must never
        // grant `local_elevated` (the `data-layer/admin`-bearing, FDAE-exempt
        // context): a caller simply naming their request "init" or "migrate"
        // would otherwise self-elevate. `local_elevated` is reserved for
        // `invoke_lifecycle_hook`, which the deploy path calls directly
        // (never through this function) and builds its own caller/epoch
        // budget without consulting `method_name` at all. Same reasoning
        // bars a *forwarded* `caller` from ever carrying `LocalElevated`
        // here -- neither of this function's two callers can construct one
        // (`execute_wasm` always passes `None`; `dispatch.rs`/`proxy.rs`
        // only ever hold a router-verified or `service_system` caller). Not
        // just a comment: a `LocalElevated` caller reaching this function
        // would hand the guest `data-layer/admin` and skip the FDAE sieve
        // outright (`HostState::resolve_query_auth`'s `LocalElevated`
        // exemption), so a debug build catches a future call site that
        // starts constructing one and passing it through here.
        debug_assert!(
            !matches!(
                &caller,
                Some(c) if matches!(c.auth, AuthLevel::LocalElevated | AuthLevel::LocalReadOnly)
            ),
            "prepare_wasm_execution must never receive a forwarded LocalElevated or LocalReadOnly \
             caller -- those contexts are reserved for invoke_lifecycle_hook and authorize_rows \
             respectively, neither of which calls this function"
        );
        let caller = caller.unwrap_or_else(|| CallerContext::service_system(service_id));
        let (mut store, instance, _max_instructions) = self
            .build_store_and_instantiate(
                service_id,
                caller,
                self.dispatch_epoch_ticks,
                InstanceOptions { invocation_origin: origin, ..InstanceOptions::default() },
            )
            .await?;

        // Use the helper to extract the function
        let (func, results_len, item) =
            Self::get_wasm_func(&mut store, &instance, Some(interface_name), method_name)?;

        debug!("extracted the interface and method export indices");

        Ok((store, func, results_len, item))
    }
}

mod auth;
mod execute;
mod guest_http;
mod guest_stream;
mod init;
mod lifecycle;

pub(crate) use self::auth::truncate_detail;
pub use self::{
    guest_http::{GuestHttpFailure, GuestHttpOutcome},
    guest_stream::StreamRequestOutcome,
};

#[cfg(test)]
mod tests;

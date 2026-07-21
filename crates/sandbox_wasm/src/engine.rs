//! WASM execution engine based on Wasmtime
//!
//! Sets up the sandboxed environment with strict CPU/memory quotas,
//! registers host capabilities, and runs WASM component binaries.

use std::{
    fmt::{self, Debug, Formatter},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, Weak},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_chunk_transfer::{self as chunk_transfer, ChunkSink};
use syneroym_core::{
    config::SubstrateConfig,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    streaming::StreamDirection,
};
use syneroym_data_blob::traits::BlobProvider;
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::Policy;
use syneroym_mqtt_broker::{MqttBroker, SubscriptionHandle};
use syneroym_rpc::{CallerContext, JsonRpcRequest, ServiceProxy};
use syneroym_wit_interfaces::{
    control_plane::exports::syneroym::control_plane::orchestrator::{
        ArtifactSource, DeployManifest, ServiceType,
    },
    host::syneroym::{
        app_config::app_config, blob_store::blob_store, data_layer::store, host::context,
        messaging::host_api, proxy::proxy, vault::vault,
    },
};
use tokio::{
    fs as tokio_fs,
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    sync::{Semaphore, oneshot},
    time,
};
use tracing::{debug, error, info, warn};
use wasmtime::{
    Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, Store, Trap,
    component::{
        Component, Func, HasSelf, Instance, InstancePre, Linker, Val, types::ComponentItem,
    },
};
use wasmtime_wasi::p2;

use crate::{
    conversions::{json_to_wasm_params, wasm_results_to_json_string},
    host_capabilities::{HostState, MessagingContext},
    stream::{self, GuestStreamCursor, GuestStreamSink, StreamContext, StreamRegistry},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmResourceQuota {
    pub max_instructions: Option<u64>,
    pub max_memory_bytes: Option<u64>,
}

/// Distinguishes a stream request the guest cleanly declined (`Err` from
/// `handle-stream-request`/`accept-stream-upload`, or no matching export)
/// from one that ran to completion -- both of which were previously
/// collapsed into the same `Ok(())` (M3B Slice 7). Callers that need to
/// surface a decline as a structured error (e.g. the HTTP chunked-upload
/// bridge in `crates/router/src/route_handler/http.rs`, which maps
/// `Declined` to HTTP 403) can now do so; the raw-QUIC-stream caller
/// (`crates/router/src/route_handler/io.rs`) doesn't need the
/// distinction and ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamRequestOutcome {
    /// The guest accepted the request and the stream ran to completion
    /// (or was aborted mid-transfer, in which case this function returns
    /// `Err` instead -- see `run_stream_protocol_request`).
    Completed,
    /// The guest declined the request (`Err` from
    /// `handle-stream-request`/`accept-stream-upload`) or doesn't export
    /// a handler for this protocol at all; the stream was closed cleanly
    /// with no bytes transferred.
    Declined,
}

/// Engine: Passive code module that wraps low-level OS operations
/// to spin up Wasmtime or Podman instances.
pub struct AppSandboxEngine {
    blobs_dir: PathBuf,
    engine: Engine,
    linker: Linker<HostState>,
    // Cache of pre-linked instances for fast instantiation
    components: DashMap<String, (InstancePre<HostState>, Option<WasmResourceQuota>)>,
    /// Resolved-policy cache, keyed by `service_id`, next to `components`.
    /// The value is itself an `Option` so that *"resolved: this service has
    /// no policy"* -- the common case -- is cached too, instead of
    /// re-querying `substrate.db` per invocation (ADR-0017).
    /// `parse_and_validate` compiles the embedded JSON Schema and
    /// re-validates on every call, which would put schema compilation on the
    /// hot path of every guest invocation if this weren't cached. Evicted on
    /// `stop_wasm` and `compile_and_cache_wasm` so a re-deploy re-resolves
    /// rather than serving the previous policy.
    fdae_policies: DashMap<String, Option<Arc<Policy>>>,
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
    fdae_policy_generation: DashMap<String, u64>,
    default_max_instructions: Option<u64>,
    default_max_memory_bytes: Option<u64>,
    _shutdown_tx: Option<oneshot::Sender<()>>,
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
    /// the `ProxyRouter` (M04A Slice A1, `syneroym-router`) are both
    /// constructed. `Weak`, not `Arc`: the proxy holds a
    /// `Weak<AppSandboxEngine>` back (its local-WASM-target dispatch path),
    /// and two strong refs would be an uncollectable cycle (the same class
    /// that hung graceful shutdown in Slice 6B).
    pub service_proxy: OnceLock<Weak<dyn ServiceProxy>>,
    /// Live guest-delivery subscriptions, keyed `(service_id,
    /// namespaced_topic)`. Dropping an entry unsubscribes from the broker
    /// (see `SubscriptionHandle::drop`).
    pub(crate) subscriptions: DashMap<(String, String), SubscriptionHandle>,
    /// `register-stream-protocol` (M3B Slice 6B, ADR-0014) writes into this
    /// same registry the router reads from, giving restart-replay and
    /// undeploy-cleanup for free -- see ADR-0014 "Where Registration Lives".
    endpoint_registry: EndpointRegistry,
    /// Per-service open-stream-instance task tracking; see `StreamRegistry`.
    stream_registry: StreamRegistry,
    max_concurrent_streams_per_service: u32,
    /// Bounds how many M3B Slice 6B stream instances may be open across
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
    stream_instance_permits: Arc<Semaphore>,
    /// Epoch-tick budget for an ordinary dispatch call (RPC/proxy
    /// invocation, message delivery, one streaming chunk) -- see
    /// `AppSandboxRole::dispatch_epoch_timeout_secs`.
    dispatch_epoch_ticks: u64,
    /// Epoch-tick budget for a component's `init()`/`migrate()` lifecycle
    /// hook -- see `AppSandboxRole::lifecycle_hook_epoch_timeout_secs`.
    lifecycle_hook_epoch_ticks: u64,
}

/// Pool slots reserved out of `max_concurrent_instances` for short-lived
/// RPC/message-delivery calls; the remainder is the budget
/// `stream_instance_permits` hands out to long-lived stream instances. See
/// that field's doc comment for the cross-service DoS this prevents.
const STREAM_INSTANCE_POOL_HEADROOM: u32 = 2;

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

impl Debug for AppSandboxEngine {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppSandboxEngine")
            .field("blobs_dir", &self.blobs_dir)
            .field("components_len", &self.components.len())
            .finish()
    }
}

impl AppSandboxEngine {
    /// Helper to validate service ID against path traversal and invalid
    /// characters
    pub fn validate_service_id(service_id: &str) -> Result<()> {
        if service_id.is_empty()
            || service_id.contains('/')
            || service_id.contains('\\')
            || service_id.contains("..")
            || Path::new(service_id).is_absolute()
        {
            return Err(anyhow::anyhow!(
                "Invalid service_id: path traversal or invalid characters"
            ));
        }
        Ok(())
    }

    /// Initializes the App Sandbox and warms up any existing WASM endpoints
    #[allow(clippy::too_many_arguments)]
    pub async fn init(
        config: &SubstrateConfig,
        endpoints: Vec<(String, String, SubstrateEndpoint)>,
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
        blob_provider: Arc<dyn BlobProvider>,
        messaging_broker: Arc<MqttBroker>,
        endpoint_registry: EndpointRegistry,
    ) -> anyhow::Result<Self> {
        let component_dir = config.storage.blobs_dir.join("app_sandbox");

        // Ensure blobs directory exists
        if !component_dir.exists() {
            tokio_fs::create_dir_all(&component_dir).await?;
        }

        // Read these limits from `config` based on the hardware tier
        let (max_instances, max_memory) = if let Some(sandbox_config) = &config.roles.app_sandbox {
            (sandbox_config.max_concurrent_instances, sandbox_config.memory_limit_bytes() as usize)
        } else {
            (10, 128 * 1024 * 1024)
        };

        let engine = Self::build_wasm_engine(Some(max_instances), Some(max_memory))?;
        let linker = Self::build_wasm_linker(&engine)?;

        // Component cache
        let components = DashMap::new();

        let (default_max_instructions, default_max_memory_bytes) =
            if let Some(sandbox_config) = &config.roles.app_sandbox {
                (sandbox_config.default_max_instructions, sandbox_config.default_max_memory_bytes)
            } else {
                (Some(10_000_000_000), Some(256 * 1024 * 1024))
            };

        let (dispatch_timeout_secs, lifecycle_hook_timeout_secs) =
            if let Some(sandbox_config) = &config.roles.app_sandbox {
                (
                    sandbox_config.dispatch_epoch_timeout_secs,
                    sandbox_config.lifecycle_hook_epoch_timeout_secs,
                )
            } else {
                (5, 30)
            };
        let dispatch_epoch_ticks = ticks_for_secs(dispatch_timeout_secs);
        let lifecycle_hook_epoch_ticks = ticks_for_secs(lifecycle_hook_timeout_secs);

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

        let max_concurrent_streams_per_service =
            config.streaming.max_concurrent_streams_per_service;

        let stream_instance_budget =
            max_instances.saturating_sub(STREAM_INSTANCE_POOL_HEADROOM).max(1);
        if max_concurrent_streams_per_service > stream_instance_budget {
            warn!(
                max_concurrent_streams_per_service,
                max_concurrent_instances = max_instances,
                stream_instance_budget,
                "a single service's stream cap alone can consume this engine's entire \
                 cross-service stream-instance budget (max_concurrent_instances minus a \
                 {STREAM_INSTANCE_POOL_HEADROOM}-slot reserve for ordinary calls); consider \
                 raising max_concurrent_instances or lowering max_concurrent_streams_per_service"
            );
        }

        let app_engine = Self {
            blobs_dir: component_dir,
            engine,
            linker,
            components,
            fdae_policies: DashMap::new(),
            fdae_policy_generation: DashMap::new(),
            default_max_instructions,
            default_max_memory_bytes,
            _shutdown_tx: Some(shutdown_tx),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            self_weak: OnceLock::new(),
            service_proxy: OnceLock::new(),
            subscriptions: DashMap::new(),
            endpoint_registry,
            stream_registry: StreamRegistry::new(),
            max_concurrent_streams_per_service,
            stream_instance_permits: Arc::new(Semaphore::new(stream_instance_budget as usize)),
            dispatch_epoch_ticks,
            lifecycle_hook_epoch_ticks,
        };

        for (service_id, _interface_name, endpoint) in endpoints {
            if let SubstrateEndpoint::WasmChannel { service_id: channel_id } = endpoint {
                info!(
                    service_id = %service_id,
                    channel_id = %channel_id,
                    "Warming up WASM component"
                );

                if let Err(e) = app_engine.load_cached_wasm(&service_id).await {
                    error!("Failed to warm up WASM component {}: {}", service_id, e);
                }
            }
        }

        let engine_clone = app_engine.engine.clone();
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_millis(EPOCH_TICK_MS));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        engine_clone.increment_epoch();
                    }
                    _ = &mut shutdown_rx => {
                        break;
                    }
                }
            }
        });

        Ok(app_engine)
    }

    /// Helper to build the Wasmtime Engine
    pub fn build_wasm_engine(
        max_instances: Option<u32>,
        max_memory: Option<usize>,
    ) -> Result<Engine> {
        let mut wasmtime_config = Config::new();
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);

        if let (Some(instances), Some(memory)) = (max_instances, max_memory) {
            wasmtime_config.memory_init_cow(true);
            let mut pooling_config = PoolingAllocationConfig::default();
            pooling_config.total_component_instances(instances);
            pooling_config.max_memory_size(memory);
            wasmtime_config
                .allocation_strategy(InstanceAllocationStrategy::Pooling(pooling_config));
        }

        Engine::new(&wasmtime_config).map_err(Into::into)
    }

    /// Helper to build the Wasmtime Linker
    pub fn build_wasm_linker(engine: &Engine) -> Result<Linker<HostState>> {
        let mut linker = Linker::new(engine);
        p2::add_to_linker_async(&mut linker)?;
        context::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        vault::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        store::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        app_config::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        blob_store::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        host_api::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        proxy::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        Ok(linker)
    }

    /// Helper to fetch WASM bytes from a source
    async fn fetch_wasm_bytes(source: &ArtifactSource) -> Result<Vec<u8>> {
        match source {
            ArtifactSource::Url(url) => {
                info!("Fetching WASM from URL: {}", url);
                Ok(reqwest::get(url)
                    .await
                    .context("Failed to fetch WASM from URL")?
                    .bytes()
                    .await
                    .context("Failed to read WASM bytes")?
                    .to_vec())
            }
            ArtifactSource::Binary(b) => Ok(b.clone()),
        }
    }

    /// Helper to verify the hash of WASM bytes
    fn verify_wasm_hash(bytes: &[u8], expected_hash: Option<&str>) -> Result<()> {
        if let Some(expected_hash) = expected_hash {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            let computed_hash = hex::encode(hasher.finalize());

            let expected_hash_clean =
                expected_hash.strip_prefix("sha256:").unwrap_or(expected_hash);

            if computed_hash != expected_hash_clean {
                return Err(anyhow::anyhow!(
                    "Hash mismatch: expected {expected_hash_clean}, got {computed_hash}"
                ));
            }
            info!("WASM hash verified successfully");
        }
        Ok(())
    }

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
    fn wasm_result_err(results: &[Val]) -> Option<&str> {
        if let Some(Val::Result(Err(Some(boxed)))) = results.first()
            && let Val::String(msg) = boxed.as_ref()
        {
            Some(msg.as_str())
        } else {
            None
        }
    }

    /// Deploy and compile a WASM component for a given service
    pub async fn deploy_wasm(&self, service_id: &str, manifest: &DeployManifest) -> Result<()> {
        Self::validate_service_id(service_id)?;
        info!("AppSandboxEngine: Deploying Wasm component for {}", service_id);

        let ServiceType::Wasm(wasm_manifest) = &manifest.service_type else {
            return Err(anyhow::anyhow!("Expected Wasm manifest"));
        };

        // 1. Fetch bytes
        let bytes = Self::fetch_wasm_bytes(&wasm_manifest.source).await?;

        // 2. Verify hash
        Self::verify_wasm_hash(&bytes, wasm_manifest.hash.as_deref())?;

        // 3. Store locally in blobs_dir
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        tokio_fs::write(&file_path, &bytes).await.context("Failed to save WASM binary locally")?;

        info!("WASM binary stored at {:?}", file_path);

        let quota = manifest.config.quota.as_ref().map(|q| WasmResourceQuota {
            max_instructions: q.max_instructions,
            max_memory_bytes: q.max_memory_bytes,
        });

        if let Some(ref q) = quota {
            let quota_path = self.blobs_dir.join(format!("{service_id}.quota.json"));
            if let Ok(quota_json) = serde_json::to_string(q) {
                let _ = tokio_fs::write(&quota_path, quota_json).await;
            }
        }

        // 4. Compile and cache the component; drop the raw bytes immediately to free
        //    memory
        self.compile_and_cache_wasm(service_id, &bytes, quota)?;
        drop(bytes);

        // 5. Invoke the guest's schema lifecycle hook: `init()` on a fresh service (no
        //    existing database), `migrate()` on a re-deploy of a service with existing
        //    state. Checked here, before anything else can lazily open the service DB
        //    and thereby create it.
        let is_first_deploy = !self
            .storage_provider
            .service_exists(service_id)
            .await
            .context("failed to check for pre-existing service state")?;
        let hook = if is_first_deploy {
            "init"
        } else {
            // TODO(M5): full snapshot/rollback safety net for migrate() is
            // deferred to M5 [LFC-VER]. migrate() may execute destructive
            // DDL; there is no automatic rollback on partial failure in M3A.
            "migrate"
        };
        self.invoke_lifecycle_hook(service_id, hook)
            .await
            .with_context(|| format!("{hook}() lifecycle hook failed for service {service_id}"))?;

        Ok(())
    }

    /// Execute a WASM component for a given service, returning the guest's
    /// results as the string-shaped boundary contract every existing caller
    /// relies on (see [`crate::conversions::wasm_results_to_json_string`]).
    pub async fn execute_wasm(
        &self,
        service_id: &str,
        interface_name: &str,
        request: &JsonRpcRequest,
    ) -> Result<String> {
        let wasm_results = self.execute_wasm_vals(service_id, interface_name, request).await?;
        wasm_results_to_json_string(&wasm_results)
    }

    /// Typed entry point (M04A Slice A1): the guest's results as a real JSON
    /// [`Value`], with no string special-case. Used by the Universal Proxy
    /// (`ProxyRouter::invoke_local`) and the inbound `JsonRpcToWasm` route.
    pub async fn execute_wasm_json(
        &self,
        service_id: &str,
        interface_name: &str,
        request: &JsonRpcRequest,
    ) -> Result<Value> {
        let wasm_results = self.execute_wasm_vals(service_id, interface_name, request).await?;
        crate::conversions::wasm_results_to_json(&wasm_results)
    }

    /// Everything shared by [`Self::execute_wasm`]/[`Self::execute_wasm_json`]:
    /// resolves and instantiates the target component, binds JSON-RPC params
    /// to its typed signature, calls it, and maps quota/memory traps -- up to
    /// but not including result serialization, which the two typed/string
    /// entry points above handle differently.
    async fn execute_wasm_vals(
        &self,
        service_id: &str,
        interface_name: &str,
        request: &JsonRpcRequest,
    ) -> Result<Vec<Val>> {
        Self::validate_service_id(service_id)?;
        struct ActiveInstanceGuard;
        impl ActiveInstanceGuard {
            fn new() -> Self {
                metrics::gauge!("substrate.wasm.active_instances").increment(1.0);
                Self
            }
        }
        impl Drop for ActiveInstanceGuard {
            fn drop(&mut self) {
                metrics::gauge!("substrate.wasm.active_instances").decrement(1.0);
            }
        }

        let _guard = ActiveInstanceGuard::new();
        debug!("starting to execute wasm");

        // TODO: Later optimize this by caching things like function parameter details
        // on first execution, so we don't have to do the same lookups every time.
        let (mut store, func, results_len, item) =
            self.prepare_wasm_execution(service_id, interface_name, &request.method).await?;

        // Parse parameters based on ComponentFunc signature
        let params_iter = match &item {
            ComponentItem::ComponentFunc(f) => f.params(),
            _ => return Err(anyhow::anyhow!("Expected a function item")),
        };

        debug!("extracted the function and parameter iter");

        // Bind JSON-RPC params to the typed signature (named or positional).
        let wasm_params = json_to_wasm_params(params_iter, &request.params)?;

        debug!("created input types");

        let mut wasm_results = vec![Val::Bool(false); results_len];
        debug!("created result types");

        let exec_start = Instant::now();
        let res = func.call_async(&mut store, &wasm_params, &mut wasm_results).await;
        metrics::histogram!("substrate.wasm.execution_ms")
            .record(exec_start.elapsed().as_secs_f64() * 1000.0);

        debug!("called wasm function, processing results");

        if let Err(e) = res {
            if let Some(Trap::OutOfFuel) = e.downcast_ref::<Trap>() {
                warn!("Wasm execution exceeded fuel limit for service: {}", service_id);
                return Err(anyhow::anyhow!("QuotaExceeded: Wasm execution exceeded fuel limit"));
            }
            let err_str = e.root_cause().to_string();
            if err_str.contains("all fuel consumed") || err_str.contains("out of fuel") {
                warn!("Wasm execution exceeded fuel limit for service: {}", service_id);
                return Err(anyhow::anyhow!("QuotaExceeded: Wasm execution exceeded fuel limit"));
            }
            if err_str.contains("exceeded its memory limits") || err_str.contains("MemoryFault") {
                return Err(anyhow::anyhow!("MemoryFault: Wasm execution exceeded memory limit"));
            }
            return Err(e.into());
        }

        Ok(wasm_results)
    }

    /// Resolves a service's FDAE policy for `build_store_and_instantiate`,
    /// via `fdae_policies` next to the component cache. On a cache miss,
    /// loads from `substrate.db` (durable across a substrate restart --
    /// `load_cached_wasm` recompiles from disk and the next instantiation
    /// re-resolves from here, not from any in-memory deploy result) and
    /// parses once. A parse failure here is fail-closed-**absent**: log and
    /// cache `None` rather than deny every read for the service. The deploy
    /// path (`control_plane`'s `orchestration.rs`) is what rejects a bad
    /// policy before it's ever persisted -- a row that fails to parse *here*
    /// means the DB was tampered with or the crate's schema moved since
    /// deploy, and the alternative (denying every read) would take a
    /// previously-working service down on a substrate upgrade rather than on
    /// the bad edit that actually caused it.
    async fn resolve_fdae_policy(&self, service_id: &str) -> Option<Arc<Policy>> {
        if let Some(cached) = self.fdae_policies.get(service_id) {
            return cached.clone();
        }
        // Captured *before* the cross-await storage read, so a concurrent
        // eviction (redeploy) that races this load can be detected below --
        // see `fdae_policy_generation`'s doc comment. `.get()` immutably
        // borrows a shard just long enough to copy the `u64` out; the shard
        // is not held across the `.await`.
        let generation_before =
            self.fdae_policy_generation.get(service_id).map(|g| *g).unwrap_or(0);
        let resolved = match self.storage_provider.load_fdae_policy(service_id).await {
            Ok(Some(doc)) => match syneroym_fdae::parse_and_validate(&doc) {
                Ok(policy) => Some(Arc::new(policy)),
                Err(e) => {
                    error!(
                        "FDAE policy for service {} failed to parse from storage (treating as \
                         policy-absent): {}",
                        service_id, e
                    );
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                error!("Failed to load FDAE policy for service {}: {}", service_id, e);
                None
            }
        };
        let generation_after = self.fdae_policy_generation.get(service_id).map(|g| *g).unwrap_or(0);
        if generation_before == generation_after {
            self.fdae_policies.insert(service_id.to_string(), resolved.clone());
        } else {
            // An eviction (redeploy) landed while this load was in flight --
            // this result may already be stale. Return it for *this* call
            // (it was the correct answer at some point during the read, and
            // returning it beats blocking or erroring), but do not cache it:
            // the next call re-resolves fresh rather than serving a policy a
            // redeploy already superseded.
            debug!(
                service_id,
                "FDAE policy resolution raced a redeploy; serving uncached this time"
            );
        }
        resolved
    }

    /// Helper shared by `prepare_wasm_execution` and `invoke_lifecycle_hook`:
    /// looks up the pre-linked component, resolves its resource quotas,
    /// builds a fresh `HostState`/`Store`, and instantiates it.
    async fn build_store_and_instantiate(
        &self,
        service_id: &str,
        caller: CallerContext,
        epoch_deadline_ticks: u64,
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
            .unwrap_or_else(crate::host_capabilities::empty_service_proxy);
        let fdae_policy = self.resolve_fdae_policy(service_id).await;
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
        );

        debug!("created wasi ctx and host state");

        // Create a new store
        let mut store = Store::new(&self.engine, host_state);

        store.limiter(|state| state);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(epoch_deadline_ticks);

        if let Some(instructions) = max_instructions {
            store.set_fuel(instructions)?;
        }

        let inst_start = Instant::now();
        let instance = instance_pre.instantiate_async(&mut store).await?;
        metrics::histogram!("substrate.wasm.instantiation_ms")
            .record(inst_start.elapsed().as_secs_f64() * 1000.0);

        debug!("instantiated store and instance");

        Ok((store, instance, max_instructions))
    }

    /// Helper to prepare WASM execution context and extract function
    async fn prepare_wasm_execution(
        &self,
        service_id: &str,
        interface_name: &str,
        method_name: &str,
    ) -> Result<(Store<HostState>, Func, usize, ComponentItem)> {
        // This is the ordinary dispatch path -- reached from wire-originated
        // JSON-RPC (`dispatch.rs`) and guest-to-guest proxy calls, both of
        // which let the caller pick `method_name` freely. It must never
        // grant `local_elevated` (the `data-layer/admin`-bearing, FDAE-exempt
        // context): a caller simply naming their request "init" or "migrate"
        // would otherwise self-elevate. `local_elevated` is reserved for
        // `invoke_lifecycle_hook`, which the deploy path calls directly
        // (never through this function) and builds its own caller/epoch
        // budget without consulting `method_name` at all.
        let caller = CallerContext::service_system(service_id);
        let (mut store, instance, _max_instructions) =
            self.build_store_and_instantiate(service_id, caller, self.dispatch_epoch_ticks).await?;

        // Use the helper to extract the function
        let (func, results_len, item) =
            Self::get_wasm_func(&mut store, &instance, Some(interface_name), method_name)?;

        debug!("extracted the interface and method export indices");

        Ok((store, func, results_len, item))
    }

    /// Invokes a guest lifecycle export (`init` or `migrate`) declared
    /// directly on the `data-layer-guest` world, if the deployed component
    /// exports it. Components that don't declare the export (e.g. a plain
    /// component with no data-layer usage, like the `greeter` test
    /// component) are left untouched -- this makes it safe to call
    /// unconditionally on every deploy.
    async fn invoke_lifecycle_hook(&self, service_id: &str, hook: &str) -> Result<()> {
        let (mut store, instance, _max_instructions) = self
            .build_store_and_instantiate(
                service_id,
                CallerContext::local_elevated(service_id),
                self.lifecycle_hook_epoch_ticks,
            )
            .await?;

        if instance.get_export(&mut store, None, hook).is_none() {
            debug!(service_id, hook, "component does not export lifecycle hook, skipping");
            return Ok(());
        }

        let (func, results_len, _item) = Self::get_wasm_func(&mut store, &instance, None, hook)?;
        let mut results = vec![Val::Bool(false); results_len];
        func.call_async(&mut store, &[], &mut results).await?;

        if let Some(msg) = Self::wasm_result_err(&results) {
            return Err(anyhow::anyhow!("{hook}() failed: {msg}"));
        }
        Ok(())
    }

    /// Core in-memory subscribe logic shared by a live guest `subscribe()`
    /// call and substrate-startup replay (the latter has no `HostState` to
    /// call through, since it runs before any request is served). Spawns a
    /// forwarding task that calls `deliver_message` per broker message and
    /// exits when the broker's receiver closes (including when this
    /// engine itself is dropped, via `MqttBroker`'s `CancellationToken`).
    pub async fn register_internal_subscription(
        &self,
        service_id: &str,
        namespaced_topic: &str,
    ) -> Result<()> {
        let key = (service_id.to_string(), namespaced_topic.to_string());
        if self.subscriptions.contains_key(&key) {
            // Already live (e.g. a guest retrying `subscribe` after a
            // transient error it couldn't distinguish from "already
            // subscribed") -- opening a second broker link here would
            // double-deliver every message on this topic until the first
            // link's handle is eventually dropped.
            return Ok(());
        }

        let (handle, mut receiver) = self
            .messaging_broker
            .subscribe(key.1.clone())
            .await
            .map_err(|e| anyhow::anyhow!("broker subscribe failed: {e}"))?;

        let engine_weak = self.self_weak.get().cloned().unwrap_or_default();
        let service_id_owned = service_id.to_string();
        tokio::spawn(async move {
            while let Some((topic, payload)) = receiver.recv().await {
                let Some(engine) = engine_weak.upgrade() else { break };
                engine.deliver_message(&service_id_owned, &topic, payload).await;
            }
        });

        self.subscriptions.insert(key, handle);
        Ok(())
    }

    /// Drops every live guest-delivery subscription for `service_id`
    /// (called from `ControlPlaneService::undeploy`'s cleanup).
    pub fn unsubscribe_all(&self, service_id: &str) {
        self.subscriptions.retain(|(sid, _topic), _handle| sid != service_id);
    }

    /// Aborts every open M3B Slice 6B stream task for `service_id` (called
    /// from `stop_wasm` and `ControlPlaneService::undeploy`, mirroring
    /// `unsubscribe_all`). `StreamRegistry`'s own `Drop` is the backstop for
    /// every other teardown path (ADR-0014).
    pub fn abort_streams(&self, service_id: &str) {
        self.stream_registry.abort_all(service_id);
    }

    /// Invokes the deployed component's exported `guest-api::handle-message`
    /// with a freshly-instantiated `Store` (same reasoning as any other
    /// invocation -- see `build_store_and_instantiate`), if it declares
    /// that export. If not, the message is silently discarded (per
    /// ADR-0010): this makes it safe to call for every subscription
    /// regardless of whether the target component implements messaging.
    async fn deliver_message(&self, service_id: &str, topic: &str, payload: Vec<u8>) {
        // `service_system`, never `local_elevated`: this is the inbound
        // broker-delivery hot path -- an accidentally elevated caller here
        // would let every delivered message pass the `execute-ddl` Admin
        // gate. The component receiving a message acts as itself.
        let (mut store, instance, _max_instructions) = match self
            .build_store_and_instantiate(
                service_id,
                CallerContext::service_system(service_id),
                self.dispatch_epoch_ticks,
            )
            .await
        {
            Ok(triple) => triple,
            Err(e) => {
                debug!(
                    service_id,
                    error = %e,
                    "messaging: failed to instantiate component for delivery"
                );
                return;
            }
        };

        const GUEST_API_INTERFACE: &str = "syneroym:messaging/guest-api@0.1.0";
        let (func, results_len, _item) = match Self::get_wasm_func(
            &mut store,
            &instance,
            Some(GUEST_API_INTERFACE),
            "handle-message",
        ) {
            Ok(found) => found,
            Err(_) => {
                debug!(
                    service_id,
                    "messaging: component does not export guest-api::handle-message, discarding"
                );
                return;
            }
        };

        let args =
            [Val::String(topic.to_string()), Val::List(payload.into_iter().map(Val::U8).collect())];
        let mut results = vec![Val::Bool(false); results_len];
        if let Err(e) = func.call_async(&mut store, &args, &mut results).await {
            warn!(service_id, error = %e, "messaging: handle-message invocation trapped");
            return;
        }

        if let Some(msg) = Self::wasm_result_err(&results) {
            warn!(service_id, error = %msg, "messaging: handle-message returned an error");
        }
    }

    /// Simple test function to invoke test context. `run` (`wit/host/host.wit`
    /// `app::run`) is zero-arg, so `request_ctx` is not threaded through as a
    /// JSON-RPC param (it never was: the pre-A0′ converter also dropped it,
    /// silently, for any zero-arg target).
    pub async fn invoke_test_context(
        &self,
        service_id: &str,
        component_id: &str,
        _request_ctx: &str,
    ) -> Result<String> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "run".to_string(), // Default method for test
            params: Value::Null,
            id: None,
        };
        self.execute_wasm(service_id, component_id, &request).await
    }

    /// Bumps `fdae_policy_generation` for `service_id`, marking any
    /// `resolve_fdae_policy` load currently in flight for it as stale --
    /// called alongside every `fdae_policies` eviction. See
    /// `fdae_policy_generation`'s doc comment.
    fn bump_fdae_policy_generation(&self, service_id: &str) {
        *self.fdae_policy_generation.entry(service_id.to_string()).or_insert(0) += 1;
    }

    /// Stop and evict a running Wasm component from the in-memory cache.
    pub async fn stop_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        info!(service_id = %service_id, "AppSandboxEngine: stopping Wasm component");
        self.components.remove(service_id);
        self.fdae_policies.remove(service_id);
        self.bump_fdae_policy_generation(service_id);
        self.abort_streams(service_id);
        metrics::gauge!("substrate.wasm.component_cache_size").set(self.components.len() as f64);
        Ok(())
    }

    /// Remove a stopped Wasm component's binary from disk.
    pub async fn remove_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        info!(service_id = %service_id, "AppSandboxEngine: removing Wasm component");
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        if file_path.exists() {
            tokio_fs::remove_file(&file_path)
                .await
                .with_context(|| format!("Failed to remove WASM file {file_path:?}"))?;
        }
        let quota_path = self.blobs_dir.join(format!("{service_id}.quota.json"));
        if quota_path.exists() {
            let _ = tokio_fs::remove_file(&quota_path).await;
        }
        Ok(())
    }

    /// Helper to load a cached WASM component from disk and compile it
    async fn load_cached_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        if file_path.exists() {
            let bytes = tokio_fs::read(&file_path)
                .await
                .context(format!("Failed to read WASM file {file_path:?}"))?;
            let quota_path = self.blobs_dir.join(format!("{service_id}.quota.json"));
            let quota = if quota_path.exists() {
                if let Ok(quota_json) = tokio_fs::read_to_string(&quota_path).await {
                    serde_json::from_str::<WasmResourceQuota>(&quota_json).ok()
                } else {
                    None
                }
            } else {
                None
            };
            self.compile_and_cache_wasm(service_id, &bytes, quota)?;
        } else {
            warn!("WASM file not found on disk for service: {:?}", file_path);
        }
        Ok(())
    }

    /// Helper to compile a WASM binary and store it in the cache
    pub fn compile_and_cache_wasm(
        &self,
        service_id: &str,
        bytes: &[u8],
        quota: Option<WasmResourceQuota>,
    ) -> Result<()> {
        let component = Component::new(&self.engine, bytes)
            .map_err(|e| anyhow::anyhow!("Failed to compile WASM component: {e}"))?;

        let instance_pre = self
            .linker
            .instantiate_pre(&component)
            .map_err(|e| anyhow::anyhow!("Failed to pre-link WASM component: {e}"))?;

        self.components.insert(service_id.to_string(), (instance_pre, quota));
        // A re-deploy compiles and re-caches the component here; evict any
        // previously resolved policy so the next instantiation re-resolves
        // from `substrate.db` rather than serving a stale one.
        self.fdae_policies.remove(service_id);
        self.bump_fdae_policy_generation(service_id);
        info!("WASM component compiled and cached for {}", service_id);
        metrics::gauge!("substrate.wasm.component_cache_size").set(self.components.len() as f64);
        Ok(())
    }

    /// Spin up a new Podman instance
    pub async fn deploy_podman(&self, _service_id: &str, _manifest: &[u8]) -> Result<()> {
        info!("AppSandboxEngine: Deploying Podman container for {}", _service_id);
        Ok(())
    }

    /// Opens a fresh, long-lived `Store`/`Instance` for one M3B Slice 6B
    /// stream's lifetime (ADR-0014 "Instance Lifetime and Quota") --
    /// distinct from `build_store_and_instantiate`'s per-*call* instances,
    /// which don't outlive a single invocation. Also returns the resolved
    /// fuel budget, re-applied before every chunk call by
    /// `GuestStreamCursor`/`GuestStreamSink`.
    async fn open_stream_instance(
        &self,
        service_id: &str,
    ) -> Result<(Store<HostState>, Instance, Option<u64>)> {
        // `service_system`, never `local_elevated` -- same reasoning as
        // `deliver_message`: the component acts as itself, not as an admin.
        self.build_store_and_instantiate(
            service_id,
            CallerContext::service_system(service_id),
            self.dispatch_epoch_ticks,
        )
        .await
    }

    /// Entry point for a peer-initiated `raw://<protocol>|<service_id>`
    /// stream (`crates/router/src/route_handler/io.rs`'s
    /// `handle_raw_stream`, per ADR-0014). Spawns one dedicated Tokio task
    /// per stream (owning the long-lived `Store`/`Instance`) *before*
    /// reserving its slot in `StreamRegistry`, since the `AbortHandle` only
    /// exists once the task has been spawned; the reservation itself is a
    /// single atomic check-and-register (see `StreamRegistry::try_reserve`),
    /// so concurrent requests can't all observe spare capacity and all get
    /// admitted. If the reservation is refused, the just-spawned task is
    /// aborted immediately (it can't have made meaningful progress yet) and
    /// the caller sees a clean over-capacity error instead of the stream
    /// briefly starting anyway.
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_stream_protocol_request(
        &self,
        service_id: &str,
        protocol: &str,
        peer_id: &str,
        direction: StreamDirection,
        initial_payload: Vec<u8>,
        reader: Box<dyn AsyncRead + Unpin + Send>,
        writer: Box<dyn AsyncWrite + Unpin + Send>,
    ) -> Result<StreamRequestOutcome> {
        let engine = self
            .self_weak
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| anyhow!("sandbox engine unavailable for stream handling"))?;

        let service_id_owned = service_id.to_string();
        let protocol_owned = protocol.to_string();
        let peer_id_owned = peer_id.to_string();
        let tracked_service_id = service_id.to_string();

        let join_handle = tokio::spawn(async move {
            engine
                .run_stream_protocol_request(
                    &service_id_owned,
                    &protocol_owned,
                    &peer_id_owned,
                    direction,
                    initial_payload,
                    reader,
                    writer,
                )
                .await
        });
        let abort_handle = join_handle.abort_handle();
        if let Err(e) = self.stream_registry.try_reserve(
            &tracked_service_id,
            self.max_concurrent_streams_per_service,
            abort_handle.clone(),
        ) {
            abort_handle.abort();
            return Err(e);
        }

        let result = join_handle.await;
        self.stream_registry.untrack(&tracked_service_id, &abort_handle);

        match result {
            Ok(inner) => inner,
            // Aborted by `stop_wasm`/`undeploy` -- not a real failure from
            // the stream's own perspective, the router already closed (or
            // is closing) the underlying QUIC stream in that case.
            Err(join_err) if join_err.is_cancelled() => Ok(StreamRequestOutcome::Completed),
            Err(join_err) => Err(anyhow!("stream task failed: {join_err}")),
        }
    }

    /// The actual per-stream work, run on its own dedicated Tokio task (see
    /// `handle_stream_protocol_request`): resolves the guest's
    /// `handle-stream-request`/`accept-stream-upload` export for `protocol`
    /// and, if it accepts, drives the pull/push loop until the stream ends.
    /// A guest that declines (`Err`) or doesn't export the relevant
    /// function closes the stream cleanly (`Ok(())`) rather than erroring --
    /// this is also the safety net for the `EndpointRegistry`-reuse caveat
    /// in ADR-0014 (a `raw://` request against a non-stream interface name
    /// simply finds no matching export).
    ///
    /// Acquires a `stream_instance_permits` permit *before* opening the
    /// stream's pooled component instance, and holds it for this function's
    /// whole lifetime (dropped on every exit path, including the early
    /// `return`s below) -- see that field's doc comment for why this
    /// engine-wide budget exists alongside the per-service
    /// `StreamRegistry` cap.
    #[allow(clippy::too_many_arguments)]
    async fn run_stream_protocol_request(
        &self,
        service_id: &str,
        protocol: &str,
        peer_id: &str,
        direction: StreamDirection,
        initial_payload: Vec<u8>,
        reader: Box<dyn AsyncRead + Unpin + Send>,
        writer: Box<dyn AsyncWrite + Unpin + Send>,
    ) -> Result<StreamRequestOutcome> {
        let _stream_instance_permit = self
            .stream_instance_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| anyhow!("stream instance semaphore closed: {e}"))?;

        let (mut store, instance, max_instructions) = self.open_stream_instance(service_id).await?;
        let mut writer = writer;

        let result = match direction {
            StreamDirection::Download => {
                let resource = match stream::call_handle_stream_request(
                    &mut store,
                    &instance,
                    protocol,
                    peer_id,
                    initial_payload,
                )
                .await
                {
                    Ok(resource) => resource,
                    Err(e) => {
                        debug!(
                            service_id,
                            protocol,
                            error = %e,
                            "stream: guest declined handle-stream-request (or does not export it)"
                        );
                        let _ = writer.shutdown().await;
                        return Ok(StreamRequestOutcome::Declined);
                    }
                };
                let cursor = GuestStreamCursor::new(
                    store,
                    instance,
                    resource,
                    max_instructions,
                    self.dispatch_epoch_ticks,
                );
                chunk_transfer::pull_until_eof(cursor, &mut writer).await
            }
            StreamDirection::Upload => {
                let resource = match stream::call_accept_stream_upload(
                    &mut store,
                    &instance,
                    protocol,
                    peer_id,
                    initial_payload,
                )
                .await
                {
                    Ok(resource) => resource,
                    Err(e) => {
                        debug!(
                            service_id,
                            protocol,
                            error = %e,
                            "stream: guest declined accept-stream-upload (or does not export it)"
                        );
                        let _ = writer.shutdown().await;
                        return Ok(StreamRequestOutcome::Declined);
                    }
                };
                let sink: Box<dyn ChunkSink> = Box::new(GuestStreamSink::new(
                    store,
                    instance,
                    resource,
                    max_instructions,
                    self.dispatch_epoch_ticks,
                ));
                chunk_transfer::push_until_eof(reader, sink).await
            }
        };

        // Neither `pull_until_eof` nor `push_until_eof` shuts `writer` down
        // (the latter doesn't touch it at all); without an explicit clean
        // close here, a peer reading this stream's other QUIC direction to
        // EOF has nothing to observe and hangs rather than completing.
        let _ = writer.shutdown().await;
        result.map(|()| StreamRequestOutcome::Completed)
    }
}

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use syneroym_core::{storage::MockStorage, test_constants};
    use syneroym_data_db::{ServiceStore, SqliteStorageProvider};
    use syneroym_mqtt_broker::MqttBrokerConfig;
    use syneroym_rpc::AuthLevel;
    use tokio::{sync::Notify, task};
    use wasmtime::component::Component;

    use super::*;
    use crate::host_capabilities::tests::{
        test_blob_provider, test_messaging_context, test_service_proxy, test_streaming_context,
    };

    /// Wraps a real `StorageProvider`, pausing `load_fdae_policy` on
    /// `release` before delegating -- lets a test deterministically land a
    /// `bump_fdae_policy_generation` call inside `resolve_fdae_policy`'s
    /// cross-await race window, rather than relying on incidental thread
    /// scheduling (which would make the test flaky in either direction).
    /// Every other method delegates straight through; `resolve_fdae_policy`
    /// only ever calls `load_fdae_policy`.
    struct RacingStorageProvider {
        inner: Arc<dyn StorageProvider>,
        release: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl StorageProvider for RacingStorageProvider {
        async fn open_service_db(
            &self,
            service_id: &str,
            key_store: &Arc<KeyStore>,
        ) -> anyhow::Result<Box<dyn ServiceStore>> {
            self.inner.open_service_db(service_id, key_store).await
        }
        async fn rotate_kek(
            &self,
            key_store: &Arc<KeyStore>,
            new_kek: [u8; 32],
        ) -> anyhow::Result<()> {
            self.inner.rotate_kek(key_store, new_kek).await
        }
        async fn load_service_dek(
            &self,
            service_id: &str,
            key_store: &Arc<KeyStore>,
        ) -> anyhow::Result<Option<zeroize::Zeroizing<[u8; 32]>>> {
            self.inner.load_service_dek(service_id, key_store).await
        }
        async fn service_exists(&self, service_id: &str) -> anyhow::Result<bool> {
            self.inner.service_exists(service_id).await
        }
        async fn save_config_generation(
            &self,
            service_id: &str,
            config_blob: &str,
        ) -> anyhow::Result<u64> {
            self.inner.save_config_generation(service_id, config_blob).await
        }
        async fn delete_config_generation(
            &self,
            service_id: &str,
            generation: u64,
        ) -> anyhow::Result<()> {
            self.inner.delete_config_generation(service_id, generation).await
        }
        async fn get_config_generation(
            &self,
            service_id: &str,
            generation: u64,
        ) -> anyhow::Result<Option<String>> {
            self.inner.get_config_generation(service_id, generation).await
        }
        async fn get_latest_config_generation(
            &self,
            service_id: &str,
        ) -> anyhow::Result<Option<(u64, String)>> {
            self.inner.get_latest_config_generation(service_id).await
        }
        async fn save_messaging_subscription(
            &self,
            service_id: &str,
            topic: &str,
        ) -> anyhow::Result<()> {
            self.inner.save_messaging_subscription(service_id, topic).await
        }
        async fn delete_messaging_subscription(
            &self,
            service_id: &str,
            topic: &str,
        ) -> anyhow::Result<()> {
            self.inner.delete_messaging_subscription(service_id, topic).await
        }
        async fn delete_all_messaging_subscriptions_for_service(
            &self,
            service_id: &str,
        ) -> anyhow::Result<()> {
            self.inner.delete_all_messaging_subscriptions_for_service(service_id).await
        }
        async fn list_all_messaging_subscriptions(&self) -> anyhow::Result<Vec<(String, String)>> {
            self.inner.list_all_messaging_subscriptions().await
        }
        async fn save_fdae_policy(
            &self,
            service_id: &str,
            policy_json: &str,
        ) -> anyhow::Result<()> {
            self.inner.save_fdae_policy(service_id, policy_json).await
        }
        async fn load_fdae_policy(&self, service_id: &str) -> anyhow::Result<Option<String>> {
            self.release.notified().await;
            self.inner.load_fdae_policy(service_id).await
        }
        async fn delete_fdae_policy(&self, service_id: &str) -> anyhow::Result<()> {
            self.inner.delete_fdae_policy(service_id).await
        }
    }

    /// Reproduces the lost-invalidation race directly: a `resolve_fdae_policy`
    /// load is paused (via `RacingStorageProvider`) after it has already
    /// captured `generation_before`, a concurrent eviction (simulating a
    /// redeploy) fires while it's still in flight, and only then is the load
    /// allowed to complete. The in-flight call must still return the correct
    /// (if now possibly stale) answer, but must **not** repopulate the cache
    /// -- otherwise the eviction it raced would be silently undone, and the
    /// stale policy would be served indefinitely until the next
    /// `stop_wasm`/redeploy.
    #[tokio::test]
    async fn fdae_policy_resolution_racing_an_eviction_is_not_cached() {
        let temp_dir = tempfile::tempdir().unwrap();
        let real_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        real_provider
            .save_fdae_policy("svc-race", r#"{"version": "fdae/v1", "definitions": {}}"#)
            .await
            .unwrap();

        let release = Arc::new(Notify::new());
        let racing_provider: Arc<dyn StorageProvider> =
            Arc::new(RacingStorageProvider { inner: real_provider, release: release.clone() });
        let app_engine = Arc::new(test_app_engine(racing_provider));

        let resolver = {
            let app_engine = app_engine.clone();
            tokio::spawn(async move { app_engine.resolve_fdae_policy("svc-race").await })
        };

        // Let the spawned task run up through its `generation_before`
        // snapshot and into `load_fdae_policy`'s `release.notified().await`
        // suspension point, before the eviction below fires.
        task::yield_now().await;
        task::yield_now().await;

        // The eviction half of a concurrent redeploy, landing while the load
        // above is still paused.
        app_engine.fdae_policies.remove("svc-race");
        app_engine.bump_fdae_policy_generation("svc-race");

        release.notify_one();
        let resolved = resolver.await.unwrap();

        assert!(resolved.is_some(), "the in-flight call must still return the correct answer");
        assert!(
            app_engine.fdae_policies.get("svc-race").is_none(),
            "a load that raced a concurrent eviction must not repopulate the cache -- doing so \
             would silently undo the eviction and serve a possibly-stale policy indefinitely"
        );
    }

    /// `prepare_wasm_execution` is the ordinary dispatch path reached from
    /// wire-originated JSON-RPC (`dispatch.rs`) and guest-to-guest proxy
    /// calls, both of which let the caller pick `method_name` freely.
    /// Naming a request "init" or "migrate" must not synthesize
    /// `CallerContext::local_elevated` -- the `data-layer/admin`-bearing
    /// context `HostState::query_auth` exempts from the FDAE sieve entirely
    /// -- or any caller could self-elevate by choosing that method name.
    /// Only `invoke_lifecycle_hook` (called directly by the deploy path,
    /// never through this function) may synthesize that context.
    #[tokio::test]
    async fn prepare_wasm_execution_grants_no_elevation_for_init_or_migrate_method_names() {
        let wat = r#"
(component
  (core module $m
    (func (export "noop"))
  )
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface
    (export "init" (func $noop))
    (export "migrate" (func $noop))
  )
  (export "test-interface" (instance $interface))
)
"#;
        let storage_provider: Arc<dyn StorageProvider> = Arc::new(
            SqliteStorageProvider::new(tempfile::tempdir().unwrap().path(), false).unwrap(),
        );
        let app_engine = test_app_engine(storage_provider);
        app_engine.compile_and_cache_wasm("svc-n1", wat.as_bytes(), None).unwrap();

        for method in ["init", "migrate"] {
            let (store, _func, _results_len, _item) = app_engine
                .prepare_wasm_execution("svc-n1", "test-interface", method)
                .await
                .unwrap();
            assert_eq!(
                store.data().caller.auth,
                AuthLevel::System,
                "a wire-dispatched call naming its method {method:?} must not be granted \
                 LocalElevated -- only invoke_lifecycle_hook may synthesize that context"
            );
            assert!(
                !store.data().caller.caller_did.contains("local-elevated"),
                "caller_did leaked a local-elevated identity for method {method:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_list_interfaces() {
        let engine = AppSandboxEngine::build_wasm_engine(None, None).unwrap();
        let linker = AppSandboxEngine::build_wasm_linker(&engine).unwrap();

        let key_store = Arc::new(KeyStore::new());
        let storage_provider = Arc::new(
            SqliteStorageProvider::new(tempfile::tempdir().unwrap().path(), false).unwrap(),
        );
        let host_state = HostState::new(
            "test_component".to_string(),
            None,
            key_store,
            storage_provider,
            test_blob_provider(),
            CallerContext::service_system("test_component"),
            0,
            test_messaging_context(),
            test_streaming_context(),
            test_service_proxy(),
            None,
        );

        let mut store = Store::new(&engine, host_state);

        let component_path = test_constants::greeter_wasm_path();
        let wasm_bytes = if let Ok(bytes) = fs::read(&component_path) {
            bytes
        } else {
            println!(
                "Skipping test_list_interfaces: WASM artifact not found at {}",
                component_path.display()
            );
            return;
        };

        let component: Component =
            Component::new(&engine, &wasm_bytes).expect("Failed to compile WASM component");
        for interface in component.component_type().exports(&engine) {
            println!("Listing interface: {interface:?}");
        }

        match linker.instantiate_async(&mut store, &component).await {
            Ok(instance) => {
                let interface_name = test_constants::GREETER_INTERFACE_NAME;
                let method_name = "greet";

                // Use the helper function to extract function and result size
                match AppSandboxEngine::get_wasm_func(
                    &mut store,
                    &instance,
                    Some(interface_name),
                    method_name,
                ) {
                    Ok((func, results_len, _item)) => {
                        println!("Function export: {func:?}");
                        let mut wasm_results = vec![Val::Bool(false); results_len];

                        let result = func
                            .call_async(
                                &mut store,
                                &[Val::String("TestUser".to_string())],
                                &mut wasm_results,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to call function: {e}"));
                        println!("Function call result: {result:?} is {wasm_results:?}");
                    }
                    Err(e) => {
                        println!("Failed to get wasm func: {e}");
                    }
                }
            }
            Err(err) => {
                println!("Error instantiating component: {err}");
            }
        }
    }

    #[tokio::test]
    async fn test_wasm_quotas() {
        let wat = r#"
(component
  (core module $m
    (func (export "loop_forever")
      (loop $l
        br $l
      )
    )
    (func (export "allocate_too_much") (param $pages i32) (result i32)
      (memory.grow (local.get $pages))
    )
    (memory (export "memory") 1)
  )
  (core instance $i (instantiate $m))
  (func $loop_forever (canon lift (core func $i "loop_forever")))
  (func $allocate_too_much (param "pages" u32) (result s32) (canon lift (core func $i "allocate_too_much")))
  (instance $interface
    (export "loop-forever" (func $loop_forever))
    (export "allocate-too-much" (func $allocate_too_much))
  )
  (export "test-interface" (instance $interface))
)
"#;
        let engine =
            AppSandboxEngine::build_wasm_engine(Some(10), Some(128 * 1024 * 1024)).unwrap();
        let linker = AppSandboxEngine::build_wasm_linker(&engine).unwrap();

        let app_engine = AppSandboxEngine {
            blobs_dir: env::temp_dir(),
            engine,
            linker,
            components: DashMap::new(),
            fdae_policies: DashMap::new(),
            fdae_policy_generation: DashMap::new(),
            default_max_instructions: Some(10_000),
            default_max_memory_bytes: Some(1024 * 1024), // 1MB
            _shutdown_tx: None,
            key_store: Arc::new(KeyStore::new()),
            storage_provider: Arc::new(SqliteStorageProvider::new(env::temp_dir(), false).unwrap()),
            blob_provider: test_blob_provider(),
            messaging_broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
            self_weak: OnceLock::new(),
            service_proxy: OnceLock::new(),
            subscriptions: DashMap::new(),
            endpoint_registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            stream_registry: StreamRegistry::new(),
            max_concurrent_streams_per_service: 8,
            stream_instance_permits: Arc::new(Semaphore::new(8)),
            dispatch_epoch_ticks: ticks_for_secs(5),
            lifecycle_hook_epoch_ticks: ticks_for_secs(30),
        };

        // Cache the test component
        app_engine.compile_and_cache_wasm("test_service", wat.as_bytes(), None).unwrap();

        // 1. Test infinite loop (fuel limit)
        let request_loop = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "loop-forever".to_string(),
            params: Value::Array(vec![]),
            id: None,
        };
        let res_loop =
            app_engine.execute_wasm("test_service", "test-interface", &request_loop).await;
        assert!(res_loop.is_err());
        let err_msg = res_loop.unwrap_err().to_string();
        assert!(err_msg.contains("QuotaExceeded"), "expected QuotaExceeded, got: {}", err_msg);

        // 2. Test memory allocation limit
        // 1 page is 64KB. We try to allocate 100 pages (6.4MB), which exceeds the 1MB
        // limit.
        let request_mem = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "allocate-too-much".to_string(),
            params: Value::Array(vec![Value::Number(serde_json::Number::from(100))]),
            id: None,
        };
        let res_mem = app_engine.execute_wasm("test_service", "test-interface", &request_mem).await;
        assert!(res_mem.is_err());
        let err_msg = res_mem.unwrap_err().to_string();
        assert!(
            err_msg.contains("MemoryFault") || err_msg.contains("failed to grow memory"),
            "expected MemoryFault or failed to grow memory, got: {}",
            err_msg
        );
    }

    fn test_app_engine(storage_provider: Arc<dyn StorageProvider>) -> AppSandboxEngine {
        let engine = AppSandboxEngine::build_wasm_engine(None, None).unwrap();
        let linker = AppSandboxEngine::build_wasm_linker(&engine).unwrap();
        AppSandboxEngine {
            blobs_dir: env::temp_dir(),
            engine,
            linker,
            components: DashMap::new(),
            fdae_policies: DashMap::new(),
            fdae_policy_generation: DashMap::new(),
            default_max_instructions: Some(10_000),
            default_max_memory_bytes: Some(1024 * 1024),
            _shutdown_tx: None,
            key_store: Arc::new(KeyStore::new()),
            storage_provider,
            blob_provider: test_blob_provider(),
            messaging_broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
            self_weak: OnceLock::new(),
            service_proxy: OnceLock::new(),
            subscriptions: DashMap::new(),
            endpoint_registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            stream_registry: StreamRegistry::new(),
            max_concurrent_streams_per_service: 8,
            stream_instance_permits: Arc::new(Semaphore::new(8)),
            dispatch_epoch_ticks: ticks_for_secs(5),
            lifecycle_hook_epoch_ticks: ticks_for_secs(30),
        }
    }

    /// A policy-absent service resolves `None` and caches it -- the common
    /// case -- without re-querying `substrate.db` on a subsequent call.
    #[tokio::test]
    async fn fdae_policy_absent_resolves_none_and_caches() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let app_engine = test_app_engine(storage_provider);

        assert!(app_engine.fdae_policies.get("svc-none").is_none(), "nothing resolved yet");
        assert!(app_engine.resolve_fdae_policy("svc-none").await.is_none());
        assert!(
            app_engine.fdae_policies.get("svc-none").is_some(),
            "the absence itself must be cached, not just a miss"
        );
        assert!(app_engine.fdae_policies.get("svc-none").unwrap().is_none());
    }

    /// A persisted policy resolves to `Some`, is cached, and a cache hit does
    /// not re-query storage (proven by mutating storage to an unparseable
    /// document after the first resolution and confirming the second call
    /// still returns the original, cached policy).
    #[tokio::test]
    async fn fdae_policy_present_resolves_some_and_cache_hit_skips_storage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        storage_provider
            .save_fdae_policy("svc-some", r#"{"version": "fdae/v1", "definitions": {}}"#)
            .await
            .unwrap();
        let app_engine = test_app_engine(storage_provider.clone());

        let policy = app_engine.resolve_fdae_policy("svc-some").await;
        assert!(policy.is_some(), "a valid persisted policy must resolve to Some");
        assert!(app_engine.fdae_policies.get("svc-some").is_some());

        // Corrupt storage after the first resolution; a cache hit must not
        // observe this -- if it did, the second call would return None.
        storage_provider.save_fdae_policy("svc-some", "not valid json").await.unwrap();
        let cached = app_engine.resolve_fdae_policy("svc-some").await;
        assert!(cached.is_some(), "a cache hit must not re-query storage");
    }

    /// `stop_wasm` and `compile_and_cache_wasm` (a re-deploy) both evict the
    /// resolved-policy cache, so the next instantiation re-resolves from
    /// storage rather than serving a stale value.
    #[tokio::test]
    async fn fdae_policy_cache_evicted_on_stop_wasm_and_recompile() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        storage_provider
            .save_fdae_policy("svc-evict", r#"{"version": "fdae/v1", "definitions": {}}"#)
            .await
            .unwrap();
        let app_engine = test_app_engine(storage_provider);

        assert!(app_engine.resolve_fdae_policy("svc-evict").await.is_some());
        assert!(app_engine.fdae_policies.get("svc-evict").is_some());

        app_engine.stop_wasm("svc-evict").await.unwrap();
        assert!(
            app_engine.fdae_policies.get("svc-evict").is_none(),
            "stop_wasm must evict the cached policy"
        );

        assert!(app_engine.resolve_fdae_policy("svc-evict").await.is_some());
        assert!(app_engine.fdae_policies.get("svc-evict").is_some());

        let minimal_component = b"(component)";
        app_engine.compile_and_cache_wasm("svc-evict", minimal_component, None).unwrap();
        assert!(
            app_engine.fdae_policies.get("svc-evict").is_none(),
            "a re-deploy's recompile must evict the cached policy"
        );
    }

    /// A malformed persisted policy is fail-closed-*absent*:
    /// `resolve_fdae_policy` logs and caches `None` rather than propagating
    /// an error that would deny every read for the service (the deploy path
    /// is what rejects a bad policy before it's ever persisted).
    #[tokio::test]
    async fn fdae_policy_unparseable_in_storage_resolves_none_not_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        storage_provider.save_fdae_policy("svc-bad", "not valid json").await.unwrap();
        let app_engine = test_app_engine(storage_provider);

        assert!(app_engine.resolve_fdae_policy("svc-bad").await.is_none());
        assert!(app_engine.fdae_policies.get("svc-bad").unwrap().is_none());
    }
}

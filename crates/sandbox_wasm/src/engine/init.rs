use super::*;

/// Tuning knobs derived from `SubstrateConfig.roles.app_sandbox`, resolved
/// once in `init` and then threaded through the pooling allocator, the
/// per-call epoch budgets and the ABAC after-step. Bundled into a struct
/// (rather than a same-typed tuple) because several fields share a type --
/// `default_max_instructions`/`default_max_memory_bytes` are both
/// `Option<u64>`, and the three `max_*_per_component` fields are all `u32`,
/// so a positional tuple would let two of them swap silently.
struct SandboxTuning {
    max_instances: u32,
    max_memory: usize,
    max_core_instances_per_component: u32,
    max_memories_per_component: u32,
    max_tables_per_component: u32,
    default_max_instructions: Option<u64>,
    default_max_memory_bytes: Option<u64>,
    dispatch_epoch_timeout_secs: u64,
    lifecycle_hook_epoch_timeout_secs: u64,
    abac_epoch_timeout_secs: u64,
    abac_max_instructions: u64,
    max_concurrent_guest_http_per_service: u32,
}

/// Cross-service instance-pool reservations derived from
/// [`SandboxTuning::max_instances`] and the streaming role's own
/// `max_concurrent_streams_per_service`. See `abac_instance_permits`'s and
/// `stream_instance_permits`'s doc comments on [`AppSandboxEngine`] for what
/// each budget guards.
struct InstanceBudgets {
    abac: u32,
    stream: u32,
}

impl AppSandboxEngine {
    /// Resolves the `app_sandbox` role config into concrete tuning values,
    /// falling back to fixed defaults when no role section is configured at
    /// all (embedding without one). A `0` guest-HTTP concurrency value is
    /// clamped to 1 here, with a loud warning, rather than left to silently
    /// 503 every guest HTTP request at admission time.
    fn resolve_sandbox_tuning(config: &SubstrateConfig) -> SandboxTuning {
        let tuning = if let Some(sandbox_config) = &config.roles.app_sandbox {
            SandboxTuning {
                max_instances: sandbox_config.max_concurrent_instances,
                max_memory: sandbox_config.memory_limit_bytes() as usize,
                max_core_instances_per_component: sandbox_config.max_core_instances_per_component,
                max_memories_per_component: sandbox_config.max_memories_per_component,
                max_tables_per_component: sandbox_config.max_tables_per_component,
                default_max_instructions: sandbox_config.default_max_instructions,
                default_max_memory_bytes: sandbox_config.default_max_memory_bytes,
                dispatch_epoch_timeout_secs: sandbox_config.dispatch_epoch_timeout_secs,
                lifecycle_hook_epoch_timeout_secs: sandbox_config.lifecycle_hook_epoch_timeout_secs,
                abac_epoch_timeout_secs: sandbox_config.abac_epoch_timeout_secs,
                abac_max_instructions: sandbox_config.abac_max_instructions,
                max_concurrent_guest_http_per_service: sandbox_config
                    .max_concurrent_guest_http_per_service,
            }
        } else {
            // 100 * 10 == 1000, matching Wasmtime's own pool-wide
            // default -- see `AppSandboxRole`'s
            // `default_max_core_instances_per_component` doc comment.
            SandboxTuning {
                max_instances: 10,
                max_memory: 128 * 1024 * 1024,
                max_core_instances_per_component: 100,
                max_memories_per_component: 100,
                max_tables_per_component: 100,
                default_max_instructions: Some(10_000_000_000),
                default_max_memory_bytes: Some(256 * 1024 * 1024),
                dispatch_epoch_timeout_secs: 5,
                lifecycle_hook_epoch_timeout_secs: 30,
                abac_epoch_timeout_secs: 2,
                abac_max_instructions: 50_000_000,
                max_concurrent_guest_http_per_service: 4,
            }
        };

        // A `0` config value builds a zero-permit semaphore: every guest
        // HTTP request then waits the full admission timeout and 503s, with
        // nothing at startup to explain why. Clamp
        // loudly rather than let that be silently discovered in the field.
        if tuning.max_concurrent_guest_http_per_service == 0 {
            warn!(
                "max_concurrent_guest_http_per_service is 0, which would admit no guest HTTP \
                 requests at all -- clamping to 1"
            );
            SandboxTuning { max_concurrent_guest_http_per_service: 1, ..tuning }
        } else {
            tuning
        }
    }

    /// Fixed, not scaled by `max_instances`: an earlier version scaled this
    /// and, computed independently of `stream_instance_budget`, let the two
    /// jointly oversubscribe the pool (default tier 8 + 3 against 10 slots).
    /// Each concurrent after-step call holds *two* pool slots at once
    /// (itself, plus the live ordinary-dispatch instance it was invoked
    /// from -- see `abac_instance_permits`'s doc comment), so this
    /// reservation is doubled below. `STREAM_INSTANCE_POOL_HEADROOM` is the
    /// existing, already-tested "slots reserved for ordinary calls
    /// generally" budget (`stream_integration.rs::test_stream_instances_
    /// across_services_bounded_by_shared_pool_budget` asserts its exact
    /// arithmetic against a small `max_concurrent_instances`, so this
    /// formula is intentionally left untouched); halving it is the largest
    /// fixed value that still keeps `stream_instance_budget +
    /// abac_instance_budget * 2 == max_concurrent_instances` for every pool
    /// size, rather than only the default one.
    fn resolve_instance_budgets(
        max_instances: u32,
        max_concurrent_streams_per_service: u32,
    ) -> InstanceBudgets {
        let abac = (STREAM_INSTANCE_POOL_HEADROOM / 2).max(1);
        let stream = max_instances.saturating_sub(STREAM_INSTANCE_POOL_HEADROOM).max(1);
        if max_concurrent_streams_per_service > stream {
            warn!(
                max_concurrent_streams_per_service,
                max_concurrent_instances = max_instances,
                stream_instance_budget = stream,
                "a single service's stream cap alone can consume this engine's entire \
                 cross-service stream-instance budget (max_concurrent_instances minus a \
                 {STREAM_INSTANCE_POOL_HEADROOM}-slot reserve for ordinary calls); consider \
                 raising max_concurrent_instances or lowering max_concurrent_streams_per_service"
            );
        }
        InstanceBudgets { abac, stream }
    }

    /// Loads every already-deployed WASM endpoint into the component cache
    /// at startup, so the first request against it does not pay a cold
    /// compile. Best-effort: a single component failing to warm up is
    /// logged, not fatal to substrate startup.
    async fn warm_up_wasm_endpoints(&self, endpoints: Vec<(String, String, SubstrateEndpoint)>) {
        for (service_id, _interface_name, endpoint) in endpoints {
            if let SubstrateEndpoint::WasmChannel { service_id: channel_id } = endpoint {
                info!(
                    service_id = %service_id,
                    channel_id = %channel_id,
                    "Warming up WASM component"
                );

                if let Err(e) = self.load_cached_wasm(&service_id).await {
                    error!("Failed to warm up WASM component {}: {}", service_id, e);
                }
            }
        }
    }

    /// Spawns the background task that advances the Wasmtime engine's epoch
    /// clock -- the tick every in-flight call's epoch-interruption deadline
    /// is measured against. Exits once `shutdown_rx` fires.
    fn spawn_epoch_ticker(engine: Engine, mut shutdown_rx: oneshot::Receiver<()>) {
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_millis(EPOCH_TICK_MS));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        engine.increment_epoch();
                    }
                    _ = &mut shutdown_rx => {
                        break;
                    }
                }
            }
        });
    }

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
        logical_resolver: Arc<LogicalResolver>,
    ) -> anyhow::Result<Self> {
        let component_dir = config.storage.blobs_dir.join("app_sandbox");

        // Ensure blobs directory exists
        if !component_dir.exists() {
            tokio_fs::create_dir_all(&component_dir).await?;
        }

        // Read these limits from `config` based on the hardware tier
        let tuning = Self::resolve_sandbox_tuning(config);

        let engine = Self::build_wasm_engine(
            Some(tuning.max_instances),
            Some(tuning.max_memory),
            tuning.max_core_instances_per_component,
            tuning.max_memories_per_component,
            tuning.max_tables_per_component,
        )?;
        let linker = Self::build_wasm_linker(&engine)?;

        // Component cache
        let components = DashMap::new();

        let dispatch_epoch_ticks = ticks_for_secs(tuning.dispatch_epoch_timeout_secs);
        let lifecycle_hook_epoch_ticks = ticks_for_secs(tuning.lifecycle_hook_epoch_timeout_secs);
        let abac_epoch_ticks = ticks_for_secs(tuning.abac_epoch_timeout_secs);

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let max_concurrent_streams_per_service =
            config.streaming.max_concurrent_streams_per_service;
        let instance_budgets = Self::resolve_instance_budgets(
            tuning.max_instances,
            max_concurrent_streams_per_service,
        );

        let app_engine = Self {
            blobs_dir: component_dir,
            engine,
            linker,
            components,
            fdae_policies: DashMap::new(),
            fdae_policy_generation: DashMap::new(),
            default_max_instructions: tuning.default_max_instructions,
            default_max_memory_bytes: tuning.default_max_memory_bytes,
            _shutdown_tx: Some(shutdown_tx),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            self_weak: OnceLock::new(),
            service_proxy: OnceLock::new(),
            conversation: OnceLock::new(),
            record_signer: OnceLock::new(),
            subscriptions: DashMap::new(),
            endpoint_registry,
            logical_resolver,
            stream_registry: StreamRegistry::new(),
            max_concurrent_streams_per_service,
            stream_instance_permits: Arc::new(Semaphore::new(instance_budgets.stream as usize)),
            abac_instance_permits: Arc::new(Semaphore::new(instance_budgets.abac as usize)),
            probe_instance_permits: Arc::new(Semaphore::new(
                STREAM_INSTANCE_POOL_HEADROOM as usize,
            )),
            dispatch_epoch_ticks,
            lifecycle_hook_epoch_ticks,
            abac_epoch_ticks,
            abac_max_instructions: tuning.abac_max_instructions,
            instantiations: AtomicU64::new(0),
            guest_http_permits: Arc::new(DashMap::new()),
            max_concurrent_guest_http_per_service: tuning.max_concurrent_guest_http_per_service,
            websocket_senders: OnceLock::new(),
            guest_websocket_permits: Arc::new(DashMap::new()),
            max_concurrent_websockets_per_service: config
                .roles
                .app_sandbox
                .as_ref()
                .map(|r| r.max_concurrent_websockets_per_service)
                .unwrap_or(50),
            max_sse_subscribers_per_service: config
                .roles
                .app_sandbox
                .as_ref()
                .map(|r| r.max_sse_subscribers_per_service)
                .unwrap_or(100),
        };

        app_engine.warm_up_wasm_endpoints(endpoints).await;
        Self::spawn_epoch_ticker(app_engine.engine.clone(), shutdown_rx);

        Ok(app_engine)
    }

    /// Helper to build the Wasmtime Engine. `max_core_instances_per_component`/
    /// `max_memories_per_component`/`max_tables_per_component` bound how much
    /// of each resource a *single* component may transitively use (Wasmtime
    /// leaves these unbounded per component by default); the pool's global
    /// totals are then `max_instances` times each of these, so a component
    /// that stays within its declared per-component budget is always
    /// guaranteed a slot regardless of how many other components are
    /// concurrently live, and one that doesn't fails to instantiate with a
    /// clear Wasmtime error rather than silently starving its neighbors'
    /// share of an unbounded shared pool. All three are ignored -- safe to
    /// pass `0` -- whenever `max_instances`/`max_memory` are `None`: that
    /// disables the pooling allocator entirely (Wasmtime's on-demand
    /// strategy, no resource caps of any kind), so there is no per-component
    /// limit for them to configure.
    pub fn build_wasm_engine(
        max_instances: Option<u32>,
        max_memory: Option<usize>,
        max_core_instances_per_component: u32,
        max_memories_per_component: u32,
        max_tables_per_component: u32,
    ) -> Result<Engine> {
        let mut wasmtime_config = Config::new();
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);

        if let (Some(instances), Some(memory)) = (max_instances, max_memory) {
            // Cranelift compiles every component from scratch on
            // `Component::new` (see `compile_and_cache_wasm`). The on-disk
            // compile cache keys compiled artifacts by wasm bytes plus
            // compiler flags, so a component seen before -- a substrate
            // restart, a redeploy, or the next test in a suite that loads the
            // same components -- loads in milliseconds instead. Wasmtime's
            // default cache directory is per-user, content-addressed, and
            // self-pruning; a broken or unwritable directory must not stop the
            // sandbox from starting, so a failure here only disables the cache.
            match Cache::new(CacheConfig::new()) {
                Ok(cache) => {
                    wasmtime_config.cache(Some(cache));
                }
                Err(e) => {
                    warn!("wasmtime compile cache disabled: {e}");
                }
            }

            wasmtime_config.memory_init_cow(true);
            let mut pooling_config = PoolingAllocationConfig::default();
            pooling_config.total_component_instances(instances);
            pooling_config.max_memory_size(memory);
            pooling_config.max_core_instances_per_component(max_core_instances_per_component);
            pooling_config.max_memories_per_component(max_memories_per_component);
            pooling_config.max_tables_per_component(max_tables_per_component);
            // `total_memories`/`total_core_instances`/`total_tables` are
            // *separate* pooling-allocator knobs from `total_component_instances`
            // -- Wasmtime defaults each to 1000 regardless of it, and (unlike
            // the `max_..._per_component` limits just above) bound the whole
            // pool's aggregate, not any one component's share of it. The
            // memory pool's actual address-space reservation is
            // `total_memories * slot_bytes` (`MemoryPool::new`), so leaving
            // `total_memories` at its default means `max_memory_size` above
            // never shrinks the real reservation -- it stays governed by
            // Wasmtime's own defaults (1000 slots) no matter how small
            // `instances`/`memory` are configured. Each total is exactly
            // `instances * max_..._per_component`: enough for every
            // concurrently-live component to use its full declared budget at
            // once, and no more.
            let total_core_instances =
                instances.checked_mul(max_core_instances_per_component).with_context(|| {
                    format!(
                        "app_sandbox role: max_concurrent_instances ({instances}) * \
                         max_core_instances_per_component ({max_core_instances_per_component}) \
                         overflows u32"
                    )
                })?;
            let total_memories =
                instances.checked_mul(max_memories_per_component).with_context(|| {
                    format!(
                        "app_sandbox role: max_concurrent_instances ({instances}) * \
                         max_memories_per_component ({max_memories_per_component}) overflows u32"
                    )
                })?;
            let total_tables =
                instances.checked_mul(max_tables_per_component).with_context(|| {
                    format!(
                        "app_sandbox role: max_concurrent_instances ({instances}) * \
                         max_tables_per_component ({max_tables_per_component}) overflows u32"
                    )
                })?;
            pooling_config.total_core_instances(total_core_instances);
            pooling_config.total_memories(total_memories);
            pooling_config.total_tables(total_tables);
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
        saga::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        websocket::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        conversation::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        signing::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        invocation::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)?;
        Ok(linker)
    }

    /// Helper to fetch WASM bytes from a source
    pub(crate) async fn fetch_wasm_bytes(source: &ArtifactSource) -> Result<Vec<u8>> {
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
    pub(crate) fn verify_wasm_hash(bytes: &[u8], expected_hash: Option<&str>) -> Result<()> {
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
}

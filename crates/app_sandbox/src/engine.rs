use anyhow::{Context, Result};
use dashmap::DashMap;
use std::path::PathBuf;
use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceType,
};
use syneroym_core::{config::SubstrateConfig, registry::SubstrateEndpoint};
use wasmtime::component::{Component, Linker};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxView};

/// Host state instantiated per-request for WASM components
pub struct HostState {
    pub wasi: WasiCtx,
    pub table: ResourceTable,
    // Custom state
    pub component_id: String,
    pub request_ctx: Option<String>,
}

impl wasmtime_wasi::WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl syneroym_bindings::host::syneroym::host::context::Host for HostState {
    async fn get_test_context(&mut self, request_ctx: String) -> String {
        let component_ctx = format!("Component: {}", self.component_id);
        if let Some(existing) = &self.request_ctx {
            format!("{} | {} | {}", component_ctx, existing, request_ctx)
        } else {
            format!("{} | {}", component_ctx, request_ctx)
        }
    }
}

/// Engine: Passive code module that wraps low-level OS operations
/// to spin up Wasmtime or Podman instances.
pub struct AppSandboxEngine {
    blobs_dir: PathBuf,
    engine: wasmtime::Engine,
    linker: Linker<HostState>,
    // Cache of compiled components for fast instantiation
    components: DashMap<String, Component>,
}

impl AppSandboxEngine {
    /// Initializes the App Sandbox and warms up any existing WASM endpoints
    pub async fn init(
        config: &SubstrateConfig,
        endpoints: Vec<(String, String, SubstrateEndpoint)>,
    ) -> anyhow::Result<Self> {
        let component_dir = config.storage.blobs_dir.join("app_sandbox");

        // Ensure blobs directory exists
        if !component_dir.exists() {
            tokio::fs::create_dir_all(&component_dir).await?;
        }

        // Configure Wasmtime engine with optimizations
        let mut wasmtime_config = wasmtime::Config::new();
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.memory_init_cow(true);

        // Configure pooling allocation for per-request isolation
        let mut pooling_config = wasmtime::PoolingAllocationConfig::default();

        // Read these limits from `config` based on the hardware tier
        let (max_instances, max_memory) = if let Some(sandbox_config) = &config.roles.app_sandbox {
            (sandbox_config.max_concurrent_instances, sandbox_config.memory_limit_bytes() as usize)
        } else {
            (10, 128 * 1024 * 1024)
        };

        // Restrict the maximum number of concurrent WASM instances
        pooling_config.total_component_instances(max_instances);
        // Restrict each WASM instance to a maximum linear memory
        pooling_config.max_memory_size(max_memory);

        wasmtime_config
            .allocation_strategy(wasmtime::InstanceAllocationStrategy::Pooling(pooling_config));

        let engine = wasmtime::Engine::new(&wasmtime_config)?;

        // Initialize linker
        let mut linker = Linker::new(&engine);

        // Add WASI capabilities
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;

        // Add custom host capabilities
        syneroym_bindings::host::syneroym::host::context::add_to_linker::<
            _,
            wasmtime::component::HasSelf<HostState>,
        >(&mut linker, |state| state)?;

        // Component cache
        let components = DashMap::new();

        let engine = Self { blobs_dir: component_dir, engine, linker, components };

        for (service_id, _interface_name, endpoint) in endpoints {
            if let SubstrateEndpoint::WasmChannel { channel_details: channel_id } = endpoint {
                tracing::info!(
                    service_id = %service_id,
                    channel_id = %channel_id,
                    "Warming up WASM component"
                );

                if let Err(e) = engine.load_cached_wasm(&service_id).await {
                    tracing::error!("Failed to warm up WASM component {}: {}", service_id, e);
                }
            }
        }

        Ok(engine)
    }

    /// Spin up a new Wasmtime instance
    pub async fn deploy_wasm(&self, service_id: &str, manifest: &DeployManifest) -> Result<()> {
        tracing::info!("AppSandboxEngine: Deploying Wasm component for {}", service_id);

        let ServiceType::Wasm(wasm_manifest) = &manifest.service_type else {
            return Err(anyhow::anyhow!("Expected Wasm manifest"));
        };

        // 1. Fetch bytes
        let bytes = match &wasm_manifest.source {
            ArtifactSource::Url(url) => {
                tracing::info!("Fetching WASM from URL: {}", url);
                reqwest::get(url)
                    .await
                    .context("Failed to fetch WASM from URL")?
                    .bytes()
                    .await
                    .context("Failed to read WASM bytes")?
                    .to_vec()
            }
            ArtifactSource::Binary(b) => b.clone(),
        };

        // 2. Verify hash
        if let Some(expected_hash) = &wasm_manifest.hash {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let computed_hash = hex::encode(hasher.finalize());

            // Allow checking with or without standard 'sha256:' prefix from OCI registries
            let expected_hash_clean =
                expected_hash.strip_prefix("sha256:").unwrap_or(expected_hash);

            if computed_hash != *expected_hash_clean {
                return Err(anyhow::anyhow!(
                    "Hash mismatch: expected {}, got {}",
                    expected_hash_clean,
                    computed_hash
                ));
            }
            tracing::info!("WASM hash verified successfully");
        }

        // 3. Store locally in blobs_dir
        let file_path = self.blobs_dir.join(format!("{}.wasm", service_id));
        tokio::fs::write(&file_path, &bytes).await.context("Failed to save WASM binary locally")?;

        tracing::info!("WASM binary stored at {:?}", file_path);

        // 4. Compile and cache the component
        self.compile_and_cache_wasm(service_id, &bytes)?;

        Ok(())
    }

    /// Execute a WASM component for a given service
    pub async fn execute_wasm(
        &self,
        service_id: &str,
        component_id: &str,
        request_ctx: Option<String>,
    ) -> Result<String> {
        // Look up the compiled component
        let component = self
            .components
            .get(service_id)
            .ok_or_else(|| anyhow::anyhow!("Component not found for service {}", service_id))?;

        // Create new WASI context and table for per-request isolation
        let wasi = WasiCtx::builder().inherit_stderr().inherit_stdout().build();
        let table = ResourceTable::new();

        // Create host state
        let host_state =
            HostState { wasi, table, component_id: component_id.to_string(), request_ctx };

        // Create a new store
        let mut store = wasmtime::Store::new(&self.engine, host_state);

        // Instantiate the component using generated HostEnvironment
        let env = syneroym_bindings::host::HostEnvironment::instantiate_async(
            &mut store,
            &component,
            &self.linker,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to instantiate component: {}", e))?;

        // Call the run function
        let result = env.syneroym_host_app().call_run(&mut store).await?;

        // Store is dropped here, enforcing cleanup

        Ok(result)
    }

    /// Simple test function to invoke test context
    pub async fn invoke_test_context(
        &self,
        service_id: &str,
        component_id: &str,
        request_ctx: &str,
    ) -> Result<String> {
        self.execute_wasm(service_id, component_id, Some(request_ctx.to_string())).await
    }

    /// Stop a running Wasm component
    pub async fn stop_wasm(&self, _service_id: &str) -> Result<()> {
        tracing::info!("AppSandboxEngine: Stopping Wasm component for {}", _service_id);
        Ok(())
    }

    /// Remove a stopped Wasm component
    pub async fn remove_wasm(&self, _service_id: &str) -> Result<()> {
        tracing::info!("AppSandboxEngine: Removing Wasm component for {}", _service_id);
        Ok(())
    }

    /// Helper to load a cached WASM component from disk and compile it
    async fn load_cached_wasm(&self, service_id: &str) -> Result<()> {
        let file_path = self.blobs_dir.join(format!("{}.wasm", service_id));
        if file_path.exists() {
            let bytes = tokio::fs::read(&file_path)
                .await
                .context(format!("Failed to read WASM file {:?}", file_path))?;
            self.compile_and_cache_wasm(service_id, &bytes)?;
        } else {
            tracing::warn!("WASM file not found on disk for service: {:?}", file_path);
        }
        Ok(())
    }

    /// Helper to compile a WASM binary and store it in the cache
    fn compile_and_cache_wasm(&self, service_id: &str, bytes: &[u8]) -> Result<()> {
        let component = Component::new(&self.engine, bytes)
            .map_err(|e| anyhow::anyhow!("Failed to compile WASM component: {}", e))?;

        self.components.insert(service_id.to_string(), component);
        tracing::info!("WASM component compiled and cached for {}", service_id);
        Ok(())
    }

    /// Spin up a new Podman instance
    pub async fn deploy_podman(&self, _service_id: &str, _manifest: &[u8]) -> Result<()> {
        tracing::info!("AppSandboxEngine: Deploying Podman container for {}", _service_id);
        Ok(())
    }
}

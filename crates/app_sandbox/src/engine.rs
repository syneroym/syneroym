use anyhow::{Context, Result};
use dashmap::DashMap;
use std::path::PathBuf;
use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceType,
};
use syneroym_core::{config::SubstrateConfig, registry::SubstrateEndpoint};
use syneroym_rpc::JsonRpcRequest;
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
        interface_name: &str,
        request: &JsonRpcRequest,
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
            HostState { wasi, table, component_id: service_id.to_string(), request_ctx: None };

        // Create a new store
        let mut store = wasmtime::Store::new(&self.engine, host_state);

        let instance = self.linker.instantiate_async(&mut store, &component).await?;

        // Extract the interface export index
        let (_, instance_idx) = instance
            .get_export(&mut store, None, interface_name)
            .ok_or_else(|| anyhow::anyhow!("Interface '{}' not found", interface_name))?;

        // Extract the method export index
        let (item, func_idx) = instance
            .get_export(&mut store, Some(&instance_idx), &request.method)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Method '{}' not found in interface '{}'",
                    request.method,
                    interface_name
                )
            })?;

        let func = instance
            .get_func(&mut store, func_idx)
            .ok_or_else(|| anyhow::anyhow!("Method is not a function"))?;

        // Parse parameters based on ComponentFunc signature
        let params_iter = match &item {
            wasmtime::component::types::ComponentItem::ComponentFunc(f) => f.params(),
            _ => return Err(anyhow::anyhow!("Expected a function item")),
        };

        let mut wasm_params = Vec::new();
        // Dynamic parameter resolution
        // The jsonrpcrequest.params is a single JSON value. If the component expects a string,
        // we serialize it or pass the exact string. If it expects a primitive, we convert it.
        // We will take a simplistic approach for now, assuming the first param is either String or maps from JSON.
        // If the method expects multiple params, we assume request.params is a JSON array.
        let json_params = match &request.params {
            serde_json::Value::Array(arr) => arr.clone(),
            other => vec![other.clone()],
        };

        for (i, (_param_name, ty)) in params_iter.enumerate() {
            let val = json_params.get(i).unwrap_or(&serde_json::Value::Null);
            use wasmtime::component::Val;
            use wasmtime::component::types::Type;
            match ty {
                Type::String => {
                    let s: String = match val {
                        serde_json::Value::String(s) => s.clone(),
                        _ => val.to_string(),
                    };
                    wasm_params.push(Val::String(s));
                }
                Type::U32 => {
                    let n = val.as_u64().unwrap_or(0) as u32;
                    wasm_params.push(Val::U32(n));
                }
                Type::Bool => {
                    let b = val.as_bool().unwrap_or(false);
                    wasm_params.push(Val::Bool(b));
                }
                // Handle basic cases
                _ => {
                    return Err(anyhow::anyhow!(
                        "Unsupported parameter type in Wasm component. Add conversion logic."
                    ));
                }
            }
        }

        // Setup results slice. We assume single result (usually string)
        let results_len = match &item {
            wasmtime::component::types::ComponentItem::ComponentFunc(f) => f.results().len(),
            _ => 0,
        };
        let mut wasm_results = vec![wasmtime::component::Val::Bool(false); results_len];

        func.call_async(&mut store, &wasm_params, &mut wasm_results).await?;

        // Convert the result to String
        if wasm_results.is_empty() {
            Ok(String::new())
        } else {
            match &wasm_results[0] {
                wasmtime::component::Val::String(s) => Ok(s.to_string()),
                wasmtime::component::Val::Result(Ok(Some(v))) => {
                    // Try to dig out a string from an Ok(Val)
                    match &**v {
                        wasmtime::component::Val::String(s) => Ok(s.to_string()),
                        _ => Ok(format!("{:?}", v)), // Fallback
                    }
                }
                wasmtime::component::Val::Result(Ok(None)) => Ok(String::new()),
                wasmtime::component::Val::Result(Err(Some(e))) => {
                    Err(anyhow::anyhow!("Component returned error: {:?}", e))
                }
                wasmtime::component::Val::Result(Err(None)) => {
                    Err(anyhow::anyhow!("Component returned empty error"))
                }
                other => Ok(format!("{:?}", other)),
            }
        }
    }

    /// Simple test function to invoke test context
    pub async fn invoke_test_context(
        &self,
        service_id: &str,
        component_id: &str,
        request_ctx: &str,
    ) -> Result<String> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "run".to_string(), // Default method for test
            params: serde_json::Value::String(request_ctx.to_string()),
            id: None,
        };
        self.execute_wasm(service_id, component_id, &request).await
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

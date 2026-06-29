//! WASM execution engine based on Wasmtime
//!
//! Sets up the sandboxed environment with strict CPU/memory quotas,
//! registers host capabilities, and runs WASM component binaries.

use std::{
    fmt::{Debug, Formatter},
    path::PathBuf,
    time::Instant,
};

use anyhow::{Context, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use syneroym_bindings::{
    control_plane::exports::syneroym::control_plane::orchestrator::{
        ArtifactSource, DeployManifest, ServiceType,
    },
    host::syneroym::host::{context, context::Host},
};
use syneroym_core::{config::SubstrateConfig, local_registry::SubstrateEndpoint};
use syneroym_rpc::JsonRpcRequest;
use tokio::fs as tokio_fs;
use tracing::debug;
use wasmtime::{
    Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, Store, StoreLimits,
    StoreLimitsBuilder,
    component::{
        Component, Func, HasSelf, Instance, InstancePre, Linker, Val, types::ComponentItem,
    },
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxView, WasiView, p2};

use crate::conversions::{json_to_wasm_params, wasm_results_to_json_string};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmResourceQuota {
    pub max_instructions: Option<u64>,
    pub max_memory_bytes: Option<u64>,
}

/// Host state instantiated per-request for WASM components
pub struct HostState {
    pub wasi: WasiCtx,
    pub table: ResourceTable,
    // Custom state
    pub component_id: String,
    pub request_ctx: Option<String>,
    pub memory_limits: StoreLimits,
}

impl Debug for HostState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostState")
            .field("component_id", &self.component_id)
            .field("request_ctx", &self.request_ctx)
            .finish_non_exhaustive()
    }
}

impl HostState {
    /// Creates a new HostState with standard WASI context.
    pub fn new(component_id: String, max_memory_bytes: Option<usize>) -> Self {
        let wasi = WasiCtx::builder().build();
        let table = ResourceTable::new();
        let memory_limits = StoreLimitsBuilder::new()
            .memory_size(max_memory_bytes.unwrap_or(usize::MAX))
            .instances(1)
            .memories(1)
            .tables(1)
            .build();
        Self { wasi, table, component_id, request_ctx: None, memory_limits }
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl Host for HostState {
    async fn get_test_context(&mut self, request_ctx: String) -> String {
        let component_ctx = format!("Component: {}", self.component_id);
        if let Some(existing) = &self.request_ctx {
            format!("{component_ctx} | {existing} | {request_ctx}")
        } else {
            format!("{component_ctx} | {request_ctx}")
        }
    }
}

impl wasmtime::ResourceLimiter for HostState {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> std::result::Result<bool, wasmtime::Error> {
        match self.memory_limits.memory_growing(current, desired, maximum) {
            Ok(true) => Ok(true),
            _ => Err(wasmtime::Error::msg("MemoryFault: Wasm execution exceeded memory limit")),
        }
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> std::result::Result<bool, wasmtime::Error> {
        self.memory_limits.table_growing(current, desired, maximum)
    }
}

/// Engine: Passive code module that wraps low-level OS operations
/// to spin up Wasmtime or Podman instances.
pub struct AppSandboxEngine {
    blobs_dir: PathBuf,
    engine: Engine,
    linker: Linker<HostState>,
    // Cache of pre-linked instances for fast instantiation
    components: DashMap<String, (InstancePre<HostState>, Option<WasmResourceQuota>)>,
    default_max_instructions: Option<u64>,
    default_max_memory_bytes: Option<u64>,
    _shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Debug for AppSandboxEngine {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
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
            || std::path::Path::new(service_id).is_absolute()
        {
            return Err(anyhow::anyhow!(
                "Invalid service_id: path traversal or invalid characters"
            ));
        }
        Ok(())
    }

    /// Initializes the App Sandbox and warms up any existing WASM endpoints
    pub async fn init(
        config: &SubstrateConfig,
        endpoints: Vec<(String, String, SubstrateEndpoint)>,
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

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let app_engine = Self {
            blobs_dir: component_dir,
            engine,
            linker,
            components,
            default_max_instructions,
            default_max_memory_bytes,
            _shutdown_tx: Some(shutdown_tx),
        };

        for (service_id, _interface_name, endpoint) in endpoints {
            if let SubstrateEndpoint::WasmChannel { service_id: channel_id } = endpoint {
                tracing::info!(
                    service_id = %service_id,
                    channel_id = %channel_id,
                    "Warming up WASM component"
                );

                if let Err(e) = app_engine.load_cached_wasm(&service_id).await {
                    tracing::error!("Failed to warm up WASM component {}: {}", service_id, e);
                }
            }
        }

        let engine_clone = app_engine.engine.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
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
        Ok(linker)
    }

    /// Helper to fetch WASM bytes from a source
    async fn fetch_wasm_bytes(source: &ArtifactSource) -> Result<Vec<u8>> {
        match source {
            ArtifactSource::Url(url) => {
                tracing::info!("Fetching WASM from URL: {}", url);
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
            tracing::info!("WASM hash verified successfully");
        }
        Ok(())
    }

    /// Helper to extract WASM function and its result length
    pub fn get_wasm_func(
        store: &mut Store<HostState>,
        instance: &Instance,
        interface_name: &str,
        method_name: &str,
    ) -> Result<(Func, usize, ComponentItem)> {
        let (_, instance_idx) = instance
            .get_export(&mut *store, None, interface_name)
            .ok_or_else(|| anyhow::anyhow!("Interface '{interface_name}' not found"))?;

        let (item, func_idx) = instance
            .get_export(&mut *store, Some(&instance_idx), method_name)
            .ok_or_else(|| {
            anyhow::anyhow!("Method '{method_name}' not found in interface '{interface_name}'")
        })?;

        let func = instance
            .get_func(&mut *store, func_idx)
            .ok_or_else(|| anyhow::anyhow!("Method is not a function"))?;

        let results_len = match &item {
            ComponentItem::ComponentFunc(f) => f.results().len(),
            _ => 0,
        };

        Ok((func, results_len, item))
    }

    /// Deploy and compile a WASM component for a given service
    pub async fn deploy_wasm(&self, service_id: &str, manifest: &DeployManifest) -> Result<()> {
        Self::validate_service_id(service_id)?;
        tracing::info!("AppSandboxEngine: Deploying Wasm component for {}", service_id);

        let ServiceType::Wasm(wasm_manifest) = &manifest.service_type else {
            return Err(anyhow::anyhow!("Expected Wasm manifest"));
        };

        // 1. Fetch bytes
        let bytes = Self::fetch_wasm_bytes(&wasm_manifest.source).await?;

        // 2. Verify hash
        Self::verify_wasm_hash(&bytes, wasm_manifest.hash.as_deref())?;

        // 3. Store locally in blobs_dir
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        tokio::fs::write(&file_path, &bytes).await.context("Failed to save WASM binary locally")?;

        tracing::info!("WASM binary stored at {:?}", file_path);

        let quota = manifest.config.quota.as_ref().map(|q| WasmResourceQuota {
            max_instructions: q.max_instructions,
            max_memory_bytes: q.max_memory_bytes,
        });

        if let Some(ref q) = quota {
            let quota_path = self.blobs_dir.join(format!("{service_id}.quota.json"));
            if let Ok(quota_json) = serde_json::to_string(q) {
                let _ = tokio::fs::write(&quota_path, quota_json).await;
            }
        }

        // 4. Compile and cache the component; drop the raw bytes immediately to free
        //    memory
        self.compile_and_cache_wasm(service_id, &bytes, quota)?;
        drop(bytes);

        Ok(())
    }

    /// Execute a WASM component for a given service
    pub async fn execute_wasm(
        &self,
        service_id: &str,
        interface_name: &str,
        request: &JsonRpcRequest,
    ) -> Result<String> {
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

        // Dynamic parameter resolution
        let json_params = match &request.params {
            serde_json::Value::Array(arr) => arr.clone(),
            other => vec![other.clone()],
        };

        let wasm_params = json_to_wasm_params(params_iter, json_params)?;

        debug!("created input types");

        let mut wasm_results = vec![Val::Bool(false); results_len];
        debug!("created result types");

        let exec_start = Instant::now();
        let res = func.call_async(&mut store, &wasm_params, &mut wasm_results).await;
        metrics::histogram!("substrate.wasm.execution_ms")
            .record(exec_start.elapsed().as_secs_f64() * 1000.0);

        debug!("called wasm function, processing results");

        if let Err(e) = res {
            if let Some(wasmtime::Trap::OutOfFuel) = e.downcast_ref::<wasmtime::Trap>() {
                tracing::warn!("Wasm execution exceeded fuel limit for service: {}", service_id);
                return Err(anyhow::anyhow!("QuotaExceeded: Wasm execution exceeded fuel limit"));
            }
            let err_str = e.root_cause().to_string();
            if err_str.contains("all fuel consumed") || err_str.contains("out of fuel") {
                tracing::warn!("Wasm execution exceeded fuel limit for service: {}", service_id);
                return Err(anyhow::anyhow!("QuotaExceeded: Wasm execution exceeded fuel limit"));
            }
            if err_str.contains("exceeded its memory limits") || err_str.contains("MemoryFault") {
                return Err(anyhow::anyhow!("MemoryFault: Wasm execution exceeded memory limit"));
            }
            return Err(e.into());
        }

        wasm_results_to_json_string(&wasm_results)
    }

    /// Helper to prepare WASM execution context and extract function
    async fn prepare_wasm_execution(
        &self,
        service_id: &str,
        interface_name: &str,
        method_name: &str,
    ) -> Result<(Store<HostState>, Func, usize, ComponentItem)> {
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

        // Create host state
        let host_state = HostState::new(service_id.to_string(), max_memory_bytes);

        debug!("created wasi ctx and host state");

        // Create a new store
        let mut store = Store::new(&self.engine, host_state);

        store.limiter(|state| state);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(50); // 50 * 100ms = 5 seconds wall-clock timeout

        if let Some(instructions) = max_instructions {
            store.set_fuel(instructions)?;
        }

        let inst_start = Instant::now();
        let instance = instance_pre.instantiate_async(&mut store).await?;
        metrics::histogram!("substrate.wasm.instantiation_ms")
            .record(inst_start.elapsed().as_secs_f64() * 1000.0);

        debug!("instantiated store and instance");

        // Use the helper to extract the function
        let (func, results_len, item) =
            Self::get_wasm_func(&mut store, &instance, interface_name, method_name)?;

        debug!("extracted the interface and method export indices");

        Ok((store, func, results_len, item))
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

    /// Stop and evict a running Wasm component from the in-memory cache.
    pub async fn stop_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        tracing::info!(service_id = %service_id, "AppSandboxEngine: stopping Wasm component");
        self.components.remove(service_id);
        metrics::gauge!("substrate.wasm.component_cache_size").set(self.components.len() as f64);
        Ok(())
    }

    /// Remove a stopped Wasm component's binary from disk.
    pub async fn remove_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        tracing::info!(service_id = %service_id, "AppSandboxEngine: removing Wasm component");
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
            tracing::warn!("WASM file not found on disk for service: {:?}", file_path);
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
        tracing::info!("WASM component compiled and cached for {}", service_id);
        metrics::gauge!("substrate.wasm.component_cache_size").set(self.components.len() as f64);
        Ok(())
    }

    /// Spin up a new Podman instance
    pub async fn deploy_podman(&self, _service_id: &str, _manifest: &[u8]) -> Result<()> {
        tracing::info!("AppSandboxEngine: Deploying Podman container for {}", _service_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use syneroym_core::test_constants;
    use wasmtime::component::Component;

    use super::*;

    #[tokio::test]
    async fn test_list_interfaces() {
        let engine = AppSandboxEngine::build_wasm_engine(None, None).unwrap();
        let linker = AppSandboxEngine::build_wasm_linker(&engine).unwrap();

        let host_state = HostState::new("test_component".to_string(), None);
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
                    interface_name,
                    method_name,
                ) {
                    Ok((func, results_len, _item)) => {
                        println!("Function export: {func:?}");
                        let mut wasm_results = vec![Val::Bool(false); results_len];

                        let result = func
                            .call_async(
                                &mut store,
                                &[wasmtime::component::Val::String("TestUser".to_string())],
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
            blobs_dir: std::env::temp_dir(),
            engine,
            linker,
            components: DashMap::new(),
            default_max_instructions: Some(10_000),
            default_max_memory_bytes: Some(1024 * 1024), // 1MB
            _shutdown_tx: None,
        };

        // Cache the test component
        app_engine.compile_and_cache_wasm("test_service", wat.as_bytes(), None).unwrap();

        // 1. Test infinite loop (fuel limit)
        let request_loop = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "loop-forever".to_string(),
            params: serde_json::Value::Array(vec![]),
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
            params: serde_json::Value::Array(vec![serde_json::Value::Number(
                serde_json::Number::from(100),
            )]),
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
}

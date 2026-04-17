use anyhow::{Context, Result};
use dashmap::DashMap;
use std::path::PathBuf;
use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceType,
};
use syneroym_core::{config::SubstrateConfig, registry::SubstrateEndpoint};
use syneroym_rpc::JsonRpcRequest;
use tracing::debug;
use wasmtime::component::{Component, Linker};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxView};

use crate::conversions::{json_to_wasm_params, wasm_results_to_json_string};

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

        let app_engine = Self { blobs_dir: component_dir, engine, linker, components };

        for (service_id, _interface_name, endpoint) in endpoints {
            if let SubstrateEndpoint::WasmChannel { channel_details: channel_id } = endpoint {
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

        Ok(app_engine)
    }

    /// Helper to build the Wasmtime Engine
    pub fn build_wasm_engine(
        max_instances: Option<u32>,
        max_memory: Option<usize>,
    ) -> Result<wasmtime::Engine> {
        let mut wasmtime_config = wasmtime::Config::new();
        wasmtime_config.wasm_component_model(true);

        if let (Some(instances), Some(memory)) = (max_instances, max_memory) {
            wasmtime_config.memory_init_cow(true);
            let mut pooling_config = wasmtime::PoolingAllocationConfig::default();
            pooling_config.total_component_instances(instances);
            pooling_config.max_memory_size(memory);
            wasmtime_config
                .allocation_strategy(wasmtime::InstanceAllocationStrategy::Pooling(pooling_config));
        }

        wasmtime::Engine::new(&wasmtime_config).map_err(Into::into)
    }

    /// Helper to build the Wasmtime Linker
    pub fn build_wasm_linker(engine: &wasmtime::Engine) -> Result<Linker<HostState>> {
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        syneroym_bindings::host::syneroym::host::context::add_to_linker::<
            _,
            wasmtime::component::HasSelf<HostState>,
        >(&mut linker, |state| state)?;
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
                    "Hash mismatch: expected {}, got {}",
                    expected_hash_clean,
                    computed_hash
                ));
            }
            tracing::info!("WASM hash verified successfully");
        }
        Ok(())
    }

    /// Helper to extract WASM function and its result length
    pub fn get_wasm_func(
        store: &mut wasmtime::Store<HostState>,
        instance: &wasmtime::component::Instance,
        interface_name: &str,
        method_name: &str,
    ) -> Result<(wasmtime::component::Func, usize, wasmtime::component::types::ComponentItem)> {
        let (_, instance_idx) = instance
            .get_export(&mut *store, None, interface_name)
            .ok_or_else(|| anyhow::anyhow!("Interface '{}' not found", interface_name))?;

        let (item, func_idx) = instance
            .get_export(&mut *store, Some(&instance_idx), method_name)
            .ok_or_else(|| {
            anyhow::anyhow!("Method '{}' not found in interface '{}'", method_name, interface_name)
        })?;

        let func = instance
            .get_func(&mut *store, func_idx)
            .ok_or_else(|| anyhow::anyhow!("Method is not a function"))?;

        let results_len = match &item {
            wasmtime::component::types::ComponentItem::ComponentFunc(f) => f.results().len(),
            _ => 0,
        };

        Ok((func, results_len, item))
    }

    /// Spin up a new Wasmtime instance
    pub async fn deploy_wasm(&self, service_id: &str, manifest: &DeployManifest) -> Result<()> {
        tracing::info!("AppSandboxEngine: Deploying Wasm component for {}", service_id);

        let ServiceType::Wasm(wasm_manifest) = &manifest.service_type else {
            return Err(anyhow::anyhow!("Expected Wasm manifest"));
        };

        // 1. Fetch bytes
        let bytes = Self::fetch_wasm_bytes(&wasm_manifest.source).await?;

        // 2. Verify hash
        Self::verify_wasm_hash(&bytes, wasm_manifest.hash.as_deref())?;

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
        debug!("starting to execute wasm");

        let (mut store, func, results_len, item) =
            self.prepare_wasm_execution(service_id, interface_name, &request.method).await?;

        // Parse parameters based on ComponentFunc signature
        let params_iter = match &item {
            wasmtime::component::types::ComponentItem::ComponentFunc(f) => f.params(),
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

        let mut wasm_results = vec![wasmtime::component::Val::Bool(false); results_len];
        debug!("created result types");

        func.call_async(&mut store, &wasm_params, &mut wasm_results).await?;

        debug!("called wasm function, processing results");

        wasm_results_to_json_string(&wasm_results)
    }

    /// Helper to prepare WASM execution context and extract function
    async fn prepare_wasm_execution(
        &self,
        service_id: &str,
        interface_name: &str,
        method_name: &str,
    ) -> Result<(
        wasmtime::Store<HostState>,
        wasmtime::component::Func,
        usize,
        wasmtime::component::types::ComponentItem,
    )> {
        // Look up the compiled component
        let component = self
            .components
            .get(service_id)
            .ok_or_else(|| anyhow::anyhow!("Component not found for service {}", service_id))?;
        debug!("looked up component");

        // Create new WASI context and table for per-request isolation
        let wasi = WasiCtx::builder().inherit_stderr().inherit_stdout().build();
        let table = ResourceTable::new();

        // Create host state
        let host_state =
            HostState { wasi, table, component_id: service_id.to_string(), request_ctx: None };

        debug!("created wasi ctx and host state");

        // Create a new store
        let mut store = wasmtime::Store::new(&self.engine, host_state);

        let instance = self.linker.instantiate_async(&mut store, &component).await?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::component::Component;
    use wasmtime_wasi::{ResourceTable, WasiCtx};

    #[tokio::test]
    async fn test_list_interfaces() {
        let engine = AppSandboxEngine::build_wasm_engine(None, None).unwrap();
        let linker = AppSandboxEngine::build_wasm_linker(&engine).unwrap();

        // Create new WASI context and table for per-request isolation
        let wasi = WasiCtx::builder().inherit_stderr().inherit_stdout().build();
        let table = ResourceTable::new();

        // Create a new store using the shared HostState
        let host_state = HostState {
            wasi,
            table,
            component_id: "test_component".to_string(),
            request_ctx: None,
        };
        let mut store = wasmtime::Store::new(&engine, host_state);

        // Attempt to read the test WASM component from relative paths
        let component_path =
            "../../test-components/greeter/target/wasm32-wasip2/release/syneroym_test_greeter.wasm";
        let wasm_bytes = std::fs::read(component_path).unwrap_or_else(|_| {
            std::fs::read(component_path).expect("Failed to read compiled test WASM component")
        });

        let component: Component =
            Component::new(&engine, &wasm_bytes).expect("Failed to compile WASM component");
        for interface in component.component_type().exports(&engine) {
            println!("Listing interface: {:?}", interface);
        }

        match linker.instantiate_async(&mut store, &component).await {
            Ok(instance) => {
                let interface_name = "syneroym-test:greeter/greet@0.1.0";
                let method_name = "greet";

                // Use the helper function to extract function and result size
                match AppSandboxEngine::get_wasm_func(
                    &mut store,
                    &instance,
                    interface_name,
                    method_name,
                ) {
                    Ok((func, results_len, _item)) => {
                        println!("Function export: {:?}", func);
                        let mut wasm_results =
                            vec![wasmtime::component::Val::Bool(false); results_len];

                        let result = func
                            .call_async(
                                &mut store,
                                &[wasmtime::component::Val::String("TestUser".to_string())],
                                &mut wasm_results,
                            )
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to call function: {}", e));
                        println!("Function call result: {:?} is {:?}", result, wasm_results);
                    }
                    Err(e) => {
                        println!("Failed to get wasm func: {}", e);
                    }
                }
            }
            Err(err) => {
                println!("Error instantiating component: {}", err);
            }
        }
    }
}

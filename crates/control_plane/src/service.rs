//! Orchestrator control service implementation
//!
//! Handles requests for registering, deploying, listing, and destroying
//! sandbox instances or services running on the node.

use std::{collections::HashMap, fmt, fs, path::PathBuf, sync::Arc};

use anyhow::Result;
use fmt::{Debug, Formatter};
use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, DeployedService, DeploymentPlan, ServiceType as WitServiceType,
};

#[async_trait::async_trait]
pub trait OrchestratorInterface {
    async fn readyz(&self, service_id: String) -> Result<(), String>;
    async fn deploy(&self, service_id: String, manifest: DeployManifest) -> Result<(), String>;
    async fn undeploy(&self, service_id: String) -> Result<(), String>;
    async fn list(&self) -> Result<Vec<DeployedService>, String>;
    async fn deploy_plan(&self, plan: DeploymentPlan) -> Result<(), String>;
}
use syneroym_core::local_registry::{EndpointRegistry, SubstrateEndpoint};
use syneroym_data_layer::traits::StorageProvider;
use syneroym_key_store::KeyStore;
use syneroym_rpc::{NativeInvocation, NativeResponse, NativeService, RpcError, RpcResult};
use tracing::info;

use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

const ORCHESTRATOR_INTERFACE: &str = "orchestrator";
const SECURITY_INTERFACE: &str = "security";

/// The Substrate Service (The Control Plane Orchestrator)
/// This service handles the deployment and lifecycle of applications
/// (`SynApps`) within the substrate. It interacts with sandbox environments
/// like Podman or Wasmtime.
pub struct ControlPlaneService {
    service_id: String,
    registry: EndpointRegistry,
    app_sandbox_engine: Arc<AppSandboxEngine>,
    podman_sandbox_engine: Arc<ContainerEngine>,
    hosted_apps_dir: PathBuf,
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
}

impl Debug for ControlPlaneService {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlPlaneService")
            .field("service_id", &self.service_id)
            .finish_non_exhaustive()
    }
}

impl ControlPlaneService {
    pub async fn init(
        service_id: String,
        app_sandbox_engine: Arc<AppSandboxEngine>,
        podman_sandbox_engine: Arc<ContainerEngine>,
        registry: EndpointRegistry,
        hosted_apps_dir: PathBuf,
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
    ) -> Result<Self> {
        info!("Initializing ControlPlaneService (Orchestrator)...");

        if !hosted_apps_dir.exists() {
            fs::create_dir_all(&hosted_apps_dir)?;
        }

        Ok(Self {
            service_id,
            registry,
            app_sandbox_engine,
            podman_sandbox_engine,
            hosted_apps_dir,
            key_store,
            storage_provider,
        })
    }
}

#[async_trait::async_trait]
impl NativeService for ControlPlaneService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        info!("Orchestrator received dispatch: {}.{}", invocation.interface, invocation.method);

        if invocation.interface.as_str() == SECURITY_INTERFACE {
            // Slice 2A keeps KEK and secret mutations on the substrate native
            // management interface. Full remote authorization with UCAN/FDAE is
            // deferred to M4.
            match invocation.method.as_str() {
                "inject-kek" => {
                    let (kek_hex,): (String,) =
                        serde_json::from_value(invocation.params).map_err(|e| {
                            RpcError::InvalidParams(format!(
                                "Failed to parse inject-kek params: {e}"
                            ))
                        })?;
                    let kek_bytes = hex::decode(kek_hex)
                        .map_err(|e| RpcError::InvalidParams(format!("Invalid hex KEK: {e}")))?;
                    if kek_bytes.len() != 32 {
                        return Err(RpcError::InvalidParams(
                            "KEK must be exactly 32 bytes".to_string(),
                        ));
                    }
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&kek_bytes);
                    self.key_store
                        .inject_kek(arr, None)
                        .map_err(|e| RpcError::InternalError(e.to_string()))?;
                    return Ok(NativeResponse {
                        payload: serde_json::json!({"status": "injected"}),
                    });
                }
                "rotate-kek" => {
                    let (new_kek_hex,): (String,) = serde_json::from_value(invocation.params)
                        .map_err(|e| {
                            RpcError::InvalidParams(format!(
                                "Failed to parse rotate-kek params: {e}"
                            ))
                        })?;
                    let new_kek_bytes = hex::decode(new_kek_hex)
                        .map_err(|e| RpcError::InvalidParams(format!("Invalid hex KEK: {e}")))?;
                    if new_kek_bytes.len() != 32 {
                        return Err(RpcError::InvalidParams(
                            "New KEK must be exactly 32 bytes".to_string(),
                        ));
                    }
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&new_kek_bytes);
                    self.storage_provider
                        .rotate_kek(&self.key_store, arr)
                        .await
                        .map_err(|e| RpcError::InternalError(e.to_string()))?;
                    return Ok(NativeResponse {
                        payload: serde_json::json!({"status": "rotated"}),
                    });
                }
                "set-secret" => {
                    let (service_id, key, value): (String, String, Vec<u8>) =
                        serde_json::from_value(invocation.params).map_err(|e| {
                            RpcError::InvalidParams(format!(
                                "Failed to parse set-secret params: {e}"
                            ))
                        })?;
                    let store = self
                        .storage_provider
                        .open_service_db(&service_id, &self.key_store)
                        .await
                        .map_err(|e| RpcError::InternalError(e.to_string()))?;
                    store
                        .write_secret(&key, &value)
                        .await
                        .map_err(|e| RpcError::InternalError(e.to_string()))?;
                    return Ok(NativeResponse {
                        payload: serde_json::json!({"status": "secret_set"}),
                    });
                }
                _ => {
                    return Err(RpcError::MethodNotFound(invocation.method));
                }
            }
        }

        if invocation.interface.as_str() != ORCHESTRATOR_INTERFACE {
            return Err(RpcError::InternalError(format!(
                "Interface {} not handled by orchestrator",
                invocation.interface
            )));
        }

        match invocation.method.as_str() {
            "readyz" => {
                let service_id = serde_json::from_value::<(String,)>(invocation.params.clone())
                    .map(|(s,)| s)
                    .or_else(|_| serde_json::from_value::<String>(invocation.params.clone()))
                    .or_else(|_| {
                        #[derive(serde::Deserialize)]
                        struct ReadyzPayload {
                            #[serde(alias = "service-id")]
                            service_id: String,
                        }
                        serde_json::from_value::<ReadyzPayload>(invocation.params)
                            .map(|p| p.service_id)
                    })
                    .unwrap_or_default();
                self.readyz(service_id).await.map_err(RpcError::InternalError)?;
                Ok(ready_response())
            }
            "deploy" => {
                let (service_id, manifest): (String, DeployManifest) =
                    serde_json::from_value(invocation.params).map_err(|e| {
                        RpcError::InvalidParams(format!("Failed to parse deploy params: {e}"))
                    })?;
                self.deploy(service_id, manifest).await.map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "deployed"}) })
            }
            "deploy-plan" => {
                let (plan,): (DeploymentPlan,) = serde_json::from_value(invocation.params.clone())
                    .or_else(|_| {
                        serde_json::from_value::<DeploymentPlan>(invocation.params).map(|p| (p,))
                    })
                    .map_err(|e| {
                        RpcError::InvalidParams(format!("Failed to parse deploy-plan params: {e}"))
                    })?;
                self.deploy_plan(plan).await.map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "deployed_plan"}) })
            }
            "undeploy" => {
                let (service_id,): (String,) = serde_json::from_value(invocation.params.clone())
                    .or_else(|_| serde_json::from_value::<String>(invocation.params).map(|s| (s,)))
                    .map_err(|e| {
                        RpcError::InvalidParams(format!("Failed to parse undeploy params: {e}"))
                    })?;
                self.undeploy(service_id).await.map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "undeployed"}) })
            }
            "list" => {
                let services = self.list().await.map_err(RpcError::InternalError)?;
                Ok(NativeResponse {
                    payload: serde_json::to_value(services).unwrap_or(serde_json::Value::Null),
                })
            }
            method => Err(RpcError::MethodNotFound(method.to_string())),
        }
    }
}

impl ControlPlaneService {
    async fn register_wasm_endpoints(
        &self,
        service_id: &str,
        interfaces: Vec<String>,
    ) -> Result<()> {
        for interface in interfaces {
            self.registry
                .register(
                    service_id.to_string(),
                    interface,
                    SubstrateEndpoint::WasmChannel { service_id: service_id.to_string() },
                )
                .await?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl OrchestratorInterface for ControlPlaneService {
    async fn readyz(&self, service_id: String) -> Result<(), String> {
        if !service_id.is_empty() {
            let endpoints = self.registry.lookup_by_service(&service_id);
            let mut is_container = false;
            for (_, endpoint) in endpoints {
                if matches!(endpoint, SubstrateEndpoint::TcpHostPort { .. }) {
                    is_container = true;
                    break;
                }
            }
            if is_container {
                self.podman_sandbox_engine
                    .readyz(&service_id)
                    .await
                    .map_err(|e| format!("Container readiness check failed: {e}"))?;
            }
        }
        Ok(())
    }

    async fn deploy(&self, service_id: String, manifest: DeployManifest) -> Result<(), String> {
        if let Some(cert) = &manifest.registry_certificate {
            let cert_path = self.hosted_apps_dir.join(format!("{service_id}.json"));
            if let Err(e) = fs::write(&cert_path, cert) {
                tracing::warn!("Failed to save registry certificate for {}: {}", service_id, e);
            } else {
                tracing::debug!(
                    "Saved registry certificate for {} at {}",
                    service_id,
                    cert_path.display()
                );
            }
        }

        // Configuration Generation & Validation
        let mut flat_config = std::collections::BTreeMap::new();
        if let Some(custom_config_str) = &manifest.config.custom_config {
            let custom_json: serde_json::Value = serde_json::from_str(custom_config_str)
                .map_err(|e| format!("custom_config is not valid JSON: {}", e))?;

            if let Some(schema_path_str) = &manifest.config.schema_path {
                let schema_path = std::path::PathBuf::from(schema_path_str);

                // Path traversal check
                if schema_path.components().any(|c| matches!(c, std::path::Component::ParentDir))
                    || schema_path.is_absolute()
                {
                    return Err(format!(
                        "Arbitrary file read prevented: Path traversal or absolute paths are not \
                         allowed in schema_path: {:?}",
                        schema_path
                    ));
                }

                let custom_json_clone = custom_json.clone();
                tokio::task::spawn_blocking(move || -> Result<(), String> {
                    let schema_str = std::fs::read_to_string(&schema_path).map_err(|e| {
                        format!("Failed to read JSON schema at {}: {}", schema_path.display(), e)
                    })?;
                    let schema_json: serde_json::Value = serde_json::from_str(&schema_str)
                        .map_err(|e| format!("JSON schema is not valid JSON: {}", e))?;

                    let compiled_schema = jsonschema::validator_for(&schema_json)
                        .map_err(|e| format!("Invalid JSON schema: {}", e))?;

                    if let Err(error) = compiled_schema.validate(&custom_json_clone) {
                        return Err(format!(
                            "Configuration validation failed: {} at {}",
                            error,
                            error.instance_path()
                        ));
                    }
                    Ok(())
                })
                .await
                .map_err(|e| format!("Failed to spawn blocking task: {}", e))??;
            }

            crate::config_utils::flatten_json_config(&custom_json, "", &mut flat_config);
        }

        let config_blob = serde_json::to_string(&flat_config)
            .map_err(|e| format!("Failed to serialize flattened config: {}", e))?;

        let new_gen = self
            .storage_provider
            .save_config_generation(&service_id, &config_blob)
            .await
            .map_err(|e| format!("Failed to save config generation: {}", e))?;
        tracing::info!("Saved configuration generation {} for service {}", new_gen, service_id);

        match &manifest.service_type {
            WitServiceType::Wasm(wasm_manifest) => {
                if let Err(e) = self.app_sandbox_engine.deploy_wasm(&service_id, &manifest).await {
                    if let Err(rollback_err) =
                        self.storage_provider.delete_config_generation(&service_id, new_gen).await
                    {
                        tracing::error!(
                            "Failed to rollback config generation {} for service {} after deploy \
                             error: {}",
                            new_gen,
                            service_id,
                            rollback_err
                        );
                    }
                    return Err(format!("WASM deployment failed: {e}"));
                }

                self.register_wasm_endpoints(&service_id, wasm_manifest.interfaces.clone())
                    .await
                    .map_err(|e| format!("Endpoint registration failed: {e}"))?;
            }
            WitServiceType::Tcp(tcp_manifest) => {
                for endpoint in &tcp_manifest.endpoints {
                    info!(
                        "Deploying TCP service {} endpoint {}: {}:{}",
                        service_id, endpoint.interface_name, endpoint.host, endpoint.port
                    );
                    self.registry
                        .register(
                            service_id.clone(),
                            endpoint.interface_name.clone(),
                            SubstrateEndpoint::TcpHostPort {
                                host: endpoint.host.clone(),
                                port: endpoint.port,
                            },
                        )
                        .await
                        .map_err(|e| format!("Endpoint registration failed: {e}"))?;
                }
            }
            WitServiceType::Container(container_manifest) => {
                info!(
                    "Deploying container service {}: image={}",
                    service_id, container_manifest.image
                );
                let actual_mappings =
                    match self.podman_sandbox_engine.deploy(&service_id, &manifest).await {
                        Ok(mappings) => mappings,
                        Err(e) => {
                            if let Err(rollback_err) = self
                                .storage_provider
                                .delete_config_generation(&service_id, new_gen)
                                .await
                            {
                                tracing::error!(
                                    "Failed to rollback config generation {} for service {} after \
                                     deploy error: {}",
                                    new_gen,
                                    service_id,
                                    rollback_err
                                );
                            }
                            return Err(format!("Container deployment failed: {e}"));
                        }
                    };

                for (interface_name, host_port) in actual_mappings {
                    self.registry
                        .register(
                            service_id.clone(),
                            interface_name,
                            SubstrateEndpoint::TcpHostPort {
                                host: "127.0.0.1".to_string(),
                                port: host_port,
                            },
                        )
                        .await
                        .map_err(|e| format!("Endpoint registration failed: {e}"))?;
                }
            }
        }

        Ok(())
    }

    async fn undeploy(&self, service_id: String) -> Result<(), String> {
        info!("Undeploying service: {}", service_id);

        let cert_path = self.hosted_apps_dir.join(format!("{service_id}.json"));
        if cert_path.exists()
            && let Err(e) = fs::remove_file(&cert_path)
        {
            tracing::warn!("Failed to remove registry certificate for {}: {}", service_id, e);
        }

        let endpoints = self.registry.lookup_by_service(&service_id);
        let mut is_wasm = false;
        let mut is_container = false;

        for (interface_name, endpoint) in endpoints {
            if matches!(endpoint, SubstrateEndpoint::WasmChannel { .. }) {
                is_wasm = true;
            } else if matches!(endpoint, SubstrateEndpoint::TcpHostPort { .. }) {
                is_container = true;
            }
            if let Err(e) = self.registry.remove(&service_id, &interface_name).await {
                tracing::warn!(
                    "Failed to remove endpoint {} for service {}: {}",
                    interface_name,
                    service_id,
                    e
                );
            }
        }

        if is_wasm {
            if let Err(e) = self.app_sandbox_engine.stop_wasm(&service_id).await {
                tracing::warn!("Failed to stop WASM engine for service {}: {}", service_id, e);
            }
            if let Err(e) = self.app_sandbox_engine.remove_wasm(&service_id).await {
                tracing::warn!("Failed to remove WASM file for service {}: {}", service_id, e);
            }
        }

        if is_container {
            if let Err(e) = self.podman_sandbox_engine.stop(&service_id).await {
                tracing::warn!("Failed to stop Container engine for service {}: {}", service_id, e);
            }
            if let Err(e) = self.podman_sandbox_engine.remove(&service_id).await {
                tracing::warn!("Failed to remove Container for service {}: {}", service_id, e);
            }
        }

        Ok(())
    }

    async fn list(&self) -> Result<Vec<DeployedService>, String> {
        let endpoints = self.registry.get_all_endpoints();
        let mut services: HashMap<String, DeployedService> = HashMap::new();

        for (service_id, interface, endpoint) in endpoints {
            let entry = services.entry(service_id.clone()).or_insert_with(|| DeployedService {
                service_id: service_id.clone(),
                interfaces: Vec::new(),
                endpoint_type: match endpoint {
                    SubstrateEndpoint::WasmChannel { .. } => "wasm".to_string(),
                    SubstrateEndpoint::PodmanSocket { .. } => "podman".to_string(),
                    SubstrateEndpoint::NativeHostChannel { .. } => "native".to_string(),
                    SubstrateEndpoint::TcpHostPort { .. } => "tcp".to_string(),
                },
            });
            entry.interfaces.push(interface);
        }

        let mut result: Vec<DeployedService> = services.into_values().collect();
        result.sort_by(|a, b| a.service_id.cmp(&b.service_id));

        Ok(result)
    }

    async fn deploy_plan(&self, plan: DeploymentPlan) -> Result<(), String> {
        for service in plan.services {
            let service_id = service.service_id.clone();

            // Only allow WASM sources that do not use path traversal and stay within an
            // allowed directory Note: Since deploy-plan is handled over RPC, we
            // restrict file source reads to the current directory
            // or an explicit sandbox.
            let mut deploy_manifest = service.manifest.clone();

            match &mut deploy_manifest.service_type {
                WitServiceType::Wasm(wasm_manifest) => {
                    if let ArtifactSource::Binary(_) = &wasm_manifest.source {
                        // Binary is fine, it was passed directly
                    } else if let ArtifactSource::Url(url_or_path) = &wasm_manifest.source
                        && !url_or_path.starts_with("http://")
                        && !url_or_path.starts_with("https://")
                    {
                        // It's a local file path
                        let path = std::path::PathBuf::from(url_or_path);

                        // Path traversal check
                        if path.components().any(|c| matches!(c, std::path::Component::ParentDir))
                            || path.is_absolute()
                        {
                            return Err(format!(
                                "Arbitrary file read prevented: Path traversal or absolute paths \
                                 are not allowed in deploy-plan: {:?}",
                                path
                            ));
                        }

                        let bytes =
                            syneroym_core::util::read_local_artifact(&path).map_err(|e| {
                                format!("Failed to read WASM file at {:?}: {}", path, e)
                            })?;
                        wasm_manifest.source = ArtifactSource::Binary(bytes);
                    }
                }
                WitServiceType::Tcp(_) | WitServiceType::Container(_) => {
                    // TCP and Container don't read host files directly in
                    // deploy_plan logic for sources
                }
            }

            self.deploy(service_id, deploy_manifest).await?;
        }

        Ok(())
    }
}

fn ready_response() -> NativeResponse {
    NativeResponse { payload: serde_json::json!({"status": "ok"}) }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
        ServiceConfig, WasmManifest,
    };
    use wit_parser::Resolve;

    use super::*;

    #[tokio::test]
    async fn test_wit_adherence() {
        let wit_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../bindings/wit/control-plane/control-plane.wit");

        let mut resolve = Resolve::default();
        let content = std::fs::read_to_string(&wit_path).expect("Failed to read WIT file");
        let group = wit_parser::UnresolvedPackageGroup::parse(&wit_path, &content)
            .expect("Failed to parse WIT file");
        let pkg = group.main;
        let pkg_id = resolve.push(pkg, 0).expect("Failed to resolve package");

        let package = &resolve.packages[pkg_id];
        let interface_id = package
            .interfaces
            .get("orchestrator")
            .copied()
            .expect("orchestrator interface not found in WIT");

        let interface = &resolve.interfaces[interface_id];

        let temp_dir = tempfile::tempdir().unwrap();
        let config = syneroym_core::config::SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider = Arc::new(
            syneroym_data_layer::SqliteStorageProvider::new(temp_dir.path(), false).unwrap(),
        );
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(&config, vec![], key_store.clone(), storage_provider.clone())
                .await
                .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry =
            EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));

        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
        )
        .await
        .unwrap();

        assert!(!interface.functions.is_empty(), "Orchestrator interface should have functions");

        for (name, _func) in &interface.functions {
            let method_name = name.strip_prefix('%').unwrap_or(name);

            let invocation = NativeInvocation {
                interface: "orchestrator".to_string(),
                method: method_name.to_string(),
                params: serde_json::Value::Null,
            };

            let res = service.dispatch(invocation).await;
            if let Err(RpcError::MethodNotFound(m)) = res {
                panic!(
                    "WIT function '{}' maps to method name '{}' but was NOT found in dispatcher",
                    name, m
                );
            }
        }
    }

    #[tokio::test]
    async fn test_deploy_plan_path_traversal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = syneroym_core::config::SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider = Arc::new(
            syneroym_data_layer::SqliteStorageProvider::new(temp_dir.path(), false).unwrap(),
        );
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(&config, vec![], key_store.clone(), storage_provider.clone())
                .await
                .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry =
            EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));

        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
        )
        .await
        .unwrap();

        // Create a deployment plan with path traversal in source
        let plan = DeploymentPlan {
            app_instance_id: "test-instance".to_string(),
            blueprint_id: "test-blueprint".to_string(),
            version: "0.1.0".to_string(),
            services: vec![syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::PlannedService {
                service_id: "did:key:test".to_string(),
                logical_ref: "test/main".to_string(),
                manifest: DeployManifest {
                    config: ServiceConfig { env: vec![], args: vec![], custom_config: None, quota: None, schema_path: None, rotation_policy: None },
                    service_type: WitServiceType::Wasm(WasmManifest {
                        source: ArtifactSource::Url("../../../../../etc/passwd".to_string()),
                        hash: None,
                        interfaces: vec![],
                    }),
                    registry_certificate: None,
                },
            }],
        };

        let result = service.deploy_plan(plan).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Arbitrary file read prevented: Path traversal"));
    }

    #[tokio::test]
    async fn test_deploy_plan_absolute_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = syneroym_core::config::SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider = Arc::new(
            syneroym_data_layer::SqliteStorageProvider::new(temp_dir.path(), false).unwrap(),
        );
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(&config, vec![], key_store.clone(), storage_provider.clone())
                .await
                .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry =
            EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));

        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
        )
        .await
        .unwrap();

        let plan = DeploymentPlan {
            app_instance_id: "test-instance".to_string(),
            blueprint_id: "test-blueprint".to_string(),
            version: "0.1.0".to_string(),
            services: vec![syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::PlannedService {
                service_id: "did:key:test".to_string(),
                logical_ref: "test/main".to_string(),
                manifest: DeployManifest {
                    config: ServiceConfig { env: vec![], args: vec![], custom_config: None, quota: None, schema_path: None, rotation_policy: None },
                    service_type: WitServiceType::Wasm(WasmManifest {
                        source: ArtifactSource::Url("/etc/passwd".to_string()),
                        hash: None,
                        interfaces: vec![],
                    }),
                    registry_certificate: None,
                },
            }],
        };

        let result = service.deploy_plan(plan).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Arbitrary file read prevented: Path traversal or absolute paths")
        );
    }

    #[tokio::test]
    async fn test_security_dispatch_returns_sdk_statuses() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = syneroym_core::config::SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider = Arc::new(
            syneroym_data_layer::SqliteStorageProvider::new(temp_dir.path(), false).unwrap(),
        );
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(&config, vec![], key_store.clone(), storage_provider.clone())
                .await
                .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry =
            EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));

        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
        )
        .await
        .unwrap();

        let kek = hex::encode([1u8; 32]);
        let inject_res = service
            .dispatch(NativeInvocation {
                interface: "security".to_string(),
                method: "inject-kek".to_string(),
                params: serde_json::to_value((kek,)).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(inject_res.payload, serde_json::json!({"status": "injected"}));

        let new_kek = hex::encode([2u8; 32]);
        let rotate_res = service
            .dispatch(NativeInvocation {
                interface: "security".to_string(),
                method: "rotate-kek".to_string(),
                params: serde_json::to_value((new_kek,)).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(rotate_res.payload, serde_json::json!({"status": "rotated"}));

        let secret_res = service
            .dispatch(NativeInvocation {
                interface: "security".to_string(),
                method: "set-secret".to_string(),
                params: serde_json::to_value((
                    "profile-store".to_string(),
                    "api_key".to_string(),
                    b"secret-from-stdin".to_vec(),
                ))
                .unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(secret_res.payload, serde_json::json!({"status": "secret_set"}));
    }
    #[tokio::test]
    async fn test_deploy_config_schema_rejection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = syneroym_core::config::SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider = Arc::new(
            syneroym_data_layer::SqliteStorageProvider::new(temp_dir.path(), false).unwrap(),
        );
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(&config, vec![], key_store.clone(), storage_provider.clone())
                .await
                .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry =
            EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));

        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
        )
        .await
        .unwrap();

        // Write a schema file with a relative path
        let schema_filename = format!("test_schema_{}.json", std::process::id());
        std::fs::write(
            &schema_filename,
            r#"{"type": "object", "properties": {"port": {"type": "integer"}}}"#,
        )
        .unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: Some(r#"{"port": "8080"}"#.to_string()), // string instead of int
                quota: None,
                schema_path: Some(schema_filename.clone()),
                rotation_policy: None,
            },
            service_type: WitServiceType::Tcp(syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
        };

        let result = service.deploy("test_service".to_string(), manifest).await;

        let _ = std::fs::remove_file(&schema_filename);

        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(err_msg.contains("Configuration validation failed"), "{}", err_msg);
    }

    #[tokio::test]
    async fn test_deploy_config_generation_rollback() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = syneroym_core::config::SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider = Arc::new(
            syneroym_data_layer::SqliteStorageProvider::new(temp_dir.path(), false).unwrap(),
        );
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(&config, vec![], key_store.clone(), storage_provider.clone())
                .await
                .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry =
            EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));

        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
        )
        .await
        .unwrap();

        // Deliberately malformed WasmManifest source to cause a deployment failure
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: Some(r#"{"key": "value"}"#.to_string()),
                quota: None,
                schema_path: None,
                rotation_policy: None,
            },
            service_type: WitServiceType::Wasm(syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::WasmManifest {
                source: syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::ArtifactSource::Url("/does_not_exist.wasm".to_string()),
                hash: None,
                interfaces: vec![],
            }),
            registry_certificate: None,
        };

        let result = service.deploy("rollback_service".to_string(), manifest).await;
        assert!(result.is_err()); // deployment must fail

        // Config generation should not exist
        let latest =
            storage_provider.get_latest_config_generation("rollback_service").await.unwrap();
        assert!(latest.is_none());
    }
}

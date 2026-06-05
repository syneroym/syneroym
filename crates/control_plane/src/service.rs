//! Orchestrator control service implementation
//!
//! Handles requests for registering, deploying, listing, and destroying
//! sandbox instances or services running on the node.

use std::{collections::HashMap, fmt, fs, path::PathBuf, sync::Arc};

use anyhow::Result;
use fmt::{Debug, Formatter};
use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
    DeployManifest, DeployedService, ServiceType,
};

#[async_trait::async_trait]
pub trait OrchestratorInterface {
    async fn readyz(&self, service_id: String) -> Result<(), String>;
    async fn deploy(&self, service_id: String, manifest: DeployManifest) -> Result<(), String>;
    async fn undeploy(&self, service_id: String) -> Result<(), String>;
    async fn list(&self) -> Result<Vec<DeployedService>, String>;
}
use syneroym_core::local_registry::{EndpointRegistry, SubstrateEndpoint};
use syneroym_rpc::{NativeInvocation, NativeResponse, NativeService, RpcError, RpcResult};
use tracing::info;

use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

const ORCHESTRATOR_INTERFACE: &str = "orchestrator";

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
        })
    }
}

#[async_trait::async_trait]
impl NativeService for ControlPlaneService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        info!("Orchestrator received dispatch: {}.{}", invocation.interface, invocation.method);

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

        match &manifest.service_type {
            ServiceType::Wasm(wasm_manifest) => {
                self.app_sandbox_engine
                    .deploy_wasm(&service_id, &manifest)
                    .await
                    .map_err(|e| format!("WASM deployment failed: {e}"))?;

                self.register_wasm_endpoints(&service_id, wasm_manifest.interfaces.clone())
                    .await
                    .map_err(|e| format!("Endpoint registration failed: {e}"))?;
            }
            ServiceType::Tcp(tcp_manifest) => {
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
            ServiceType::Container(container_manifest) => {
                info!(
                    "Deploying container service {}: image={}",
                    service_id, container_manifest.image
                );
                let actual_mappings = self
                    .podman_sandbox_engine
                    .deploy(&service_id, &manifest)
                    .await
                    .map_err(|e| format!("Container deployment failed: {e}"))?;

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
}

fn ready_response() -> NativeResponse {
    NativeResponse { payload: serde_json::json!({"status": "ok"}) }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use wit_parser::Resolve;

    use super::*;

    #[tokio::test]
    async fn test_wit_adherence() {
        let wit_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../bindings/wit/control-plane.wit");

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
        let app_sandbox = Arc::new(AppSandboxEngine::init(&config, vec![]).await.unwrap());
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path()));
        let registry =
            EndpointRegistry::new_mock(Arc::new(syneroym_core::storage::MockStorage::new()));
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
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
}

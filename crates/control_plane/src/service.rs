//! Orchestrator control service implementation
//!
//! Handles requests for registering, deploying, listing, and destroying
//! sandbox instances or services running on the node.

use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};
use anyhow::Result;
use std::fmt;
use std::sync::Arc;
use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
    DeployManifest, ServiceType,
};
use syneroym_core::registry::{EndpointRegistry, SubstrateEndpoint};
use syneroym_rpc::{NativeInvocation, NativeResponse, NativeService, RpcError, RpcResult};
use tracing::info;

const ORCHESTRATOR_INTERFACE: &str = "orchestrator";

/// The Substrate Service (The Control Plane Orchestrator)
/// This service handles the deployment and lifecycle of applications (`SynApps`)
/// within the substrate. It interacts with sandbox environments like Podman or Wasmtime.
pub struct ControlPlaneService {
    service_id: String,
    registry: EndpointRegistry,
    app_sandbox_engine: Arc<AppSandboxEngine>,
    podman_sandbox_engine: Arc<ContainerEngine>,
}

impl fmt::Debug for ControlPlaneService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    ) -> Result<Self> {
        info!("Initializing ControlPlaneService (Orchestrator)...");

        Ok(Self { service_id, registry, app_sandbox_engine, podman_sandbox_engine })
    }
}

#[async_trait::async_trait]
impl NativeService for ControlPlaneService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        info!("Orchestrator received dispatch: {}.{}", invocation.interface, invocation.method);

        match (invocation.interface.as_str(), invocation.method.as_str()) {
            (ORCHESTRATOR_INTERFACE, "readyz") => self.readyz(invocation.params).await,
            (ORCHESTRATOR_INTERFACE, "deploy") => self.deploy(invocation.params).await,
            (ORCHESTRATOR_INTERFACE, "undeploy") => self.undeploy(invocation.params).await,
            (ORCHESTRATOR_INTERFACE, "list") => self.list().await,
            (ORCHESTRATOR_INTERFACE, _) => Err(RpcError::MethodNotFound(invocation.method.clone())),
            _ => Err(RpcError::InternalError(format!(
                "Interface {} not handled by orchestrator",
                invocation.interface
            ))),
        }
    }
}

impl ControlPlaneService {
    async fn readyz(&self, params: serde_json::Value) -> RpcResult<NativeResponse> {
        let (service_id,): (String,) = serde_json::from_value(params.clone())
            .or_else(|_| serde_json::from_value::<String>(params.clone()).map(|s| (s,)))
            .unwrap_or((String::new(),));

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
                self.podman_sandbox_engine.readyz(&service_id).await.map_err(|e| {
                    RpcError::InternalError(format!("Container readiness check failed: {e}"))
                })?;
            }
        }
        Ok(ready_response())
    }

    async fn deploy(&self, params: serde_json::Value) -> RpcResult<NativeResponse> {
        // NOTE: We use a positional tuple for parameters here because WASM component-model
        // metadata often strips argument names during compilation, making named parameter
        // matching unreliable for cross-platform toolchains. Positional parameters ensure
        // consistent behavior across all guest environments.
        let (service_id, interfaces, manifest): (String, Vec<String>, DeployManifest) =
            serde_json::from_value(params).map_err(|e| {
                RpcError::InvalidParams(format!("Failed to parse deploy params: {e}"))
            })?;

        match &manifest.service_type {
            ServiceType::Wasm(_) => {
                self.app_sandbox_engine
                    .deploy_wasm(&service_id, &manifest)
                    .await
                    .map_err(|e| RpcError::InternalError(format!("WASM deployment failed: {e}")))?;

                self.register_wasm_endpoints(&service_id, interfaces).await.map_err(|e| {
                    RpcError::InternalError(format!("Endpoint registration failed: {e}"))
                })?;
            }
            ServiceType::Tcp(manifest) => {
                info!("Deploying TCP service {}: {}:{}", service_id, manifest.host, manifest.port);
                for interface in interfaces {
                    self.registry
                        .register(
                            service_id.clone(),
                            interface,
                            SubstrateEndpoint::TcpHostPort {
                                host: manifest.host.clone(),
                                port: manifest.port,
                            },
                        )
                        .await
                        .map_err(|e| {
                            RpcError::InternalError(format!("Endpoint registration failed: {e}"))
                        })?;
                }
            }
            ServiceType::Container(container_manifest) => {
                info!(
                    "Deploying container service {}: image={}",
                    service_id, container_manifest.image
                );
                let actual_mappings =
                    self.podman_sandbox_engine.deploy(&service_id, &manifest).await.map_err(
                        |e| RpcError::InternalError(format!("Container deployment failed: {e}")),
                    )?;

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
                        .map_err(|e| {
                            RpcError::InternalError(format!("Endpoint registration failed: {e}"))
                        })?;
                }
            }
        }

        Ok(NativeResponse { payload: serde_json::json!({"status": "deployed"}) })
    }

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

    async fn undeploy(&self, params: serde_json::Value) -> RpcResult<NativeResponse> {
        let (service_id,): (String,) = serde_json::from_value(params.clone())
            .or_else(|_| serde_json::from_value::<String>(params.clone()).map(|s| (s,)))
            .map_err(|e| {
                RpcError::InvalidParams(format!("Failed to parse undeploy params: {e}"))
            })?;

        info!("Undeploying service: {}", service_id);

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

        Ok(NativeResponse { payload: serde_json::json!({"status": "undeployed"}) })
    }

    async fn list(&self) -> RpcResult<NativeResponse> {
        let endpoints = self.registry.get_all_endpoints();
        let mut services: std::collections::HashMap<String, DeployedService> =
            std::collections::HashMap::new();

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

        Ok(NativeResponse {
            payload: serde_json::to_value(result).unwrap_or(serde_json::Value::Null),
        })
    }
}

#[derive(serde::Serialize)]
struct DeployedService {
    service_id: String,
    interfaces: Vec<String>,
    endpoint_type: String,
}

fn ready_response() -> NativeResponse {
    NativeResponse { payload: serde_json::json!({"status": "ok"}) }
}

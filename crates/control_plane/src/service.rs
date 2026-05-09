use crate::dummy_sandbox::AppSandboxEngine;
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
/// This service handles the deployment and lifecycle of applications (SynApps)
/// within the substrate. It interacts with sandbox environments like Podman or Wasmtime.
pub struct ControlPlaneService {
    service_id: String,
    registry: EndpointRegistry,
    app_sandbox_engine: Arc<AppSandboxEngine>,
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
        registry: EndpointRegistry,
    ) -> Result<Self> {
        info!("Initializing ControlPlaneService (Orchestrator)...");

        Ok(Self { service_id, registry, app_sandbox_engine })
    }
}

#[async_trait::async_trait]
impl NativeService for ControlPlaneService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        info!("Orchestrator received dispatch: {}.{}", invocation.interface, invocation.method);

        match (invocation.interface.as_str(), invocation.method.as_str()) {
            (ORCHESTRATOR_INTERFACE, "readyz") => Ok(ready_response()),
            (ORCHESTRATOR_INTERFACE, "deploy") => self.deploy(invocation.params).await,
            (ORCHESTRATOR_INTERFACE, _) => {
                Err(RpcError::MethodNotFound(invocation.method.to_string()))
            }
            _ => Err(RpcError::InternalError(format!(
                "Interface {} not handled by orchestrator",
                invocation.interface
            ))),
        }
    }
}

impl ControlPlaneService {
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
            _ => {
                return Err(RpcError::InvalidParams(
                    "Unsupported service type for deployment".to_string(),
                ));
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
}

fn ready_response() -> NativeResponse {
    NativeResponse { payload: serde_json::json!({"status": "ok"}) }
}

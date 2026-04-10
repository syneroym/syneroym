use crate::dummy_sandbox::AppSandboxEngine;
use anyhow::{Result, anyhow};
use std::fmt;
use std::sync::Arc;
use syneroym_bindings::control_plane::exports::syneroym::control_plane::orchestrator::{
    DeployManifest, ServiceType,
};
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::{EndpointRegistry, SubstrateEndpoint};
use syneroym_rpc::{NativeInvocation, NativeResponse, NativeService};
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
        config: &SubstrateConfig,
        registry: EndpointRegistry,
    ) -> Result<Self> {
        info!("Initializing ControlPlaneService (Orchestrator)...");
        let app_sandbox_engine =
            Arc::new(AppSandboxEngine::init(config, registry.get_all_endpoints()).await?);

        Ok(Self { service_id, registry, app_sandbox_engine })
    }
}

#[async_trait::async_trait]
impl NativeService for ControlPlaneService {
    async fn dispatch(&self, invocation: NativeInvocation) -> Result<NativeResponse> {
        info!("Orchestrator received dispatch: {}.{}", invocation.interface, invocation.method);

        match (invocation.interface.as_str(), invocation.method.as_str()) {
            (ORCHESTRATOR_INTERFACE, "readyz") => Ok(ready_response()),
            (ORCHESTRATOR_INTERFACE, "deploy") => self.deploy(invocation.params).await,
            _ => Err(anyhow!("Orchestrator dispatch logic is not fully implemented yet.")),
        }
    }
}

impl ControlPlaneService {
    async fn deploy(&self, params: serde_json::Value) -> Result<NativeResponse> {
        let (service_id, manifest): (String, DeployManifest) = serde_json::from_value(params)?;

        match &manifest.service_type {
            ServiceType::Wasm(_) => {
                self.app_sandbox_engine.deploy_wasm(&service_id, &manifest).await?;
                self.register_wasm_endpoint(&service_id).await?;
            }
            _ => return Err(anyhow!("Unsupported service type for deployment")),
        }

        Ok(NativeResponse { payload: serde_json::json!({"status": "deployed"}) })
    }

    async fn register_wasm_endpoint(&self, service_id: &str) -> Result<()> {
        // Deployed WASM services are currently exposed as wRPC-capable pass-through targets.
        self.registry
            .register(
                service_id.to_string(),
                ORCHESTRATOR_INTERFACE.to_string(),
                SubstrateEndpoint::WasmChannel { channel_details: service_id.to_string() },
            )
            .await
    }
}

fn ready_response() -> NativeResponse {
    NativeResponse { payload: serde_json::json!({"status": "ok"}) }
}

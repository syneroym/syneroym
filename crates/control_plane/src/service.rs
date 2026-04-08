use crate::dummy_sandbox::AppSandboxEngine;
use crate::wit_bindings::exports::syneroym::control_plane::orchestrator::DeployManifest;
use anyhow::{Result, anyhow};
use std::fmt;
use std::sync::Arc;
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::EndpointRegistry;
use syneroym_rpc::{NativeInvocation, NativeResponse, NativeService};
use tracing::info;

/// The Substrate Service (The Control Plane Orchestrator)
/// This service handles the deployment and lifecycle of applications (SynApps)
/// within the substrate. It interacts with sandbox environments like Podman or Wasmtime.
pub struct ControlPlaneService {
    _service_id: String,
    _config: SubstrateConfig,
    _registry: EndpointRegistry,
    _app_sandbox_engine: Arc<AppSandboxEngine>,
}

impl fmt::Debug for ControlPlaneService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlPlaneService")
            .field("service_id", &self._service_id)
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
        let _app_sandbox_engine =
            Arc::new(AppSandboxEngine::init(config, registry.get_all_endpoints()).await?);

        Ok(Self {
            _service_id: service_id,
            _config: config.clone(),
            _registry: registry,
            _app_sandbox_engine,
        })
    }
}

#[async_trait::async_trait]
impl NativeService for ControlPlaneService {
    async fn dispatch(&self, invocation: NativeInvocation) -> Result<NativeResponse> {
        info!("Orchestrator received dispatch: {}.{}", invocation.interface, invocation.method);

        // TODO: Here, we would parse the invocation and interact with the
        // AppSandboxEngine (e.g., Podman or Wasmtime) to deploy, stop,
        // or remove an application.
        match (invocation.interface.as_str(), invocation.method.as_str()) {
            ("orchestrator", "readyz") => {
                Ok(NativeResponse { payload: serde_json::json!({"status": "ok"}) })
            }
            ("orchestrator", "deploy") => {
                let _args: DeployManifest = serde_json::from_value(invocation.params)?;
                // Example: self.app_sandbox_engine.deploy_wasm(...).await?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "deployed"}) })
            }
            _ => Err(anyhow!("Orchestrator dispatch logic is not fully implemented yet.")),
        }
    }
}

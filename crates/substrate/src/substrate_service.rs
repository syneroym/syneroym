use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use syneroym_app_sandbox::AppSandboxEngine;
use syneroym_core::{
    config::SubstrateConfig,
    registry::{EndpointRegistry, SubstrateEndpoint},
};
use syneroym_router::{NativeInvocation, NativeResponse, NativeService};
use tracing::{info, warn};

/// SubstrateService: Native deployable service that handles
/// the `syneroym:substrate/substrate-service` WIT world capabilities.
pub struct SubstrateService {
    pub(crate) service_id: String,
    sandbox_engine: Arc<AppSandboxEngine>,
    registry: Arc<EndpointRegistry>,
}

impl SubstrateService {
    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    pub fn new(
        service_id: String,
        config: &SubstrateConfig,
        registry: Arc<EndpointRegistry>,
    ) -> Self {
        #[cfg(feature = "app_sandbox")]
        let sandbox_engine = Arc::new(syneroym_app_sandbox::AppSandboxEngine::new(config));

        Self { service_id, sandbox_engine, registry }
    }

    // Implementing the 'orchestrator' WIT interface natively:

    pub async fn deploy(&self, target_service_id: &str, manifest: &[u8]) -> Result<(), String> {
        info!("Orchestrator: deploying {}", target_service_id);
        // Dispatch to sandbox engine (Wasm or Podman based on manifest format, assumed wasm for stub)
        self.sandbox_engine
            .deploy_wasm(target_service_id, manifest)
            .await
            .map_err(|e| e.to_string())?;

        // After successful deploy, register the new service in registry
        let new_endpoint = SubstrateEndpoint::WasmChannel {
            channel_id: target_service_id.to_string(), // placeholder logic
        };
        self.registry
            .register(target_service_id.to_string(), new_endpoint)
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    pub async fn stop(&self, target_service_id: &str) -> Result<(), String> {
        info!("Orchestrator: stopping {}", target_service_id);
        self.sandbox_engine.stop_wasm(target_service_id).await.map_err(|e| e.to_string())
    }

    pub async fn remove(&self, target_service_id: &str) -> Result<(), String> {
        info!("Orchestrator: removing {}", target_service_id);
        self.sandbox_engine.remove_wasm(target_service_id).await.map_err(|e| e.to_string())?;

        self.registry.remove(target_service_id).await.map_err(|e| e.to_string())?;

        Ok(())
    }

    // Implementing the 'health' WIT interface natively:

    pub async fn ping(&self) -> Result<String, String> {
        Ok("pong".to_string())
    }

    fn parse_service_id(params: Option<Value>) -> anyhow::Result<String> {
        let params = params.unwrap_or(Value::Null);
        match params {
            Value::String(service_id) => Ok(service_id),
            Value::Object(map) => map
                .get("service_id")
                .or_else(|| map.get("service-id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| anyhow::anyhow!("Missing service_id parameter")),
            Value::Array(items) => items
                .first()
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| anyhow::anyhow!("Missing service_id parameter")),
            _ => Err(anyhow::anyhow!("Unsupported params format for service_id")),
        }
    }

    fn parse_deploy_params(params: Option<Value>) -> anyhow::Result<(String, String)> {
        let params = params.unwrap_or(Value::Null);
        match params {
            Value::Object(map) => {
                let service_id = map
                    .get("service_id")
                    .or_else(|| map.get("service-id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("Missing service_id parameter"))?
                    .to_string();
                let manifest =
                    map.get("manifest").and_then(Value::as_str).unwrap_or_default().to_string();
                Ok((service_id, manifest))
            }
            Value::Array(items) => {
                let service_id = items
                    .first()
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("Missing service_id parameter"))?
                    .to_string();
                let manifest = items.get(1).and_then(Value::as_str).unwrap_or_default().to_string();
                Ok((service_id, manifest))
            }
            _ => Err(anyhow::anyhow!("Unsupported params format for deploy")),
        }
    }
}

#[async_trait]
impl NativeService for SubstrateService {
    async fn dispatch(&self, invocation: NativeInvocation) -> anyhow::Result<NativeResponse> {
        let interface = &invocation.interface;
        let method = &invocation.method;
        let params = invocation.params;

        match (interface.as_str(), method.as_str()) {
            ("health", "ping") => self
                .ping()
                .await
                .map(|pong| NativeResponse { result: Value::String(pong) })
                .map_err(|e| anyhow::anyhow!(e)),
            ("orchestrator", "deploy") => {
                let (service_id, manifest) = Self::parse_deploy_params(params)?;
                self.deploy(&service_id, manifest.as_bytes())
                    .await
                    .map(|_| NativeResponse { result: Value::Null })
                    .map_err(|e| anyhow::anyhow!(e))
            }
            ("orchestrator", "stop") => {
                let service_id = Self::parse_service_id(params)?;
                self.stop(&service_id)
                    .await
                    .map(|_| NativeResponse { result: Value::Null })
                    .map_err(|e| anyhow::anyhow!(e))
            }
            ("orchestrator", "remove") => {
                let service_id = Self::parse_service_id(params)?;
                self.remove(&service_id)
                    .await
                    .map(|_| NativeResponse { result: Value::Null })
                    .map_err(|e| anyhow::anyhow!(e))
            }
            _ => {
                warn!("SubstrateService: Unknown interface method {}.{}", interface, method);
                Err(anyhow::anyhow!("Unknown interface method {}.{}", interface, method))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use syneroym_core::config::SubstrateConfig;

    #[tokio::test]
    async fn health_ping_dispatch_returns_pong() {
        let registry =
            Arc::new(EndpointRegistry::new("sqlite::memory:").await.expect("in-memory registry"));
        let service = Arc::new(SubstrateService::new(
            "substrate-test".to_string(),
            &SubstrateConfig::default(),
            registry,
        ));

        let invocation = NativeInvocation {
            interface: "health".to_string(),
            method: "ping".to_string(),
            params: None,
        };

        let response = service.dispatch(invocation).await.expect("health ping response");
        assert_eq!(response.result, json!("pong"));
    }
}

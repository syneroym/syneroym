//! Deploy/undeploy/list lifecycle for the orchestrator.
//!
//! Handles validating and applying a `DeployManifest` (wasm/container/tcp),
//! wiring up the native-capability endpoints and dispatch registration every
//! deployed service gets, and tearing all of that back down on undeploy.
//! Distinct from `service`'s own concern (`NativeService::dispatch`'s JSON-RPC
//! routing table and the KEK/secret management calls it handles directly).

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Component, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use serde_json::Value;
use syneroym_core::{local_registry::SubstrateEndpoint, util};
use syneroym_rpc::NativeService;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, ContainerManifest, DeployManifest, DeployedService, DeploymentPlan,
    ServiceType as WitServiceType, TcpManifest, WasmManifest,
};
use tokio::task;
use tracing::info;

use super::ControlPlaneService;
use crate::{config_utils, http_routes, synsvc_native::SynSvcNativeService};

#[async_trait::async_trait]
pub trait OrchestratorInterface {
    async fn readyz(&self, service_id: String) -> Result<(), String>;
    async fn deploy(&self, service_id: String, manifest: DeployManifest) -> Result<(), String>;
    async fn undeploy(&self, service_id: String) -> Result<(), String>;
    async fn list(&self) -> Result<Vec<DeployedService>, String>;
    async fn deploy_plan(&self, plan: DeploymentPlan) -> Result<(), String>;
}

// "http-native", not the bare "http": `roymctl svc deploy --interfaces http
// --tcp ...` is an existing, real convention for declaring a TCP/container
// service's own plain HTTP-serving interface (see
// `crates/substrate/tests/e2e/global-setup.ts`) -- reserving the bare
// "http" name here collided with it (registering this native-capability
// endpoint under the same interface name silently overwrote the app's own
// `TcpHostPort` registration, discovered via `mise run test:e2e` breaking
// end to end during Slice 7's own verification).
const NATIVE_CAPABILITY_INTERFACES: [&str; 6] =
    ["data-layer", "vault", "app-config", "blob-store", "messaging", "http-native"];

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

    /// Logs (but does not propagate) a failure to roll back a config
    /// generation saved just before a deploy that then failed. Best-effort:
    /// the deploy error itself is what gets returned to the caller.
    async fn rollback_config_generation(&self, service_id: &str, generation: u64) {
        if let Err(rollback_err) =
            self.storage_provider.delete_config_generation(service_id, generation).await
        {
            tracing::error!(
                "Failed to rollback config generation {} for service {} after deploy error: {}",
                generation,
                service_id,
                rollback_err
            );
        }
    }

    async fn deploy_wasm_service(
        &self,
        service_id: &str,
        manifest: &DeployManifest,
        wasm_manifest: &WasmManifest,
        new_gen: u64,
    ) -> Result<(), String> {
        if let Err(e) = self.app_sandbox_engine.deploy_wasm(service_id, manifest).await {
            self.rollback_config_generation(service_id, new_gen).await;
            return Err(format!("WASM deployment failed: {e}"));
        }

        self.register_wasm_endpoints(service_id, wasm_manifest.interfaces.clone())
            .await
            .map_err(|e| format!("Endpoint registration failed: {e}"))
    }

    async fn deploy_tcp_service(
        &self,
        service_id: &str,
        tcp_manifest: &TcpManifest,
    ) -> Result<(), String> {
        for endpoint in &tcp_manifest.endpoints {
            info!(
                "Deploying TCP service {} endpoint {}: {}:{}",
                service_id, endpoint.interface_name, endpoint.host, endpoint.port
            );
            self.registry
                .register(
                    service_id.to_string(),
                    endpoint.interface_name.clone(),
                    SubstrateEndpoint::TcpHostPort {
                        host: endpoint.host.clone(),
                        port: endpoint.port,
                    },
                )
                .await
                .map_err(|e| format!("Endpoint registration failed: {e}"))?;
        }
        Ok(())
    }

    async fn deploy_container_service(
        &self,
        service_id: &str,
        manifest: &DeployManifest,
        container_manifest: &ContainerManifest,
        new_gen: u64,
    ) -> Result<(), String> {
        info!("Deploying container service {}: image={}", service_id, container_manifest.image);
        let actual_mappings = match self.podman_sandbox_engine.deploy(service_id, manifest).await {
            Ok(mappings) => mappings,
            Err(e) => {
                self.rollback_config_generation(service_id, new_gen).await;
                return Err(format!("Container deployment failed: {e}"));
            }
        };

        for (interface_name, host_port) in actual_mappings {
            self.registry
                .register(
                    service_id.to_string(),
                    interface_name,
                    SubstrateEndpoint::TcpHostPort {
                        host: "127.0.0.1".to_string(),
                        port: host_port,
                    },
                )
                .await
                .map_err(|e| format!("Endpoint registration failed: {e}"))?;
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
        let mut flat_config = BTreeMap::new();
        // M3B Slice 7: `http_routes` is a reserved top-level key inside
        // `custom_config`'s JSON (see `crate::http_routes`) -- parsed here,
        // alongside the existing flatten step, since this is already the
        // one place `custom_config` gets interpreted rather than treated as
        // opaque. A malformed `http_routes` value fails deploy the same way
        // a schema violation does, rather than silently discarding routes.
        let mut http_routes = Vec::new();
        if let Some(custom_config_str) = &manifest.config.custom_config {
            let custom_json: Value = serde_json::from_str(custom_config_str)
                .map_err(|e| format!("custom_config is not valid JSON: {}", e))?;
            http_routes = http_routes::parse_http_routes(&custom_json)?;

            if let Some(schema_path_str) = &manifest.config.schema_path {
                let schema_path = PathBuf::from(schema_path_str);

                // Path traversal check
                if schema_path.components().any(|c| matches!(c, Component::ParentDir))
                    || schema_path.is_absolute()
                {
                    return Err(format!(
                        "Arbitrary file read prevented: Path traversal or absolute paths are not \
                         allowed in schema_path: {:?}",
                        schema_path
                    ));
                }

                let custom_json_clone = custom_json.clone();
                task::spawn_blocking(move || -> Result<(), String> {
                    let schema_str = fs::read_to_string(&schema_path).map_err(|e| {
                        format!("Failed to read JSON schema at {}: {}", schema_path.display(), e)
                    })?;
                    let schema_json: Value = serde_json::from_str(&schema_str)
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

            config_utils::flatten_json_config(&custom_json, "", &mut flat_config);
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
                self.deploy_wasm_service(&service_id, &manifest, wasm_manifest, new_gen).await?;
            }
            WitServiceType::Tcp(tcp_manifest) => {
                self.deploy_tcp_service(&service_id, tcp_manifest).await?;
            }
            WitServiceType::Container(container_manifest) => {
                self.deploy_container_service(&service_id, &manifest, container_manifest, new_gen)
                    .await?;
            }
        }

        // Data-layer/vault/app-config/blob-store access is a host-provided
        // capability orthogonal to how the service's own business logic
        // runs (wasm/container/tcp), so every deployed service gets a
        // native-callable channel for it regardless of type.
        for interface in NATIVE_CAPABILITY_INTERFACES {
            if let Err(e) = self
                .registry
                .register(
                    service_id.clone(),
                    interface.to_string(),
                    SubstrateEndpoint::NativeHostChannel { service_id: service_id.clone() },
                )
                .await
            {
                if let Err(undeploy_err) = self.undeploy(service_id.clone()).await {
                    tracing::error!(
                        "Failed to roll back partially deployed service {} after native \
                         capability registration error: {}",
                        service_id,
                        undeploy_err
                    );
                }
                self.rollback_config_generation(&service_id, new_gen).await;
                return Err(format!("Native capability registration failed: {e}"));
            }
        }
        if let Some(native_dispatch) = self.native_dispatch.upgrade() {
            native_dispatch.insert(
                service_id.clone(),
                Arc::new(SynSvcNativeService::new(
                    service_id.clone(),
                    self.key_store.clone(),
                    self.storage_provider.clone(),
                    self.blob_provider.clone(),
                    self.messaging_broker.clone(),
                )) as Arc<dyn NativeService>,
            );
        } else {
            tracing::error!(
                "Native dispatch registry unavailable for service {}: registered its native \
                 capability endpoints but could not insert a dispatch entry, so calls into them \
                 will fail",
                service_id
            );
        }
        if http_routes.is_empty() {
            self.http_routes.remove(&service_id);
        } else {
            self.http_routes.insert(service_id.clone(), http_routes);
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

        // Messaging subscriptions have no analogue among the other 4 native
        // capabilities: they're a long-lived stateful subsystem (persisted
        // rows plus live broker registrations), not pure request/response,
        // so they need an explicit "forget this service" step the
        // endpoint-registry loop above doesn't cover.
        if let Err(e) =
            self.storage_provider.delete_all_messaging_subscriptions_for_service(&service_id).await
        {
            tracing::warn!(
                "Failed to remove messaging subscriptions for service {}: {}",
                service_id,
                e
            );
        }
        if is_wasm {
            self.app_sandbox_engine.unsubscribe_all(&service_id);
        }

        // The endpoint-registry loop above already removed the 6 native
        // capability interfaces generically (it iterates every registered
        // interface for this service_id); just drop the in-memory dispatch
        // entry too.
        if let Some(native_dispatch) = self.native_dispatch.upgrade() {
            native_dispatch.remove(&service_id);
        } else {
            tracing::error!(
                "Native dispatch registry unavailable while undeploying service {}: its in-memory \
                 dispatch entry, if any, was left behind",
                service_id
            );
        }
        self.http_routes.remove(&service_id);

        Ok(())
    }

    async fn list(&self) -> Result<Vec<DeployedService>, String> {
        let endpoints = self.registry.get_all_endpoints();
        let mut services: HashMap<String, DeployedService> = HashMap::new();

        for (service_id, interface, endpoint) in endpoints {
            // The native-capability interfaces (data-layer/vault/app-config/
            // blob-store/messaging/http) are host-provided plumbing registered
            // on every deployed service regardless of type -- they must not be
            // mistaken for the service's own declared interfaces, nor
            // influence `endpoint_type` (every deployed service also always
            // has its real wasm/container/tcp endpoint registered).
            if NATIVE_CAPABILITY_INTERFACES.contains(&interface.as_str()) {
                continue;
            }
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
                        let path = PathBuf::from(url_or_path);

                        // Path traversal check
                        if path.components().any(|c| matches!(c, Component::ParentDir))
                            || path.is_absolute()
                        {
                            return Err(format!(
                                "Arbitrary file read prevented: Path traversal or absolute paths \
                                 are not allowed in deploy-plan: {:?}",
                                path
                            ));
                        }

                        let bytes = util::read_local_artifact(&path).map_err(|e| {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dashmap::DashMap;
    use syneroym_core::{
        config::SubstrateConfig, http_routes::HttpRouteRegistry, local_registry::EndpointRegistry,
        storage::MockStorage,
    };
    use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
    use syneroym_data_db::{SqliteStorageProvider, traits::StorageProvider};
    use syneroym_data_keystore::KeyStore;
    use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
    use syneroym_rpc::NativeDispatchRegistry;
    use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
        PlannedService, ServiceConfig,
    };

    use super::*;
    use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

    #[tokio::test]
    async fn test_deploy_plan_path_traversal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
        )
        .await
        .unwrap();

        // Create a deployment plan with path traversal in source
        let plan = DeploymentPlan {
            app_instance_id: "test-instance".to_string(),
            blueprint_id: "test-blueprint".to_string(),
            version: "0.1.0".to_string(),
            services: vec![PlannedService {
                service_id: "did:key:test".to_string(),
                logical_ref: "test/main".to_string(),
                manifest: DeployManifest {
                    config: ServiceConfig {
                        env: vec![],
                        args: vec![],
                        custom_config: None,
                        quota: None,
                        schema_path: None,
                        rotation_policy: None,
                    },
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
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
        )
        .await
        .unwrap();

        let plan = DeploymentPlan {
            app_instance_id: "test-instance".to_string(),
            blueprint_id: "test-blueprint".to_string(),
            version: "0.1.0".to_string(),
            services: vec![PlannedService {
                service_id: "did:key:test".to_string(),
                logical_ref: "test/main".to_string(),
                manifest: DeployManifest {
                    config: ServiceConfig {
                        env: vec![],
                        args: vec![],
                        custom_config: None,
                        quota: None,
                        schema_path: None,
                        rotation_policy: None,
                    },
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
    async fn test_deploy_config_schema_rejection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
        )
        .await
        .unwrap();

        // Write a schema file with a relative path
        let schema_filename = format!("test_schema_{}.json", std::process::id());
        fs::write(
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
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
        };

        let result = service.deploy("test_service".to_string(), manifest).await;

        let _ = fs::remove_file(&schema_filename);

        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(err_msg.contains("Configuration validation failed"), "{}", err_msg);
    }

    #[tokio::test]
    async fn test_deploy_config_generation_rollback() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
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
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Url("/does_not_exist.wasm".to_string()),
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

    /// M3B Slice 7: `deploy()` parses `http_routes` out of `custom_config`
    /// and populates the shared `HttpRouteRegistry` (the same `Arc` handed
    /// to `RouteHandlerInner` in production); `undeploy()` clears it. A TCP
    /// manifest is enough -- `http_routes` parsing/storage is independent
    /// of `service_type`.
    #[tokio::test]
    async fn test_http_routes_populated_on_deploy_and_cleared_on_undeploy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            native_dispatch,
            http_routes.clone(),
        )
        .await
        .unwrap();

        let service_id = "http-routes-svc".to_string();
        let custom_config = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/orders/{id}", "target": "data-layer",
                 "operation": "get", "collection": "orders"},
                {"method": "POST", "path": "/orders", "target": "data-layer",
                 "operation": "put", "collection": "orders"},
            ]
        })
        .to_string();
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: Some(custom_config),
                quota: None,
                schema_path: None,
                rotation_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
        };
        service.deploy(service_id.clone(), manifest).await.unwrap();

        let routes = http_routes.get(&service_id).expect("http_routes populated on deploy");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].collection.as_deref(), Some("orders"));
        drop(routes);

        service.undeploy(service_id.clone()).await.unwrap();
        assert!(
            http_routes.get(&service_id).is_none(),
            "http_routes entry must be removed on undeploy"
        );
    }

    /// M3B Slice 7: a service deployed with no `http_routes` key gets no
    /// entry in the shared registry at all (not an empty-`Vec` entry) --
    /// keeps the registry from growing with a no-op entry per ordinary
    /// deployed service.
    #[tokio::test]
    async fn test_no_http_routes_entry_when_custom_config_has_none() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            native_dispatch,
            http_routes.clone(),
        )
        .await
        .unwrap();

        let service_id = "no-http-routes-svc".to_string();
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema_path: None,
                rotation_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
        };
        service.deploy(service_id.clone(), manifest).await.unwrap();

        assert!(http_routes.get(&service_id).is_none());
    }
}

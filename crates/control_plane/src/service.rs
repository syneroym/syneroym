//! Orchestrator control service implementation
//!
//! Handles requests for registering, deploying, listing, and destroying
//! sandbox instances or services running on the node.

use std::{
    collections::{BTreeMap, HashMap},
    fmt, fs,
    path::{Component, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use fmt::{Debug, Formatter};
use serde_json::Value;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, ContainerManifest, DeployManifest, DeployedService, DeploymentPlan,
    ServiceType as WitServiceType, TcpManifest, WasmManifest,
};

#[async_trait::async_trait]
pub trait OrchestratorInterface {
    async fn readyz(&self, service_id: String) -> Result<(), String>;
    async fn deploy(&self, service_id: String, manifest: DeployManifest) -> Result<(), String>;
    async fn undeploy(&self, service_id: String) -> Result<(), String>;
    async fn list(&self) -> Result<Vec<DeployedService>, String>;
    async fn deploy_plan(&self, plan: DeploymentPlan) -> Result<(), String>;
}
use syneroym_core::{
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    util,
};
use syneroym_data_blob::BlobProvider;
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::MqttBroker;
use syneroym_rpc::{
    NativeDispatchRegistry, NativeInvocation, NativeResponse, NativeService, RpcError, RpcResult,
    WeakNativeDispatchRegistry,
};
use tokio::task;
use tracing::info;

use crate::{
    config_utils,
    dummy_sandbox::{AppSandboxEngine, ContainerEngine},
    http_routes,
    synsvc_native::SynSvcNativeService,
};

const ORCHESTRATOR_INTERFACE: &str = "orchestrator";
const SECURITY_INTERFACE: &str = "security";
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
    blob_provider: Arc<dyn BlobProvider>,
    messaging_broker: Arc<MqttBroker>,
    // `Weak`, not `NativeDispatchRegistry` -- see the cycle explained in
    // `syneroym_rpc::dispatch_registry`'s module docs. `RouteHandlerInner`
    // owns the strong clone for as long as the router itself is alive.
    native_dispatch: WeakNativeDispatchRegistry,
    // Strong, unlike `native_dispatch`: `ControlPlaneService` is never
    // itself keyed into this map, so there is no reference-cycle hazard
    // (see `crate::http_routes`'s module docs). `RouteHandlerInner` holds
    // the same `Arc` for lookup from `crates/router/src/route_handler/http.rs`.
    http_routes: HttpRouteRegistry,
}

impl Debug for ControlPlaneService {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlPlaneService")
            .field("service_id", &self.service_id)
            .finish_non_exhaustive()
    }
}

impl ControlPlaneService {
    #[allow(clippy::too_many_arguments)]
    pub async fn init(
        service_id: String,
        app_sandbox_engine: Arc<AppSandboxEngine>,
        podman_sandbox_engine: Arc<ContainerEngine>,
        registry: EndpointRegistry,
        hosted_apps_dir: PathBuf,
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
        blob_provider: Arc<dyn BlobProvider>,
        messaging_broker: Arc<MqttBroker>,
        native_dispatch: NativeDispatchRegistry,
        http_routes: HttpRouteRegistry,
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
            blob_provider,
            messaging_broker,
            native_dispatch: Arc::downgrade(&native_dispatch),
            http_routes,
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
                    payload: serde_json::to_value(services).unwrap_or(Value::Null),
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

fn ready_response() -> NativeResponse {
    NativeResponse { payload: serde_json::json!({"status": "ok"}) }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use dashmap::DashMap;
    use syneroym_core::{config::SubstrateConfig, storage::MockStorage, test_constants};
    use syneroym_data_blob::ObjectStoreBlobProvider;
    use syneroym_data_db::SqliteStorageProvider;
    use syneroym_mqtt_broker::MqttBrokerConfig;
    use syneroym_rpc::JsonRpcRequest;
    use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
        ArtifactSource, PlannedService, ServiceConfig, TcpManifest, WasmManifest,
    };
    use tokio::time::Instant;
    use wit_parser::Resolve;

    use super::*;

    const MESSAGING_TEST_DRIVER_INTERFACE: &str =
        "syneroym-test:messaging-pubsub-test/test-driver@0.1.0";

    fn messaging_wasm_manifest(bytes: Vec<u8>) -> DeployManifest {
        DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema_path: None,
                rotation_policy: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(bytes),
                hash: None,
                interfaces: vec![MESSAGING_TEST_DRIVER_INTERFACE.to_string()],
            }),
            registry_certificate: None,
        }
    }

    async fn call_test_driver(
        engine: &AppSandboxEngine,
        service_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> String {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: None,
        };
        engine.execute_wasm(service_id, MESSAGING_TEST_DRIVER_INTERFACE, &request).await.unwrap()
    }

    #[tokio::test]
    async fn test_wit_adherence() {
        let wit_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../wit_interfaces/wit/control-plane/control-plane.wit");

        let mut resolve = Resolve::default();
        let content = fs::read_to_string(&wit_path).expect("Failed to read WIT file");
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

        assert!(!interface.functions.is_empty(), "Orchestrator interface should have functions");

        for (name, _func) in &interface.functions {
            let method_name = name.strip_prefix('%').unwrap_or(name);

            let invocation = NativeInvocation {
                interface: "orchestrator".to_string(),
                method: method_name.to_string(),
                params: Value::Null,
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
    async fn test_security_dispatch_returns_sdk_statuses() {
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

    /// Slice 5: deploy a service (TCP type -- no WASM component needed),
    /// then exercise data-layer and blob-store entirely through
    /// `SynSvcNativeService::dispatch`, with no WASM component involved at
    /// all. Confirms `undeploy` removes the native dispatch registration.
    #[tokio::test]
    async fn test_native_dispatch_data_layer_and_blob_store_round_trip() {
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
            key_store.clone(),
            storage_provider.clone(),
            blob_provider,
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
        )
        .await
        .unwrap();

        let service_id = "native-test-svc".to_string();
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

        let native = service
            .native_dispatch
            .upgrade()
            .expect("native_dispatch registry still alive")
            .get(&service_id)
            .expect("native service registered on deploy")
            .clone();

        // data-layer: create-collection, put, get
        native
            .dispatch(NativeInvocation {
                interface: "data-layer".to_string(),
                method: "create-collection".to_string(),
                params: serde_json::json!({"name": "items", "indexes": []}),
            })
            .await
            .unwrap();
        native
            .dispatch(NativeInvocation {
                interface: "data-layer".to_string(),
                method: "put".to_string(),
                params: serde_json::json!({
                    "collection": "items",
                    "value": {"id": "1", "payload": b"{\"x\":1}".to_vec()},
                }),
            })
            .await
            .unwrap();
        let get_resp = native
            .dispatch(NativeInvocation {
                interface: "data-layer".to_string(),
                method: "get".to_string(),
                params: serde_json::json!({"collection": "items", "id": "1"}),
            })
            .await
            .unwrap();
        assert_eq!(get_resp.payload["id"], "1");

        // blob-store one-shot: put-blob / get-blob
        let put_resp = native
            .dispatch(NativeInvocation {
                interface: "blob-store".to_string(),
                method: "put-blob".to_string(),
                params: serde_json::json!({"data": b"hello native world".to_vec()}),
            })
            .await
            .unwrap();
        let hash: String = serde_json::from_value(put_resp.payload).unwrap();
        let get_blob_resp = native
            .dispatch(NativeInvocation {
                interface: "blob-store".to_string(),
                method: "get-blob".to_string(),
                params: serde_json::json!({"hash": hash}),
            })
            .await
            .unwrap();
        let data: Vec<u8> = serde_json::from_value(get_blob_resp.payload).unwrap();
        assert_eq!(data, b"hello native world".to_vec());

        // blob-store streaming:
        // open-upload/write-chunk/finish-upload/open-download/read-chunk
        let open_upload_resp = native
            .dispatch(NativeInvocation {
                interface: "blob-store".to_string(),
                method: "open-upload".to_string(),
                params: serde_json::json!({}),
            })
            .await
            .unwrap();
        let upload_id = open_upload_resp.payload["upload_id"].as_str().unwrap().to_string();
        native
            .dispatch(NativeInvocation {
                interface: "blob-store".to_string(),
                method: "write-chunk".to_string(),
                params: serde_json::json!({"upload_id": upload_id, "chunk": b"streamed ".to_vec()}),
            })
            .await
            .unwrap();
        native
            .dispatch(NativeInvocation {
                interface: "blob-store".to_string(),
                method: "write-chunk".to_string(),
                params: serde_json::json!({"upload_id": upload_id, "chunk": b"content".to_vec()}),
            })
            .await
            .unwrap();
        let finish_resp = native
            .dispatch(NativeInvocation {
                interface: "blob-store".to_string(),
                method: "finish-upload".to_string(),
                params: serde_json::json!({"upload_id": upload_id}),
            })
            .await
            .unwrap();
        let streamed_hash = finish_resp.payload["hash"].as_str().unwrap().to_string();

        let open_download_resp = native
            .dispatch(NativeInvocation {
                interface: "blob-store".to_string(),
                method: "open-download".to_string(),
                params: serde_json::json!({"hash": streamed_hash, "offset": 0}),
            })
            .await
            .unwrap();
        let download_id = open_download_resp.payload["download_id"].as_str().unwrap().to_string();
        let read_resp = native
            .dispatch(NativeInvocation {
                interface: "blob-store".to_string(),
                method: "read-chunk".to_string(),
                params: serde_json::json!({"download_id": download_id, "max_bytes": 1024}),
            })
            .await
            .unwrap();
        let chunk: Vec<u8> = serde_json::from_value(read_resp.payload["chunk"].clone()).unwrap();
        assert_eq!(chunk, b"streamed content".to_vec());
        assert_eq!(read_resp.payload["eof"], false);

        // vault: reveal a secret written directly via the service's own
        // ServiceStore (previously had dispatch code but no dedicated
        // round-trip test anywhere in the repo).
        let store = storage_provider.open_service_db(&service_id, &key_store).await.unwrap();
        store.write_secret("db-password", b"s3cr3t").await.unwrap();
        let reveal_resp = native
            .dispatch(NativeInvocation {
                interface: "vault".to_string(),
                method: "reveal".to_string(),
                params: serde_json::json!({"key": "db-password"}),
            })
            .await
            .unwrap();
        let revealed: Vec<u8> = serde_json::from_value(reveal_resp.payload).unwrap();
        assert_eq!(revealed, b"s3cr3t".to_vec());

        // app-config: get a key from a config generation saved directly
        // (previously had dispatch code but no dedicated round-trip test
        // anywhere in the repo).
        storage_provider
            .save_config_generation(&service_id, r#"{"greeting": "hello"}"#)
            .await
            .unwrap();
        let config_resp = native
            .dispatch(NativeInvocation {
                interface: "app-config".to_string(),
                method: "get".to_string(),
                params: serde_json::json!({"key": "greeting"}),
            })
            .await
            .unwrap();
        assert_eq!(config_resp.payload, serde_json::json!("hello"));

        // messaging: publish (subscribe/unsubscribe go through the
        // router-level push-delivery path, not this request/response one --
        // see ADR-0010 Finding A2). Subscribed directly through the broker
        // (bypassing the native dispatch layer, which has no subscribe) so
        // the assertion below proves actual delivery, not just that the
        // RPC call didn't error.
        let (_sub_handle, mut sub_rx) =
            messaging_broker.subscribe(format!("svc/{service_id}/orders/new")).await.unwrap();
        let publish_resp = native
            .dispatch(NativeInvocation {
                interface: "messaging".to_string(),
                method: "publish".to_string(),
                params: serde_json::json!({"topic": "orders/new", "payload": b"order-1".to_vec()}),
            })
            .await
            .unwrap();
        assert_eq!(publish_resp.payload, Value::Null);
        let (delivered_topic, delivered_payload) =
            tokio::time::timeout(Duration::from_secs(2), sub_rx.recv())
                .await
                .expect("did not time out waiting for native publish to be delivered")
                .expect("subscriber channel closed unexpectedly");
        assert_eq!(delivered_topic, format!("svc/{service_id}/orders/new"));
        assert_eq!(delivered_payload, b"order-1");

        // undeploy removes the native dispatch registration
        service.undeploy(service_id.clone()).await.unwrap();
        assert!(
            service
                .native_dispatch
                .upgrade()
                .expect("native_dispatch registry still alive")
                .get(&service_id)
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_native_dispatch_create_collection_with_indexes_and_batch_mutate() {
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
            blob_provider,
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
        )
        .await
        .unwrap();

        let service_id = "native-mutation-test-svc".to_string();
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

        let native = service
            .native_dispatch
            .upgrade()
            .expect("native_dispatch registry still alive")
            .get(&service_id)
            .expect("native service registered on deploy")
            .clone();

        // create-collection with a non-empty `indexes` array: exercises the
        // `IndexDefinition` translation layer (its bindgen-generated `type_`
        // field must accept plain `"type"` over the wire).
        native
            .dispatch(NativeInvocation {
                interface: "data-layer".to_string(),
                method: "create-collection".to_string(),
                params: serde_json::json!({
                    "name": "scored_items",
                    "indexes": [{"field_name": "score", "type": "Numeric"}],
                }),
            })
            .await
            .unwrap();

        // batch-mutate exercising all three `Mutation` variants in one
        // transaction: exercises the `MutationDto` translation layer (the
        // externally-tagged `{"Put": {...}}` shape doesn't match the
        // snake_case `{"type": "put", "value": {...}}` this API expects).
        native
            .dispatch(NativeInvocation {
                interface: "data-layer".to_string(),
                method: "batch-mutate".to_string(),
                params: serde_json::json!({
                    "collection": "scored_items",
                    "mutations": [
                        {"type": "put", "value": {"id": "a", "payload": b"{\"score\":1}".to_vec()}},
                        {"type": "put", "value": {"id": "b", "payload": b"{\"score\":2}".to_vec()}},
                        {"type": "patch", "value": {"id": "a", "patch_json": b"{\"score\":9}".to_vec()}},
                        {"type": "delete", "value": "b"},
                    ],
                }),
            })
            .await
            .unwrap();

        let get_a = native
            .dispatch(NativeInvocation {
                interface: "data-layer".to_string(),
                method: "get".to_string(),
                params: serde_json::json!({"collection": "scored_items", "id": "a"}),
            })
            .await
            .unwrap();
        let payload_a: Vec<u8> = serde_json::from_value(get_a.payload["payload"].clone()).unwrap();
        assert_eq!(payload_a, br#"{"score":9}"#.to_vec());

        let get_b = native
            .dispatch(NativeInvocation {
                interface: "data-layer".to_string(),
                method: "get".to_string(),
                params: serde_json::json!({"collection": "scored_items", "id": "b"}),
            })
            .await
            .unwrap();
        assert!(get_b.payload.is_null());
    }

    #[tokio::test]
    async fn test_native_dispatch_blob_store_errors_preserve_fidelity() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        // A tiny per-blob quota so a normal-sized upload can trigger
        // `BlobError::QuotaExceeded`.
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(4, None));
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
            blob_provider,
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
        )
        .await
        .unwrap();

        let service_id = "native-blob-error-test-svc".to_string();
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
        let native_dispatch = service.native_dispatch.upgrade().unwrap();
        let native = native_dispatch.get(&service_id).unwrap();

        // Not-found must surface as a distinct `Custom` code, not a generic
        // `InternalError` that a client can't distinguish from any other
        // internal failure.
        let not_found_err = native
            .dispatch(NativeInvocation {
                interface: "blob-store".to_string(),
                method: "get-blob".to_string(),
                params: serde_json::json!({"hash": "0".repeat(64)}),
            })
            .await
            .unwrap_err();
        assert!(matches!(not_found_err, RpcError::Custom(-32001, _, _)));

        // Quota-exceeded must likewise surface distinctly.
        let quota_err = native
            .dispatch(NativeInvocation {
                interface: "blob-store".to_string(),
                method: "put-blob".to_string(),
                params: serde_json::json!({"data": vec![0u8; 100]}),
            })
            .await
            .unwrap_err();
        assert!(matches!(quota_err, RpcError::Custom(-32002, _, _)));
    }

    /// M3B Slice 6A: a guest subscription's `messaging_subscriptions` row
    /// and live broker registration are both removed by `undeploy`, and a
    /// publish to that topic afterward does not error (nothing is left to
    /// deliver to).
    #[tokio::test]
    async fn test_messaging_undeploy_removes_subscriptions() {
        let Ok(wasm_bytes) = std::fs::read(test_constants::messaging_pubsub_test_wasm_path())
        else {
            eprintln!(
                "Skipping test_messaging_undeploy_removes_subscriptions: messaging-pubsub-test \
                 WASM artifact not found (run `cargo build --target wasm32-wasip2 --release` in \
                 test-components/messaging-pubsub-test)"
            );
            return;
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider: Arc<dyn StorageProvider> =
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
        app_sandbox.self_weak.set(Arc::downgrade(&app_sandbox)).unwrap();
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
            blob_provider,
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
        )
        .await
        .unwrap();

        let service_id = "messaging-undeploy-svc".to_string();
        service.deploy(service_id.clone(), messaging_wasm_manifest(wasm_bytes)).await.unwrap();
        call_test_driver(
            &service.app_sandbox_engine,
            &service_id,
            "subscribe-to",
            serde_json::json!(["orders/new"]),
        )
        .await;

        let namespaced_topic = format!("svc/{service_id}/orders/new");
        let persisted = storage_provider.list_all_messaging_subscriptions().await.unwrap();
        assert_eq!(persisted, vec![(service_id.clone(), namespaced_topic.clone())]);

        service.undeploy(service_id.clone()).await.unwrap();

        let after_undeploy = storage_provider.list_all_messaging_subscriptions().await.unwrap();
        assert!(after_undeploy.is_empty(), "subscription row must be gone after undeploy");

        // Publishing to the now-unsubscribed topic must not error, even
        // though the (undeployed) component can no longer be delivered to.
        messaging_broker.publish(namespaced_topic, b"post-undeploy".to_vec()).await.unwrap();
    }

    /// M3B Slice 6A: service A cannot receive messages published in
    /// service B's own namespace without the explicit fully-qualified
    /// `svc/<other>/...` opt-in (ADR-0010's Topic Namespace Isolation).
    #[tokio::test]
    async fn test_messaging_namespace_isolation() {
        let Ok(wasm_bytes) = std::fs::read(test_constants::messaging_pubsub_test_wasm_path())
        else {
            eprintln!(
                "Skipping test_messaging_namespace_isolation: messaging-pubsub-test WASM artifact \
                 not found (run `cargo build --target wasm32-wasip2 --release` in \
                 test-components/messaging-pubsub-test)"
            );
            return;
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider: Arc<dyn StorageProvider> =
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
        app_sandbox.self_weak.set(Arc::downgrade(&app_sandbox)).unwrap();
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
            blob_provider,
            messaging_broker,
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
        )
        .await
        .unwrap();

        let service_a = "messaging-isolation-a".to_string();
        let service_b = "messaging-isolation-b".to_string();
        service
            .deploy(service_a.clone(), messaging_wasm_manifest(wasm_bytes.clone()))
            .await
            .unwrap();
        service.deploy(service_b.clone(), messaging_wasm_manifest(wasm_bytes)).await.unwrap();

        // B subscribes to its own bare namespace only.
        call_test_driver(
            &service.app_sandbox_engine,
            &service_b,
            "subscribe-to",
            serde_json::json!(["orders/new"]),
        )
        .await;
        // A publishes to *its own* bare namespace -- a different topic.
        call_test_driver(
            &service.app_sandbox_engine,
            &service_a,
            "publish-to",
            serde_json::json!(["orders/new", "should not cross into B"]),
        )
        .await;

        // Give any (incorrect) delivery a real chance to happen before
        // asserting the negative.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let received = call_test_driver(
            &service.app_sandbox_engine,
            &service_b,
            "get-received-messages",
            serde_json::json!([]),
        )
        .await;
        assert!(
            received.is_empty(),
            "service B must not observe service A's own-namespace publish"
        );
    }

    /// M3B Slice 6A (ADR-0010 Finding A1): a guest subscription's
    /// `messaging_subscriptions` row survives a substrate restart, and
    /// replaying it into a freshly-constructed broker/engine (the same
    /// steps `RouteHandler::init` performs on real startup) restores
    /// delivery without the guest calling `subscribe` again.
    #[tokio::test]
    async fn test_messaging_subscriptions_survive_restart_replay() {
        let Ok(wasm_bytes) = std::fs::read(test_constants::messaging_pubsub_test_wasm_path())
        else {
            eprintln!(
                "Skipping test_messaging_subscriptions_survive_restart_replay: \
                 messaging-pubsub-test WASM artifact not found (run `cargo build --target \
                 wasm32-wasip2 --release` in test-components/messaging-pubsub-test)"
            );
            return;
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let service_id = "messaging-restart-svc".to_string();
        let manifest = messaging_wasm_manifest(wasm_bytes);

        // "First boot": deploy, subscribe, then drop the broker/engine
        // entirely (nothing here survives a real restart except the DB).
        {
            let broker1 = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
            let engine1 = Arc::new(
                AppSandboxEngine::init(
                    &config,
                    vec![],
                    key_store.clone(),
                    storage_provider.clone(),
                    blob_provider.clone(),
                    broker1,
                    EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
                )
                .await
                .unwrap(),
            );
            engine1.self_weak.set(Arc::downgrade(&engine1)).unwrap();
            engine1.deploy_wasm(&service_id, &manifest).await.unwrap();
            call_test_driver(
                &engine1,
                &service_id,
                "subscribe-to",
                serde_json::json!(["orders/new"]),
            )
            .await;
        }

        let namespaced_topic = format!("svc/{service_id}/orders/new");
        let persisted = storage_provider.list_all_messaging_subscriptions().await.unwrap();
        assert_eq!(
            persisted,
            vec![(service_id.clone(), namespaced_topic.clone())],
            "subscription row must survive across the simulated restart"
        );

        // "Second boot": fresh broker + engine, re-deploy the same wasm
        // bytes (mirrors AppSandboxEngine::init's own endpoint-driven
        // warmup, which isn't exercised directly here), then replay every
        // persisted row -- exactly what `RouteHandler::init` does before
        // accepting connections.
        let broker2 = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let engine2 = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store,
                storage_provider.clone(),
                blob_provider,
                broker2.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        engine2.self_weak.set(Arc::downgrade(&engine2)).unwrap();
        engine2.deploy_wasm(&service_id, &manifest).await.unwrap();

        for (subscribed_service_id, topic) in
            storage_provider.list_all_messaging_subscriptions().await.unwrap()
        {
            engine2.register_internal_subscription(&subscribed_service_id, &topic).await.unwrap();
        }

        broker2.publish(namespaced_topic.clone(), b"post-restart".to_vec()).await.unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut received = String::new();
        while Instant::now() < deadline {
            received = call_test_driver(
                &engine2,
                &service_id,
                "get-received-messages",
                serde_json::json!([]),
            )
            .await;
            if !received.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(received, format!("{namespaced_topic}\tpost-restart"));
    }

    const STREAM_TEST_DRIVER_INTERFACE: &str = "syneroym-test:stream-test/test-driver@0.1.0";
    const STREAM_PROTOCOL: &str = "file-transfer";

    fn stream_wasm_manifest(bytes: Vec<u8>) -> DeployManifest {
        DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema_path: None,
                rotation_policy: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(bytes),
                hash: None,
                interfaces: vec![STREAM_TEST_DRIVER_INTERFACE.to_string()],
            }),
            registry_certificate: None,
        }
    }

    /// M3B Slice 6B (ADR-0014): `ControlPlaneService::undeploy` already
    /// iterates every registered interface for a `service_id` and removes
    /// it generically (see the ADR's "Where Registration Lives") -- this
    /// proves that generic loop also cleans up a `register-stream-protocol`
    /// registration (the stream-test fixture registers `"file-transfer"`
    /// from its own `init()`), with no Slice-6B-specific cleanup code
    /// needed in `undeploy` itself.
    #[tokio::test]
    async fn test_stream_protocol_undeploy_removes_registration() {
        let Ok(wasm_bytes) = std::fs::read(test_constants::stream_test_wasm_path()) else {
            eprintln!(
                "Skipping test_stream_protocol_undeploy_removes_registration: stream-test WASM \
                 artifact not found (run `cargo build --target wasm32-wasip2 --release` in \
                 test-components/stream-test)"
            );
            return;
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                registry.clone(),
            )
            .await
            .unwrap(),
        );
        app_sandbox.self_weak.set(Arc::downgrade(&app_sandbox)).unwrap();
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            app_sandbox,
            container_engine,
            registry.clone(),
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
        )
        .await
        .unwrap();

        let service_id = "stream-undeploy-svc".to_string();
        service.deploy(service_id.clone(), stream_wasm_manifest(wasm_bytes)).await.unwrap();

        assert!(
            registry.lookup(&service_id, STREAM_PROTOCOL).is_some(),
            "register-stream-protocol (called from the fixture's init()) must be visible in the \
             registry after deploy"
        );

        service.undeploy(service_id.clone()).await.unwrap();

        assert!(
            registry.lookup(&service_id, STREAM_PROTOCOL).is_none(),
            "undeploy must remove the stream-protocol registration along with every other \
             registered interface"
        );
    }
}

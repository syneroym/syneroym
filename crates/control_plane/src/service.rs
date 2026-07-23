//! Orchestrator control service implementation
//!
//! Handles requests for registering, deploying, listing, and destroying
//! sandbox instances or services running on the node.

use std::{
    fmt,
    fmt::{Debug, Formatter},
    fs,
    path::PathBuf,
    sync::Arc,
};

use anyhow::Result;
use serde_json::Value;
use syneroym_core::{http_routes::HttpRouteRegistry, local_registry::EndpointRegistry};
use syneroym_data_blob::BlobProvider;
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::MqttBroker;
use syneroym_rpc::{
    Ability, CallerContext, NativeDispatchRegistry, NativeInvocation, NativeResponse,
    NativeService, ResourceUri, RpcError, RpcResult, WeakNativeDispatchRegistry,
};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    DeployManifest, DeploymentPlan,
};
use tracing::info;

use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

mod orchestration;

use orchestration::OrchestratorInterface;

const ORCHESTRATOR_INTERFACE: &str = "orchestrator";
const SECURITY_INTERFACE: &str = "security";

/// The Substrate Service (The Control Plane Orchestrator)
/// This service handles the deployment and lifecycle of applications
/// (`SynApps`) within the substrate. It interacts with sandbox environments
/// like Podman or Wasmtime.
pub struct ControlPlaneService {
    service_id: String,
    /// This node's own DID; `substrate:<node_did>` resources name it (M04A
    /// Slice B7a). Distinct field from `service_id` above (which happens to
    /// hold the same value in production) because it is used as an
    /// *identity*, not a routing key -- mirrors `RouteHandlerInner::node_did`.
    node_did: String,
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
    // (contrast `syneroym_rpc::dispatch_registry`'s module docs, which
    // explain why `native_dispatch` above can't use the same plain-`Arc`
    // approach). `RouteHandlerInner` holds the same `Arc` (the type lives in
    // `syneroym_core::http_routes`) for lookup from
    // `crates/router/src/route_handler/http.rs`.
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
        node_did: String,
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
            node_did,
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

    /// Whether `caller` holds a specific **node-wide** orchestrator ability:
    /// the substrate owner (whose `substrate/admin` entails every ability),
    /// or -- on an unowned substrate -- any verified caller (M04A Slice B7a,
    /// F4). There is deliberately no "is the substrate owned?" branch
    /// anywhere else, because the unowned posture is expressed as an issued
    /// capability, not as a skipped check (design §6.1.1).
    ///
    /// **Parameterized by `ability`, not hardcoded to one** (post-review
    /// fix): B7b's design (§3.1 A2) deliberately keeps the three
    /// `orchestrator/*` abilities flat and independently grantable ("deploy
    /// but not undeploy" must stay expressible), so a future grantee could
    /// hold `orchestrator/status` alone. Checking a single hardcoded ability
    /// here for every caller-side use -- deploy's takeover override, undeploy's
    /// gate, and list's visibility -- would let a *read-only, status-only*
    /// grantee also override another owner's deploy/undeploy, a privilege
    /// escalation once B7b mints such a grant (not reachable in B7a itself:
    /// F4 only ever issues all three abilities together, and no tooling
    /// exists yet to mint a partial grant). Each call site below must pass
    /// the ability it actually needs to exercise -- `ORCHESTRATOR_DEPLOY` to
    /// override a takeover, `ORCHESTRATOR_UNDEPLOY` to override an undeploy
    /// gate, `ORCHESTRATOR_STATUS` for list's broader visibility bar (a
    /// monitoring-only grantee is meant to see the list; it is not thereby
    /// meant to deploy/undeploy over someone else's app).
    ///
    /// The resource is the **bare** `substrate:<node_did>` -- node-wide (F2).
    /// That excludes an app-scoped B7b grantee (`substrate:<node>/app/foo`):
    /// their capability carries a selector, so it is not `is_substrate_scope`
    /// (`ResourceUri::is_substrate_scope`, narrowed at M04A Slice B7b to
    /// exclude selector-bearing resources -- landed alongside this gate, so
    /// the exclusion is real, not merely inert-by-absence as it was at B7a).
    /// They are prefix-covered by `covers_resource` instead, at each gate's
    /// own selectored resource check (deploy/undeploy/per-service readyz).
    fn has_node_wide_ability(&self, caller: &CallerContext, ability: &'static str) -> bool {
        caller
            .has_capability(&ResourceUri::substrate(&self.node_did), &Ability(ability.to_string()))
    }
}

#[async_trait::async_trait]
impl NativeService for ControlPlaneService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        info!("Orchestrator received dispatch: {}.{}", invocation.interface, invocation.method);

        if invocation.interface.as_str() == SECURITY_INTERFACE {
            // TODO(M04B/FDAE): security ops (KEK/secret) are node-owner
            // operations; final authorization is FDAE against
            // caller.session (substrate/admin). B0 threads the caller
            // (`invocation.caller`, available to every arm below) but does
            // not yet gate -- roymctl carries only a self-asserted identity.
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
                        .inject_kek(arr)
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
                self.readyz(service_id, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(ready_response())
            }
            "deploy" => {
                let (service_id, manifest): (String, DeployManifest) =
                    serde_json::from_value(invocation.params).map_err(|e| {
                        RpcError::InvalidParams(format!("Failed to parse deploy params: {e}"))
                    })?;
                self.deploy(service_id, manifest, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
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
                self.deploy_plan(plan, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "deployed_plan"}) })
            }
            "undeploy" => {
                let (service_id,): (String,) = serde_json::from_value(invocation.params.clone())
                    .or_else(|_| serde_json::from_value::<String>(invocation.params).map(|s| (s,)))
                    .map_err(|e| {
                        RpcError::InvalidParams(format!("Failed to parse undeploy params: {e}"))
                    })?;
                self.undeploy(service_id, &invocation.caller)
                    .await
                    .map_err(RpcError::InternalError)?;
                Ok(NativeResponse { payload: serde_json::json!({"status": "undeployed"}) })
            }
            "list" => {
                let services =
                    self.list(&invocation.caller).await.map_err(RpcError::InternalError)?;
                Ok(NativeResponse {
                    payload: serde_json::to_value(services).unwrap_or(Value::Null),
                })
            }
            method => Err(RpcError::MethodNotFound(method.to_string())),
        }
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
    use syneroym_rpc::{CallerContext, JsonRpcRequest};
    use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
        ArtifactSource, ServiceConfig, ServiceType as WitServiceType, TcpManifest, WasmManifest,
    };
    use tokio::time::Instant;
    use wit_parser::Resolve;

    use super::*;

    /// M04A Slice B7b: a caller holding node-wide orchestrator authority on
    /// `"did:key:zTestNode"` (every test in this module inits
    /// `ControlPlaneService` with that node DID) -- the shape `build_caller`
    /// issues for the F4 unowned-substrate bootstrap grant. `deploy`/
    /// `undeploy` now gate on an explicit `orchestrator/{deploy,undeploy}`
    /// capability (§3.2), so any test that deploys/undeploys a service as
    /// setup for exercising a *different* interface (data-layer, blob-store,
    /// messaging) needs a caller that holds it --
    /// `CallerContext::service_system` (zero capabilities) no longer
    /// suffices for that setup step. Not used for the native-interface
    /// dispatch calls themselves, which stay ungated (F3.1/Q2: B7 does not
    /// close the five data interfaces).
    fn node_wide_caller(caller_did: &str) -> CallerContext {
        use syneroym_rpc::{Ability, AuthLevel, Capability, ResourceUri, SessionContext};

        let resource = ResourceUri::substrate("did:key:zTestNode");
        CallerContext {
            caller_did: caller_did.to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: caller_did.to_string(),
                capabilities: vec![
                    Capability {
                        with: resource.clone(),
                        can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
                        caveats: None,
                    },
                    Capability {
                        with: resource,
                        can: Ability(Ability::ORCHESTRATOR_UNDEPLOY.to_string()),
                        caveats: None,
                    },
                ],
                ..Default::default()
            },
            auth: AuthLevel::Delegated,
            proof: None,
        }
    }

    const MESSAGING_TEST_DRIVER_INTERFACE: &str =
        "syneroym-test:messaging-pubsub-test/test-driver@0.1.0";

    fn messaging_wasm_manifest(bytes: Vec<u8>) -> DeployManifest {
        DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
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
            "did:key:zTestNode".to_string(),
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
                caller: CallerContext::service_system("test-caller"),
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
            "did:key:zTestNode".to_string(),
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
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
                interface: "security".to_string(),
                method: "rotate-kek".to_string(),
                params: serde_json::to_value((new_kek,)).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(rotate_res.payload, serde_json::json!({"status": "rotated"}));

        let secret_res = service
            .dispatch(NativeInvocation {
                caller: CallerContext::service_system("test-caller"),
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
            "did:key:zTestNode".to_string(),
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
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
        };
        let test_caller = node_wide_caller("test-caller");
        service.deploy(service_id.clone(), manifest, &test_caller).await.unwrap();

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
                caller: CallerContext::service_system("test-caller"),
                interface: "data-layer".to_string(),
                method: "create-collection".to_string(),
                params: serde_json::json!({"name": "items", "indexes": []}),
            })
            .await
            .unwrap();
        native
            .dispatch(NativeInvocation {
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
                interface: "blob-store".to_string(),
                method: "put-blob".to_string(),
                params: serde_json::json!({"data": b"hello native world".to_vec()}),
            })
            .await
            .unwrap();
        let hash: String = serde_json::from_value(put_resp.payload).unwrap();
        let get_blob_resp = native
            .dispatch(NativeInvocation {
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
                interface: "blob-store".to_string(),
                method: "open-upload".to_string(),
                params: serde_json::json!({}),
            })
            .await
            .unwrap();
        let upload_id = open_upload_resp.payload["upload_id"].as_str().unwrap().to_string();
        native
            .dispatch(NativeInvocation {
                caller: CallerContext::service_system("test-caller"),
                interface: "blob-store".to_string(),
                method: "write-chunk".to_string(),
                params: serde_json::json!({"upload_id": upload_id, "chunk": b"streamed ".to_vec()}),
            })
            .await
            .unwrap();
        native
            .dispatch(NativeInvocation {
                caller: CallerContext::service_system("test-caller"),
                interface: "blob-store".to_string(),
                method: "write-chunk".to_string(),
                params: serde_json::json!({"upload_id": upload_id, "chunk": b"content".to_vec()}),
            })
            .await
            .unwrap();
        let finish_resp = native
            .dispatch(NativeInvocation {
                caller: CallerContext::service_system("test-caller"),
                interface: "blob-store".to_string(),
                method: "finish-upload".to_string(),
                params: serde_json::json!({"upload_id": upload_id}),
            })
            .await
            .unwrap();
        let streamed_hash = finish_resp.payload["hash"].as_str().unwrap().to_string();

        let open_download_resp = native
            .dispatch(NativeInvocation {
                caller: CallerContext::service_system("test-caller"),
                interface: "blob-store".to_string(),
                method: "open-download".to_string(),
                params: serde_json::json!({"hash": streamed_hash, "offset": 0}),
            })
            .await
            .unwrap();
        let download_id = open_download_resp.payload["download_id"].as_str().unwrap().to_string();
        let read_resp = native
            .dispatch(NativeInvocation {
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
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
        service.undeploy(service_id.clone(), &test_caller).await.unwrap();
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
            "did:key:zTestNode".to_string(),
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
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
        };
        service
            .deploy(service_id.clone(), manifest, &node_wide_caller("test-caller"))
            .await
            .unwrap();

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
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
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
            "did:key:zTestNode".to_string(),
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
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
        };
        service
            .deploy(service_id.clone(), manifest, &node_wide_caller("test-caller"))
            .await
            .unwrap();
        let native_dispatch = service.native_dispatch.upgrade().unwrap();
        let native = native_dispatch.get(&service_id).unwrap();

        // Not-found must surface as a distinct `Custom` code, not a generic
        // `InternalError` that a client can't distinguish from any other
        // internal failure.
        let not_found_err = native
            .dispatch(NativeInvocation {
                caller: CallerContext::service_system("test-caller"),
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
                caller: CallerContext::service_system("test-caller"),
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
            "did:key:zTestNode".to_string(),
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
        let test_caller = node_wide_caller("test-caller");
        service
            .deploy(service_id.clone(), messaging_wasm_manifest(wasm_bytes), &test_caller)
            .await
            .unwrap();
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

        service.undeploy(service_id.clone(), &test_caller).await.unwrap();

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
            "did:key:zTestNode".to_string(),
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
        let test_caller = node_wide_caller("test-caller");
        service
            .deploy(service_a.clone(), messaging_wasm_manifest(wasm_bytes.clone()), &test_caller)
            .await
            .unwrap();
        service
            .deploy(service_b.clone(), messaging_wasm_manifest(wasm_bytes), &test_caller)
            .await
            .unwrap();

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
    /// steps `syneroym_substrate::runtime::build_route_handler_deps`
    /// performs on real startup) restores delivery without the guest
    /// calling `subscribe` again.
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
        // persisted row -- exactly what
        // `syneroym_substrate::runtime::build_route_handler_deps` does
        // before the router starts accepting connections.
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
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
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
            "did:key:zTestNode".to_string(),
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
        let test_caller = node_wide_caller("test-caller");
        service
            .deploy(service_id.clone(), stream_wasm_manifest(wasm_bytes), &test_caller)
            .await
            .unwrap();

        assert!(
            registry.lookup(&service_id, STREAM_PROTOCOL).is_some(),
            "register-stream-protocol (called from the fixture's init()) must be visible in the \
             registry after deploy"
        );

        service.undeploy(service_id.clone(), &test_caller).await.unwrap();

        assert!(
            registry.lookup(&service_id, STREAM_PROTOCOL).is_none(),
            "undeploy must remove the stream-protocol registration along with every other \
             registered interface"
        );
    }
}

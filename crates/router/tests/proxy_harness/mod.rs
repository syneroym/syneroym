#![allow(dead_code)]

use std::{fs, path::Path, sync::Arc, time::Duration};

use dashmap::DashMap;
use serde_json::{Value, json};
use syneroym_async_queue::QueueConfig;
use syneroym_core::{
    config::{AppSandboxRole, RolesConfig, StorageConfig, SubstrateConfig},
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
    test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{
    AdaptationStage, EncryptionStage, ProxyOutbox, RouteHandler, RouteHandlerDeps, RoutePipeline,
    RoutePreamble, ServiceStage, TransportStage,
};
use syneroym_rpc::{
    NativeDispatchRegistry, NativeInvocation, NativeResponse, NativeService, RpcResult,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};
use tokio::time;

#[derive(Debug, Default)]
pub struct NoopControlPlane;

#[async_trait::async_trait]
impl NativeService for NoopControlPlane {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        Ok(NativeResponse { payload: Value::Null })
    }
}

#[derive(Debug, Default)]
pub struct SleeperService;

#[async_trait::async_trait]
impl NativeService for SleeperService {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        time::sleep(Duration::from_secs(5)).await;
        Ok(NativeResponse { payload: Value::Null })
    }
}

pub fn test_substrate_config(db_dir: &Path) -> SubstrateConfig {
    SubstrateConfig {
        storage: StorageConfig { db_dir: db_dir.to_path_buf(), ..Default::default() },
        roles: RolesConfig {
            app_sandbox: Some(AppSandboxRole {
                max_concurrent_instances: 4,
                memory_limit: "16Mi".to_string(),
                default_max_memory_bytes: Some(16 * 1024 * 1024),
                max_core_instances_per_component: 4,
                max_memories_per_component: 4,
                max_tables_per_component: 4,
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn wasm_deploy_manifest(bytes: Vec<u8>) -> DeployManifest {
    DeployManifest::unpublished(
        ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
            visibility: None,
        },
        ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces: vec![],
        }),
    )
}

pub async fn test_route_handler_with_proxy_components() -> Option<RouteHandler> {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = test_substrate_config(temp_dir.path());
    test_route_handler_with_config(config, temp_dir.path()).await
}

pub async fn test_route_handler_with_config(
    config: SubstrateConfig,
    db_path: &Path,
) -> Option<RouteHandler> {
    test_route_handler_with_outbox(config, db_path).await.map(|(handler, _)| handler)
}

async fn deploy_test_components(
    engine: &AppSandboxEngine,
    proxy_test_bytes: Vec<u8>,
    greeter_bytes: Vec<u8>,
) {
    engine
        .deploy_wasm("proxy-caller", &wasm_deploy_manifest(proxy_test_bytes.clone()))
        .await
        .unwrap();
    engine.deploy_wasm("proxy-callee", &wasm_deploy_manifest(greeter_bytes)).await.unwrap();
    engine.deploy_wasm("proxy-spinner", &wasm_deploy_manifest(proxy_test_bytes)).await.unwrap();
}

async fn register_test_endpoints(registry: &EndpointRegistry) {
    registry
        .register(
            "proxy-caller".to_string(),
            test_constants::PROXY_TEST_DRIVER_INTERFACE.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "proxy-caller".to_string() },
        )
        .await
        .unwrap();
    registry
        .register(
            "proxy-callee".to_string(),
            test_constants::GREETER_INTERFACE_NAME.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "proxy-callee".to_string() },
        )
        .await
        .unwrap();
    registry
        .register(
            "proxy-spinner".to_string(),
            test_constants::PROXY_TEST_DRIVER_INTERFACE.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "proxy-spinner".to_string() },
        )
        .await
        .unwrap();
    registry
        .register(
            "sleeper-service".to_string(),
            "sleeper".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "sleeper-service".to_string() },
        )
        .await
        .unwrap();
}

pub async fn test_route_handler_with_outbox(
    config: SubstrateConfig,
    db_path: &Path,
) -> Option<(RouteHandler, Arc<ProxyOutbox>)> {
    let proxy_test_bytes = fs::read(test_constants::proxy_test_wasm_path()).ok()?;
    let greeter_bytes = fs::read(test_constants::greeter_wasm_path()).ok()?;

    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(db_path, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let logical_resolver = syneroym_app_orchestration::empty_resolver();

    let outbox = Arc::new(ProxyOutbox::new(
        storage_provider.clone(),
        key_store.clone(),
        logical_resolver.clone(),
        QueueConfig::from(&config.roles.app_sandbox.clone().unwrap_or_default()),
    ));

    let app_sandbox_engine = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            registry.clone(),
            logical_resolver.clone(),
        )
        .await
        .unwrap(),
    );
    app_sandbox_engine.self_weak.set(Arc::downgrade(&app_sandbox_engine)).unwrap();
    app_sandbox_engine.websocket_senders.set(syneroym_rpc::WebSocketSenders::new()).unwrap();

    deploy_test_components(&app_sandbox_engine, proxy_test_bytes, greeter_bytes).await;

    let native_dispatch = NativeDispatchRegistry::default();
    native_dispatch.insert("sleeper-service".to_string(), Arc::new(SleeperService));

    register_test_endpoints(&registry).await;

    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let deps = RouteHandlerDeps {
        logical_resolver,
        key_store,
        storage_provider,
        app_sandbox_engine,
        messaging_broker,
        native_dispatch,
        native_http: Arc::new(DashMap::new()),
        websocket_senders: syneroym_rpc::WebSocketSenders::new(),
        http_routes,
        assets: Arc::new(DashMap::new()),
        sse_permits: Arc::new(DashMap::new()),
        control_plane_service: Arc::new(NoopControlPlane),
        control_plane: None,
        session_revocation: None,
        event_bus: None,
    };

    let handler = RouteHandler::init(
        "test-orchestrator".to_string(),
        &config,
        registry,
        [9u8; 32],
        None,
        deps,
    )
    .await
    .unwrap();

    Some((handler, outbox))
}

pub fn call_peer_pipeline() -> RoutePipeline {
    RoutePipeline {
        encryption: EncryptionStage::None,
        transport: TransportStage::Binary,
        adaptation: AdaptationStage::JsonRpcToWasm,
        service: ServiceStage::WasmComponent { service_id: "proxy-caller".to_string() },
    }
}

pub fn call_peer_preamble() -> RoutePreamble {
    RoutePreamble::binary_json_rpc("proxy-caller", test_constants::PROXY_TEST_DRIVER_INTERFACE)
}

pub fn json_rpc_body(method: &str, params: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1}))
        .unwrap()
}

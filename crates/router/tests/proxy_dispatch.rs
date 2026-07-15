#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Slice A1 (M04A): Universal Proxy dispatch integration tests -- drives the
//! guest-facing `syneroym:proxy/proxy::call` host function end to end
//! through a real `RouteHandler::init` composition (which wires
//! `AppSandboxEngine::service_proxy` to a live `ProxyRouter`), the same
//! harness style as `native_dispatch_identity.rs`. Complements the
//! Rust-level `ProxyRouter::invoke` unit tests in `crates/router/src/proxy.rs`,
//! which never exercise the guest WIT boundary itself.
//!
//! Skips if the `proxy-test`/`greeter` wasm artifacts haven't been built
//! (`cargo build --target wasm32-wasip2 --release` in
//! `test-components/proxy-test` and `test-components/greeter`).

use std::{fs, sync::Arc};

use dashmap::DashMap;
use serde_json::{Value, json};
use syneroym_core::{
    config::SubstrateConfig,
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
    AdaptationStage, EncryptionStage, RouteHandler, RouteHandlerDeps, RoutePipeline, RoutePreamble,
    RouteProtocol, RouteTransport, ServiceStage, TransportStage,
};
use syneroym_rpc::{
    NativeDispatchRegistry, NativeInvocation, NativeResponse, NativeService, RpcResult,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};

#[derive(Debug, Default)]
struct NoopControlPlane;

#[async_trait::async_trait]
impl NativeService for NoopControlPlane {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        Ok(NativeResponse { payload: Value::Null })
    }
}

fn wasm_deploy_manifest(bytes: Vec<u8>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema_path: None,
            rotation_policy: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
    }
}

/// Builds a `RouteHandler` (mirroring `native_dispatch_identity.rs`'s own
/// helper) with two WASM components deployed onto the shared
/// `AppSandboxEngine`/`EndpointRegistry`: `proxy-caller` (the `proxy-test`
/// fixture, importing `syneroym:proxy/proxy`) and `proxy-callee` (`greeter`).
/// Returns `None` if either wasm artifact hasn't been built.
async fn test_route_handler_with_proxy_components() -> Option<RouteHandler> {
    let proxy_test_bytes = fs::read(test_constants::proxy_test_wasm_path()).ok()?;
    let greeter_bytes = fs::read(test_constants::greeter_wasm_path()).ok()?;

    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let app_sandbox_engine = Arc::new(
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
    app_sandbox_engine.self_weak.set(Arc::downgrade(&app_sandbox_engine)).unwrap();

    app_sandbox_engine
        .deploy_wasm("proxy-caller", &wasm_deploy_manifest(proxy_test_bytes))
        .await
        .unwrap();
    app_sandbox_engine
        .deploy_wasm("proxy-callee", &wasm_deploy_manifest(greeter_bytes))
        .await
        .unwrap();

    // `AppSandboxEngine::deploy_wasm` compiles/caches the component and runs
    // lifecycle hooks, but registering the interface->endpoint mapping is
    // `ControlPlaneService`'s job in production; done directly here since
    // this test doesn't exercise the control plane.
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

    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let deps = RouteHandlerDeps {
        key_store,
        storage_provider,
        app_sandbox_engine,
        messaging_broker,
        native_dispatch: NativeDispatchRegistry::default(),
        http_routes,
        control_plane_service: Arc::new(NoopControlPlane),
    };

    Some(
        RouteHandler::init(
            "test-orchestrator".to_string(),
            &config,
            registry,
            [9u8; 32],
            None,
            deps,
        )
        .await
        .unwrap(),
    )
}

fn call_peer_pipeline() -> RoutePipeline {
    RoutePipeline {
        encryption: EncryptionStage::None,
        transport: TransportStage::Binary,
        adaptation: AdaptationStage::JsonRpcToWasm,
        service: ServiceStage::WasmComponent { service_id: "proxy-caller".to_string() },
    }
}

fn call_peer_preamble() -> RoutePreamble {
    RoutePreamble {
        transport: RouteTransport::Binary,
        protocol: RouteProtocol::JsonRpc,
        interface: test_constants::PROXY_TEST_DRIVER_INTERFACE.to_string(),
        service_id: "proxy-caller".to_string(),
        enc: None,
        pubkey: None,
        delegation: None,
        dir: None,
    }
}

fn json_rpc_body(method: &str, params: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1}))
        .unwrap()
}

/// Guest-to-guest, same node: `proxy-caller` calls `proxy-callee`'s
/// `greet` through `syneroym:proxy/proxy::call` and gets its typed result
/// back -- the full guest-WIT-import round trip, not just the Rust-level
/// `ProxyRouter::invoke`.
#[tokio::test]
async fn guest_to_guest_same_node_proxy_call_returns_typed_result() {
    let Some(route_handler) = test_route_handler_with_proxy_components().await else {
        eprintln!("skipping: proxy-test/greeter wasm artifacts not built");
        return;
    };

    let params = json!({
        "service": "proxy-callee",
        "interface": test_constants::GREETER_INTERFACE_NAME,
        "method": "greet",
        "params": "[\"World\"]",
    });
    let body = json_rpc_body("call-peer", params);

    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response_bytes).unwrap();
    assert!(response.get("error").is_none(), "call-peer failed: {response:?}");
    let result = response.get("result").and_then(Value::as_str).unwrap_or_default();
    assert!(
        result.contains("Hello, World!"),
        "expected the callee's greeting in the result, got: {result:?}"
    );
}

/// A guest reaching another service's native capability (`data-layer`)
/// through the proxy is denied -- the §5.3 guest native-capability gate,
/// exercised end to end through the WIT boundary (the callee doesn't even
/// need to exist: the gate fires before any registry lookup).
#[tokio::test]
async fn guest_cross_service_native_capability_through_proxy_is_permission_denied() {
    let Some(route_handler) = test_route_handler_with_proxy_components().await else {
        eprintln!("skipping: proxy-test/greeter wasm artifacts not built");
        return;
    };

    let params = json!({
        "service": "some-other-service",
        "interface": "data-layer",
        "method": "get",
        "params": "{}",
    });
    let body = json_rpc_body("call-peer", params);

    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response_bytes).unwrap();
    // The guest's `call-peer` returns `result<string, string>`; an `Err`
    // (the debug-formatted `proxy-error`) crosses the WIT boundary as a WIT
    // `result::err`, which the A0' boundary contract turns into a
    // *transport*-level JSON-RPC error, not a `result` value -- matching
    // `wasm_results_to_json`'s documented `Result(Err(_))` handling.
    let message = response
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        message.contains("PermissionDenied"),
        "expected a permission-denied proxy-error, got: {response:?}"
    );
}

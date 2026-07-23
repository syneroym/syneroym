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
use syneroym_control_plane::SynSvcNativeService;
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
use syneroym_fdae::{Policy, parse_and_validate};
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
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
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
        ucan: None,
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

// -- Guest self-proxy ingress -------------------------------------------
//
// `proxy::Host::call` always synthesizes `CallerContext::service_system` for
// the guest, and the proxy gate's same-service exception (`proxy.rs:224-231`)
// deliberately permits a component to reach its **own** service's native
// `data-layer` this way. This ingress carries the same capability-less
// identity as the direct WIT `store::Host` path (D-04-02-h, task.md), so it
// must observably return empty under a deployed policy too -- pinned here
// since nothing else exercises it in either direction.

/// A minimal `items`/`user` policy shape identical to the one
/// `native_dispatch_identity.rs`'s headline native-FDAE test uses --
/// `service_system`'s empty `capabilities` can never be entitled to the
/// `view` permission's `["creator", "caller"]` path, so `compile_read` falls
/// to `deny_all()` regardless of which row is asked for.
fn self_proxy_items_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "items": {
                    "table": "items",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]}
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

/// Builds the same `proxy-caller`/`proxy-callee` harness as
/// `test_route_handler_with_proxy_components`, plus a real
/// `SynSvcNativeService` registered for `proxy-caller`'s own `data-layer`
/// interface (the same-service self-proxy ingress). `fdae_policy` lets the
/// two tests below construct that service with and without a deployed
/// policy.
async fn test_route_handler_with_self_native_data_layer(
    fdae_policy: Option<Arc<Policy>>,
) -> Option<RouteHandler> {
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
    // Same-service native `data-layer` channel for `proxy-caller` -- what
    // makes the self-proxy ingress reachable at all (`proxy.rs:224-231`'s
    // same-service exception).
    registry
        .register(
            "proxy-caller".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "proxy-caller".to_string() },
        )
        .await
        .unwrap();

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let native_service = Arc::new(SynSvcNativeService::new(
        "proxy-caller".to_string(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider.clone(),
        messaging_broker.clone(),
        fdae_policy,
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
    ));
    native_dispatch.insert("proxy-caller".to_string(), native_service as Arc<dyn NativeService>);

    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let deps = RouteHandlerDeps {
        key_store,
        storage_provider,
        app_sandbox_engine,
        messaging_broker,
        native_dispatch,
        http_routes,
        control_plane_service: Arc::new(NoopControlPlane),
    };

    Some(
        RouteHandler::init(
            "test-orchestrator".to_string(),
            &config,
            registry,
            [12u8; 32],
            None,
            deps,
        )
        .await
        .unwrap(),
    )
}

/// Drives `proxy-caller`'s `call_peer` against its own `data-layer`
/// interface: `create-collection` + `put` + `get`, all through
/// `syneroym:proxy/proxy::call`.
async fn self_proxy_call(route_handler: &RouteHandler, method: &str, params: Value) -> Value {
    let call_params = json!({
        "service": "proxy-caller",
        "interface": "data-layer",
        "method": method,
        "params": params.to_string(),
    });
    let body = json_rpc_body("call-peer", call_params);
    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    serde_json::from_slice(&response_bytes).unwrap()
}

/// Baseline, policy-absent: a guest proxying to its **own** service's
/// `data-layer` reaches `SynSvcNativeService` and reads normally -- pins the
/// same-service exception as intended behavior, worth having regardless of
/// FDAE (a future tightening of the gate that broke it would fail this).
#[tokio::test]
async fn guest_self_proxy_data_layer_reads_normally_when_policy_absent() {
    let Some(route_handler) = test_route_handler_with_self_native_data_layer(None).await else {
        eprintln!("skipping: proxy-test/greeter wasm artifacts not built");
        return;
    };

    let resp = self_proxy_call(&route_handler, "create-collection", json!({"name": "items"})).await;
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    let resp = self_proxy_call(
        &route_handler,
        "put",
        json!({"collection": "items", "value": {"id": "1", "payload": b"{}".to_vec()}}),
    )
    .await;
    assert!(resp.get("error").is_none(), "put failed: {resp:?}");

    let resp =
        self_proxy_call(&route_handler, "get", json!({"collection": "items", "id": "1"})).await;
    assert!(resp.get("error").is_none(), "get failed: {resp:?}");
    let result = resp.get("result").and_then(Value::as_str).unwrap_or_default();
    assert_ne!(result, "null", "policy-absent self-proxy read must return the row: {result:?}");
    assert!(result.contains("\"id\":\"1\""), "expected the seeded row, got: {result:?}");
}

/// Policy-present pin: the same self-proxy `get` against a service
/// constructed with `Some(policy)` returns empty, because `proxy::Host::call`
/// synthesizes `service_system` (`host_capabilities.rs:670`), which holds no
/// capability the policy's `view` permission can be entitled through.
/// D-04-02-h (`task.md`): whoever threads real caller identity into this
/// ingress should flip this assertion to the rows the real caller can see.
#[tokio::test]
async fn guest_self_proxy_data_layer_returns_empty_when_policy_present() {
    let policy = Arc::new(self_proxy_items_policy());
    let Some(route_handler) = test_route_handler_with_self_native_data_layer(Some(policy)).await
    else {
        eprintln!("skipping: proxy-test/greeter wasm artifacts not built");
        return;
    };

    let resp = self_proxy_call(&route_handler, "create-collection", json!({"name": "items"})).await;
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    let resp = self_proxy_call(
        &route_handler,
        "put",
        json!({"collection": "items", "value": {"id": "1", "payload": b"{}".to_vec()}}),
    )
    .await;
    assert!(resp.get("error").is_none(), "put failed: {resp:?}");

    let resp =
        self_proxy_call(&route_handler, "get", json!({"collection": "items", "id": "1"})).await;
    assert!(resp.get("error").is_none(), "get failed: {resp:?}");
    let result = resp.get("result").and_then(Value::as_str).unwrap_or_default();
    assert_eq!(
        result, "null",
        "a guest's self-proxy read under a loaded policy must be empty -- D-04-02-h: {result:?}"
    );
}

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

use std::{
    fs,
    sync::{Arc, Weak},
};

use dashmap::DashMap;
use serde_json::{Value, json};
use syneroym_app_orchestration::{
    AppInstanceId, LogicalResolver, LogicalServiceName, ServiceId, StaticInventory, TopologyEntry,
    TopologyEpoch, TopologyMode,
};
use syneroym_control_plane::SynSvcNativeService;
use syneroym_core::{
    config::SubstrateConfig,
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
    test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{
    SqliteStorageProvider, StorageProvider, host_store::RecordWriteValue as HostRecordWriteValue,
};
use syneroym_data_keystore::KeyStore;
use syneroym_fdae::{Policy, parse_and_validate};
use syneroym_identity::Identity;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{
    AdaptationStage, EncryptionStage, RouteHandler, RouteHandlerDeps, RoutePipeline, RoutePreamble,
    RouteProtocol, RouteTransport, ServiceStage, TransportStage,
};
use syneroym_rpc::{
    Ability, AuthLevel, CallerContext, Capability, NativeDispatchRegistry, NativeInvocation,
    NativeResponse, NativeService, ResourceUri, RowAuthorizer, RpcResult, SessionContext,
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
        instance_certificate: None,
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
            syneroym_app_orchestration::empty_resolver(),
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
        control_plane: None,
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

/// Same as `test_route_handler_with_proxy_components`, but `proxy-caller` is
/// deployed as part of app instance `"app-1"`, with a declared dependency
/// `"callee-dep"` bound to `proxy-callee` -- so a guest driving `call-peer`
/// with `target-kind = "dependency"` exercises A2's real host-side
/// resolution path end to end, not just the Rust-level unit tests in
/// `sandbox_wasm::host_capabilities`.
async fn test_route_handler_with_a_bound_dependency() -> Option<(RouteHandler, Arc<LogicalResolver>)>
{
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

    registry
        .set_app_context(
            "proxy-caller".to_string(),
            "app-1".to_string(),
            "proxy-caller-svc".to_string(),
        )
        .await
        .unwrap();

    let app_registry = Arc::new(StaticInventory::new());
    let logical_resolver = Arc::new(LogicalResolver::new(app_registry));
    logical_resolver.register(
        AppInstanceId::new("app-1"),
        LogicalServiceName::new("callee-dep"),
        TopologyEntry {
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new("did:key:zProxyCallee")],
            sharding_strategy: None,
            epoch: TopologyEpoch::default(),
            cache_ttl: std::time::Duration::from_secs(60),
        },
    );

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

    app_sandbox_engine
        .deploy_wasm("proxy-caller", &wasm_deploy_manifest(proxy_test_bytes))
        .await
        .unwrap();
    app_sandbox_engine
        .deploy_wasm("did:key:zProxyCallee", &wasm_deploy_manifest(greeter_bytes))
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
            "did:key:zProxyCallee".to_string(),
            test_constants::GREETER_INTERFACE_NAME.to_string(),
            SubstrateEndpoint::WasmChannel { service_id: "did:key:zProxyCallee".to_string() },
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
        control_plane: None,
    };

    Some((
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
        logical_resolver,
    ))
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
        "target-kind": "service",
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

/// A2 (ADR-0021 §2), the guest side, driven live: `proxy-caller` names its
/// declared dependency `"callee-dep"` -- not `proxy-callee`'s DID -- and the
/// host resolves it before the request is built. Then the binding is
/// re-registered to a different member, and the *same* declared name reaches
/// the new target on the next call with no guest-visible change -- proving a
/// guest never holds the resolved identifier and cannot snapshot it past a
/// re-push (the same claim `dependency_binding_e2e` would prove across two
/// real substrates).
#[tokio::test]
async fn guest_dependency_target_reaches_the_bound_member_and_a_re_registration_takes_effect_on_the_next_call()
 {
    let Some((route_handler, logical_resolver)) =
        test_route_handler_with_a_bound_dependency().await
    else {
        eprintln!("skipping: proxy-test/greeter wasm artifacts not built");
        return;
    };

    let params = json!({
        "service": "callee-dep",
        "interface": test_constants::GREETER_INTERFACE_NAME,
        "method": "greet",
        "params": "[\"World\"]",
        "target-kind": "dependency",
    });
    let body = json_rpc_body("call-peer", params.clone());

    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response_bytes).unwrap();
    assert!(response.get("error").is_none(), "call-peer failed: {response:?}");
    let result = response.get("result").and_then(Value::as_str).unwrap_or_default();
    assert!(
        result.contains("Hello, World!"),
        "expected the bound member's greeting in the result, got: {result:?}"
    );

    // Re-register the same declared name onto a target that doesn't exist
    // (`greeter`'s WIT interface isn't registered under this id) -- if the
    // guest had somehow captured `proxy-callee`'s resolved DID rather than
    // re-resolving `callee-dep` on every call, this would still succeed.
    logical_resolver.register(
        AppInstanceId::new("app-1"),
        LogicalServiceName::new("callee-dep"),
        TopologyEntry {
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new("did:key:zNoSuchMember")],
            sharding_strategy: None,
            epoch: TopologyEpoch(1),
            cache_ttl: std::time::Duration::from_secs(60),
        },
    );

    let body = json_rpc_body("call-peer", params);
    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), None, &body)
        .await
        .unwrap();
    let response: Value = serde_json::from_slice(&response_bytes).unwrap();
    let result = response.get("result").and_then(Value::as_str).unwrap_or_default();
    assert!(
        result.contains("ServiceNotFound") || response.get("error").is_some(),
        "re-registering the binding must change what the *same* declared name resolves to on the \
         very next call, proving the guest never held a snapshot of the old target: {response:?}"
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
        "target-kind": "service",
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
/// tests below construct that service with and without a deployed policy.
/// Also returns `storage_provider`/`key_store` so a test can seed a row
/// directly (bypassing the guest's own `put`) to control exactly which
/// principal a row belongs to.
async fn test_route_handler_with_self_native_data_layer(
    fdae_policy: Option<Arc<Policy>>,
) -> Option<(RouteHandler, Arc<dyn StorageProvider>, Arc<KeyStore>)> {
    let proxy_test_bytes = fs::read(test_constants::proxy_test_wasm_path()).ok()?;
    let greeter_bytes = fs::read(test_constants::greeter_wasm_path()).ok()?;

    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
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
            syneroym_app_orchestration::empty_resolver(),
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
        Arc::new(Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    native_dispatch.insert("proxy-caller".to_string(), native_service as Arc<dyn NativeService>);

    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let deps = RouteHandlerDeps {
        key_store: key_store.clone(),
        storage_provider: storage_provider.clone(),
        app_sandbox_engine,
        messaging_broker,
        native_dispatch,
        http_routes,
        control_plane_service: Arc::new(NoopControlPlane),
        control_plane: None,
    };

    let route_handler = RouteHandler::init(
        "test-orchestrator".to_string(),
        &config,
        registry,
        [12u8; 32],
        None,
        deps,
    )
    .await
    .unwrap();

    Some((route_handler, storage_provider, key_store))
}

/// Same shape as `test_route_handler_with_self_native_data_layer`, but
/// wires `proxy-caller`'s `SynSvcNativeService` with a **real**
/// `Weak<dyn RowAuthorizer>` (`row_authorizer_for`, mirroring
/// `abac_integration.rs`'s own helper) instead of `empty_row_authorizer()`.
/// `proxy-caller` (the `proxy-test` fixture) exports
/// `syneroym:data-layer/authorizer` for exactly this reason -- Slice
/// B4-fdae's router-side ingress-(ii) proof that a self-proxy `get` actually
/// invokes the stage-4 after-step, not just the sieve.
async fn test_route_handler_with_self_native_data_layer_and_stage4(
    fdae_policy: Arc<Policy>,
) -> Option<(RouteHandler, Arc<dyn StorageProvider>, Arc<KeyStore>)> {
    let proxy_test_bytes = fs::read(test_constants::proxy_test_wasm_path()).ok()?;
    let greeter_bytes = fs::read(test_constants::greeter_wasm_path()).ok()?;

    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
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
            syneroym_app_orchestration::empty_resolver(),
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
    registry
        .register(
            "proxy-caller".to_string(),
            "data-layer".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: "proxy-caller".to_string() },
        )
        .await
        .unwrap();

    // Coerces the concrete engine to `Arc<dyn RowAuthorizer>` at this typed
    // `let`, then downgrades -- same unsized-coercion pattern
    // `abac_integration.rs::row_authorizer_for` uses.
    let row_authorizer: Weak<dyn RowAuthorizer> = {
        let trait_object: Arc<dyn RowAuthorizer> = app_sandbox_engine.clone();
        Arc::downgrade(&trait_object)
    };

    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let native_service = Arc::new(SynSvcNativeService::new(
        "proxy-caller".to_string(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider.clone(),
        messaging_broker.clone(),
        Some(fdae_policy),
        Arc::new(Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        row_authorizer,
        None,
    ));
    native_dispatch.insert("proxy-caller".to_string(), native_service as Arc<dyn NativeService>);

    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let deps = RouteHandlerDeps {
        key_store: key_store.clone(),
        storage_provider: storage_provider.clone(),
        app_sandbox_engine,
        messaging_broker,
        native_dispatch,
        http_routes,
        control_plane_service: Arc::new(NoopControlPlane),
        control_plane: None,
    };

    let route_handler = RouteHandler::init(
        "test-orchestrator".to_string(),
        &config,
        registry,
        [13u8; 32],
        None,
        deps,
    )
    .await
    .unwrap();

    Some((route_handler, storage_provider, key_store))
}

/// Drives `proxy-caller`'s `call_peer` against its own `data-layer`
/// interface: `create-collection` + `put` + `get`, all through
/// `syneroym:proxy/proxy::call`. `caller` is forwarded to
/// `dispatch_json_rpc_once` verbatim -- `None` for an unauthenticated
/// connection (today's baseline), `Some` for a router-verified caller
/// (D-04-02-h ingress (ii)'s closure).
async fn self_proxy_call(
    route_handler: &RouteHandler,
    method: &str,
    params: Value,
    caller: Option<&CallerContext>,
) -> Value {
    let call_params = json!({
        "service": "proxy-caller",
        "interface": "data-layer",
        "method": method,
        "params": params.to_string(),
        "target-kind": "service",
    });
    let body = json_rpc_body("call-peer", call_params);
    let response_bytes = route_handler
        .dispatch_json_rpc_once(&call_peer_pipeline(), &call_peer_preamble(), caller, &body)
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
    let Some((route_handler, _storage_provider, _key_store)) =
        test_route_handler_with_self_native_data_layer(None).await
    else {
        eprintln!("skipping: proxy-test/greeter wasm artifacts not built");
        return;
    };

    let resp =
        self_proxy_call(&route_handler, "create-collection", json!({"name": "items"}), None).await;
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    let resp = self_proxy_call(
        &route_handler,
        "put",
        json!({"collection": "items", "value": {"id": "1", "payload": b"{}".to_vec()}}),
        None,
    )
    .await;
    assert!(resp.get("error").is_none(), "put failed: {resp:?}");

    let resp =
        self_proxy_call(&route_handler, "get", json!({"collection": "items", "id": "1"}), None)
            .await;
    assert!(resp.get("error").is_none(), "get failed: {resp:?}");
    let result = resp.get("result").and_then(Value::as_str).unwrap_or_default();
    assert_ne!(result, "null", "policy-absent self-proxy read must return the row: {result:?}");
    assert!(result.contains("\"id\":\"1\""), "expected the seeded row, got: {result:?}");
}

/// Pins the attribution spec: `put`/`batch-mutate`'s `creator_id` stamping
/// (`SynSvcNativeService::dispatch_data_layer`, via
/// `CallerContext::write_attribution`) attributes a self-proxy write to the
/// real forwarded caller, not the service. The guest's *direct* WIT `put`
/// (`host_capabilities.rs`) calls the very same `write_attribution` now, so
/// the two ingresses agree whenever a real principal is present -- they
/// diverge only when the caller is a synthesized substrate identity
/// (`System`/`LocalElevated`/`LocalReadOnly`), which has none to attribute
/// to, and stamps the service's own `component_id` instead. Originally
/// pinned as an open inconsistency; now the settled spec, not a known gap.
#[tokio::test]
async fn guest_self_proxy_put_attributes_creator_id_to_the_real_caller_not_the_service() {
    let Some((route_handler, _storage_provider, _key_store)) =
        test_route_handler_with_self_native_data_layer(None).await
    else {
        eprintln!("skipping: proxy-test/greeter wasm artifacts not built");
        return;
    };

    const REAL_CALLER_DID: &str = "did:key:zSelfProxyWriterB35";
    let real_caller = CallerContext {
        caller_did: REAL_CALLER_DID.to_string(),
        app_instance: None,
        // `subject_did` mirrors `caller_did`, as production's `build_caller`
        // always sets it -- `write_attribution` reads the verified
        // session identity, not the raw `caller_did` field.
        session: SessionContext { subject_did: REAL_CALLER_DID.to_string(), ..Default::default() },
        auth: AuthLevel::Ucan,
        proof: None,
    };

    let resp =
        self_proxy_call(&route_handler, "create-collection", json!({"name": "items"}), None).await;
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    let resp = self_proxy_call(
        &route_handler,
        "put",
        json!({"collection": "items", "value": {"id": "1", "payload": b"{}".to_vec()}}),
        Some(&real_caller),
    )
    .await;
    assert!(resp.get("error").is_none(), "put failed: {resp:?}");

    let resp =
        self_proxy_call(&route_handler, "get", json!({"collection": "items", "id": "1"}), None)
            .await;
    assert!(resp.get("error").is_none(), "get failed: {resp:?}");
    let result = resp.get("result").and_then(Value::as_str).unwrap_or_default();
    assert!(
        result.contains(&format!("\"creator_id\":\"{REAL_CALLER_DID}\"")),
        "a self-proxy write must attribute creator_id to the real caller, not the service -- got \
         {result:?}"
    );
}

/// No-verified-caller pin: the same self-proxy `get` against a service
/// constructed with `Some(policy)`, dispatched with `caller: None` (an
/// unauthenticated connection -- WASM guests admit these, design §6.1.2),
/// returns empty, because `HostState.caller` (and so `proxy::Host::call`'s
/// forwarded self-proxy caller) is `service_system`, which holds no
/// capability the policy's `view` permission can be entitled through. This
/// is the one D-04-02-h case Slice B3.5-fdae does **not** change -- an
/// anonymous connection still can't be filtered *for* anyone. See
/// `guest_self_proxy_data_layer_filters_for_a_real_caller_d04_02_h_closed`
/// below for the closed case: a real, router-verified caller.
#[tokio::test]
async fn guest_self_proxy_data_layer_returns_empty_when_policy_present() {
    let policy = Arc::new(self_proxy_items_policy());
    let Some((route_handler, storage_provider, key_store)) =
        test_route_handler_with_self_native_data_layer(Some(policy)).await
    else {
        eprintln!("skipping: proxy-test/greeter wasm artifacts not built");
        return;
    };

    let resp =
        self_proxy_call(&route_handler, "create-collection", json!({"name": "items"}), None).await;
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    // Seeded directly against the store, `auth: None` -- a self-proxy `put`
    // with no verified caller is exactly the `AuthLevel::System` write the
    // write-side gate denies closed under a policy (this fixture's
    // `self_proxy_items_policy` declares no `data-layer/write` permission at
    // all, so the fixture's own `put` would fail regardless of caller
    // identity). This test is about the *read* side (D-04-02-h); seeding
    // must not itself exercise the write-side gate.
    let store = storage_provider.open_service_db("proxy-caller", &key_store).await.unwrap();
    store
        .put(
            "items",
            &HostRecordWriteValue { id: "1".to_string(), payload: b"{}".to_vec() },
            "proxy-caller",
            None,
        )
        .await
        .unwrap();
    drop(store);

    let resp =
        self_proxy_call(&route_handler, "get", json!({"collection": "items", "id": "1"}), None)
            .await;
    assert!(resp.get("error").is_none(), "get failed: {resp:?}");
    let result = resp.get("result").and_then(Value::as_str).unwrap_or_default();
    assert_eq!(
        result, "null",
        "an unauthenticated connection's self-proxy read under a loaded policy must be empty -- \
         D-04-02-h: {result:?}"
    );
}

/// A `principal_column`-direct policy (`items.creator_id`, the physical
/// column the host already stamps on every `put`) matched straight against
/// the caller -- no `creator`/`user` join, since the point here is proving
/// the caller now *reaches* `HostState`/`NativeInvocation.caller` at all,
/// not exercising the ReBAC join compiler (already covered elsewhere).
fn self_proxy_items_principal_column_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "items": {
                    "table": "items",
                    "principal_column": "creator_id",
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["caller"]]}
                    }
                }
            }
        }"#,
    )
    .unwrap()
}

/// Slice B3.5-fdae closure of D-04-02-h ingress (ii): a **real**,
/// router-verified caller reaching the guest (`dispatch_json_rpc_once`'s
/// `caller: Some(&real_caller)`) now flows all the way through
/// `HostState.caller` into `proxy::Host::call`'s self-proxy branch (the
/// service's own `data-layer`), which forwards it instead of re-synthesizing
/// `service_system` -- so `SynSvcNativeService::resolve_query_auth` (no
/// `AuthLevel` carve-out, by design) sees who is actually asking.
///
/// Seeds two rows directly (bypassing the guest's self-proxy `put`, so each
/// row's `creator_id` is exactly the principal this test intends, not
/// whatever `put`'s own caller-derived stamping would produce) -- one owned
/// by the real caller, one by a different principal -- and asserts the
/// self-proxy `get` reaches only its own.
#[tokio::test]
async fn guest_self_proxy_data_layer_filters_for_a_real_caller_d04_02_h_closed() {
    let policy = Arc::new(self_proxy_items_principal_column_policy());
    let Some((route_handler, storage_provider, key_store)) =
        test_route_handler_with_self_native_data_layer(Some(policy)).await
    else {
        eprintln!("skipping: proxy-test/greeter wasm artifacts not built");
        return;
    };

    let resp =
        self_proxy_call(&route_handler, "create-collection", json!({"name": "items"}), None).await;
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    const REAL_CALLER_DID: &str = "did:key:zSelfProxyRealCallerB35";
    let real_caller = CallerContext {
        caller_did: REAL_CALLER_DID.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: REAL_CALLER_DID.to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::service("proxy-caller", "proxy-caller"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };

    let store = storage_provider.open_service_db("proxy-caller", &key_store).await.unwrap();
    store
        .put(
            "items",
            &HostRecordWriteValue { id: "own".to_string(), payload: b"{}".to_vec() },
            REAL_CALLER_DID,
            None,
        )
        .await
        .unwrap();
    store
        .put(
            "items",
            &HostRecordWriteValue { id: "someone-elses".to_string(), payload: b"{}".to_vec() },
            "did:key:zSomeoneElse",
            None,
        )
        .await
        .unwrap();
    drop(store);

    let resp = self_proxy_call(
        &route_handler,
        "get",
        json!({"collection": "items", "id": "own"}),
        Some(&real_caller),
    )
    .await;
    assert!(resp.get("error").is_none(), "get(own) failed: {resp:?}");
    let own_result = resp.get("result").and_then(Value::as_str).unwrap_or_default();
    assert_ne!(own_result, "null", "the real caller must reach their own row: {own_result:?}");
    assert!(own_result.contains("\"id\":\"own\""), "expected the own row, got: {own_result:?}");

    let resp = self_proxy_call(
        &route_handler,
        "get",
        json!({"collection": "items", "id": "someone-elses"}),
        Some(&real_caller),
    )
    .await;
    assert!(resp.get("error").is_none(), "get(someone-elses) failed: {resp:?}");
    let other_result = resp.get("result").and_then(Value::as_str).unwrap_or_default();
    assert_eq!(
        other_result, "null",
        "the real caller must not reach a row owned by a different principal: {other_result:?}"
    );
}

// -- Slice B4-fdae: ingress (ii) applies the stage-4 after-step -----------

/// Same `items`/`principal_column` shape as
/// `self_proxy_items_principal_column_policy`, but `view` opts into the
/// stage-4 after-step (ADR-0017 §7).
fn self_proxy_items_principal_column_policy_with_stage4() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "items": {
                    "table": "items",
                    "principal_column": "creator_id",
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["caller"]],
                            "authorize_rows": true
                        }
                    }
                }
            }
        }"#,
    )
    .unwrap()
}

/// Router-side ingress-(ii) proof (`SynSvcNativeService::dispatch_data_layer`'s
/// `"get"` arm): a guest's self-proxy `get`, reached through
/// `syneroym:proxy/proxy::call` exactly like
/// `guest_self_proxy_data_layer_filters_for_a_real_caller_d04_02_h_closed`,
/// runs the stage-4 after-step on top of the sieve -- not just the sieve
/// alone. Seeds two rows the sieve equally admits (both owned by the real
/// caller); `proxy-caller`'s own exported `authorize-rows` (`proxy-test`'s
/// fixture behavior, `src/lib.rs`) denies only the one seeded with id
/// `"secret"`, so a row surviving the sieve is still reachable only if the
/// after-step also allows it.
#[tokio::test]
async fn guest_self_proxy_data_layer_applies_stage4() {
    let policy = Arc::new(self_proxy_items_principal_column_policy_with_stage4());
    let Some((route_handler, storage_provider, key_store)) =
        test_route_handler_with_self_native_data_layer_and_stage4(policy).await
    else {
        eprintln!("skipping: proxy-test/greeter wasm artifacts not built");
        return;
    };

    let resp =
        self_proxy_call(&route_handler, "create-collection", json!({"name": "items"}), None).await;
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    const REAL_CALLER_DID: &str = "did:key:zSelfProxyStage4Caller";
    let real_caller = CallerContext {
        caller_did: REAL_CALLER_DID.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: REAL_CALLER_DID.to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::service("proxy-caller", "proxy-caller"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };

    let store = storage_provider.open_service_db("proxy-caller", &key_store).await.unwrap();
    store
        .put(
            "items",
            &HostRecordWriteValue { id: "secret".to_string(), payload: b"{}".to_vec() },
            REAL_CALLER_DID,
            None,
        )
        .await
        .unwrap();
    store
        .put(
            "items",
            &HostRecordWriteValue { id: "normal".to_string(), payload: b"{}".to_vec() },
            REAL_CALLER_DID,
            None,
        )
        .await
        .unwrap();
    drop(store);

    let resp = self_proxy_call(
        &route_handler,
        "get",
        json!({"collection": "items", "id": "secret"}),
        Some(&real_caller),
    )
    .await;
    assert!(resp.get("error").is_none(), "get(secret) failed: {resp:?}");
    let secret_result = resp.get("result").and_then(Value::as_str).unwrap_or_default();
    assert_eq!(
        secret_result, "null",
        "the sieve admits this row (same caller owns it), but the after-step must still deny it: \
         {secret_result:?}"
    );

    let resp = self_proxy_call(
        &route_handler,
        "get",
        json!({"collection": "items", "id": "normal"}),
        Some(&real_caller),
    )
    .await;
    assert!(resp.get("error").is_none(), "get(normal) failed: {resp:?}");
    let normal_result = resp.get("result").and_then(Value::as_str).unwrap_or_default();
    assert_ne!(
        normal_result, "null",
        "a row the after-step allows must still reach the caller: {normal_result:?}"
    );
    assert!(
        normal_result.contains("\"id\":\"normal\""),
        "expected the normal row, got: {normal_result:?}"
    );
}

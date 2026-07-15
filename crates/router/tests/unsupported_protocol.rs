#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Slice A1 (M04A): a caller declaring a protocol scheme this node does not
//! speak (`wrpc://`, any `Other(_)` scheme) gets a typed *unsupported-
//! protocol* JSON-RPC error (`-32091`), not the confusing "missing dir="
//! error the pre-A1 code produced by falling into the ADR-0014 raw-stream
//! path (see plan.md Flag F2). The minimal `[LFC-VER]` behavior kept from
//! the deferred protocol-negotiation slice (A.7).

use std::sync::Arc;

use dashmap::DashMap;
use serde_json::{Value, json};
use syneroym_core::{
    config::SubstrateConfig,
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{RouteHandler, RouteHandlerDeps, RoutePreamble};
use syneroym_rpc::{
    NativeDispatchRegistry, NativeInvocation, NativeResponse, NativeService, RpcResult,
};
use syneroym_sandbox_wasm::AppSandboxEngine;

#[derive(Debug, Default)]
struct NoopControlPlane;

#[async_trait::async_trait]
impl NativeService for NoopControlPlane {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        Ok(NativeResponse { payload: Value::Null })
    }
}

async fn test_route_handler() -> RouteHandler {
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
    RouteHandler::init("test-orchestrator".to_string(), &config, registry, [11u8; 32], None, deps)
        .await
        .unwrap()
}

#[tokio::test]
async fn wrpc_scheme_yields_a_typed_unsupported_protocol_error() {
    let route_handler = test_route_handler().await;

    let preamble = RoutePreamble::parse("wrpc://some-iface|some-svc").unwrap();
    let endpoint = SubstrateEndpoint::WasmChannel { service_id: "some-svc".to_string() };
    let pipeline = route_handler.plan_pipeline(&preamble, &endpoint);

    let body =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": "whatever", "params": {}, "id": 7}))
            .unwrap();
    let response_bytes =
        route_handler.dispatch_json_rpc_once(&pipeline, &preamble, None, &body).await.unwrap();
    let response: Value = serde_json::from_slice(&response_bytes).unwrap();

    assert_eq!(
        response["error"]["code"], -32091,
        "expected the reserved unsupported-protocol code, got: {response:?}"
    );
    assert_eq!(response["id"], json!(7), "the request id should still round-trip");
    assert!(
        response["error"]["message"].as_str().unwrap_or_default().contains("json-rpc/v1"),
        "message should name what this node actually speaks: {response:?}"
    );
}

/// An unknown/custom scheme (`RouteProtocol::Other`) gets the same typed
/// treatment as `wrpc://`, not just the one hardcoded scheme.
#[tokio::test]
async fn other_custom_scheme_also_yields_unsupported_protocol() {
    let route_handler = test_route_handler().await;

    let preamble = RoutePreamble::parse("xrpc://iface|svc").unwrap();
    let endpoint = SubstrateEndpoint::NativeHostChannel { service_id: "svc".to_string() };
    let pipeline = route_handler.plan_pipeline(&preamble, &endpoint);

    let body =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": "x", "params": {}, "id": null}))
            .unwrap();
    let response_bytes =
        route_handler.dispatch_json_rpc_once(&pipeline, &preamble, None, &body).await.unwrap();
    let response: Value = serde_json::from_slice(&response_bytes).unwrap();

    assert_eq!(response["error"]["code"], -32091, "got: {response:?}");
}

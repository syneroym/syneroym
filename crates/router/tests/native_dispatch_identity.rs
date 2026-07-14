#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Slice B0 (M04A): native-dispatch identity threading -- "the single most
//! important test in this milestone" (task.md Tests Summary). Drives
//! `RouteHandler::dispatch_json_rpc_once`/`handle_http_stream` directly
//! (the wire handshake itself is `crates/router/src/handshake.rs`'s own
//! test responsibility) to prove:
//!
//! 1. An anonymous (`caller: None`) request to every native-capability
//!    interface (`data-layer`/`vault`/`app-config`/`blob-store`/ `messaging`)
//!    is rejected *before* the native service is invoked.
//! 2. The HTTP bridge rejects the same anonymous request, mapped to 401.
//! 3. An authenticated caller's identity reaches `SynSvcNativeService`'s
//!    `dispatch_data_layer` and becomes the stored `creator_id` -- not the
//!    service being called.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use dashmap::DashMap;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use syneroym_control_plane::SynSvcNativeService;
use syneroym_core::{
    config::SubstrateConfig,
    http_routes::{HttpRoute, HttpRouteRegistry},
    local_registry::EndpointRegistry,
    storage::MockStorage,
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
    AuthLevel, CallerContext, NativeDispatchRegistry, NativeInvocation, NativeResponse,
    NativeService, RpcResult, SessionContext,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

/// A native-service double recording whether `dispatch` was ever invoked --
/// proves rejection happens *before* the native service sees the request,
/// not just that the envelope reports an error.
#[derive(Debug, Default)]
struct RecordingNativeService {
    invoked: AtomicBool,
}

impl RecordingNativeService {
    fn was_invoked(&self) -> bool {
        self.invoked.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl NativeService for RecordingNativeService {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        self.invoked.store(true, Ordering::SeqCst);
        Ok(NativeResponse { payload: Value::Null })
    }
}

fn test_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext::default(),
        auth: AuthLevel::Delegated,
    }
}

/// Builds a minimal `RouteHandler` with empty `native_dispatch`/
/// `http_routes` tables the test populates itself.
async fn test_route_handler() -> (RouteHandler, HttpRouteRegistry) {
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
        http_routes: http_routes.clone(),
        control_plane_service: Arc::new(RecordingNativeService::default()),
    };

    let route_handler =
        RouteHandler::init("test-orchestrator".to_string(), &config, registry, [7u8; 32], deps)
            .await
            .unwrap();

    (route_handler, http_routes)
}

fn raw_pipeline(service_id: &str) -> RoutePipeline {
    RoutePipeline {
        encryption: EncryptionStage::None,
        transport: TransportStage::Binary,
        adaptation: AdaptationStage::None,
        service: ServiceStage::NativeService { service_id: service_id.to_string() },
    }
}

fn preamble_for(service_id: &str, interface: &str) -> RoutePreamble {
    RoutePreamble {
        transport: RouteTransport::Binary,
        protocol: RouteProtocol::JsonRpc,
        interface: interface.to_string(),
        service_id: service_id.to_string(),
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

const NATIVE_CAPABILITY_INTERFACES: &[&str] =
    &["data-layer", "vault", "app-config", "blob-store", "messaging"];

#[tokio::test]
async fn anonymous_caller_rejected_before_native_dispatch_for_every_interface() {
    let (route_handler, _http_routes) = test_route_handler().await;
    let service = Arc::new(RecordingNativeService::default());
    route_handler.register_native_service("raw-test-svc".to_string(), service.clone());

    let pipeline = raw_pipeline("raw-test-svc");
    let body = json_rpc_body("get", json!({}));

    for interface in NATIVE_CAPABILITY_INTERFACES {
        let preamble = preamble_for("raw-test-svc", interface);
        let result = route_handler.dispatch_json_rpc_once(&pipeline, &preamble, None, &body).await;
        assert!(result.is_err(), "interface {interface} must reject an anonymous caller");
        assert!(
            !service.was_invoked(),
            "native service must not be invoked for interface {interface} with no caller"
        );
    }
}

#[tokio::test]
async fn authenticated_caller_reaches_native_dispatch() {
    let (route_handler, _http_routes) = test_route_handler().await;
    let service = Arc::new(RecordingNativeService::default());
    route_handler.register_native_service("raw-test-svc-2".to_string(), service.clone());

    let pipeline = raw_pipeline("raw-test-svc-2");
    let preamble = preamble_for("raw-test-svc-2", "data-layer");
    let body = json_rpc_body("get", json!({}));
    let caller = test_caller("did:key:z6MkTestCaller");

    let result =
        route_handler.dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body).await;
    assert!(result.is_ok(), "authenticated caller must be admitted: {result:?}");
    assert!(service.was_invoked(), "native service must be invoked for an authenticated caller");
}

#[tokio::test]
async fn authenticated_caller_identity_becomes_creator_id_not_service_id() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "creator-id-test-svc".to_string();
    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.clone(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller_did = "did:key:z6MkAuthenticatedCaller";
    let caller = test_caller(caller_did);

    let create_body = json_rpc_body("create-collection", json!({"name": "items", "indexes": []}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &create_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    let put_body = json_rpc_body(
        "put",
        json!({"collection": "items", "value": {"id": "1", "payload": b"{}".to_vec()}}),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &put_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "put failed: {resp:?}");

    let get_body = json_rpc_body("get", json!({"collection": "items", "id": "1"}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &get_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    let result = resp.get("result").expect("get must return a result");
    assert_eq!(
        result["creator_id"], caller_did,
        "creator_id must be the caller's DID, not the callee service"
    );
    assert_ne!(result["creator_id"], service_id);
}

#[tokio::test]
async fn http_bridge_rejects_anonymous_caller_with_401() {
    let (route_handler, http_routes) = test_route_handler().await;
    let service = Arc::new(RecordingNativeService::default());
    let service_id = "http-test-svc".to_string();
    route_handler.register_native_service(service_id.clone(), service.clone());

    http_routes.insert(
        service_id.clone(),
        vec![HttpRoute {
            method: "GET".to_string(),
            path: "/items/{id}".to_string(),
            target: "data-layer".to_string(),
            operation: "get".to_string(),
            collection: Some("items".to_string()),
            topic: None,
            protocol: None,
        }],
    );

    let pipeline = RoutePipeline {
        encryption: EncryptionStage::None,
        transport: TransportStage::Http,
        adaptation: AdaptationStage::None,
        service: ServiceStage::NativeService { service_id: service_id.clone() },
    };
    let preamble = RoutePreamble {
        transport: RouteTransport::Http,
        protocol: RouteProtocol::JsonRpc,
        interface: String::new(),
        service_id: service_id.clone(),
        enc: None,
        pubkey: None,
        delegation: None,
        dir: None,
    };

    let (mut client, server) = duplex(4096);
    let io = TokioIo::new(server);
    let handle =
        tokio::spawn(route_handler.clone().handle_http_stream(io, preamble, pipeline, None));

    client
        .write_all(b"GET /items/abc HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    client.shutdown().await.unwrap();

    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8_lossy(&response);
    assert!(response.starts_with("HTTP/1.1 401"), "expected 401, got: {response}");

    handle.await.unwrap().unwrap();
    assert!(
        !service.was_invoked(),
        "native service must not be invoked for an anonymous HTTP request"
    );
}

/// `messaging/subscribe` is a native-capability call that never reaches
/// `dispatch_json_rpc_once` (it's a long-lived push stream special-cased in
/// `handle_binary_stream`, not a request/response `NativeService::dispatch`
/// call) -- it needs its own `None`-caller gate, checked here directly
/// rather than through the request/response tests above.
#[tokio::test]
async fn messaging_subscribe_rejected_for_anonymous_caller() {
    let (route_handler, _http_routes) = test_route_handler().await;
    let service_id = "messaging-subscribe-test-svc".to_string();
    route_handler
        .register_native_service(service_id.clone(), Arc::new(RecordingNativeService::default()));

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "messaging");

    let (mut client, server) = duplex(4096);
    let (server_read, server_write) = tokio::io::split(server);
    let reader = tokio::io::BufReader::new(server_read);

    let handle = tokio::spawn(async move {
        let preamble = preamble;
        let pipeline = pipeline;
        route_handler
            .handle_binary_stream(
                reader,
                server_write,
                &preamble,
                &pipeline,
                None,
                Box::pin(std::future::pending()),
            )
            .await
    });

    let request = json_rpc_body("subscribe", json!({"topic": "orders/new"}));
    syneroym_rpc::framing::write_frame(&mut client, &request).await.unwrap();

    let response_frame = syneroym_rpc::framing::read_frame(&mut client).await.unwrap();
    let response: Value = serde_json::from_slice(&response_frame).unwrap();
    assert!(
        response.get("error").is_some(),
        "anonymous subscribe must yield an error frame, got: {response:?}"
    );
    assert_ne!(
        response.get("result"),
        Some(&Value::String("subscribed".to_string())),
        "anonymous caller must never receive a subscribe ack"
    );

    drop(client);
    let _ = handle.await;
}

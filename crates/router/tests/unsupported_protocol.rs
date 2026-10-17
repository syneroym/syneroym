#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A caller declaring a protocol scheme this node does not
//! speak (`wrpc://`, any `Other(_)` scheme) gets a typed *unsupported-
//! protocol* JSON-RPC error (`-32091`), not the confusing "missing dir="
//! error an earlier version produced by falling into the ADR-0014
//! raw-stream path. The minimal `[LFC-VER]` behavior kept from the
//! deferred protocol-negotiation work.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;
use serde_json::{Value, json};
use syneroym_core::{
    config::SubstrateConfig,
    dht_registry::{EndpointInfo, EndpointMailbox, EndpointType},
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{Identity, substrate};
use syneroym_mailbox_format::MailboxPublicKey;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{
    RouteHandler, RouteHandlerDeps, RoutePreamble, peer_records::PeerRecordStore,
};
use syneroym_rpc::{
    CallOrigin, CallerContext, NativeDispatchRegistry, NativeInvocation, NativeResponse,
    NativeService, ProxyError, ProxyProtocol, ProxyRequest, RpcResult, ServiceProxy,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

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
    let mut config = SubstrateConfig::default();
    config.storage.db_dir = temp_dir.path().to_path_buf();
    route_handler_with(&config, temp_dir.path()).await
}

async fn route_handler_with(config: &SubstrateConfig, temp_dir: &Path) -> RouteHandler {
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let app_sandbox_engine = Arc::new(
        AppSandboxEngine::init(
            config,
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
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let deps = RouteHandlerDeps {
        logical_resolver: syneroym_app_orchestration::empty_resolver(),
        key_store,
        storage_provider,
        app_sandbox_engine,
        messaging_broker,
        native_dispatch: NativeDispatchRegistry::default(),
        native_http: Arc::new(DashMap::new()),
        websocket_senders: syneroym_rpc::WebSocketSenders::new(),
        http_routes,
        assets: Arc::new(DashMap::new()),
        sse_permits: Arc::new(DashMap::new()),
        control_plane_service: Arc::new(NoopControlPlane),
        control_plane: None,
        session_revocation: None,
    };
    RouteHandler::init("test-orchestrator".to_string(), config, registry, [11u8; 32], None, deps)
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

/// The proxy keeps the record of a peer with a mailbox in the node's
/// database directory, so a node can still find a mailbox when the peer's
/// own records are gone from the registry.
#[tokio::test]
async fn the_proxy_keeps_records_of_peers_with_a_mailbox_under_the_node_db_dir() {
    let master = Identity::generate().unwrap();
    let peer = substrate::derive_did_key(&master.public_key());
    let record = EndpointInfo {
        service_id: peer.clone(),
        // A substrate with no record anywhere: the peer is asleep.
        substrate_id: substrate::derive_did_key(&Identity::generate().unwrap().public_key()),
        endpoint_type: EndpointType::Service,
        mechanisms: vec![],
        nickname: None,
        is_private: false,
        ttl: None,
        not_after: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 3600,
        generation: 0,
        mailbox: Some(EndpointMailbox {
            service_id: peer.clone(),
            key: MailboxPublicKey::from_bytes([5; 32]),
        }),
    }
    .sign(&master)
    .unwrap();

    // A registry that knows only this record.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = SubstrateConfig::default();
    config.substrate.registry_url = Some(format!("http://{}", listener.local_addr().unwrap()));
    let body = serde_json::to_string(&record).unwrap();
    let wanted = format!("GET /lookup/{peer} ");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let (body, wanted) = (body.clone(), wanted.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let response = if String::from_utf8_lossy(&buf[..n]).starts_with(&wanted) {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
                         {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                };
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });

    let dir = tempfile::tempdir().unwrap();
    config.storage.db_dir = dir.path().join("db");
    let handler = route_handler_with(&config, dir.path()).await;
    let store_file = dir.path().join("db").join("peer_records.db");
    assert!(store_file.exists(), "the node must open its peer record store at boot");

    let request = ProxyRequest {
        target_service: peer.clone(),
        interface: "default".to_string(),
        method: "get".to_string(),
        params: Value::Null,
        caller: CallerContext::service_system("caller"),
        origin: CallOrigin::Native { service_id: None },
        protocol: ProxyProtocol::JsonRpcV1,
        idempotency_key: None,
        idempotent: false,
        timeout: Some(Duration::from_secs(10)),
    };
    let result = handler.proxy().expect("the node has a proxy").invoke(request).await;
    assert!(matches!(result, Err(ProxyError::ServiceNotFound(_))), "{result:?}");

    // The record is on disk, in the file the node opened.
    let kept = PeerRecordStore::open(&store_file, 0).unwrap().recall(&peer).await;
    assert_eq!(kept.map(|r| r.pkarr_packet_hex), Some(record.pkarr_packet_hex));
}

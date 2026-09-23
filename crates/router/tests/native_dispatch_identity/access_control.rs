use hyper_util::rt::TokioIo;
use syneroym_core::{http_routes::HttpRoute, local_registry::NATIVE_CAPABILITY_INTERFACES};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

use super::helpers::*;

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
        None,
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
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

/// `execute-ddl` (native) is gated on `data-layer/admin` on the service's
/// own resource (ADR-0015/0016) -- an ordinary caller must be denied.
#[tokio::test]
async fn execute_ddl_denied_for_ordinary_native_caller() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "ddl-deny-svc".to_string();
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
        None,
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller = test_caller("did:key:z6MkOrdinaryCaller");

    let body = json_rpc_body("execute-ddl", json!({"sql": "CREATE TABLE x (id TEXT)"}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32010,
        "an ordinary caller must be denied execute-ddl: {resp:?}"
    );
}

/// `drop-collection` (native) is gated on `data-layer/admin`, identical to
/// `execute-ddl`: dropping an existing collection bypasses any per-row
/// policy on it entirely, so an ordinary caller holding no admin capability
/// must be denied even though it could reach the same service's ordinary
/// write operations.
#[tokio::test]
async fn drop_collection_denied_for_ordinary_native_caller() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "drop-collection-deny-svc".to_string();
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
        None,
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller = test_caller("did:key:z6MkOrdinaryDropCollectionCaller");

    let body = json_rpc_body("drop-collection", json!({"name": "items"}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32010,
        "an ordinary caller must be denied drop-collection: {resp:?}"
    );
}

/// A caller matching `[iam].admin_ucan_root` -- represented by the
/// `substrate/admin` grant `build_caller` constructs for it -- must be
/// admitted to native `execute-ddl`.
#[tokio::test]
async fn execute_ddl_allowed_for_admin_ucan_root_native_caller() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "ddl-admin-svc".to_string();
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
        None,
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller = admin_caller("did:key:z6MkAdminRoot");

    let body = json_rpc_body("execute-ddl", json!({"sql": "CREATE TABLE x (id TEXT)"}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "admin_ucan_root caller must be admitted: {resp:?}");
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
            public: false,
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
        ucan: None,
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

/// The original `POST`+`application/json` JSON-RPC bridge fallthrough
/// (`handle_json_rpc_bridge`, reached when no `http_routes` entry matches)
/// must also reject an anonymous caller targeting a native service, and with
/// 401 -- not the 500 a raw `dispatch_json_rpc_once` rejection would surface.
#[tokio::test]
async fn json_rpc_bridge_rejects_anonymous_caller_with_401() {
    let (route_handler, _http_routes) = test_route_handler().await;
    let service = Arc::new(RecordingNativeService::default());
    let service_id = "json-rpc-bridge-test-svc".to_string();
    route_handler.register_native_service(service_id.clone(), service.clone());

    let pipeline = RoutePipeline {
        encryption: EncryptionStage::None,
        transport: TransportStage::Http,
        adaptation: AdaptationStage::None,
        service: ServiceStage::NativeService { service_id: service_id.clone() },
    };
    let preamble = preamble_for(&service_id, "data-layer");

    let (mut client, server) = duplex(4096);
    let io = TokioIo::new(server);
    let handle =
        tokio::spawn(route_handler.clone().handle_http_stream(io, preamble, pipeline, None));

    let body = json_rpc_body("get", json!({}));
    let request = format!(
        "POST / HTTP/1.1\r\nHost: test\r\nContent-Type: application/json\r\nContent-Length: \
         {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    client.write_all(request.as_bytes()).await.unwrap();
    client.write_all(&body).await.unwrap();
    client.shutdown().await.unwrap();

    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8_lossy(&response);
    assert!(response.starts_with("HTTP/1.1 401"), "expected 401, got: {response}");

    handle.await.unwrap().unwrap();
    assert!(
        !service.was_invoked(),
        "native service must not be invoked for an anonymous JSON-RPC bridge request"
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

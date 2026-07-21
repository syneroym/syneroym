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
    local_registry::{EndpointRegistry, NATIVE_CAPABILITY_INTERFACES},
    storage::MockStorage,
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
    Ability, AuthLevel, CallerContext, Capability, NativeDispatchRegistry, NativeInvocation,
    NativeResponse, NativeService, ResourceUri, RpcResult, SessionContext,
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
        proof: None,
    }
}

/// The same `[iam].admin_ucan_root` grant `build_caller`
/// (`crates/router/src/route_handler/io.rs`) constructs for a caller whose
/// master DID matches the configured admin root: `substrate/admin` on
/// `substrate:<did>`.
fn admin_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: did.to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::substrate(did),
                can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
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

    let route_handler = RouteHandler::init(
        "test-orchestrator".to_string(),
        &config,
        registry,
        [7u8; 32],
        None,
        deps,
    )
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
        ucan: None,
        dir: None,
    }
}

fn json_rpc_body(method: &str, params: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1}))
        .unwrap()
}

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

/// A caller matching `[iam].admin_ucan_root` -- represented by the
/// `substrate/admin` grant `build_caller` constructs for it -- must be
/// admitted to native `execute-ddl` (B0.md §11.2).
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

/// `query-raw` (Slice B5, ADR-0011) is gated on `data-layer/admin`, mirroring
/// `execute-ddl` -- an ordinary caller must be denied.
#[tokio::test]
async fn ordinary_caller_denied_query_raw() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "query-raw-deny-svc".to_string();
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
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller = test_caller("did:key:z6MkOrdinaryQueryRawCaller");

    let body = json_rpc_body("query-raw", json!({"sql": "SELECT 1", "params": []}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32010,
        "an ordinary caller must be denied query-raw: {resp:?}"
    );
}

/// A caller holding `data-layer/admin` on the service resource is admitted to
/// `query-raw`, and the returned payload carries the `columns`/`rows` shape
/// (D1 of B5.md, not the fixed `query-result` record shape).
#[tokio::test]
async fn admin_caller_admitted_query_raw() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "query-raw-admin-svc".to_string();
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
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller = admin_caller("did:key:z6MkQueryRawAdminRoot");

    let body = json_rpc_body("query-raw", json!({"sql": "SELECT 1 AS one", "params": []}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "admin caller must be admitted to query-raw: {resp:?}");
    let result = resp.get("result").expect("query-raw must return a result");
    assert_eq!(result["columns"], json!(["one"]));
    assert!(result.get("rows").is_some(), "result must carry a rows field: {result:?}");
}

/// End-to-end injection resistance (task.md:439): a `query-raw` `params`
/// value containing SQL-injection-shaped text is bound, never interpolated.
#[tokio::test]
async fn query_raw_binds_params_no_injection() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "query-raw-injection-svc".to_string();
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
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller = admin_caller("did:key:z6MkQueryRawInjectionAdmin");

    let create_body = json_rpc_body("create-collection", json!({"name": "items", "indexes": []}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &create_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    let put_body = json_rpc_body(
        "put",
        json!({"collection": "items", "value": {"id": "1", "payload": br#"{"name": "alice"}"#.to_vec()}}),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &put_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "put failed: {resp:?}");

    let injection_body = json_rpc_body(
        "query-raw",
        json!({
            "sql": "SELECT id FROM items WHERE json_extract(payload,'$.name') = ?",
            "params": [{"type": "text", "value": "x'; DROP TABLE items; --"}],
        }),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &injection_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "query-raw failed: {resp:?}");
    assert_eq!(resp["result"]["rows"], json!([]), "injection string must match no rows");

    // The table must still exist and be queryable afterwards.
    let count_body =
        json_rpc_body("query-raw", json!({"sql": "SELECT count(*) AS n FROM items", "params": []}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &count_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["result"]["rows"],
        json!([[{"type": "integer", "value": 1}]]),
        "items table must survive intact"
    );
}

/// Flag F1 (B5.md §4/§7): the hand-rolled `SqlValueDto`'s unit `Null` variant
/// under `#[serde(tag = "type", content = "value")]` must deserialize from
/// `{"type": "null"}` (no `value` key) -- exercised here through the real
/// wire path rather than as an isolated unit test, since the DTO is scoped
/// inside `dispatch_data_layer`'s `query-raw` match arm.
#[tokio::test]
async fn query_raw_null_param_round_trips() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "query-raw-null-svc".to_string();
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
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller = admin_caller("did:key:z6MkQueryRawNullAdmin");

    let body =
        json_rpc_body("query-raw", json!({"sql": "SELECT ? AS v", "params": [{"type": "null"}]}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "null param must deserialize and bind: {resp:?}");
    let returned_cell = resp["result"]["rows"][0][0].clone();
    assert_eq!(returned_cell, json!({"type": "null"}));

    // The output encoding must match the input DTO convention exactly, so a
    // returned cell can be fed straight back into a subsequent `params`
    // array without re-encoding (correctness finding C1).
    let round_trip_body =
        json_rpc_body("query-raw", json!({"sql": "SELECT ? AS v", "params": [returned_cell]}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &round_trip_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "round-tripped cell must deserialize: {resp:?}");
    assert_eq!(resp["result"]["rows"], json!([[{"type": "null"}]]));
}

/// Correctness finding C1: an `Integer` cell returned from `query-raw` must
/// be directly resubmittable as a `params` entry in a later call, proving
/// the output encoding is the same snake-case tag+content convention the
/// input DTO uses (not the bindgen default PascalCase external tag).
#[tokio::test]
async fn query_raw_result_cells_are_round_trippable_as_params() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "query-raw-roundtrip-svc".to_string();
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
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller = admin_caller("did:key:z6MkQueryRawRoundtripAdmin");

    let first_body = json_rpc_body("query-raw", json!({"sql": "SELECT 42 AS v", "params": []}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &first_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    let returned_cell = resp["result"]["rows"][0][0].clone();
    assert_eq!(returned_cell, json!({"type": "integer", "value": 42}));

    let second_body =
        json_rpc_body("query-raw", json!({"sql": "SELECT ? AS echoed", "params": [returned_cell]}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &second_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "round-tripped cell must be a valid param: {resp:?}");
    assert_eq!(resp["result"]["rows"], json!([[{"type": "integer", "value": 42}]]));
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

/// `aggregate` (Slice B4, ADR-0007) is deliberately unprivileged, like
/// `query` -- the deliberate contrast with B5's `ordinary_caller_denied_
/// query_raw`: an ordinary (non-admin) caller must be admitted, and the
/// payload carries the `columns`/`rows` `raw-query-result` shape.
#[tokio::test]
async fn ordinary_caller_admitted_aggregate() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "aggregate-ordinary-svc".to_string();
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
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller = test_caller("did:key:z6MkOrdinaryAggregateCaller");

    let create_body = json_rpc_body("create-collection", json!({"name": "people", "indexes": []}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &create_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    let put_body = json_rpc_body(
        "put",
        json!({"collection": "people", "value": {"id": "p1", "payload": br#"{"category": "a"}"#.to_vec()}}),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &put_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "put failed: {resp:?}");

    let aggregate_body = json_rpc_body(
        "aggregate",
        json!({
            "collection": "people",
            "pipeline": r#"{"$group":{"_id":"category","n":{"$sum":1}}}"#,
        }),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &aggregate_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(
        resp.get("error").is_none(),
        "an ordinary caller must be admitted to aggregate: {resp:?}"
    );
    let result = resp.get("result").expect("aggregate must return a result");
    assert_eq!(result["columns"], json!(["_id", "n"]));
    assert!(result.get("rows").is_some(), "result must carry a rows field: {result:?}");
}

/// A malformed `aggregate` pipeline (missing the required `$group` stage)
/// must map to a JSON-RPC `data-layer` schema-violation error (`-32012`)
/// through the native dispatch arm, the same error family `query-raw`'s own
/// malformed-SQL path already maps to (`data_layer_error`'s
/// `DataLayerError::SchemaViolation` arm) -- not just verified at the
/// `aggregate::compile` unit level.
#[tokio::test]
async fn aggregate_malformed_pipeline_is_schema_violation() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "aggregate-malformed-svc".to_string();
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
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");
    let caller = test_caller("did:key:z6MkAggregateMalformedCaller");

    let create_body = json_rpc_body("create-collection", json!({"name": "people", "indexes": []}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &create_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "create-collection failed: {resp:?}");

    let aggregate_body = json_rpc_body(
        "aggregate",
        json!({"collection": "people", "pipeline": r#"{"$match":{"active":true}}"#}),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &aggregate_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"],
        json!(-32012),
        "a $group-less pipeline must surface a schema-violation error: {resp:?}"
    );
}

// -- Native FDAE enforcement ------------------------------------------------
//
// `dispatch.rs`'s native arm threads the router-verified `CallerContext`
// into `NativeInvocation.caller` -- the one ingress ADR-0017 can honestly
// claim as *enforced* (§2.1). This is the headline proof: a deployed policy,
// reached through the real `dispatch_json_rpc_once` path, row-filters and
// column-masks for two distinct verified callers.

/// `document` --creator--> `user`, `view` permission reachable only via the
/// creator relation, plus `fields.deny: ["ssn"]` -- mirrors
/// `sandbox_wasm::host_capabilities`'s `fdae_cls_policy`, the WASM-host-path
/// analog of this same policy shape.
fn native_fdae_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["creator", "caller"]],
                            "fields": {"deny": ["ssn"]}
                        }
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

fn native_fdae_resource(service_id: &str, collection: &str) -> ResourceUri {
    ResourceUri(format!(
        "{}/collection/{collection}",
        ResourceUri::service(service_id, service_id).0
    ))
}

/// A verified caller entitled to `data-layer/read` on `documents` -- the
/// router-threaded `CallerContext` shape a real external caller carries,
/// distinct from `test_caller` (no capabilities) and `admin_caller`
/// (node-wide authority).
fn fdae_reader_caller(subject_did: &str, service_id: &str) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities: vec![Capability {
                with: native_fdae_resource(service_id, "documents"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

#[tokio::test]
async fn native_fdae_policy_row_filters_and_masks_for_two_distinct_verified_callers() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "native-fdae-svc".to_string();
    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let policy = Arc::new(native_fdae_policy());
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.clone(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        Some(policy),
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");

    // Seed via an elevated, unentitled-by-policy caller: `put`/
    // `create-collection` carry no FDAE gate (write-side Tier 3 is Slice
    // B5-fdae), so any verified caller can seed fixture rows.
    let seeder = test_caller("did:key:z6MkSeeder");
    for (collection, id, payload) in [
        ("users", "u-alice", json!({"did": "did:key:alice"})),
        ("users", "u-bob", json!({"did": "did:key:bob"})),
        ("documents", "doc-1", json!({"creator_uuid": "u-alice", "ssn": "111-11-1111"})),
        ("documents", "doc-2", json!({"creator_uuid": "u-bob", "ssn": "222-22-2222"})),
    ] {
        let create_body = json_rpc_body("create-collection", json!({"name": collection}));
        let _ = route_handler
            .dispatch_json_rpc_once(&pipeline, &preamble, Some(&seeder), &create_body)
            .await;
        let put_body = json_rpc_body(
            "put",
            json!({"collection": collection, "value": {"id": id, "payload": payload.to_string().into_bytes()}}),
        );
        let resp = route_handler
            .dispatch_json_rpc_once(&pipeline, &preamble, Some(&seeder), &put_body)
            .await
            .unwrap();
        let resp: Value = serde_json::from_slice(&resp).unwrap();
        assert!(resp.get("error").is_none(), "seeding {collection}/{id} failed: {resp:?}");
    }

    // Alice sees only her own document, with `ssn` stripped.
    let alice = fdae_reader_caller("did:key:alice", &service_id);
    let get_body = json_rpc_body("get", json!({"collection": "documents", "id": "doc-1"}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &get_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    let result = resp.get("result").expect("alice must reach her own document: {resp:?}");
    assert!(!result.is_null(), "alice's own document must be reachable: {resp:?}");
    let payload: Value = serde_json::from_slice(
        &serde_json::from_value::<Vec<u8>>(result["payload"].clone()).unwrap(),
    )
    .unwrap();
    assert!(
        payload.get("ssn").is_none(),
        "ssn must be stripped from alice's own payload: {payload:?}"
    );

    let get_other_body = json_rpc_body("get", json!({"collection": "documents", "id": "doc-2"}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &get_other_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(
        resp.get("result").map(Value::is_null).unwrap_or(true),
        "bob's document must be unreachable for alice, not an error (ADR-0007): {resp:?}"
    );

    let query_body = json_rpc_body(
        "query",
        json!({"collection": "documents", "opts": {"filter": null, "limit": null, "cursor": null}}),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &query_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    let records = resp["result"]["records"].as_array().expect("query must return records");
    assert_eq!(records.len(), 1, "alice's query must exclude bob's document: {resp:?}");
    assert_eq!(records[0]["id"], "doc-1");

    // Bob, a distinct verified caller, sees only his own document.
    let bob = fdae_reader_caller("did:key:bob", &service_id);
    let bob_query_body = json_rpc_body(
        "query",
        json!({"collection": "documents", "opts": {"filter": null, "limit": null, "cursor": null}}),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &bob_query_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    let records = resp["result"]["records"].as_array().expect("query must return records");
    assert_eq!(records.len(), 1, "bob's query must exclude alice's document: {resp:?}");
    assert_eq!(records[0]["id"], "doc-2");
}

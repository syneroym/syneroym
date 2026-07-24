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
use syneroym_fdae::{FetchResult, MAX_FETCH_IDS, Mode, Policy, parse_and_validate};
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
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
// claim as *enforced*. This is the headline proof: a deployed policy,
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
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

/// A `manage` permission covering `data-layer/write`, reachable via the same
/// creator relation as `native_fdae_policy`'s `view` -- exercises
/// `delete_many`'s `QueryAuth` wiring, which the headline test above does
/// not touch (`get`/`query` only).
fn native_fdae_write_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                    "permissions": {
                        "manage": {"allows": ["data-layer/write"], "paths": [["creator", "caller"]]}
                    }
                },
                "user": {"table": "users", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap()
}

/// Same shape as `native_fdae_policy`, minus the CLS `fields.deny` --
/// `aggregate` fails a CLS-active sieve closed outright (`data_db`'s own
/// documented behavior), so a policy for exercising aggregate's *RLS* half
/// specifically must not also be CLS-active.
fn native_fdae_rls_only_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
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

fn fdae_writer_caller(subject_did: &str, service_id: &str) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities: vec![Capability {
                with: native_fdae_resource(service_id, "documents"),
                can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// `delete_many`'s `QueryAuth` wiring, exercised through native dispatch:
/// a write-capable verified caller's `delete-many` deletes only the row
/// their ReBAC chain reaches, leaving an unreachable row untouched.
#[tokio::test]
async fn native_delete_many_is_row_filtered_as_a_write_operation() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "native-fdae-delete-svc".to_string();
    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let policy = Arc::new(native_fdae_write_policy());
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.clone(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        Some(policy),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");

    let seeder = test_caller("did:key:z6MkDeleteSeeder");
    for (collection, id, payload) in [
        ("users", "u-alice", json!({"did": "did:key:alice"})),
        ("users", "u-bob", json!({"did": "did:key:bob"})),
        ("documents", "doc-1", json!({"creator_uuid": "u-alice"})),
        ("documents", "doc-2", json!({"creator_uuid": "u-bob"})),
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

    let alice = fdae_writer_caller("did:key:alice", &service_id);
    let delete_body =
        json_rpc_body("delete-many", json!({"collection": "documents", "filter": null}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &delete_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "delete-many failed: {resp:?}");
    assert_eq!(
        resp["result"],
        json!(1),
        "only alice's own reachable row must be deleted: {resp:?}"
    );

    // Ground-truth check via `query-raw` (an admin caller, not sieve-aware --
    // the point here is to observe actual table state, independent of the
    // RLS being tested, not to re-exercise it).
    let admin = admin_caller("did:key:z6MkDeleteVerifier");
    let ids_body = json_rpc_body(
        "query-raw",
        json!({"sql": "SELECT id FROM documents ORDER BY id", "params": []}),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&admin), &ids_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["result"]["rows"],
        json!([[{"type": "text", "value": "doc-2"}]]),
        "alice's row must be gone and bob's must survive untouched: {resp:?}"
    );
}

/// `aggregate`'s `QueryAuth` wiring, exercised through native dispatch: the
/// RLS half (CLS is already fail-closed in `data_db` -- a non-empty
/// `masked_fields` denies the whole aggregate outright, so there is no
/// column-masking case to cover here).
#[tokio::test]
async fn native_aggregate_is_row_filtered_through_native_dispatch() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "native-fdae-aggregate-svc".to_string();
    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let policy = Arc::new(native_fdae_rls_only_policy());
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.clone(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        Some(policy),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");

    let seeder = test_caller("did:key:z6MkAggregateSeeder");
    for (collection, id, payload) in [
        ("users", "u-alice", json!({"did": "did:key:alice"})),
        ("users", "u-bob", json!({"did": "did:key:bob"})),
        ("documents", "doc-1", json!({"creator_uuid": "u-alice"})),
        ("documents", "doc-2", json!({"creator_uuid": "u-bob"})),
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

    let alice = fdae_reader_caller("did:key:alice", &service_id);
    let aggregate_body = json_rpc_body(
        "aggregate",
        json!({
            "collection": "documents",
            "pipeline": r#"{"$group":{"_id":null,"n":{"$sum":1}}}"#,
        }),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &aggregate_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "aggregate failed: {resp:?}");
    assert_eq!(
        resp["result"]["rows"],
        json!([[{"type": "integer", "value": 1}]]),
        "alice's aggregate must count only her own reachable document: {resp:?}"
    );
}

// -- Slice B3 Phase 3: native `resolve-relation` (the cross-service
// relationship-proof fetch's receiving side, D-B3-3) --------------------

/// A single `employee` definition supporting both D-B3-3 authorization
/// models: `view_self`'s zero-hop `paths: [["caller"]]` is what A1 (the
/// existing capability-gated sieve) evaluates, and
/// `resolvable_without_capability: true` is the explicit per-definition
/// opt-in A2 (bare `principal_column` match) requires.
fn resolvable_employee_policy() -> Policy {
    parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "employee": {
                    "table": "employees",
                    "principal_column": "did",
                    "resolvable_without_capability": true,
                    "permissions": {
                        "view_self": {"allows": ["data-layer/read"], "paths": [["caller"]]}
                    }
                }
            }
        }"#,
    )
    .unwrap()
}

/// A verified caller entitled to `data-layer/read` on the `employee`
/// resource -- the A1 path.
fn employee_reader_caller(subject_did: &str, service_id: &str) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities: vec![Capability {
                // Scoped to the physical table ("employees"), not the
                // policy definition key ("employee") -- `compile_read`
                // builds its resource URI from whatever `collection` string
                // actually reaches it, which is the resolved table
                // (`definition_table`), matching every other capability
                // scoping convention in this file (e.g. `fdae_reader_caller`
                // above uses "documents", not "document").
                with: native_fdae_resource(service_id, "employees"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// A verified caller holding a capability scoped to a **different**
/// resource entirely -- B3-07: the A1/A2 fork must key on capabilities
/// scoped to *this* resource, not "holds any capability at all", so this
/// caller correctly routes to A2 (as if capability-less for `employees`),
/// not to a real-but-unrelated A1 grant check.
fn unrelated_resource_capability_caller(subject_did: &str, service_id: &str) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities: vec![Capability {
                with: native_fdae_resource(service_id, "some_other_collection"),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// A verified caller holding a capability scoped to the **right**
/// resource (`employees`) but for an ability `view_self`'s `allows:
/// ["data-layer/read"]` doesn't cover -- routes to A1 (a capability *is*
/// scoped here), which then genuinely denies via the grant∩policy
/// intersection. Exercises A1's real (not-A2-rescued) deny.
///
/// Must be an ability data-layer/read doesn't entail: the `data-layer`
/// namespace is a *tiered* hierarchy (`admin ⊇ write ⊇ read`,
/// `Ability::entails`), so `data-layer/write` would actually cover
/// `data-layer/read` here -- a flat, unrelated ability (`blob/read`) is
/// what genuinely fails to entail it.
fn wrong_ability_on_the_right_resource_caller(
    subject_did: &str,
    service_id: &str,
) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: subject_did.to_string(),
            capabilities: vec![Capability {
                with: native_fdae_resource(service_id, "employees"),
                can: Ability(Ability::BLOB_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// A verified caller holding **zero** capabilities -- a real
/// `session.subject_did` (unlike `test_caller`, whose `SessionContext`
/// default leaves it empty), the shape `build_caller`
/// (`crates/router/src/route_handler/io.rs`) always produces for any
/// verified connection, capability-laden or not.
fn zero_capability_caller(subject_did: &str) -> CallerContext {
    CallerContext {
        caller_did: subject_did.to_string(),
        app_instance: None,
        session: SessionContext { subject_did: subject_did.to_string(), ..Default::default() },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

async fn resolve_relation_service_and_pipeline(
    service_id: &str,
    policy: Option<Policy>,
) -> (RouteHandler, RoutePipeline, RoutePreamble, tempfile::TempDir) {
    resolve_relation_service_and_pipeline_with(
        service_id,
        policy,
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
    )
    .await
}

/// Same as [`resolve_relation_service_and_pipeline`], but lets the caller
/// pin `node_identity`/`owner_did` explicitly -- needed to construct two
/// services that share one or the other while varying just the dimension
/// under test (e.g. same node, different owner).
async fn resolve_relation_service_and_pipeline_with(
    service_id: &str,
    policy: Option<Policy>,
    node_identity: Arc<syneroym_identity::Identity>,
    owner_did: &str,
) -> (RouteHandler, RoutePipeline, RoutePreamble, tempfile::TempDir) {
    let (route_handler, _http_routes) = test_route_handler().await;
    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.to_string(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        policy.map(Arc::new),
        node_identity,
        owner_did,
    ));
    route_handler.register_native_service(service_id.to_string(), data_service);

    let pipeline = raw_pipeline(service_id);
    let preamble = preamble_for(service_id, "data-layer");

    // Seed via an elevated, unentitled-by-policy caller -- `put`/
    // `create-collection` carry no FDAE gate (write-side Tier 3 is Slice
    // B5-fdae).
    let seeder = test_caller("did:key:z6MkResolveRelationSeeder");
    for (id, did) in [("emp-alice", "did:key:alice"), ("emp-bob", "did:key:bob")] {
        let create_body = json_rpc_body("create-collection", json!({"name": "employees"}));
        let _ = route_handler
            .dispatch_json_rpc_once(&pipeline, &preamble, Some(&seeder), &create_body)
            .await;
        let put_body = json_rpc_body(
            "put",
            json!({
                "collection": "employees",
                "value": {"id": id, "payload": json!({"did": did}).to_string().into_bytes()}
            }),
        );
        let resp = route_handler
            .dispatch_json_rpc_once(&pipeline, &preamble, Some(&seeder), &put_body)
            .await
            .unwrap();
        let resp: Value = serde_json::from_slice(&resp).unwrap();
        assert!(resp.get("error").is_none(), "seeding employees/{id} failed: {resp:?}");
    }

    (route_handler, pipeline, preamble, temp_dir)
}

/// Seeds `count` employee rows, all sharing `did` (so a single principal's
/// `view_self`/structural lookup reaches all of them), via `batch-mutate`
/// calls capped at `data_db`'s own `MAX_BATCH_SIZE` (200) per call --
/// needed to construct an id-set bigger than `MAX_FETCH_IDS` (1000) for
/// the overflow tests below.
async fn seed_many_employees(
    route_handler: &RouteHandler,
    pipeline: &RoutePipeline,
    preamble: &RoutePreamble,
    did: &str,
    count: usize,
) {
    const BATCH: usize = 200;
    let seeder = test_caller("did:key:z6MkBulkSeeder");
    let mut seeded = 0usize;
    while seeded < count {
        let this_batch = BATCH.min(count - seeded);
        let mutations: Vec<Value> = (0..this_batch)
            .map(|i| {
                let id = format!("emp-bulk-{}", seeded + i);
                json!({
                    "type": "put",
                    "value": {"id": id, "payload": json!({"did": did}).to_string().into_bytes()}
                })
            })
            .collect();
        let body = json_rpc_body(
            "batch-mutate",
            json!({"collection": "employees", "mutations": mutations}),
        );
        let resp = route_handler
            .dispatch_json_rpc_once(pipeline, preamble, Some(&seeder), &body)
            .await
            .unwrap();
        let resp: Value = serde_json::from_slice(&resp).unwrap();
        assert!(resp.get("error").is_none(), "bulk seeding failed: {resp:?}");
        seeded += this_batch;
    }
}

/// B3 plan §5 fan-out containment: A1's `ServiceStore::query` limit is
/// `MAX_FETCH_IDS` (1000); when more rows are actually reachable,
/// `next_cursor` comes back `Some`, and `resolve_relation` must map that
/// to `quota-exceeded`, not silently return a truncated -- and therefore
/// incomplete but misleadingly-final-looking -- 1000-id answer.
#[tokio::test]
async fn resolve_relation_a1_overflow_maps_to_quota_exceeded() {
    let service_id = "resolve-relation-a1-overflow-svc";
    let (route_handler, pipeline, preamble, _temp_dir) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;
    seed_many_employees(&route_handler, &pipeline, &preamble, "did:key:alice", MAX_FETCH_IDS + 1)
        .await;

    let alice = employee_reader_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:alice");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32013,
        "an A1 result exceeding MAX_FETCH_IDS must map to quota-exceeded: {resp:?}"
    );
}

/// The A2 mirror: `query_raw`'s explicit `LIMIT MAX_FETCH_IDS + 1` is what
/// makes the overflow observable at all (raw SQL has no automatic page cap
/// the way `query` does) -- confirms it's actually wired up, not merely
/// present in the SQL text.
#[tokio::test]
async fn resolve_relation_a2_overflow_maps_to_quota_exceeded() {
    let service_id = "resolve-relation-a2-overflow-svc";
    let (route_handler, pipeline, preamble, _temp_dir) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;
    seed_many_employees(&route_handler, &pipeline, &preamble, "did:key:bob", MAX_FETCH_IDS + 1)
        .await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32013,
        "an A2 result exceeding MAX_FETCH_IDS must map to quota-exceeded: {resp:?}"
    );
}

fn resolve_relation_body(relation: &str, principal: &str) -> Vec<u8> {
    json_rpc_body("resolve-relation", json!({"relation": relation, "principal": principal}))
}

/// A1: a caller holding a real capability entitling `employee`'s
/// `view_self` permission resolves to their own row via the existing
/// capability-gated sieve -- and the returned `RelationshipProof` verifies
/// against its own `asserter_did`.
#[tokio::test]
async fn resolve_relation_a1_resolves_via_the_capability_gated_sieve_and_verifies() {
    let service_id = "resolve-relation-a1-svc";
    let (route_handler, pipeline, preamble, _temp_dir) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let alice = employee_reader_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:alice");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "resolve-relation must succeed: {resp:?}");
    let result = &resp["result"];
    assert_eq!(result["ids"], json!(["emp-alice"]), "A1 must resolve only alice's own row");
    assert_eq!(result["relation"], "employee");
    assert_eq!(result["principal"], "did:key:alice");

    let asserter_did = result["asserter_did"].as_str().unwrap();
    let signature = result["signature"].as_str().unwrap();
    let mut unsigned = result.clone();
    unsigned["signature"] = json!("");
    syneroym_identity::substrate::verify_json_signature(asserter_did, &unsigned, signature)
        .expect("the returned proof must verify against its own asserter_did");
}

/// Two services co-hosted on the same node (same `node_identity`, same
/// `owner_did`) but with distinct `service_id`s must sign their
/// `RelationshipProof`s under distinct `asserter_did`s -- the multi-tenancy
/// concern ADR-0017 §6/§7's "`hr-svc` asserts..." model is meant to
/// address: a shared node-wide signing identity would make every co-hosted
/// service's assertions cryptographically indistinguishable.
#[tokio::test]
async fn resolve_relation_co_hosted_services_sign_with_distinct_asserter_dids() {
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let owner_did = "did:key:zSharedOwner";

    let (hr_handler, hr_pipeline, hr_preamble, _hr_dir) =
        resolve_relation_service_and_pipeline_with(
            "hr-svc",
            Some(resolvable_employee_policy()),
            node_identity.clone(),
            owner_did,
        )
        .await;
    let (finance_handler, finance_pipeline, finance_preamble, _finance_dir) =
        resolve_relation_service_and_pipeline_with(
            "finance-svc",
            Some(resolvable_employee_policy()),
            node_identity,
            owner_did,
        )
        .await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");

    let hr_resp = hr_handler
        .dispatch_json_rpc_once(&hr_pipeline, &hr_preamble, Some(&bob), &body)
        .await
        .unwrap();
    let hr_resp: Value = serde_json::from_slice(&hr_resp).unwrap();
    assert!(hr_resp.get("error").is_none(), "resolve-relation must succeed: {hr_resp:?}");
    let hr_asserter = hr_resp["result"]["asserter_did"].as_str().unwrap();

    let finance_resp = finance_handler
        .dispatch_json_rpc_once(&finance_pipeline, &finance_preamble, Some(&bob), &body)
        .await
        .unwrap();
    let finance_resp: Value = serde_json::from_slice(&finance_resp).unwrap();
    assert!(finance_resp.get("error").is_none(), "resolve-relation must succeed: {finance_resp:?}");
    let finance_asserter = finance_resp["result"]["asserter_did"].as_str().unwrap();

    assert_ne!(
        hr_asserter, finance_asserter,
        "two co-hosted services under the same owner must sign as distinct asserter_dids"
    );

    // Each proof must verify only against its own service's asserter_did.
    let mut hr_unsigned = hr_resp["result"].clone();
    hr_unsigned["signature"] = json!("");
    let hr_signature = hr_resp["result"]["signature"].as_str().unwrap();
    syneroym_identity::substrate::verify_json_signature(hr_asserter, &hr_unsigned, hr_signature)
        .expect("hr-svc's proof must verify against its own asserter_did");
    assert!(
        syneroym_identity::substrate::verify_json_signature(
            finance_asserter,
            &hr_unsigned,
            hr_signature
        )
        .is_err(),
        "hr-svc's proof must not verify under finance-svc's asserter_did"
    );
}

/// A `service_id` freed by undeploy and redeployed under a **different**
/// owner must not inherit the old owner's signing key: same node, same
/// `service_id`, different `owner_did` must still derive distinct
/// `asserter_did`s. Otherwise a stale `RelationshipProof` cached from the
/// old tenancy would still verify under the new tenant's identity.
#[tokio::test]
async fn resolve_relation_service_id_reused_by_a_different_owner_signs_distinctly() {
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let service_id = "reused-service-id-svc";

    let (old_handler, old_pipeline, old_preamble, _old_dir) =
        resolve_relation_service_and_pipeline_with(
            service_id,
            Some(resolvable_employee_policy()),
            node_identity.clone(),
            "did:key:zOldOwner",
        )
        .await;
    let (new_handler, new_pipeline, new_preamble, _new_dir) =
        resolve_relation_service_and_pipeline_with(
            service_id,
            Some(resolvable_employee_policy()),
            node_identity,
            "did:key:zNewOwner",
        )
        .await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");

    let old_resp = old_handler
        .dispatch_json_rpc_once(&old_pipeline, &old_preamble, Some(&bob), &body)
        .await
        .unwrap();
    let old_resp: Value = serde_json::from_slice(&old_resp).unwrap();
    let old_asserter = old_resp["result"]["asserter_did"].as_str().unwrap();

    let new_resp = new_handler
        .dispatch_json_rpc_once(&new_pipeline, &new_preamble, Some(&bob), &body)
        .await
        .unwrap();
    let new_resp: Value = serde_json::from_slice(&new_resp).unwrap();
    let new_asserter = new_resp["result"]["asserter_did"].as_str().unwrap();

    assert_ne!(
        old_asserter, new_asserter,
        "a service_id reused under a different owner must derive a distinct asserter_did"
    );
}

/// B3-07: a capability scoped to a completely unrelated resource must not
/// change the answer relative to holding zero capabilities -- it routes to
/// A2 (structural resolution), the same as `zero_capability_caller` would,
/// not to a real-but-irrelevant A1 grant check.
#[tokio::test]
async fn resolve_relation_an_unrelated_resource_capability_still_gets_a2() {
    let service_id = "resolve-relation-unrelated-resource-svc";
    let (route_handler, pipeline, preamble, _temp_dir) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let caller = unrelated_resource_capability_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:alice");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "resolve-relation must succeed: {resp:?}");
    assert_eq!(
        resp["result"]["ids"],
        json!(["emp-alice"]),
        "an unrelated-resource capability must not block A2's structural resolution: {resp:?}"
    );
}

/// A1's real deny (a capability scoped to `employees` but for an ability
/// `view_self` doesn't cover) is final -- it must **not** be rescued by
/// A2, even though the definition has opted into
/// `resolvable_without_capability`. Mutually exclusive per request, not a
/// fallback chain (D-B3-3).
#[tokio::test]
async fn resolve_relation_a1_deny_is_not_rescued_by_a2() {
    let service_id = "resolve-relation-a1-deny-svc";
    let (route_handler, pipeline, preamble, _temp_dir) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let mallory = wrong_ability_on_the_right_resource_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:alice");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&mallory), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "resolve-relation must still succeed with an empty set");
    assert_eq!(
        resp["result"]["ids"],
        json!([]),
        "an unrelated capability must not trigger A1 grant, nor fall through to A2: {resp:?}"
    );
}

/// A2: a caller holding **zero** capabilities structurally resolves via
/// the bare `principal_column` match, since `employee` opted into
/// `resolvable_without_capability`.
#[tokio::test]
async fn resolve_relation_a2_resolves_structurally_with_zero_capabilities() {
    let service_id = "resolve-relation-a2-svc";
    let (route_handler, pipeline, preamble, _temp_dir) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "resolve-relation must succeed: {resp:?}");
    assert_eq!(resp["result"]["ids"], json!(["emp-bob"]), "A2 must resolve bob's own row");
}

/// A caller with zero capabilities against a definition that has **not**
/// opted into `resolvable_without_capability` gets an empty result --
/// neither A1 (no capabilities to evaluate) nor A2 (not opted in) applies.
#[tokio::test]
async fn resolve_relation_denies_when_not_opted_in_and_no_capabilities() {
    let service_id = "resolve-relation-not-opted-in-svc";
    let policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "employee": {"table": "employees", "principal_column": "did"}
            }
        }"#,
    )
    .unwrap();
    let (route_handler, pipeline, preamble, _temp_dir) =
        resolve_relation_service_and_pipeline(service_id, Some(policy)).await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none());
    assert_eq!(resp["result"]["ids"], json!([]));
}

/// A `relation` naming no definition at all must deny outright, never fall
/// through to `ServiceStore::query`'s ordinary no-definition-means-
/// unfiltered pass-through -- there is no grant-layer admission backing a
/// cross-service relationship ask the way there is for an ordinary read.
#[tokio::test]
async fn resolve_relation_denies_for_an_undeclared_relation_not_unfiltered() {
    let service_id = "resolve-relation-undeclared-svc";
    let (route_handler, pipeline, preamble, _temp_dir) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("nonexistent_relation", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none());
    assert_eq!(
        resp["result"]["ids"],
        json!([]),
        "an unrecognized relation name must never leak an unfiltered dump: {resp:?}"
    );
}

/// `principal` is a caller-declared label that must match the wire
/// caller's own re-verified identity -- a caller cannot ask about a
/// different principal's relationships.
#[tokio::test]
async fn resolve_relation_denies_when_principal_does_not_match_the_caller() {
    let service_id = "resolve-relation-mismatch-svc";
    let (route_handler, pipeline, preamble, _temp_dir) =
        resolve_relation_service_and_pipeline(service_id, Some(resolvable_employee_policy())).await;

    let alice = employee_reader_caller("did:key:alice", service_id);
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32010,
        "asking about a principal other than the verified caller must be denied: {resp:?}"
    );
}

/// No policy deployed: nothing to resolve against, an empty (not error)
/// result -- the same "no definition" treatment an unpoliced service gives
/// every other FDAE-aware method.
#[tokio::test]
async fn resolve_relation_is_empty_when_no_policy_is_deployed() {
    let service_id = "resolve-relation-no-policy-svc";
    let (route_handler, pipeline, preamble, _temp_dir) =
        resolve_relation_service_and_pipeline(service_id, None).await;

    let bob = zero_capability_caller("did:key:bob");
    let body = resolve_relation_body("employee", "did:key:bob");
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none());
    assert_eq!(resp["result"]["ids"], json!([]));
}

/// The join the review calls out as missing: `plan_read` (`crates/fdae`)
/// -> `resolve-relation` (native dispatch on a *second*, distinct service)
/// -> `finalize`, wired together by hand -- there is no `ServiceProxy`
/// orchestration yet (Phase 4), so this test plays that role manually --
/// proving the sender and receiver actually agree on what they're asking
/// each other, not just that each half is correct in isolation. This is
/// exactly the test that would have caught B3-01 (principal mismatch) and
/// B3-02 (relation namespace mismatch) immediately; both are fixed, and
/// this pins the fix at the seam where they were found.
#[tokio::test]
async fn plan_read_resolve_relation_finalize_join_end_to_end() {
    // -- the remote (data-owning) service: hr-svc. `resolvable_employee_policy`
    // seeds emp-alice (did:key:alice) and emp-bob (did:key:bob).
    let hr_service_id = "hr-svc-join-test";
    let (hr_route_handler, hr_pipeline, hr_preamble, _hr_temp_dir) =
        resolve_relation_service_and_pipeline(hr_service_id, Some(resolvable_employee_policy()))
            .await;

    // -- the local (requesting) service: app-svc, whose own policy names a
    // remote relation pointing at hr-svc.
    let local_service_id = "app-svc-join-test";
    let local_policy = parse_and_validate(
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "document": {
                    "table": "documents",
                    "relations": {"owner": {
                        "target": "employee", "service": "hr-svc", "join_column": "owner_uuid"
                    }},
                    "permissions": {
                        "view": {"allows": ["data-layer/read"], "paths": [["owner", "anchor"]]}
                    }
                }
            }
        }"#,
    )
    .unwrap();

    // A caller presenting as `svc-A` (whoever actually authenticated this
    // connection to app-svc) proxying for anchor `alice`.
    let proxying_service = SessionContext {
        subject_did: "did:key:svc-A".to_string(),
        anchor_did: Some("did:key:alice".to_string()),
        capabilities: vec![Capability {
            // Scoped to "document" (the `collection` string this test
            // passes to `plan_read` below), not "documents" -- `plan_read`
            // builds its resource URI from whatever `collection` argument
            // it's called with directly, unlike `ServiceStore::query`'s
            // literal-table-name addressing.
            with: native_fdae_resource(local_service_id, "document"),
            can: Ability(Ability::DATA_LAYER_READ.to_string()),
            caveats: None,
        }],
        ..Default::default()
    };

    // Step 1: `plan_read` on the *local* policy -- must produce a
    // `RemoteFetch` (B2's old behavior was to fail closed here), asking
    // about the anchor, never the proxying caller.
    let mut plan = syneroym_fdae::plan_read(
        &local_policy,
        "document",
        &proxying_service,
        local_service_id,
        &Ability(Ability::DATA_LAYER_READ.to_string()),
        Mode::Filter,
    )
    .unwrap();
    assert!(
        plan.local.is_none(),
        "a policy needing a remote fetch must not resolve to a local sieve"
    );
    assert_eq!(plan.fetches.len(), 1);
    let fetch = plan.fetches[0].clone();
    assert_eq!(fetch.service, "hr-svc");
    assert_eq!(fetch.relation, "employee", "B3-02: the wire relation is the remote object type");
    assert_eq!(fetch.principal_did, "did:key:alice", "B3-01: the fetch asks about the anchor");

    // Step 2: issue the fetch as a *real* resolve-relation call against
    // hr-svc's native dispatch -- standing in for Phase 4's orchestration,
    // which would forward the anchor's own re-verified identity as the
    // wire caller of this specific request (the piece Phase 4 still needs
    // to build; here it's constructed by hand to close the loop).
    let anchor_as_direct_caller = test_caller(&fetch.principal_did);
    let anchor_as_direct_caller = CallerContext {
        session: SessionContext {
            subject_did: fetch.principal_did.clone(),
            ..anchor_as_direct_caller.session
        },
        ..anchor_as_direct_caller
    };
    let resolve_body = resolve_relation_body(&fetch.relation, &fetch.principal_did);
    let resolve_resp = hr_route_handler
        .dispatch_json_rpc_once(
            &hr_pipeline,
            &hr_preamble,
            Some(&anchor_as_direct_caller),
            &resolve_body,
        )
        .await
        .unwrap();
    let resolve_resp: Value = serde_json::from_slice(&resolve_resp).unwrap();
    assert!(resolve_resp.get("error").is_none(), "resolve-relation must succeed: {resolve_resp:?}");
    let ids: Vec<String> = resolve_resp["result"]["ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        ids,
        vec!["emp-alice"],
        "hr-svc must resolve alice's own employee row: {resolve_resp:?}"
    );

    // Step 3: `finalize` the plan with the real fetched id-set, and run
    // the resulting sieve against a locally-seeded `documents` table --
    // the same raw-SQL verification `crates/fdae`'s own `finalize` tests
    // use, proving the compiled predicate is not just well-typed but
    // actually correct.
    let pending = plan.pending.take().unwrap();
    let sieve = syneroym_fdae::finalize(pending, &[FetchResult { slot: fetch.slot, ids }]).unwrap();

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE documents (
            id TEXT PRIMARY KEY, creator_id TEXT, created_at INTEGER, updated_at INTEGER,
            payload TEXT NOT NULL DEFAULT '{}'
        );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents (id, payload) VALUES ('doc-1', ?1)",
        [json!({"owner_uuid": "emp-alice"}).to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents (id, payload) VALUES ('doc-2', ?1)",
        [json!({"owner_uuid": "emp-bob"}).to_string()],
    )
    .unwrap();

    let sql = format!("SELECT id FROM documents WHERE {} ORDER BY id", sieve.where_clause);
    let mut stmt = conn.prepare(&sql).unwrap();
    let visible: Vec<String> = stmt
        .query_map(rusqlite::params_from_iter(sieve.params.iter()), |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(
        visible,
        vec!["doc-1"],
        "only alice's own document is reachable through the full plan -> fetch -> finalize join"
    );
}

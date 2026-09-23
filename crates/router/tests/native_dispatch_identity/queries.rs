use super::helpers::*;

/// `query-raw` (ADR-0011) is gated on `data-layer/admin`, mirroring
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
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
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

/// End-to-end injection resistance: a `query-raw` `params`
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
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
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

/// The hand-rolled `SqlValueDto`'s unit `Null` variant
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
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
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

/// `aggregate` (ADR-0007) is deliberately unprivileged, like
/// `query` -- the deliberate contrast with `ordinary_caller_denied_query_raw`:
/// an ordinary (non-admin) caller must be admitted, and the
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
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
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
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
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

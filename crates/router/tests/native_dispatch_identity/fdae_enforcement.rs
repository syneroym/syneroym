use super::helpers::*;

#[tokio::test]
#[allow(clippy::too_many_lines)]
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
    let storage_provider: Arc<dyn StorageProvider> = storage_provider;
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.clone(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        messaging_broker,
        Some(policy),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");

    for (collection, id, payload) in [
        ("users", "u-alice", json!({"did": "did:key:alice"})),
        ("users", "u-bob", json!({"did": "did:key:bob"})),
        ("documents", "doc-1", json!({"creator_uuid": "u-alice", "ssn": "111-11-1111"})),
        ("documents", "doc-2", json!({"creator_uuid": "u-bob", "ssn": "222-22-2222"})),
    ] {
        seed_via_store(&storage_provider, &key_store, &service_id, collection, id, &payload).await;
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

/// Mirrors `native_fdae_policy_row_
/// filters_and_masks_for_two_distinct_verified_callers` above, for the
/// write side. Two verified callers, each holding a `data-layer/write`
/// capability on `documents`, may each patch only their own row and are
/// denied patching the other's.
#[tokio::test]
async fn native_fdae_policy_authorizes_writes_for_one_verified_caller_and_denies_another() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "native-fdae-write-authz-svc".to_string();
    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let policy = Arc::new(native_fdae_write_policy());
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.clone(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        messaging_broker,
        Some(policy),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");

    for (collection, id, payload) in [
        ("users", "u-alice", json!({"did": "did:key:alice"})),
        ("users", "u-bob", json!({"did": "did:key:bob"})),
        ("documents", "doc-1", json!({"creator_uuid": "u-alice"})),
        ("documents", "doc-2", json!({"creator_uuid": "u-bob"})),
    ] {
        seed_via_store(&storage_provider, &key_store, &service_id, collection, id, &payload).await;
    }

    let alice = fdae_writer_caller("did:key:alice", &service_id);
    let bob = fdae_writer_caller("did:key:bob", &service_id);

    // Alice may patch her own row.
    let patch_own = json_rpc_body(
        "patch",
        json!({
            "collection": "documents", "id": "doc-1",
            "patch_json": b"{\"nickname\":\"al\"}".to_vec()
        }),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &patch_own)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "alice patching her own row must succeed: {resp:?}");

    // Alice is denied patching bob's row.
    let patch_other = json_rpc_body(
        "patch",
        json!({
            "collection": "documents", "id": "doc-2",
            "patch_json": b"{\"nickname\":\"stolen\"}".to_vec()
        }),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &patch_other)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(resp["error"]["code"], -32010, "alice patching bob's row must be denied: {resp:?}");

    // Bob may still patch his own row afterward.
    let bob_patch = json_rpc_body(
        "patch",
        json!({
            "collection": "documents", "id": "doc-2",
            "patch_json": b"{\"nickname\":\"bo\"}".to_vec()
        }),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&bob), &bob_patch)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(resp.get("error").is_none(), "bob patching his own row must succeed: {resp:?}");
}

/// The headline write-authz test above only exercises `patch`. `put`
/// (both create and update), `delete`, and `batch-mutate` all wrap the same
/// `authorize_and_mutate` envelope but are otherwise untested end to end
/// through the native JSON-RPC dispatch path -- this pins that each denies
/// with the same `-32010` permission-denied code, not an opaque internal
/// error, and that a denied `batch-mutate` rolls back a mutation earlier in
/// the same batch that would otherwise have succeeded.
#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn native_fdae_policy_denies_put_delete_and_batch_mutate_for_an_unreachable_row() {
    let (route_handler, _http_routes) = test_route_handler().await;

    let service_id = "native-fdae-write-denials-svc".to_string();
    let key_store = Arc::new(KeyStore::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let policy = Arc::new(native_fdae_write_policy());
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.clone(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        messaging_broker,
        Some(policy),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");

    for (collection, id, payload) in [
        ("users", "u-alice", json!({"did": "did:key:alice"})),
        ("users", "u-bob", json!({"did": "did:key:bob"})),
        ("documents", "doc-alice", json!({"creator_uuid": "u-alice"})),
        ("documents", "doc-bob", json!({"creator_uuid": "u-bob"})),
    ] {
        seed_via_store(&storage_provider, &key_store, &service_id, collection, id, &payload).await;
    }

    let alice = fdae_writer_caller("did:key:alice", &service_id);

    // `put`-create: alice creating a row attributed to bob is denied --
    // the post-image is unreachable to her.
    let create_other = json_rpc_body(
        "put",
        json!({
            "collection": "documents",
            "value": {"id": "doc-new", "payload": json!({"creator_uuid": "u-bob"}).to_string().into_bytes()}
        }),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &create_other)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32010,
        "alice creating a row attributed to bob must be denied: {resp:?}"
    );

    // `delete`: alice deleting bob's row is denied -- unreachable to her.
    let delete_other = json_rpc_body("delete", json!({"collection": "documents", "id": "doc-bob"}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &delete_other)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(resp["error"]["code"], -32010, "alice deleting bob's row must be denied: {resp:?}");

    // `batch-mutate`: alice's own patch (would succeed alone) followed by a
    // patch of bob's row (denied) must roll back the whole batch, not just
    // the offending mutation.
    let batch = json_rpc_body(
        "batch-mutate",
        json!({
            "collection": "documents",
            "mutations": [
                {"type": "patch", "value": {"id": "doc-alice", "patch_json": b"{\"nickname\":\"al\"}".to_vec()}},
                {"type": "patch", "value": {"id": "doc-bob", "patch_json": b"{\"nickname\":\"stolen\"}".to_vec()}},
            ]
        }),
    );
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &batch)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32010,
        "a batch containing a denied mutation must be denied: {resp:?}"
    );

    let get_body = json_rpc_body("get", json!({"collection": "documents", "id": "doc-alice"}));
    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&alice), &get_body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    let result = resp.get("result").expect("get must return a result");
    let payload: Value = serde_json::from_slice(
        &serde_json::from_value::<Vec<u8>>(result["payload"].clone()).unwrap(),
    )
    .unwrap();
    assert!(
        payload.get("nickname").is_none(),
        "the denied batch's earlier, otherwise-valid mutation must have rolled back too: \
         {payload:?}"
    );
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
    let storage_provider: Arc<dyn StorageProvider> = storage_provider;
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.clone(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        messaging_broker,
        Some(policy),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");

    for (collection, id, payload) in [
        ("users", "u-alice", json!({"did": "did:key:alice"})),
        ("users", "u-bob", json!({"did": "did:key:bob"})),
        ("documents", "doc-1", json!({"creator_uuid": "u-alice"})),
        ("documents", "doc-2", json!({"creator_uuid": "u-bob"})),
    ] {
        seed_via_store(&storage_provider, &key_store, &service_id, collection, id, &payload).await;
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
    let storage_provider: Arc<dyn StorageProvider> = storage_provider;
    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.clone(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        messaging_broker,
        Some(policy),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        "did:key:zTestOwner",
        syneroym_sandbox_wasm::empty_service_proxy(),
        syneroym_rpc::empty_row_authorizer(),
        None,
    ));
    route_handler.register_native_service(service_id.clone(), data_service);

    let pipeline = raw_pipeline(&service_id);
    let preamble = preamble_for(&service_id, "data-layer");

    for (collection, id, payload) in [
        ("users", "u-alice", json!({"did": "did:key:alice"})),
        ("users", "u-bob", json!({"did": "did:key:bob"})),
        ("documents", "doc-1", json!({"creator_uuid": "u-alice"})),
        ("documents", "doc-2", json!({"creator_uuid": "u-bob"})),
    ] {
        seed_via_store(&storage_provider, &key_store, &service_id, collection, id, &payload).await;
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

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! End-to-end Slice 3A integration test: deploy a WASM component that
//! imports `syneroym:data-layer/store`, verify `init()` runs on first
//! deploy, exercise CRUD through the real host functions, verify
//! host-injected `creator-id`, then re-deploy and verify `migrate()` runs
//! instead of `init()` and prior data survives.

use std::{fs, path::Path, sync::Arc};

use syneroym_core::{
    config::SubstrateConfig, local_registry::EndpointRegistry, storage::MockStorage, test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider, host_store::RecordWriteValue};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{
    Ability, AuthLevel, CallerContext, Capability, JsonRpcRequest, ResourceUri, SessionContext,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};

const TEST_DRIVER_INTERFACE: &str = "syneroym-test:data-layer-test/test-driver@0.1.0";
const SERVICE_ID: &str = "data-layer-test-svc";

async fn make_engine(dir: &Path) -> AppSandboxEngine {
    let mut config = SubstrateConfig {
        app_local_data_dir: dir.join("data"),
        app_data_dir: dir.join("user_data"),
        app_cache_dir: dir.join("cache"),
        app_log_dir: dir.join("logs"),
        profile: "full".to_string(),
        ..SubstrateConfig::default()
    };
    config.resolve_paths();

    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(&config.storage.db_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    AppSandboxEngine::init(
        &config,
        vec![],
        key_store,
        storage_provider,
        blob_provider,
        Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
    )
    .await
    .unwrap()
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
            interfaces: vec![TEST_DRIVER_INTERFACE.to_string()],
        }),
        registry_certificate: None,
    }
}

async fn run_crud_scenario(engine: &AppSandboxEngine, count: u32) -> u32 {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "run-crud-scenario".to_string(),
        params: serde_json::json!([count]),
        id: None,
    };
    // `execute_wasm` returns a successful `result<string, _>` guest value as
    // the raw string, not JSON-quoted -- see
    // `crates/sandbox_wasm/src/conversions.rs::wasm_results_to_json_string`.
    let result = engine.execute_wasm(SERVICE_ID, TEST_DRIVER_INTERFACE, &request).await.unwrap();
    result.parse::<u32>().unwrap()
}

/// Drives only the guest's own `query` (no `put`) -- needed under a
/// write-gated FDAE policy, where the guest's own `put` (running as a
/// capability-less `service_system` caller) would deny closed before ever
/// reaching the query half `run_crud_scenario` combines it with.
async fn run_query_scenario(engine: &AppSandboxEngine, limit: u32) -> u32 {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "run-query-scenario".to_string(),
        params: serde_json::json!([limit]),
        id: None,
    };
    let result = engine.execute_wasm(SERVICE_ID, TEST_DRIVER_INTERFACE, &request).await.unwrap();
    result.parse::<u32>().unwrap()
}

async fn make_engine_with_storage(
    dir: &Path,
) -> (AppSandboxEngine, Arc<dyn StorageProvider>, Arc<KeyStore>) {
    let mut config = SubstrateConfig {
        app_local_data_dir: dir.join("data"),
        app_data_dir: dir.join("user_data"),
        app_cache_dir: dir.join("cache"),
        app_log_dir: dir.join("logs"),
        profile: "full".to_string(),
        ..SubstrateConfig::default()
    };
    config.resolve_paths();

    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(&config.storage.db_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    let engine = AppSandboxEngine::init(
        &config,
        vec![],
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
    )
    .await
    .unwrap();
    (engine, storage_provider, key_store)
}

async fn get_creator_id(engine: &AppSandboxEngine, id: &str) -> String {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "get-creator-id".to_string(),
        params: serde_json::json!([id]),
        id: None,
    };
    engine.execute_wasm(SERVICE_ID, TEST_DRIVER_INTERFACE, &request).await.unwrap()
}

#[tokio::test]
async fn test_deploy_init_crud_creator_id_and_migrate() {
    let Ok(wasm_bytes) = fs::read(test_constants::data_layer_test_wasm_path()) else {
        eprintln!(
            "Skipping test_deploy_init_crud_creator_id_and_migrate: data-layer-test WASM artifact \
             not found (run `cargo build --target wasm32-wasip2 --release` in \
             test-components/data-layer-test)"
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;

    // First deploy: init() must run, creating the `profiles` collection.
    let manifest = wasm_deploy_manifest(wasm_bytes.clone());
    engine.deploy_wasm(SERVICE_ID, &manifest).await.unwrap();

    // CRUD: put 100 records, then query them all back.
    let observed = run_crud_scenario(&engine, 100).await;
    assert_eq!(observed, 100, "expected all 100 records to be observed by the query");

    // creator_id is host-injected to the deploying service's own id.
    let creator_id = get_creator_id(&engine, "p0").await;
    assert_eq!(creator_id, SERVICE_ID);

    // Re-deploy: migrate() must run (not init()), adding a `nickname` column
    // without disturbing existing records.
    engine.deploy_wasm(SERVICE_ID, &manifest).await.unwrap();
    let still_there = get_creator_id(&engine, "p0").await;
    assert_eq!(still_there, SERVICE_ID, "records from before the redeploy must survive migrate()");
}

/// D-04-02-h pin (task.md Decision Register): a deployed FDAE policy is
/// loaded at instantiation, but a guest-originated read runs under
/// `prepare_wasm_execution`'s synthesized `service_system` caller, which
/// holds no capability the policy's `view` permission can be entitled
/// through -- so `compile_read` falls to `deny_all()` and the guest's own
/// `query` sees none of the seeded rows. Whoever threads real caller
/// identity into this ingress should flip this assertion to the count
/// actually reachable.
///
/// Seeds directly against the store (`auth: None`), not via the guest's own
/// `put`: a write also runs the FDAE gate, and `service_system` (no
/// capabilities) would deny it closed under this policy before the guest's
/// query ever ran -- see `run_query_scenario`'s doc comment.
#[tokio::test]
async fn test_deployed_policy_yields_empty_guest_originated_query_d04_02_h() {
    let Ok(wasm_bytes) = fs::read(test_constants::data_layer_test_wasm_path()) else {
        eprintln!(
            "Skipping test_deployed_policy_yields_empty_guest_originated_query_d04_02_h: \
             data-layer-test WASM artifact not found (run `cargo build --target wasm32-wasip2 \
             --release` in test-components/data-layer-test)"
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let (engine, storage_provider, key_store) = make_engine_with_storage(dir.path()).await;

    // Persisted before `deploy_wasm` runs `init()`, mirroring the deploy-time
    // ordering the control-plane path uses (parse/persist before
    // instantiation) so the very first instantiation already resolves it.
    storage_provider
        .save_fdae_policy(
            SERVICE_ID,
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "profiles": {
                        "table": "profiles",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#,
        )
        .await
        .unwrap();

    let manifest = wasm_deploy_manifest(wasm_bytes);
    engine.deploy_wasm(SERVICE_ID, &manifest).await.unwrap();

    let store = storage_provider.open_service_db(SERVICE_ID, &key_store).await.unwrap();
    for i in 0..100u32 {
        store
            .put(
                "profiles",
                &RecordWriteValue {
                    id: format!("p{i}"),
                    payload: format!(r#"{{"age": {i}}}"#).into_bytes(),
                },
                SERVICE_ID,
                None,
            )
            .await
            .unwrap();
    }
    drop(store);

    let observed = run_query_scenario(&engine, 100).await;
    assert_eq!(
        observed, 0,
        "a guest-originated read under a loaded policy must be empty for an unauthenticated \
         connection (no verified `CallerContext` reaches this ingress at all, per design §6.1.2's \
         'WASM guests admit anonymous callers') -- the guest's own query, running as \
         service_system, can reach none of the 100 seeded rows. Slice B3.5-fdae closed D-04-02-h \
         for a *real* caller -- see \
         `test_deployed_policy_filters_guest_originated_query_for_a_real_caller_d04_02_h_closed` \
         below, which drives the same guest through `execute_wasm_json`'s new `caller` param."
    );
}

/// Slice B3.5-fdae closure of D-04-02-h ingress (i): a **real** caller
/// reaching the guest (via `execute_wasm_json`'s `caller` param, the same
/// one `dispatch.rs`'s `JsonRpcToWasm` branch now threads from a
/// router-verified connection) is no longer synthesized as `service_system`
/// inside `HostState` -- so the guest's own `query`, running as that real
/// caller, now reaches exactly the rows the policy grants it, not zero.
///
/// Uses a `principal_column`-direct policy (`profiles.creator_uuid`, a
/// payload field via `json_extract`, matched straight against the caller --
/// no `creator`/`user` join) because the write path's `creator_id` is always
/// host-stamped to the *service's* own `component_id`
/// (`host_capabilities.rs`'s `put`), never the caller's DID -- so a
/// caller-owned row has to be seeded directly through `ServiceStore`, the
/// same way the FDAE `data_db`/`sandbox_wasm` host tests already do, rather
/// than through the guest's own (unrelated, still-ungated) `put`.
#[tokio::test]
async fn test_deployed_policy_filters_guest_originated_query_for_a_real_caller_d04_02_h_closed() {
    let Ok(wasm_bytes) = fs::read(test_constants::data_layer_test_wasm_path()) else {
        eprintln!(
            "Skipping \
             test_deployed_policy_filters_guest_originated_query_for_a_real_caller_d04_02_h_closed: \
             data-layer-test WASM artifact not found (run `cargo build --target wasm32-wasip2 \
             --release` in test-components/data-layer-test)"
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let (engine, storage_provider, key_store) = make_engine_with_storage(dir.path()).await;

    storage_provider
        .save_fdae_policy(
            SERVICE_ID,
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "profiles": {
                        "table": "profiles",
                        "principal_column": "creator_uuid",
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["caller"]]}
                        }
                    }
                }
            }"#,
        )
        .await
        .unwrap();

    let manifest = wasm_deploy_manifest(wasm_bytes);
    engine.deploy_wasm(SERVICE_ID, &manifest).await.unwrap();

    const REAL_CALLER_DID: &str = "did:key:zRealCallerB35";
    let real_caller = CallerContext {
        caller_did: REAL_CALLER_DID.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: REAL_CALLER_DID.to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::service(SERVICE_ID, SERVICE_ID),
                can: Ability(Ability::DATA_LAYER_READ.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };

    // Seed two rows directly, bypassing the guest's own `put` (whose
    // host-stamped `creator_id` is always `SERVICE_ID`, never a real
    // caller's DID): one owned (per the policy) by the real caller, one
    // owned by a *different* principal that must stay unreachable --
    // mirroring `router/tests/proxy_dispatch.rs`'s ingress-(ii) test, which
    // asserts both the reached row and the denied one.
    const OTHER_PRINCIPAL_DID: &str = "did:key:zSomeoneElseB35";
    let store = storage_provider.open_service_db(SERVICE_ID, &key_store).await.unwrap();
    store
        .put(
            "profiles",
            &RecordWriteValue {
                id: "seeded-by-real-caller".to_string(),
                payload: format!(r#"{{"creator_uuid":"{REAL_CALLER_DID}"}}"#).into_bytes(),
            },
            SERVICE_ID,
            None,
        )
        .await
        .unwrap();
    store
        .put(
            "profiles",
            &RecordWriteValue {
                id: "seeded-for-someone-else".to_string(),
                payload: format!(r#"{{"creator_uuid":"{OTHER_PRINCIPAL_DID}"}}"#).into_bytes(),
            },
            SERVICE_ID,
            None,
        )
        .await
        .unwrap();
    // Five more *unrelated* rows (`{"age": 0..5}`, no `creator_uuid`),
    // seeded the same way -- standing in for what `run-crud-scenario`'s own
    // write half used to contribute before writes were gated too. This
    // policy declares no `data-layer/write` permission at all, so seeding
    // these through the guest's own (now-gated) `put` would deny closed
    // regardless of caller; seeding directly is what proves the *read* half
    // in isolation.
    for i in 0..5u32 {
        store
            .put(
                "profiles",
                &RecordWriteValue {
                    id: format!("unrelated-{i}"),
                    payload: format!(r#"{{"age": {i}}}"#).into_bytes(),
                },
                SERVICE_ID,
                None,
            )
            .await
            .unwrap();
    }
    drop(store);

    // The table now holds 7 rows total (2 seeded-with-`creator_uuid` + 5
    // unrelated), so `limit: 7` alone would satisfy `observed == 7` under
    // *no* filtering at all -- a smaller limit here would be
    // indistinguishable from correct filtering (any non-empty result
    // reaches the limit either way, per D-04-02-h ingress (i)'s original
    // review finding). With ingress (i) actually closed, the sieve admits
    // exactly the one row owned by the real caller (neither the 5 unrelated
    // rows nor the other principal's row match `creator_uuid`), so the true
    // count -- not a limit truncation -- is what makes `observed == 1`.
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "run-query-scenario".to_string(),
        params: serde_json::json!([7]),
        id: None,
    };
    let result = engine
        .execute_wasm_json(SERVICE_ID, TEST_DRIVER_INTERFACE, &request, Some(real_caller))
        .await
        .unwrap();
    let observed: u32 = result.as_str().unwrap().parse().unwrap();
    assert_eq!(
        observed, 1,
        "D-04-02-h ingress (i): a real caller's own guest-originated query must reach exactly the \
         one row the policy grants it, excluding both the 5 unrelated rows and the other \
         principal's row, out of 7 rows total (limit: 7) -- got {result:?}"
    );
}

/// Guest-originated ingress (i): the guest's own `put`
/// (`run-crud-scenario`'s write half) runs as whatever caller
/// `execute_wasm_json` forwards through `HostState.caller` -- so a real
/// caller holding `data-layer/write` on a `principal_column: "creator_id"`
/// policy can write and read back their own rows (the host-stamped
/// `creator_id` is exactly their own DID, via `write_attribution`), while a
/// real caller holding no capability at all is denied on the very first
/// write.
#[tokio::test]
async fn test_deployed_policy_authorizes_guest_originated_writes_for_a_real_caller_and_denies_another()
 {
    let Ok(wasm_bytes) = fs::read(test_constants::data_layer_test_wasm_path()) else {
        eprintln!(
            "Skipping \
             test_deployed_policy_authorizes_guest_originated_writes_for_a_real_caller_and_denies_another: \
             data-layer-test WASM artifact not found (run `cargo build --target wasm32-wasip2 \
             --release` in test-components/data-layer-test)"
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let (engine, storage_provider, _key_store) = make_engine_with_storage(dir.path()).await;

    storage_provider
        .save_fdae_policy(
            SERVICE_ID,
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "profiles": {
                        "table": "profiles",
                        "principal_column": "creator_id",
                        "permissions": {
                            "manage": {
                                "allows": ["data-layer/read", "data-layer/write"],
                                "paths": [["caller"]]
                            }
                        }
                    }
                }
            }"#,
        )
        .await
        .unwrap();

    let manifest = wasm_deploy_manifest(wasm_bytes);
    engine.deploy_wasm(SERVICE_ID, &manifest).await.unwrap();

    const WRITER_DID: &str = "did:key:zWriterB5";
    let writer = CallerContext {
        caller_did: WRITER_DID.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: WRITER_DID.to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::service(SERVICE_ID, SERVICE_ID),
                can: Ability(Ability::DATA_LAYER_WRITE.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    };

    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "run-crud-scenario".to_string(),
        params: serde_json::json!([3]),
        id: None,
    };
    let result = engine
        .execute_wasm_json(SERVICE_ID, TEST_DRIVER_INTERFACE, &request, Some(writer))
        .await
        .unwrap();
    let observed: u32 = result.as_str().unwrap().parse().unwrap();
    assert_eq!(
        observed, 3,
        "the writer must both create and read back all 3 of their own rows: {result:?}"
    );

    const STRANGER_DID: &str = "did:key:zStrangerB5";
    let stranger = CallerContext {
        caller_did: STRANGER_DID.to_string(),
        app_instance: None,
        session: SessionContext { subject_did: STRANGER_DID.to_string(), ..Default::default() },
        auth: AuthLevel::Ucan,
        proof: None,
    };
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "run-crud-scenario".to_string(),
        params: serde_json::json!([1]),
        id: None,
    };
    let err = engine
        .execute_wasm_json(SERVICE_ID, TEST_DRIVER_INTERFACE, &request, Some(stranger))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("PermissionDenied"),
        "a capability-less caller's guest-originated write must deny closed: {err:?}"
    );
}

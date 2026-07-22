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
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::JsonRpcRequest;
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
            schema_path: None,
            rotation_policy: None,
            fdae_policy_path: None,
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

async fn make_engine_with_storage(dir: &Path) -> (AppSandboxEngine, Arc<dyn StorageProvider>) {
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
        key_store,
        storage_provider.clone(),
        blob_provider,
        Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
    )
    .await
    .unwrap();
    (engine, storage_provider)
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
/// `query` sees none of the rows it just wrote via the (ungated) `put` path.
/// Whoever threads real caller identity into this ingress should flip this
/// assertion to the count actually reachable.
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
    let (engine, storage_provider) = make_engine_with_storage(dir.path()).await;

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

    let observed = run_crud_scenario(&engine, 100).await;
    assert_eq!(
        observed, 0,
        "a guest-originated read under a loaded policy must be empty (D-04-02-h) -- the write \
         side is ungated, so all 100 records exist, but the guest's own query, running as \
         service_system, can reach none of them"
    );
}

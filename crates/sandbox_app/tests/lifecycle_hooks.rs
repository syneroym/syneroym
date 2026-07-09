#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests for Slice 3A's schema lifecycle-hook gating: `execute-ddl`
//! must be denied outside an `init`/`migrate` context, and deploying a
//! component that doesn't export `init`/`migrate` at all must not error.

use std::sync::Arc;

use syneroym_core::{config::SubstrateConfig, test_constants};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_sandbox_app::{AppSandboxEngine, HostState};
use syneroym_wit_interfaces::{
    control_plane::exports::syneroym::control_plane::orchestrator::{
        ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
    },
    host::syneroym::data_layer::store::{DataLayerError, Host as DataLayerHost},
};

async fn make_engine(dir: &std::path::Path) -> AppSandboxEngine {
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
    let blob_provider: Arc<dyn syneroym_data_blob::BlobProvider> =
        Arc::new(syneroym_data_blob::ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    AppSandboxEngine::init(&config, vec![], key_store, storage_provider, blob_provider)
        .await
        .unwrap()
}

fn wasm_deploy_manifest(bytes: Vec<u8>, interfaces: Vec<String>) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema_path: None,
            rotation_policy: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces,
        }),
        registry_certificate: None,
    }
}

#[tokio::test]
async fn test_execute_ddl_denied_outside_lifecycle_context() {
    let dir = tempfile::tempdir().unwrap();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());

    let blob_provider: Arc<dyn syneroym_data_blob::BlobProvider> =
        Arc::new(syneroym_data_blob::ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    // is_init_context = false: a normal (non-lifecycle) invocation context.
    let mut host_state = HostState::new(
        "ddl-test-svc".to_string(),
        None,
        key_store,
        storage_provider,
        blob_provider,
        false,
        0,
    );

    let err = DataLayerHost::execute_ddl(&mut host_state, "CREATE TABLE x (id TEXT)".to_string())
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

#[tokio::test]
async fn test_deploy_skips_lifecycle_hook_gracefully_for_component_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;

    let Ok(wasm_bytes) = std::fs::read(test_constants::greeter_wasm_path()) else {
        eprintln!(
            "Skipping test_deploy_skips_lifecycle_hook_gracefully_for_component_without_it: \
             greeter WASM artifact not found"
        );
        return;
    };

    let manifest =
        wasm_deploy_manifest(wasm_bytes, vec![test_constants::GREETER_INTERFACE_NAME.to_string()]);

    // The greeter component exports no `init`/`migrate` -- deploy must
    // succeed without attempting (and failing on) those hooks.
    engine.deploy_wasm("greeter-svc", &manifest).await.unwrap();
}

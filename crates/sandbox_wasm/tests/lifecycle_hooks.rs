#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests for Slice 3A's schema lifecycle-hook gating: `execute-ddl`
//! must be denied outside an `init`/`migrate` context, and deploying a
//! component that doesn't export `init`/`migrate` at all must not error.

use std::{
    fs,
    path::Path,
    sync::{Arc, Weak},
};

use syneroym_core::{
    config::SubstrateConfig, local_registry::EndpointRegistry, storage::MockStorage, test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{Ability, AuthLevel, CallerContext, Capability, ResourceUri, SessionContext};
use syneroym_sandbox_wasm::{
    AppSandboxEngine, HostState, MessagingContext, StreamContext, empty_service_proxy,
};
use syneroym_wit_interfaces::{
    control_plane::exports::syneroym::control_plane::orchestrator::{
        ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
    },
    host::syneroym::data_layer::store::{DataLayerError, Host as DataLayerHost, SqlValue},
};

fn test_messaging_context() -> MessagingContext {
    MessagingContext {
        broker: Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap()),
        engine: Weak::new(),
    }
}

fn test_streaming_context() -> StreamContext {
    StreamContext {
        registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
        engine: Weak::new(),
    }
}

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

    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    // `service_system`: a normal (non-lifecycle, non-admin) invocation
    // context -- carries no `data-layer/admin` capability.
    let mut host_state = HostState::new(
        "ddl-test-svc".to_string(),
        None,
        key_store,
        storage_provider,
        blob_provider,
        CallerContext::service_system("ddl-test-svc"),
        0,
        test_messaging_context(),
        test_streaming_context(),
        empty_service_proxy(),
    );

    let err = DataLayerHost::execute_ddl(&mut host_state, "CREATE TABLE x (id TEXT)".to_string())
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

#[tokio::test]
async fn test_execute_ddl_allowed_for_local_elevated_lifecycle_context() {
    let dir = tempfile::tempdir().unwrap();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());

    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    // `local_elevated`: the substrate-injected init/migrate lifecycle
    // context, which carries `data-layer/admin` on this component's own
    // resource (ADR-0015/0016) -- the Admin gate must let it through.
    let mut host_state = HostState::new(
        "ddl-admin-svc".to_string(),
        None,
        key_store,
        storage_provider,
        blob_provider,
        CallerContext::local_elevated("ddl-admin-svc"),
        0,
        test_messaging_context(),
        test_streaming_context(),
        empty_service_proxy(),
    );

    DataLayerHost::execute_ddl(&mut host_state, "CREATE TABLE x (id TEXT)".to_string())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_execute_ddl_allowed_for_admin_ucan_root_caller() {
    let dir = tempfile::tempdir().unwrap();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());

    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    // A caller matching `[iam].admin_ucan_root` -- represented by the
    // `substrate/admin` grant `build_caller`
    // (`crates/router/src/route_handler/io.rs`) constructs for it -- must be
    // admitted to guest `execute-ddl` too (ADR-0015/0016, B0.md §11.2).
    let admin_did = "did:key:z6MkAdminRoot";
    let admin_caller = CallerContext {
        caller_did: admin_did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: admin_did.to_string(),
            capabilities: vec![Capability {
                with: ResourceUri::substrate(admin_did),
                can: Ability(Ability::SUBSTRATE_ADMIN.to_string()),
                caveats: None,
            }],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    };

    let mut host_state = HostState::new(
        "ddl-admin-root-svc".to_string(),
        None,
        key_store,
        storage_provider,
        blob_provider,
        admin_caller,
        0,
        test_messaging_context(),
        test_streaming_context(),
        empty_service_proxy(),
    );

    DataLayerHost::execute_ddl(&mut host_state, "CREATE TABLE x (id TEXT)".to_string())
        .await
        .unwrap();
}

/// `query-raw` (Slice B5, ADR-0011) is gated identically to `execute-ddl`:
/// an ordinary (non-lifecycle, non-admin) caller must be denied.
#[tokio::test]
async fn test_query_raw_denied_for_ordinary_caller() {
    let dir = tempfile::tempdir().unwrap();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    let mut host_state = HostState::new(
        "query-raw-deny-svc".to_string(),
        None,
        key_store,
        storage_provider,
        blob_provider,
        CallerContext::service_system("query-raw-deny-svc"),
        0,
        test_messaging_context(),
        test_streaming_context(),
        empty_service_proxy(),
    );

    let err = DataLayerHost::query_raw(&mut host_state, "SELECT 1".to_string(), vec![])
        .await
        .unwrap_err();
    assert!(matches!(err, DataLayerError::PermissionDenied));
}

/// A lifecycle-elevated caller (`init`/`migrate`) is admitted to `query-raw`,
/// same as `execute-ddl`.
#[tokio::test]
async fn test_query_raw_allowed_for_local_elevated_lifecycle_context() {
    let dir = tempfile::tempdir().unwrap();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));

    let mut host_state = HostState::new(
        "query-raw-admin-svc".to_string(),
        None,
        key_store,
        storage_provider,
        blob_provider,
        CallerContext::local_elevated("query-raw-admin-svc"),
        0,
        test_messaging_context(),
        test_streaming_context(),
        empty_service_proxy(),
    );

    let result = DataLayerHost::query_raw(
        &mut host_state,
        "SELECT 1 AS one".to_string(),
        Vec::<SqlValue>::new(),
    )
    .await
    .unwrap();
    assert_eq!(result.columns, vec!["one".to_string()]);
}

#[tokio::test]
async fn test_deploy_skips_lifecycle_hook_gracefully_for_component_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;

    let Ok(wasm_bytes) = fs::read(test_constants::greeter_wasm_path()) else {
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

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests for `syneroym:http/websocket-handler` dynamic
//! marshalling.

use std::{fs, path::Path, sync::Arc};

use syneroym_core::{
    config::SubstrateConfig, local_registry::EndpointRegistry, storage::MockStorage, test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_sandbox_wasm::{AppSandboxEngine, FrameKind};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};

const SERVICE_ID: &str = "websocket-guest-svc";

async fn make_engine(dir: &Path) -> Arc<AppSandboxEngine> {
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
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let engine = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            registry,
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    engine.self_weak.set(Arc::downgrade(&engine)).expect("self_weak set once");
    engine
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
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces: vec!["syneroym:http/websocket-handler@0.1.0".to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_websocket_marshalling() {
    let wasm_bytes = fs::read(test_constants::websocket_guest_test_wasm_path()).expect(
        "websocket-guest-test.wasm not built. Run `mise run build:test-components` to build test \
         fixtures.",
    );
    let temp_dir = tempfile::tempdir().unwrap();
    let engine = make_engine(temp_dir.path()).await;

    let manifest = wasm_deploy_manifest(wasm_bytes);
    engine.deploy_wasm(SERVICE_ID, &manifest).await.unwrap();

    let conn_id = "test-conn-1";
    let mut rx = engine.register_websocket_sender(SERVICE_ID, conn_id);

    // Call on-open, should send "welcome" as Text frame
    engine.handle_websocket_on_open(SERVICE_ID, conn_id, None).await;
    let (welcome, kind) = rx.recv().await.unwrap();
    assert_eq!(welcome, b"welcome");
    assert_eq!(kind, FrameKind::Text);

    // Call on-message with Text frame, should echo back as Text frame
    engine
        .handle_websocket_on_message(SERVICE_ID, conn_id, b"hello".to_vec(), FrameKind::Text, None)
        .await;
    let (echo, kind) = rx.recv().await.unwrap();
    assert_eq!(echo, b"hello");
    assert_eq!(kind, FrameKind::Text);

    // Call on-message with Binary frame, should echo back as Binary frame
    engine
        .handle_websocket_on_message(SERVICE_ID, conn_id, vec![1, 2, 3, 4], FrameKind::Binary, None)
        .await;
    let (echo_bin, kind_bin) = rx.recv().await.unwrap();
    assert_eq!(echo_bin, vec![1, 2, 3, 4]);
    assert_eq!(kind_bin, FrameKind::Binary);

    // Call on-close, does nothing
    engine.handle_websocket_on_close(SERVICE_ID, conn_id, None).await;

    // Deregister sender
    engine.deregister_websocket_sender(SERVICE_ID, conn_id);
}

use std::sync::Arc;

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use syneroym_core::{
    asset_manifest::AssetRegistry, config::SubstrateConfig, http_routes::HttpRouteRegistry,
    local_registry::EndpointRegistry, storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::NativeDispatchRegistry;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    AssetBundle as WitAssetBundle, ServiceConfig,
};

use super::{super::*, helpers::*};
use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

/// A service deployed with no `http_routes` key gets no
/// entry in the shared registry at all (not an empty-`Vec` entry) --
/// keeps the registry from growing with a no-op entry per ordinary
/// deployed service.
#[tokio::test]
async fn test_no_http_routes_entry_when_custom_config_has_none() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        native_dispatch,
        http_routes.clone(),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let service_id = "no-http-routes-svc".to_string();
    let manifest = DeployManifest {
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
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    };
    service.deploy(service_id.clone(), manifest, &node_wide_caller("test-caller")).await.unwrap();

    assert!(http_routes.get(&service_id).is_none());
}

/// A deploy declaring `assets` unpacks them into blobs,
/// registers a `ServiceAssets` entry the router can serve from, and
/// undeploy removes both the registry entry and the underlying blobs.
#[tokio::test]
async fn test_asset_bundle_populated_on_deploy_and_cleared_on_undeploy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let asset_registry: AssetRegistry = Arc::new(DashMap::new());
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider,
        blob_provider.clone(),
        messaging_broker,
        native_dispatch,
        http_routes,
        asset_registry.clone(),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let service_id = "asset-svc".to_string();
    let archive = make_asset_archive(&[("index.html", b"<html>hi</html>")]);
    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: Some(WitAssetBundle {
                archive: ArtifactSource::Binary(archive),
                hash: None,
                visibility: Some(WitVisibility::Public),
            }),
            visibility: None,
        },
        // An asset bundle is only servable for a `Wasm` service (a
        // `Tcp`/`Container` endpoint is raw passthrough and never
        // reaches the asset-serving HTTP path) -- `minimal_wasm_component`
        // is a real, trivial component so `deploy_wasm_service` itself
        // succeeds.
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(minimal_wasm_component()),
            hash: None,
            interfaces: vec!["asset-svc-interface".to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let caller = node_wide_caller("test-caller");
    service.deploy(service_id.clone(), manifest, &caller).await.unwrap();

    let entry = asset_registry.get(&service_id).expect("asset bundle populated on deploy");
    assert!(entry.public);
    assert_eq!(entry.manifest.entries.len(), 1);
    let asset = entry.manifest.entries.get("/index.html").unwrap();
    let stored = blob_provider.get_blob(&service_id, &asset.hash, None).await.unwrap();
    assert_eq!(stored, b"<html>hi</html>");
    let manifest_hash = entry.manifest_hash.clone();
    drop(entry);

    service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
    assert!(
        asset_registry.get(&service_id).is_none(),
        "asset registry entry must be removed on undeploy"
    );
    assert!(
        blob_provider.get_blob(&service_id, &manifest_hash, None).await.is_err(),
        "the manifest blob itself must be deleted on undeploy"
    );
}

/// A redeploy that changes only some files keeps every
/// blob the new manifest still shares with the old one, and deletes
/// only what genuinely dropped out.
#[tokio::test]
async fn test_asset_bundle_redeploy_keeps_shared_blobs_and_drops_removed_ones() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let asset_registry: AssetRegistry = Arc::new(DashMap::new());
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider,
        blob_provider.clone(),
        messaging_broker,
        native_dispatch,
        http_routes,
        asset_registry.clone(),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let service_id = "asset-redeploy-svc".to_string();
    let caller = node_wide_caller("test-caller");

    let bundle = |archive: Vec<u8>| DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: Some(WitAssetBundle {
                archive: ArtifactSource::Binary(archive),
                hash: None,
                visibility: Some(WitVisibility::Public),
            }),
            visibility: None,
        },
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(minimal_wasm_component()),
            hash: None,
            interfaces: vec!["asset-redeploy-svc-interface".to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };

    let first_archive =
        make_asset_archive(&[("shared.txt", b"unchanged"), ("old_only.txt", b"gone soon")]);
    service.deploy(service_id.clone(), bundle(first_archive), &caller).await.unwrap();
    let first_hashes = {
        let entry = asset_registry.get(&service_id).unwrap();
        (
            entry.manifest.entries.get("/shared.txt").unwrap().hash.clone(),
            entry.manifest.entries.get("/old_only.txt").unwrap().hash.clone(),
        )
    };
    let (shared_hash, old_only_hash) = first_hashes;

    let second_archive = make_asset_archive(&[("shared.txt", b"unchanged")]);
    service.deploy(service_id.clone(), bundle(second_archive), &caller).await.unwrap();

    assert!(
        blob_provider.get_blob(&service_id, &shared_hash, None).await.is_ok(),
        "unchanged file's blob must survive the redeploy"
    );
    assert!(
        blob_provider.get_blob(&service_id, &old_only_hash, None).await.is_err(),
        "the dropped file's blob must be garbage-collected"
    );

    service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
}

/// The backward asset rollback, driven through a real
/// deploy failure rather than `delete_hashes` called directly as pure
/// set arithmetic. `rollback_asset_bundle` is reached from five
/// separate failure branches in `deploy_with_context`; this exercises
/// the one already covered for FDAE-policy/config-generation rollback
/// by `test_deploy_failure_after_successful_wasm_compile_rolls_back_gen_and_policy`
/// (same `FailingEndpointStorage` fixture and `minimal_wasm_component`,
/// same failure point -- `register_wasm_endpoints`, which runs after
/// the asset block has already written the new generation's blobs, and
/// after the component itself compiled successfully), but proves the
/// asset half: the failed redeploy's own writes are gone, *and* every
/// blob the still-live previous generation references survives.
/// `Wasm`, not `Tcp`, since an asset bundle is only accepted for a
/// `Wasm` service as of the same review pass that added this test.
#[expect(clippy::too_many_lines, reason = "linear asset bundle rollback scenario")]
#[tokio::test]
async fn test_asset_bundle_rollback_on_a_real_deploy_failure_keeps_the_old_generation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new(Arc::new(FailingEndpointStorage {
        inner: MockStorage::new(),
        fail_interface: "fails-to-register".to_string(),
    }))
    .await
    .unwrap();

    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let asset_registry: AssetRegistry = Arc::new(DashMap::new());
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider,
        blob_provider.clone(),
        messaging_broker,
        native_dispatch,
        http_routes,
        asset_registry.clone(),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let service_id = "asset-wasm-rollback-svc".to_string();
    let caller = node_wide_caller("test-caller");

    let manifest = |archive: Vec<u8>, interface_name: &str| DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: Some(WitAssetBundle {
                archive: ArtifactSource::Binary(archive),
                hash: None,
                visibility: Some(WitVisibility::Public),
            }),
            visibility: None,
        },
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(minimal_wasm_component()),
            hash: None,
            interfaces: vec![interface_name.to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };

    // First deploy succeeds: generation 0's asset bundle is live.
    let first_archive = make_asset_archive(&[("index.html", b"gen0")]);
    service
        .deploy(service_id.clone(), manifest(first_archive, "safe-interface"), &caller)
        .await
        .unwrap();
    let (gen0_manifest_hash, gen0_asset_hash) = {
        let entry = asset_registry.get(&service_id).unwrap();
        (
            entry.manifest_hash.clone(),
            entry.manifest.entries.get("/index.html").unwrap().hash.clone(),
        )
    };

    // Redeploy with a *different* asset bundle (different content, so a
    // different blob hash) plus an interface name the registry is
    // rigged to reject -- the asset block runs and writes gen 1's blob
    // successfully, the component itself compiles fine, and then
    // `register_wasm_endpoints` fails, well after the asset write.
    let second_archive = make_asset_archive(&[("index.html", b"gen1 -- must not survive")]);
    let result = service
        .deploy(service_id.clone(), manifest(second_archive, "fails-to-register"), &caller)
        .await;
    assert!(result.is_err(), "endpoint registration must fail: {result:?}");
    assert!(result.unwrap_err().contains("Endpoint registration failed"));

    let entry = asset_registry.get(&service_id).expect(
        "a failed redeploy must leave the still-live generation 0 asset registry entry in place",
    );
    assert_eq!(
        entry.manifest_hash, gen0_manifest_hash,
        "the registry must still point at generation 0's manifest, not a half-applied generation 1"
    );
    let gen0_entry = entry.manifest.entries.get("/index.html").unwrap();
    assert_eq!(gen0_entry.hash, gen0_asset_hash);
    drop(entry);

    assert_eq!(
        blob_provider.get_blob(&service_id, &gen0_asset_hash, None).await.unwrap(),
        b"gen0",
        "generation 0's blob, still referenced by the live manifest, must survive the failed \
         redeploy's rollback"
    );
    assert!(
        blob_provider.get_blob(&service_id, &gen0_manifest_hash, None).await.is_ok(),
        "generation 0's manifest blob must survive too"
    );

    // The failed generation 1's own write must be gone -- find it by
    // hashing the content directly, since nothing in the live manifest
    // references it to look it up by.
    let gen1_hash = hex::encode(Sha256::digest(b"gen1 -- must not survive"));
    assert!(
        blob_provider.get_blob(&service_id, &gen1_hash, None).await.is_err(),
        "the failed redeploy's own blob write must have been rolled back, not orphaned alongside \
         generation 0"
    );

    service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
}

/// An asset bundle is only reachable through a `Wasm`
/// service's HTTP path -- a `Tcp`/`Container` endpoint is registered as
/// `SubstrateEndpoint::TcpHostPort`, which the router's `dispatch.rs`
/// unconditionally routes to raw passthrough regardless of what the
/// client actually sends, so an asset bundle attached to one is
/// silently unreachable dead data. Rejected at deploy instead, before
/// the asset block (or anything else fallible) has run -- asserted by
/// checking nothing was written, not just that the call errored.
#[tokio::test]
async fn test_asset_bundle_is_rejected_for_a_tcp_service() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let asset_registry: AssetRegistry = Arc::new(DashMap::new());
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider,
        blob_provider.clone(),
        messaging_broker,
        native_dispatch,
        http_routes,
        asset_registry.clone(),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let service_id = "tcp-with-assets-svc".to_string();
    let archive = make_asset_archive(&[("index.html", b"unreachable")]);
    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: Some(WitAssetBundle {
                archive: ArtifactSource::Binary(archive),
                hash: None,
                visibility: Some(WitVisibility::Public),
            }),
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let caller = node_wide_caller("test-caller");
    let err = service
        .deploy(service_id.clone(), manifest, &caller)
        .await
        .expect_err("a Tcp service must not accept an asset bundle");
    assert!(err.contains("only servable for a 'Wasm' service"), "{err}");
    assert!(
        asset_registry.get(&service_id).is_none(),
        "rejected at validation, before the asset registry is ever touched"
    );
}

/// Same as `test_asset_bundle_is_rejected_for_a_tcp_service`, for a
/// `Container` service -- `deploy_container_service` also registers a
/// `SubstrateEndpoint::TcpHostPort` (`crates/control_plane/src/service/
/// orchestration.rs`'s own `deploy_container_service`), so it is raw
/// passthrough for the identical reason.
#[tokio::test]
async fn test_asset_bundle_is_rejected_for_a_container_service() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let asset_registry: AssetRegistry = Arc::new(DashMap::new());
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider,
        blob_provider.clone(),
        messaging_broker,
        native_dispatch,
        http_routes,
        asset_registry.clone(),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let service_id = "container-with-assets-svc".to_string();
    let archive = make_asset_archive(&[("index.html", b"unreachable")]);
    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: Some(WitAssetBundle {
                archive: ArtifactSource::Binary(archive),
                hash: None,
                visibility: Some(WitVisibility::Public),
            }),
            visibility: None,
        },
        service_type: WitServiceType::Container(ContainerManifest {
            source: ArtifactSource::Url("docker.io/library/nginx:1.27".to_string()),
            hash: None,
            image: "docker.io/library/nginx:1.27".to_string(),
            ports: vec![],
            volumes: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let caller = node_wide_caller("test-caller");
    let err = service
        .deploy(service_id.clone(), manifest, &caller)
        .await
        .expect_err("a Container service must not accept an asset bundle");
    assert!(err.contains("only servable for a 'Wasm' service"), "{err}");
    assert!(asset_registry.get(&service_id).is_none());
}

/// Same reasoning as
/// `test_asset_bundle_is_rejected_for_a_tcp_service` -- a `Tcp`
/// service's endpoint is raw passthrough, so a declared `guest` route
/// would be silent dead configuration.
#[tokio::test]
async fn test_guest_route_is_rejected_for_a_tcp_service() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let asset_registry: AssetRegistry = Arc::new(DashMap::new());
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider.clone(),
        blob_provider.clone(),
        messaging_broker,
        native_dispatch,
        http_routes.clone(),
        asset_registry,
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let service_id = "tcp-with-guest-route-svc".to_string();
    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: Some(guest_route_custom_config()),
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let caller = node_wide_caller("test-caller");
    let err = service
        .deploy(service_id.clone(), manifest, &caller)
        .await
        .expect_err("a Tcp service must not accept a guest route");
    assert!(err.contains("is only servable for a 'Wasm' service"), "{err}");
    assert!(
        storage_provider.get_latest_config_generation(&service_id).await.unwrap().is_none(),
        "rejected before anything fallible runs -- no config generation saved"
    );
    assert!(http_routes.get(&service_id).is_none());
}

/// Same as `test_guest_route_is_rejected_for_a_tcp_service`, for a
/// `Container` service.
#[tokio::test]
async fn test_guest_route_is_rejected_for_a_container_service() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let asset_registry: AssetRegistry = Arc::new(DashMap::new());
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider.clone(),
        blob_provider.clone(),
        messaging_broker,
        native_dispatch,
        http_routes.clone(),
        asset_registry,
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let service_id = "container-with-guest-route-svc".to_string();
    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: Some(guest_route_custom_config()),
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Container(ContainerManifest {
            source: ArtifactSource::Url("docker.io/library/nginx:1.27".to_string()),
            hash: None,
            image: "docker.io/library/nginx:1.27".to_string(),
            ports: vec![],
            volumes: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let caller = node_wide_caller("test-caller");
    let err = service
        .deploy(service_id.clone(), manifest, &caller)
        .await
        .expect_err("a Container service must not accept a guest route");
    assert!(err.contains("is only servable for a 'Wasm' service"), "{err}");
    assert!(http_routes.get(&service_id).is_none());
}

/// A declared `guest` route whose compiled component
/// does not export `handle-request` must fail the deploy -- rolling
/// back the config generation, the FDAE policy, and any asset bundle
/// already written, exactly as `test_stage4_policy_without_the_export_
/// fails_deploy` does for the stage-4 export gate.
#[tokio::test]
async fn test_guest_route_without_the_export_fails_deploy_and_rolls_back() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let asset_registry: AssetRegistry = Arc::new(DashMap::new());
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider.clone(),
        blob_provider.clone(),
        messaging_broker,
        native_dispatch,
        http_routes.clone(),
        asset_registry.clone(),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let service_id = "guest-route-missing-export-svc".to_string();
    let archive = make_asset_archive(&[("index.html", b"hi")]);
    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: Some(guest_route_custom_config()),
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: Some(WitAssetBundle {
                archive: ArtifactSource::Binary(archive),
                hash: None,
                visibility: Some(WitVisibility::Public),
            }),
            visibility: None,
        },
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(WASM_WITHOUT_AUTHORIZE_ROWS_EXPORT.as_bytes().to_vec()),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let caller = node_wide_caller("test-caller");
    let err = service
        .deploy(service_id.clone(), manifest, &caller)
        .await
        .expect_err("a component without the handler export must not deploy with a guest route");
    assert!(
        err.contains("target=guest") && err.contains("does not export"),
        "expected the D-A2-10b error, got: {err}"
    );
    assert!(
        storage_provider.get_latest_config_generation(&service_id).await.unwrap().is_none(),
        "config generation must be rolled back"
    );
    assert!(
        storage_provider.load_fdae_policy(&service_id).await.unwrap().is_none(),
        "fdae policy must be rolled back"
    );
    assert!(asset_registry.get(&service_id).is_none(), "asset bundle must be rolled back");
}

#[tokio::test]
async fn test_websocket_route_without_the_export_fails_deploy_and_rolls_back() {
    let temp_dir = tempfile::tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let app_sandbox = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine =
        Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let asset_registry: AssetRegistry = Arc::new(DashMap::new());
    let service = ControlPlaneService::init(
        "orchestrator".to_string(),
        "did:key:zTestNode".to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.path().to_path_buf(),
        key_store,
        storage_provider.clone(),
        blob_provider.clone(),
        messaging_broker,
        native_dispatch,
        http_routes.clone(),
        asset_registry.clone(),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let service_id = "ws-route-missing-export-svc".to_string();
    let custom_config = serde_json::json!({
        "http_routes": [
            {"method": "GET", "path": "/ws", "target": "websocket", "operation": "handle-upgrade"}
        ]
    })
    .to_string();

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: Some(custom_config),
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: None,
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(WASM_WITHOUT_AUTHORIZE_ROWS_EXPORT.as_bytes().to_vec()),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let caller = node_wide_caller("test-caller");
    let err = service.deploy(service_id.clone(), manifest, &caller).await.expect_err(
        "a component without the websocket export must not deploy with a websocket route",
    );
    assert!(
        err.contains("target=websocket") && err.contains("does not export"),
        "expected websocket export error, got: {err}"
    );
    assert!(
        storage_provider.get_latest_config_generation(&service_id).await.unwrap().is_none(),
        "config generation must be rolled back"
    );
}

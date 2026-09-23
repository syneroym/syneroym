use std::{
    fs,
    sync::{Arc, Mutex},
};

use dashmap::DashMap;
use syneroym_core::{
    config::SubstrateConfig, http_routes::HttpRouteRegistry, local_registry::EndpointRegistry,
    storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::NativeDispatchRegistry;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::ServiceConfig;

use super::{super::*, helpers::*};
use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

/// `validate_stage4_export` (ADR-0017 §8): a policy that opts
/// into the stage-4 after-step but whose compiled WASM component does
/// not export `syneroym:data-layer/authorizer#authorize-rows` must fail
/// the deploy, not ship a service that silently denies every read
/// through that permission at runtime.
#[tokio::test]
async fn test_stage4_policy_without_the_export_fails_deploy() {
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
        messaging_broker.clone(),
        native_dispatch.clone(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Inline(STAGE4_POLICY.to_string())),
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

    let result = service
        .deploy("stage4_missing_export_svc".to_string(), manifest, &node_wide_caller("test-caller"))
        .await;
    assert!(result.is_err(), "a stage-4-opted policy on a component without the export must fail");
    let err = result.unwrap_err();
    assert!(
        err.contains("authorize_rows: true") && err.contains("does not export"),
        "expected the validate_stage4_export error, got: {err}"
    );
}

/// Same shape as `deploy_wasm_service`'s gate above, for the two
/// service types that have no guest component to call at all: a TCP
/// service can never satisfy a stage-4 opt-in, so it is rejected up
/// front, before any endpoint registration.
#[tokio::test]
async fn test_stage4_policy_on_a_tcp_service_fails_deploy() {
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
        messaging_broker.clone(),
        native_dispatch.clone(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Inline(STAGE4_POLICY.to_string())),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    };

    let result = service
        .deploy("stage4_tcp_svc".to_string(), manifest, &node_wide_caller("test-caller"))
        .await;
    assert!(result.is_err(), "a stage-4-opted policy on a TCP service must fail deploy");
    let err = result.unwrap_err();
    assert!(
        err.contains("authorize_rows: true") && err.contains("no guest component"),
        "expected the TCP-service stage-4 rejection, got: {err}"
    );
}

/// A rejected stage-4 deploy must not leave its (already-persisted, per
/// `deploy`'s save-then-validate ordering) policy row in force --
/// `rollback_fdae_policy` must restore whatever was there before (here,
/// nothing at all: `stage4_rollback_svc` has never deployed before).
#[tokio::test]
async fn test_stage4_policy_rejection_rolls_back_the_policy_row() {
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
        messaging_broker.clone(),
        native_dispatch.clone(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Inline(STAGE4_POLICY.to_string())),
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

    let result = service
        .deploy("stage4_rollback_svc".to_string(), manifest, &node_wide_caller("test-caller"))
        .await;
    assert!(result.is_err(), "the deploy must still be rejected: {result:?}");
    assert_eq!(
        storage_provider.load_fdae_policy("stage4_rollback_svc").await.unwrap(),
        None,
        "a rejected stage-4 deploy must roll back the policy row it saved before validating the \
         export, not leave the rejected policy in force"
    );
}

/// A schema-invalid document that is itself sensitive-looking content
/// (not a policy at all) must not have that content echoed back to the
/// remote deploy caller. `jsonschema::ValidationError`'s `Display` embeds
/// the offending JSON *instance* -- for a top-level type mismatch, that
/// instance is the whole file -- so `PolicyError::Schema`'s `to_string()`
/// must never be forwarded verbatim into the returned error.
#[tokio::test]
async fn test_deploy_fdae_policy_error_does_not_echo_file_contents() {
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
        messaging_broker.clone(),
        native_dispatch.clone(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    let policy_filename = format!("test_fdae_secret_leak_{}.json", std::process::id());
    let secret = "SUPER_SECRET_API_KEY_abc123";
    fs::write(&policy_filename, format!("\"{secret}\"")).unwrap();

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Path(policy_filename.clone())),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    };

    let result = service
        .deploy("fdae_leak_service".to_string(), manifest, &node_wide_caller("test-caller"))
        .await;

    let _ = fs::remove_file(&policy_filename);

    let err = result.unwrap_err();
    assert!(err.contains("FDAE policy validation failed"), "{err}");
    assert!(!err.contains(secret), "policy file content leaked into the deploy error: {err}");
}

#[tokio::test]
async fn test_deploy_fdae_policy_traversal_and_absolute_rejected() {
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
        messaging_broker.clone(),
        native_dispatch.clone(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    for bad_path in ["../../../../etc/fdae-policy.json", "/etc/fdae-policy.json"] {
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(bad_path.to_string())),
                health_check: None,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy("fdae_traversal_service".to_string(), manifest, &node_wide_caller("t"))
            .await;
        assert!(result.is_err(), "{bad_path} should be rejected");
        assert!(
            result.unwrap_err().contains("Arbitrary file read prevented: Path traversal"),
            "{bad_path} should fail on the traversal guard"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn test_deploy_fdae_policy_symlink_escape_rejected() {
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
        messaging_broker.clone(),
        native_dispatch.clone(),
        Arc::new(DashMap::new()),
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap();

    // Same symlink-escape gap as the schema guard, on the
    // fdae_policy guard: no `..` component, not absolute, but the
    // symlink target lives outside the working directory.
    let outside_dir = tempfile::tempdir().unwrap();
    let outside_policy = outside_dir.path().join("fdae-policy.json");
    fs::write(&outside_policy, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();

    let symlink_name = format!("test_fdae_policy_symlink_{}.json", std::process::id());
    std::os::unix::fs::symlink(&outside_policy, &symlink_name).unwrap();

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Path(symlink_name.clone())),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    };

    let result = service
        .deploy(
            "symlink_fdae_policy_service".to_string(),
            manifest,
            &node_wide_caller("test-caller"),
        )
        .await;

    let _ = fs::remove_file(&symlink_name);

    assert!(result.is_err());
    let err_msg = result.unwrap_err();
    assert!(
        err_msg.contains("resolves outside the working directory via a symlink"),
        "{}",
        err_msg
    );
}

#[test]
fn test_warn_on_policy_collection_mismatch_fires_in_both_directions() {
    use std::io;

    use tracing_subscriber::prelude::*;

    let logs = Arc::new(Mutex::new(Vec::new()));
    let logs_clone = logs.clone();

    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let make_writer = move || MockWriter { logs: logs_clone.clone() };
    let layer = tracing_subscriber::fmt::layer().with_writer(make_writer).with_ansi(false);
    let subscriber = tracing_subscriber::registry().with(layer);

    // "widget" -> "widgets" is present in `collections` (no warning
    // expected). "gizmo" -> "gizmos" is a `definitions:` entry whose
    // table doesn't exist yet (direction 2). "orphan_table" exists in
    // `collections` with no matching definition (direction 1).
    let policy = syneroym_fdae::parse_and_validate(
        r#"{
                "version": "fdae/v1",
                "definitions": {
                    "widget": { "table": "widgets" },
                    "gizmo": { "table": "gizmos" }
                }
            }"#,
    )
    .unwrap();

    tracing::subscriber::with_default(subscriber, || {
        warn_on_policy_collection_mismatch(
            "svc-a",
            &policy,
            &["widgets".to_string(), "orphan_table".to_string()],
        );
    });

    let output = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(
        output.contains("orphan_table") && output.contains("has no FDAE definition"),
        "direction 1 (table with no definition) should warn: {output}"
    );
    assert!(
        output.contains("gizmos") && output.contains("no such collection exists"),
        "direction 2 (definition with no table) should warn: {output}"
    );
    assert!(
        !output.contains("collection=\"widgets\""),
        "a collection with a matching definition must not warn: {output}"
    );
}

#[test]
fn test_warn_on_ambiguous_public_permission() {
    use std::io;

    use tracing_subscriber::prelude::*;

    struct MockWriter {
        logs: Arc<Mutex<Vec<u8>>>,
    }
    impl io::Write for MockWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.logs.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let run = |policy_json: &str| -> String {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let logs_clone = logs.clone();
        let make_writer = move || MockWriter { logs: logs_clone.clone() };
        let layer = tracing_subscriber::fmt::layer().with_writer(make_writer).with_ansi(false);
        let subscriber = tracing_subscriber::registry().with(layer);
        let policy = syneroym_fdae::parse_and_validate(policy_json).unwrap();
        tracing::subscriber::with_default(subscriber, || {
            warn_on_ambiguous_public_permission("svc-a", &policy);
        });
        String::from_utf8(logs.lock().unwrap().clone()).unwrap()
    };

    // "audit" is unconditionally public and shares `data-layer/read`
    // with the path-restricted "view", with no `includes` link between
    // them -- exactly the shape that silently widens "view" for any
    // caller holding a generic read capability.
    let ambiguous = run(r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]},
                            "audit": {"allows": ["data-layer/read"], "paths": []}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#);
    assert!(
        ambiguous.contains("public_permission=\"audit\"")
            && ambiguous.contains("restricted_permission=\"view\""),
        "an unlinked public/restricted pair sharing an ability should warn: {ambiguous}"
    );

    // Same shape, but "audit" declares `includes: ["view"]` -- the
    // author made the relationship explicit, so no warning.
    let linked = run(r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]},
                            "audit": {"allows": ["data-layer/read"], "paths": [], "includes": ["view"]}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#);
    assert!(linked.is_empty(), "an explicit `includes` link must not warn: {linked}");

    // "audit" and "view" don't share a covering ability at all (write
    // vs. read, and neither entails the other) -- no warning.
    let disjoint = run(r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["rpc/move"], "paths": [["creator", "caller"]]},
                            "audit": {"allows": ["data-layer/read"], "paths": []}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#);
    assert!(disjoint.is_empty(), "disjoint abilities must not warn: {disjoint}");
}

/// `deploy()` parses `http_routes` out of `custom_config`
/// and populates the shared `HttpRouteRegistry` (the same `Arc` handed
/// to `RouteHandlerInner` in production); `undeploy()` clears it. A TCP
/// manifest is enough -- `http_routes` parsing/storage is independent
/// of `service_type`.
#[tokio::test]
async fn test_http_routes_populated_on_deploy_and_cleared_on_undeploy() {
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

    let service_id = "http-routes-svc".to_string();
    let custom_config = serde_json::json!({
        "http_routes": [
            {"method": "GET", "path": "/orders/{id}", "target": "data-layer",
             "operation": "get", "collection": "orders"},
            {"method": "POST", "path": "/orders", "target": "data-layer",
             "operation": "put", "collection": "orders"},
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
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let caller = node_wide_caller("test-caller");
    service.deploy(service_id.clone(), manifest, &caller).await.unwrap();

    let routes = http_routes.get(&service_id).expect("http_routes populated on deploy");
    assert_eq!(routes.len(), 2);
    assert_eq!(routes[0].collection.as_deref(), Some("orders"));
    drop(routes);

    service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
    assert!(
        http_routes.get(&service_id).is_none(),
        "http_routes entry must be removed on undeploy"
    );
}

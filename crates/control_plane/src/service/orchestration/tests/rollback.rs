use std::{fs, sync::Arc};

use dashmap::DashMap;
use syneroym_core::{
    config::SubstrateConfig, local_registry::EndpointRegistry, storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::NativeDispatchRegistry;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    NetworkEndpoint, ServiceConfig,
};

use super::{super::*, helpers::*};
use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

#[tokio::test]
async fn deploy_refuses_a_component_whose_compensation_has_no_forward_operation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let (service, _storage) = saga_gate_test_service(temp_dir.path()).await;
    let manifest =
        saga_gate_manifest(WASM_UNDO_WITH_NO_FORWARD, vec!["test-interface".to_string()]);
    let result = service
        .deploy("saga_missing_forward_svc".to_string(), manifest, &node_wide_caller("test"))
        .await;
    assert!(result.is_err(), "a saga-undo- export with no forward operation must fail deploy");
    let err = result.unwrap_err();
    assert!(
        err.contains("saga-undo-reserve") && err.contains("no 'reserve' beside it"),
        "expected the saga compensation gate error, got: {err}"
    );
}

#[tokio::test]
async fn deploy_accepts_a_component_exporting_both_halves() {
    let temp_dir = tempfile::tempdir().unwrap();
    let (service, _storage) = saga_gate_test_service(temp_dir.path()).await;
    let manifest = saga_gate_manifest(WASM_UNDO_WITH_FORWARD, vec!["test-interface".to_string()]);
    let result = service
        .deploy("saga_both_halves_svc".to_string(), manifest, &node_wide_caller("test"))
        .await;
    assert!(result.is_ok(), "both halves present must deploy cleanly: {result:?}");
}

/// The false-refusal the reserved `saga-undo-` prefix exists to
/// prevent: `undo-last-update` is a legal business verb with no
/// `last-update` beside it, and must not be refused.
#[tokio::test]
async fn deploy_accepts_a_component_exporting_a_plain_undo_prefixed_function() {
    let temp_dir = tempfile::tempdir().unwrap();
    let (service, _storage) = saga_gate_test_service(temp_dir.path()).await;
    let manifest = saga_gate_manifest(WASM_PLAIN_UNDO_PREFIX, vec!["test-interface".to_string()]);
    let result = service
        .deploy("saga_plain_undo_svc".to_string(), manifest, &node_wide_caller("test"))
        .await;
    assert!(result.is_ok(), "a plain undo- business verb must not be refused: {result:?}");
}

/// The common case: a service with no compensations at all deploys
/// cleanly, with no declaration required anywhere.
#[tokio::test]
async fn deploy_accepts_a_component_with_no_compensations_at_all() {
    let temp_dir = tempfile::tempdir().unwrap();
    let (service, _storage) = saga_gate_test_service(temp_dir.path()).await;
    let manifest =
        saga_gate_manifest(WASM_WITHOUT_AUTHORIZE_ROWS_EXPORT, vec!["test-interface".to_string()]);
    let result = service
        .deploy("saga_no_compensations_svc".to_string(), manifest, &node_wide_caller("test"))
        .await;
    assert!(result.is_ok(), "a component with no compensations must deploy cleanly: {result:?}");
}

#[tokio::test]
async fn a_refused_compensation_pairing_rolls_back_the_config_generation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let (service, storage) = saga_gate_test_service(temp_dir.path()).await;
    let manifest =
        saga_gate_manifest(WASM_UNDO_WITH_NO_FORWARD, vec!["test-interface".to_string()]);
    let result =
        service.deploy("saga_rollback_svc".to_string(), manifest, &node_wide_caller("test")).await;
    assert!(result.is_err());
    assert!(
        storage.get_latest_config_generation("saga_rollback_svc").await.unwrap().is_none(),
        "a refused saga compensation pairing must roll back the config generation it wrote before \
         validating exports"
    );
}

/// A known limit, pinned so the behavior is a choice and not an
/// accident: `exported_functions` returns `None` for an interface the
/// manifest never declared, which turns a declared-but-absent
/// compensation pairing into a silent pass rather than a refusal.
/// Backlog: "A declared interface that is not a component export is not
/// refused at deploy".
#[tokio::test]
async fn a_compensation_on_an_interface_the_manifest_does_not_declare_is_not_examined() {
    let temp_dir = tempfile::tempdir().unwrap();
    let (service, _storage) = saga_gate_test_service(temp_dir.path()).await;
    // The component exports `saga-undo-reserve` with no `reserve`, but
    // the manifest's own `interfaces` list never names `test-interface`
    // -- so the gate's loop (which walks only declared interfaces)
    // never looks at it.
    let manifest = saga_gate_manifest(WASM_UNDO_WITH_NO_FORWARD, vec![]);
    let result = service
        .deploy("saga_undeclared_iface_svc".to_string(), manifest, &node_wide_caller("test"))
        .await;
    assert!(
        result.is_ok(),
        "a compensation on an undeclared interface is not examined by this gate: {result:?}"
    );
}

#[tokio::test]
async fn test_undeploy_removes_fdae_policy() {
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

    let policy_filename = format!("test_fdae_undeploy_policy_{}.json", std::process::id());
    fs::write(&policy_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();

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

    let caller = node_wide_caller("test-caller");
    service.deploy("undeploy_fdae_svc".to_string(), manifest, &caller).await.unwrap();
    let _ = fs::remove_file(&policy_filename);
    assert!(storage_provider.load_fdae_policy("undeploy_fdae_svc").await.unwrap().is_some());

    service.undeploy("undeploy_fdae_svc".to_string(), 0, &caller).await.unwrap();
    assert_eq!(
        storage_provider.load_fdae_policy("undeploy_fdae_svc").await.unwrap(),
        None,
        "undeploy must clear a service's persisted FDAE policy"
    );
}

#[tokio::test]
async fn test_redeploy_without_fdae_block_clears_previous_policy() {
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

    let policy_filename = format!("test_fdae_redeploy_policy_{}.json", std::process::id());
    fs::write(&policy_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();

    let caller = node_wide_caller("test-caller");
    let with_policy = DeployManifest {
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
    service.deploy("redeploy_fdae_svc".to_string(), with_policy, &caller).await.unwrap();
    let _ = fs::remove_file(&policy_filename);
    assert!(storage_provider.load_fdae_policy("redeploy_fdae_svc").await.unwrap().is_some());

    // Re-deploy the same service_id with no `fdae` block at all.
    let without_policy = DeployManifest {
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
    service.deploy("redeploy_fdae_svc".to_string(), without_policy, &caller).await.unwrap();
    assert_eq!(
        storage_provider.load_fdae_policy("redeploy_fdae_svc").await.unwrap(),
        None,
        "a re-deploy whose manifest drops the fdae block must clear the previous policy, not \
         leave it for the WASM engine to resurrect from storage"
    );
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn test_deploy_failure_restores_previous_fdae_policy_not_the_new_one() {
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

    let caller = node_wide_caller("test-caller");

    // First, a successful deploy with policy P1.
    let policy_1_filename = format!("test_fdae_rollback_p1_{}.json", std::process::id());
    fs::write(&policy_1_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
    let first = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Path(policy_1_filename.clone())),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    };
    service.deploy("rollback_fdae_svc".to_string(), first, &caller).await.unwrap();
    let _ = fs::remove_file(&policy_1_filename);
    assert_eq!(
        storage_provider.load_fdae_policy("rollback_fdae_svc").await.unwrap(),
        Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string())
    );

    // Re-deploy the same service_id as WASM, with a new policy P2 and a
    // WASM source that doesn't exist -- `deploy_wasm` fails, which must
    // restore P1, not leave P2 (already persisted before the failure)
    // or an empty row in place.
    let policy_2_filename = format!("test_fdae_rollback_p2_{}.json", std::process::id());
    fs::write(&policy_2_filename, r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#)
        .unwrap();
    let second = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Path(policy_2_filename.clone())),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Url("/does_not_exist.wasm".to_string()),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let result = service.deploy("rollback_fdae_svc".to_string(), second, &caller).await;
    let _ = fs::remove_file(&policy_2_filename);
    assert!(result.is_err(), "the WASM deploy must fail: {result:?}");

    assert_eq!(
        storage_provider.load_fdae_policy("rollback_fdae_svc").await.unwrap(),
        Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string()),
        "a failed re-deploy must restore the previous policy, not leave the new one in force or \
         drop the row entirely -- the still-running previous version's engine cache would \
         otherwise resurrect the failed deploy's policy on its next miss"
    );
}

#[tokio::test]
async fn test_deploy_failure_restores_a_policy_the_new_manifest_dropped() {
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

    let caller = node_wide_caller("test-caller");

    // First, a successful deploy with a policy.
    let policy_filename = format!("test_fdae_dropped_rollback_{}.json", std::process::id());
    fs::write(&policy_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
    let first = DeployManifest {
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
    service.deploy("dropped_rollback_svc".to_string(), first, &caller).await.unwrap();
    let _ = fs::remove_file(&policy_filename);
    assert_eq!(
        storage_provider.load_fdae_policy("dropped_rollback_svc").await.unwrap(),
        Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string())
    );

    // Re-deploy the same service_id as WASM, with no `fdae` block at all
    // (the new manifest's `config` fully declares this deploy's policy
    // state, so absence deletes the previous row up front) and a WASM
    // source that doesn't exist, so `deploy_wasm` fails after the
    // deletion already happened. The failure must restore the policy
    // that was there before this deploy attempt, not leave the row
    // deleted -- an already-running previous version must not lose its
    // policy to an unrelated failed re-deploy.
    let second = DeployManifest {
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
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Url("/does_not_exist.wasm".to_string()),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let result = service.deploy("dropped_rollback_svc".to_string(), second, &caller).await;
    assert!(result.is_err(), "the WASM deploy must fail: {result:?}");

    assert_eq!(
        storage_provider.load_fdae_policy("dropped_rollback_svc").await.unwrap(),
        Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string()),
        "a failed re-deploy whose manifest dropped the fdae block must restore the policy that \
         existed before this attempt, not leave it deleted -- the still-running previous \
         version's engine cache would otherwise resolve no policy on its next miss"
    );
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn test_deploy_failure_after_successful_wasm_compile_rolls_back_gen_and_policy() {
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
    // The endpoint registry itself fails to persist one specific
    // interface -- simulating `register_wasm_endpoints` (called *after*
    // `deploy_wasm` has already compiled/cached the component and run
    // its lifecycle hook) hitting a real storage error.
    let registry = EndpointRegistry::new(Arc::new(FailingEndpointStorage {
        inner: MockStorage::new(),
        fail_interface: "fails-to-register".to_string(),
    }))
    .await
    .unwrap();

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

    let caller = node_wide_caller("test-caller");

    // First, a successful TCP deploy with policy P1, establishing a
    // baseline config generation and policy for the same service_id.
    let policy_1_filename = format!("test_fdae_endpoint_reg_p1_{}.json", std::process::id());
    fs::write(&policy_1_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
    let first = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Path(policy_1_filename.clone())),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
        registry_certificate: None,
        instance_certificate: None,
    };
    service.deploy("endpoint_reg_svc".to_string(), first, &caller).await.unwrap();
    let _ = fs::remove_file(&policy_1_filename);
    let (gen_before, _) =
        storage_provider.get_latest_config_generation("endpoint_reg_svc").await.unwrap().unwrap();
    assert_eq!(
        storage_provider.load_fdae_policy("endpoint_reg_svc").await.unwrap(),
        Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string())
    );

    // Re-deploy as WASM with a real, minimal, valid component (so
    // `deploy_wasm` itself succeeds) and a new policy P2, but declaring
    // the interface name the registry is rigged to reject -- so the
    // failure happens in `register_wasm_endpoints`, *after* the
    // component was already compiled/cached and P2 already persisted.
    let wat = r#"
(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface (export "greet" (func $noop)))
  (export "test-interface" (instance $interface))
)
"#;
    let policy_2_filename = format!("test_fdae_endpoint_reg_p2_{}.json", std::process::id());
    fs::write(&policy_2_filename, r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#)
        .unwrap();
    let second = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Path(policy_2_filename.clone())),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wat.as_bytes().to_vec()),
            hash: None,
            interfaces: vec!["fails-to-register".to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let result = service.deploy("endpoint_reg_svc".to_string(), second, &caller).await;
    let _ = fs::remove_file(&policy_2_filename);
    assert!(result.is_err(), "endpoint registration must fail: {result:?}");
    assert!(result.unwrap_err().contains("Endpoint registration failed"));

    assert_eq!(
        storage_provider.load_fdae_policy("endpoint_reg_svc").await.unwrap(),
        Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string()),
        "a register_wasm_endpoints failure -- after the component was already compiled and the \
         new policy already persisted -- must restore the previous policy, not leave the new one \
         (P2) in force"
    );
    let (gen_after, _) =
        storage_provider.get_latest_config_generation("endpoint_reg_svc").await.unwrap().unwrap();
    assert_eq!(
        gen_after, gen_before,
        "a register_wasm_endpoints failure must roll back the config generation this deploy \
         attempt saved, not leave it in force alongside a rolled-back policy"
    );
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn test_deploy_tcp_endpoint_registration_failure_rolls_back_gen_and_policy() {
    // Regression: `deploy_tcp_service` used to have no rollback at all
    // -- a failed TCP redeploy left the new policy (P2) persisted and
    // the config generation bumped, with the previous, still-running
    // version's policy row silently replaced. Same shape as the
    // already-covered WASM/container arms, using the same
    // `FailingEndpointStorage` fixture to force the failure
    // deterministically instead of a real network error.
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

    let caller = node_wide_caller("test-caller");

    // First, a successful TCP deploy with policy P1, using an interface
    // name the registry accepts.
    let policy_1_filename = format!("test_tcp_rollback_p1_{}.json", std::process::id());
    fs::write(&policy_1_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
    let first = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Path(policy_1_filename.clone())),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest {
            endpoints: vec![NetworkEndpoint {
                interface_name: "safe-interface".to_string(),
                host: "127.0.0.1".to_string(),
                port: 9000,
            }],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    service.deploy("tcp_rollback_svc".to_string(), first, &caller).await.unwrap();
    let _ = fs::remove_file(&policy_1_filename);
    assert_eq!(
        storage_provider.load_fdae_policy("tcp_rollback_svc").await.unwrap(),
        Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string())
    );
    let (gen_before, _) =
        storage_provider.get_latest_config_generation("tcp_rollback_svc").await.unwrap().unwrap();

    // Re-deploy the same TCP service with a new policy P2, declaring the
    // interface name the registry is rigged to reject.
    let policy_2_filename = format!("test_tcp_rollback_p2_{}.json", std::process::id());
    fs::write(&policy_2_filename, r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#)
        .unwrap();
    let second = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema: None,
            rotation_policy: None,
            fdae_policy: Some(DocumentSource::Path(policy_2_filename.clone())),
            health_check: None,
            assets: None,
            visibility: None,
        },
        service_type: WitServiceType::Tcp(TcpManifest {
            endpoints: vec![NetworkEndpoint {
                interface_name: "fails-to-register".to_string(),
                host: "127.0.0.1".to_string(),
                port: 9001,
            }],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let result = service.deploy("tcp_rollback_svc".to_string(), second, &caller).await;
    let _ = fs::remove_file(&policy_2_filename);
    assert!(result.is_err(), "TCP endpoint registration must fail: {result:?}");
    assert!(result.unwrap_err().contains("Endpoint registration failed"));

    assert_eq!(
        storage_provider.load_fdae_policy("tcp_rollback_svc").await.unwrap(),
        Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string()),
        "a failed TCP redeploy must restore the previous policy, not leave the new one (P2) in \
         force"
    );
    let (gen_after, _) =
        storage_provider.get_latest_config_generation("tcp_rollback_svc").await.unwrap().unwrap();
    assert_eq!(
        gen_after, gen_before,
        "a failed TCP redeploy must roll back the config generation this attempt saved"
    );
}

#[tokio::test]
async fn test_deploy_fdae_policy_schema_invalid_rejected_and_not_persisted() {
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

    let policy_filename = format!("test_fdae_bad_policy_{}.json", std::process::id());
    // Missing required "definitions" key -- fails JSON-Schema validation.
    fs::write(&policy_filename, r#"{"version": "fdae/v1"}"#).unwrap();

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
        .deploy("fdae_bad_service".to_string(), manifest, &node_wide_caller("test-caller"))
        .await;

    let _ = fs::remove_file(&policy_filename);

    assert!(result.is_err());
    assert!(result.unwrap_err().contains("FDAE policy validation failed"));
    assert_eq!(
        storage_provider.load_fdae_policy("fdae_bad_service").await.unwrap(),
        None,
        "an invalid policy must never reach fdae_policies"
    );
}

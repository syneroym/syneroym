use std::{fs, sync::Arc};

use dashmap::DashMap;
use syneroym_core::{
    config::SubstrateConfig,
    local_registry::EndpointRegistry,
    storage::MockStorage,
    test_constants::{GREETER_INTERFACE_NAME, greeter_wasm_path},
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::NativeDispatchRegistry;
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    HttpProbe as WitHttpProbe, PlannedService, RpcProbe as WitRpcProbe, ServiceConfig,
    TcpProbe as WitTcpProbe,
};

use super::{super::*, helpers::*};
use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

#[tokio::test]
async fn test_deploy_plan_path_traversal() {
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

    // Create a deployment plan with path traversal in source
    let plan = DeploymentPlan {
        app_instance_id: "test-instance".to_string(),
        blueprint_id: "test-blueprint".to_string(),
        version: "0.1.0".to_string(),
        services: vec![PlannedService {
            service_id: "did:key:test".to_string(),
            logical_ref: "test/main".to_string(),
            manifest: DeployManifest {
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
                    source: ArtifactSource::Url("../../../../../etc/passwd".to_string()),
                    hash: None,
                    interfaces: vec![],
                }),
                registry_certificate: None,
                instance_certificate: None,
            },
            app_context: None,
        }],
    };

    let result = service.deploy_plan(plan, &CallerContext::service_system("test-caller")).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Arbitrary file read prevented: Path traversal"));
}

#[tokio::test]
async fn test_deploy_plan_absolute_path() {
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

    let plan = DeploymentPlan {
        app_instance_id: "test-instance".to_string(),
        blueprint_id: "test-blueprint".to_string(),
        version: "0.1.0".to_string(),
        services: vec![PlannedService {
            service_id: "did:key:test".to_string(),
            logical_ref: "test/main".to_string(),
            manifest: DeployManifest {
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
                    source: ArtifactSource::Url("/etc/passwd".to_string()),
                    hash: None,
                    interfaces: vec![],
                }),
                registry_certificate: None,
                instance_certificate: None,
            },
            app_context: None,
        }],
    };

    let result = service.deploy_plan(plan, &CallerContext::service_system("test-caller")).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("Arbitrary file read prevented: Path traversal or absolute paths")
    );
}

#[tokio::test]
async fn test_deploy_config_schema_rejection() {
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

    // Write a schema file with a relative path
    let schema_filename = format!("test_schema_{}.json", std::process::id());
    fs::write(
        &schema_filename,
        r#"{"type": "object", "properties": {"port": {"type": "integer"}}}"#,
    )
    .unwrap();

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: Some(r#"{"port": "8080"}"#.to_string()), // string instead of int
            quota: None,
            schema: Some(DocumentSource::Path(schema_filename.clone())),
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

    let result = service
        .deploy("test_service".to_string(), manifest, &node_wide_caller("test-caller"))
        .await;

    let _ = fs::remove_file(&schema_filename);

    assert!(result.is_err());
    let err_msg = result.unwrap_err();
    assert!(err_msg.contains("Configuration validation failed"), "{}", err_msg);
}

#[cfg(unix)]
#[tokio::test]
async fn test_deploy_schema_symlink_escape_rejected() {
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

    // A symlink under the working directory whose target lives outside
    // it. No `..` component and not absolute, so the component check
    // alone would let it through; only canonicalizing the resolved path
    // catches it.
    let outside_dir = tempfile::tempdir().unwrap();
    let outside_schema = outside_dir.path().join("schema.json");
    fs::write(&outside_schema, r#"{"type": "object"}"#).unwrap();

    let symlink_name = format!("test_schema_symlink_{}.json", std::process::id());
    std::os::unix::fs::symlink(&outside_schema, &symlink_name).unwrap();

    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: Some(r#"{"port": 8080}"#.to_string()),
            quota: None,
            schema: Some(DocumentSource::Path(symlink_name.clone())),
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

    let result = service
        .deploy("symlink_schema_service".to_string(), manifest, &node_wide_caller("test-caller"))
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

#[tokio::test]
async fn test_deploy_config_generation_rollback() {
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

    // Deliberately malformed WasmManifest source to cause a deployment failure
    let manifest = DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: Some(r#"{"key": "value"}"#.to_string()),
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

    let result = service
        .deploy("rollback_service".to_string(), manifest, &node_wide_caller("test-caller"))
        .await;
    assert!(result.is_err()); // deployment must fail

    // Config generation should not exist
    let latest = storage_provider.get_latest_config_generation("rollback_service").await.unwrap();
    assert!(latest.is_none());
}

/// `instance_certificate`/`registry_certificate`
/// are minted fresh by `certify_placed_members` on every real apply
/// (a new signature, a `SystemTime::now()`-derived expiry), so the
/// test above -- which leaves both `None` on every call, like every
/// other idempotency test -- never exercised the actual supervisor/
/// `roymctl app deploy` path: hashing the whole manifest made those
/// two fields alone change the hash every time, epoch or no epoch,
/// so the no-op branch was unreachable from either real deploy path.
/// Two independently-issued, genuinely different-in-bytes certificates
/// for the *same* member -- like two real applies of the same desired
/// state -- must still dedup as a no-op: without this assertion, this
/// test passes against that bug.
#[tokio::test]
async fn an_identical_redeploy_with_freshly_minted_certificates_is_still_a_no_op() {
    let temp_dir = tempfile::tempdir().unwrap();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    let (service, _registry) =
        service_with_node_identity(temp_dir.path(), node_identity.clone()).await;
    let caller = node_wide_caller("did:key:zOwner");

    let member_master = syneroym_identity::Identity::generate().unwrap();
    let service_id = derive_did_key(&member_master.public_key());
    let derived = node_identity.derive_service_identity(&caller.caller_did, &service_id);

    // Different `expires_in_secs` guarantees different `expires_at_secs`
    // (and therefore a different signature) even if both calls land in
    // the same wall-clock second -- the churn E-1 is about, reproduced
    // deterministically rather than raced against real time.
    let cert_a = DelegationCertificate::issue(
        &member_master,
        derived.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    let cert_b = DelegationCertificate::issue(
        &member_master,
        derived.public_key(),
        7200,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    assert_ne!(
        cert_a.to_json().unwrap(),
        cert_b.to_json().unwrap(),
        "the two certificates must actually differ in bytes for this test to mean anything"
    );

    let ctx = app_context("app-1", "frontend", vec![]);
    let mut first = owner_test_manifest();
    first.instance_certificate = Some(cert_a.to_json().unwrap());
    let mut second = owner_test_manifest();
    second.instance_certificate = Some(cert_b.to_json().unwrap());

    service
        .deploy_with_context(
            service_id.clone(),
            first,
            Some(AppContext { generation: 1, ..ctx.clone() }),
            &caller,
        )
        .await
        .unwrap();
    let gen_before =
        service.storage_provider.get_latest_config_generation(&service_id).await.unwrap();

    service
        .deploy_with_context(
            service_id.clone(),
            second,
            Some(AppContext { generation: 2, ..ctx }),
            &caller,
        )
        .await
        .unwrap();
    let gen_after =
        service.storage_provider.get_latest_config_generation(&service_id).await.unwrap();
    assert_eq!(
        gen_before, gen_after,
        "a redeploy with identical content but a freshly re-issued certificate for the same \
         member must still be a no-op -- certificate freshness is not a content change"
    );
}

/// Row 10's boundary: an identical redeploy of a service the substrate
/// no longer considers running must still reinstall it -- `restart` is
/// the cheap path, `deploy` is the repair path.
#[tokio::test]
async fn an_identical_redeploy_of_a_stopped_service_still_reinstalls_it() {
    let wasm_bytes = fs::read(greeter_wasm_path())
        .expect("greeter fixture must be built (see test-components/greeter's own build step)");
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = node_wide_caller("owner");

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
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![GREETER_INTERFACE_NAME.to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    service.deploy("greeter-svc".to_string(), manifest.clone(), &owner).await.unwrap();
    let gen_after_first =
        service.storage_provider.get_latest_config_generation("greeter-svc").await.unwrap();

    // Simulate the instance stopping without an `undeploy` -- the
    // substrate still holds the deploy facts (and their
    // manifest_hash), but nothing is loaded any more.
    service.app_sandbox_engine.stop_wasm("greeter-svc").await.unwrap();

    service.deploy("greeter-svc".to_string(), manifest, &owner).await.unwrap();
    let gen_after_second =
        service.storage_provider.get_latest_config_generation("greeter-svc").await.unwrap();
    assert_ne!(
        gen_after_first, gen_after_second,
        "a redeploy of a stopped service must reinstall, not no-op"
    );
}

/// The three route tables (`native_dispatch`/`http_routes`/`assets`)
/// are process-local and empty on every boot, and the sandbox warm-up
/// restores only the WASM instance -- so a redeploy after a substrate
/// restart must run the full deploy to re-register them, even though
/// the persisted `manifest_hash` and owner match and the instance is
/// warm. A fresh `ControlPlaneService` over the same storage stands in
/// for the restarted process. Once it has run one full deploy, an
/// immediate identical redeploy *is* a no-op again.
#[tokio::test]
async fn a_redeploy_in_a_fresh_process_reinstalls_even_when_the_manifest_is_unchanged() {
    let temp_dir = tempfile::tempdir().unwrap();
    let caller = node_wide_caller("did:key:zAlice");
    let manifest = inline_manifest(None, None, None);
    let ctx = app_context(
        "app-1",
        "frontend",
        vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
    );

    // First process: one full deploy, then drop the service (and its
    // in-memory route tables and witness) as a process exit would.
    {
        let service = service_for_inline_tests(temp_dir.path()).await;
        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                manifest.clone(),
                Some(AppContext { generation: 1, ..ctx.clone() }),
                &caller,
            )
            .await
            .unwrap();
        assert!(
            service
                .storage_provider
                .get_latest_config_generation("frontend-svc")
                .await
                .unwrap()
                .is_some()
        );
    }

    // Second process over the same storage: identical manifest, same
    // owner, persisted `manifest_hash` matches -- but the route tables
    // are empty, so the dedup must not fire.
    let service = service_for_inline_tests(temp_dir.path()).await;
    let gen_before =
        service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap();
    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            manifest.clone(),
            Some(AppContext { generation: 2, ..ctx.clone() }),
            &caller,
        )
        .await
        .unwrap();
    let gen_after =
        service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap();
    assert_ne!(
        gen_before, gen_after,
        "a redeploy in a fresh process must run the full deploy to re-register the route tables"
    );

    // Now the witness is set: an identical redeploy in *this* process
    // is a no-op again.
    let gen_settled = gen_after;
    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            manifest,
            Some(AppContext { generation: 3, ..ctx }),
            &caller,
        )
        .await
        .unwrap();
    assert_eq!(
        gen_settled,
        service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap(),
        "once this process has done a full deploy, an identical redeploy is deduped"
    );
}

/// The dedup key hashes what a deploy *sends*, not what a later
/// `write-bindings` push installs, so a repair redeploy of
/// byte-identical content after a push must not match the stale hash
/// and take the no-op path -- that would leave the pushed bindings in
/// place under a deploy that reports success, defeating "restart is
/// the cheap path, deploy is the repair path".
#[tokio::test]
async fn a_redeploy_after_a_binding_push_reinstalls_the_manifests_own_bindings() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let caller = node_wide_caller("did:key:zAlice");
    let manifest = inline_manifest(None, None, None);
    let ctx = app_context(
        "app-1",
        "frontend",
        vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
    );

    service
        .deploy_with_context(
            "frontend-svc".to_string(),
            manifest.clone(),
            Some(ctx.clone()),
            &caller,
        )
        .await
        .unwrap();

    // A push moves the installed binding to a different target at a
    // higher epoch, as a supervisor's `write-bindings` would.
    service
        .write_bindings(
            BindingWrite {
                service_id: "frontend-svc".to_string(),
                app_instance_id: "app-1".to_string(),
                bindings: vec![DependencyBinding {
                    epoch: 1,
                    members: vec!["did:key:zPushedMember".to_string()],
                    ..dependency_binding("backend", vec!["did:key:zPushedMember"])
                }],
                generation: 0,
            },
            &caller,
        )
        .await
        .unwrap();
    let pushed = service.registry.binding_of("frontend-svc", "backend").await.unwrap().unwrap();
    assert!(pushed.contains("zPushedMember"), "{pushed}");

    // An operator, unaware of the push, redeploys the identical
    // manifest and context to repair the app -- this must reinstall
    // the manifest's own bindings, not no-op against the pushed state.
    service
        .deploy_with_context("frontend-svc".to_string(), manifest, Some(ctx), &caller)
        .await
        .unwrap();
    let repaired = service.registry.binding_of("frontend-svc", "backend").await.unwrap().unwrap();
    assert!(
        repaired.contains("zBackendMember") && !repaired.contains("zPushedMember"),
        "a repair redeploy after a push must reinstall the manifest's own bindings: {repaired}"
    );
}

/// The dedup check's own regression guard: the idempotency case is
/// "the same caller retrying a lost response", not "any caller sending
/// identical bytes". A *different*, authorized caller presenting
/// byte-identical content must still take ownership -- `set_owner` runs
/// unconditionally on every successful deploy -- rather
/// than being silently skipped by the dedup no-op.
#[tokio::test]
async fn an_identical_redeploy_by_a_different_caller_still_transfers_ownership() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let alice = node_wide_caller("did:key:zAlice");
    let bob = node_wide_caller("did:key:zBob");

    service
        .deploy("shared-svc".to_string(), inline_manifest(None, None, None), &alice)
        .await
        .unwrap();
    assert_eq!(service.registry.owner_of("shared-svc"), Some("did:key:zAlice".to_string()));

    service
        .deploy("shared-svc".to_string(), inline_manifest(None, None, None), &bob)
        .await
        .unwrap();
    assert_eq!(
        service.registry.owner_of("shared-svc"),
        Some("did:key:zBob".to_string()),
        "a different caller's byte-identical redeploy must still transfer ownership, not be \
         deduplicated as a no-op retry"
    );
}

/// The hash is written only on full deploy success -- a half-failed
/// deploy must not be deduplicated on the next attempt.
#[tokio::test]
async fn a_half_failed_deploy_does_not_record_a_manifest_hash() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = node_wide_caller("owner");

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
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Url("/does_not_exist.wasm".to_string()),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let result = service.deploy("half-failed-svc".to_string(), manifest, &owner).await;
    assert!(result.is_err());

    assert!(
        service.registry.deploy_facts("half-failed-svc").is_none(),
        "a deploy that fails before set_deploy_facts must not have recorded any deploy facts, \
         manifest_hash included"
    );
}

/// The management stamp records *who is writing*,
/// not what was installed, so it must survive a deploy that fails
/// after the generation gate -- unlike the bindings, it is not behind
/// the defer-until-everything-succeeds rule.
#[tokio::test]
async fn a_deploy_that_fails_after_the_gate_still_recorded_its_writer() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = node_wide_caller("owner");

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
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Url("/does_not_exist.wasm".to_string()),
            hash: None,
            interfaces: vec![],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    let ctx = app_context("app-1", "frontend", vec![]);
    let result = service
        .deploy_with_context(
            "half-failed-svc".to_string(),
            manifest,
            Some(AppContext { generation: 3, ..ctx }),
            &owner,
        )
        .await;
    assert!(result.is_err());

    let management = service
        .registry
        .app_instance_management_of("app-1")
        .expect("the generation gate's persist must survive a later deploy failure");
    assert_eq!(management.generation, 3);
}

/// The remediation half of restart-in-place: evicting and recompiling
/// a wasm component from the artifact the substrate already holds,
/// with no redeploy and no identity work.
#[tokio::test]
async fn restart_reloads_a_wasm_component_from_disk() {
    let wasm_bytes = fs::read(greeter_wasm_path())
        .expect("greeter fixture must be built (see test-components/greeter's own build step)");
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;
    let owner = node_wide_caller("owner");

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
        service_type: WitServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(wasm_bytes),
            hash: None,
            interfaces: vec![GREETER_INTERFACE_NAME.to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    };
    service.deploy("greeter-restart-svc".to_string(), manifest, &owner).await.unwrap();
    assert!(service.app_sandbox_engine.is_deployed("greeter-restart-svc"));

    service.app_sandbox_engine.stop_wasm("greeter-restart-svc").await.unwrap();
    assert!(!service.app_sandbox_engine.is_deployed("greeter-restart-svc"));

    service.restart("greeter-restart-svc".to_string(), 0, &owner).await.unwrap();
    assert!(
        service.app_sandbox_engine.is_deployed("greeter-restart-svc"),
        "restart must recompile the component from the artifact on disk"
    );
}

#[tokio::test]
async fn test_deploy_inline_schema_rejects_violating_config() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let schema = DocumentSource::Inline(
        r#"{"type":"object","properties":{"port":{"type":"integer"}}}"#.to_string(),
    );
    let err = service
        .deploy(
            "inline_schema_bad".to_string(),
            // A string where the schema demands an integer.
            inline_manifest(Some(r#"{"port": "8080"}"#), Some(schema), None),
            &node_wide_caller("test-caller"),
        )
        .await
        .unwrap_err();

    assert!(err.contains("Configuration validation failed"), "{err}");
}

#[tokio::test]
async fn test_deploy_inline_fdae_policy_without_a_staged_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let policy = DocumentSource::Inline(r#"{"version":"fdae/v1","definitions":{}}"#.to_string());
    let result = service
        .deploy(
            "inline_policy_ok".to_string(),
            inline_manifest(None, None, Some(policy)),
            &node_wide_caller("test-caller"),
        )
        .await;

    assert!(result.is_ok(), "{:?}", result.unwrap_err());
}

/// An inline policy is caller-supplied, so the rule that a policy
/// validation error never echoes the offending document back matters more
/// here than it did for a host-side file.
#[tokio::test]
async fn test_deploy_inline_fdae_policy_error_does_not_echo_the_document() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let secret = "s3cret-marker-in-policy";
    let policy = DocumentSource::Inline(format!(
        r#"{{"version":"fdae/v1","definitions":{{"{secret}":"not-an-object"}}}}"#
    ));
    let err = service
        .deploy(
            "inline_policy_bad".to_string(),
            inline_manifest(None, None, Some(policy)),
            &node_wide_caller("test-caller"),
        )
        .await
        .unwrap_err();

    assert!(err.contains("FDAE policy validation failed"), "{err}");
    assert!(!err.contains(secret), "policy content leaked to the caller: {err}");
}

#[tokio::test]
async fn test_deploy_rejects_oversize_inline_document() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let oversize = DocumentSource::Inline(
        "x".repeat(syneroym_core::deploy_docs::MAX_DEPLOY_DOCUMENT_BYTES as usize + 1),
    );
    let err = service
        .deploy(
            "oversize_schema".to_string(),
            inline_manifest(Some(r#"{"port": 8080}"#), Some(oversize), None),
            &node_wide_caller("test-caller"),
        )
        .await
        .unwrap_err();

    assert!(err.contains("exceeding the"), "{err}");
}

#[tokio::test]
async fn a_probe_kind_that_cannot_address_the_service_type_is_rejected_at_deploy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    // `rpc` on a container.
    let rpc_on_container = container_manifest_with(Some(WitHealthCheck::Rpc(WitRpcProbe {
        interface_name: "main".to_string(),
        method: "ping".to_string(),
        timeout_ms: 1000,
    })));
    let err = service
        .deploy("rpc-on-container".to_string(), rpc_on_container, &node_wide_caller("owner"))
        .await
        .unwrap_err();
    assert!(err.contains("cannot address"), "{err}");

    // `http-get` on wasm.
    let http_on_wasm = wasm_manifest_with(Some(WitHealthCheck::HttpGet(WitHttpProbe {
        interface_name: "main".to_string(),
        path: "/healthz".to_string(),
        expect_status: 200,
        timeout_ms: 1000,
    })));
    let err = service
        .deploy("http-on-wasm".to_string(), http_on_wasm, &node_wide_caller("owner"))
        .await
        .unwrap_err();
    assert!(err.contains("cannot address"), "{err}");
}

#[tokio::test]
async fn an_http_probe_path_that_does_not_start_with_a_slash_is_rejected_at_deploy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let manifest = tcp_manifest_with(
        9,
        Some(WitHealthCheck::HttpGet(WitHttpProbe {
            interface_name: "main".to_string(),
            path: "healthz".to_string(),
            expect_status: 200,
            timeout_ms: 1000,
        })),
    );
    let err = service
        .deploy("bad-path".to_string(), manifest, &node_wide_caller("owner"))
        .await
        .unwrap_err();
    assert!(err.contains("must start with '/'"), "{err}");
}

#[tokio::test]
async fn a_deploy_records_its_service_type_and_health_check_and_undeploy_removes_them() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let manifest = tcp_manifest_with(
        9,
        Some(WitHealthCheck::TcpConnect(WitTcpProbe {
            interface_name: "main".to_string(),
            timeout_ms: 1000,
        })),
    );
    service.deploy("facts-svc".to_string(), manifest, &node_wide_caller("owner")).await.unwrap();

    let (service_type, check_json, ..) = service.registry.deploy_facts("facts-svc").unwrap();
    assert_eq!(service_type, "tcp");
    // Stored as the wire variant's own JSON, not the app model's
    // kebab-case one -- `run_probe` deserializes back into the same
    // wire type it reads here, so the two must agree on shape.
    let stored: WitHealthCheck = serde_json::from_str(&check_json.unwrap()).unwrap();
    assert!(matches!(stored, WitHealthCheck::TcpConnect(_)), "{stored:?}");

    service.undeploy("facts-svc".to_string(), 0, &node_wide_caller("owner")).await.unwrap();
    assert!(service.registry.deploy_facts("facts-svc").is_none());
}

#[tokio::test]
async fn a_redeploy_without_a_health_check_clears_the_stored_one() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let with_check = tcp_manifest_with(
        9,
        Some(WitHealthCheck::TcpConnect(WitTcpProbe {
            interface_name: "main".to_string(),
            timeout_ms: 1000,
        })),
    );
    service
        .deploy("redeploy-svc".to_string(), with_check, &node_wide_caller("owner"))
        .await
        .unwrap();
    assert!(service.registry.deploy_facts("redeploy-svc").unwrap().1.is_some());

    let without_check = tcp_manifest_with(9, None);
    service
        .deploy("redeploy-svc".to_string(), without_check, &node_wide_caller("owner"))
        .await
        .unwrap();
    let (service_type, check_json, ..) = service.registry.deploy_facts("redeploy-svc").unwrap();
    assert_eq!(service_type, "tcp");
    assert!(check_json.is_none());
}

#[tokio::test]
async fn a_container_and_a_tcp_service_are_distinguished_by_the_recorded_type() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    // Both register the identical `TcpHostPort` endpoint variant,
    // so the distinction must come from the recorded fact, not the
    // endpoint.
    service
        .registry
        .register(
            "container-svc".to_string(),
            "main".to_string(),
            SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
        )
        .await
        .unwrap();
    service
        .registry
        .set_deploy_facts("container-svc".to_string(), "container".to_string(), None, None, None)
        .await
        .unwrap();
    service
        .registry
        .register(
            "tcp-svc".to_string(),
            "main".to_string(),
            SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
        )
        .await
        .unwrap();
    service
        .registry
        .set_deploy_facts("tcp-svc".to_string(), "tcp".to_string(), None, None, None)
        .await
        .unwrap();

    let container_phase = service.instance_phase("container-svc", Some("container")).await;
    let tcp_phase = service.instance_phase("tcp-svc", Some("tcp")).await;

    assert!(
        matches!(container_phase, InstancePhase::NotRunning(_)),
        "expected NotRunning, got {container_phase:?}"
    );
    assert!(matches!(tcp_phase, InstancePhase::Unknown(_)), "expected Unknown, got {tcp_phase:?}");
}

#[test]
fn validate_publication_public_with_matching_record_succeeds() {
    let record = test_signed_record("test-svc", false);
    let res = validate_publication("test-svc", Some(WitVisibility::Public), Some(&record));
    assert_eq!(res, Ok(AppVisibility::Public));
}

#[test]
fn validate_publication_public_without_certificate_fails() {
    let res = validate_publication("test-svc", Some(WitVisibility::Public), None);
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("no registry certificate was supplied"));
}

#[test]
fn validate_publication_public_with_mismatched_service_id_fails() {
    let record = test_signed_record("other-svc", false);
    let res = validate_publication("test-svc", Some(WitVisibility::Public), Some(&record));
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("names service 'other-svc'"));
}

#[test]
fn validate_publication_public_with_is_private_true_fails() {
    let record = test_signed_record("test-svc", true);
    let res = validate_publication("test-svc", Some(WitVisibility::Public), Some(&record));
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("is_private=true"));
}

#[test]
fn validate_publication_internal_with_matching_record_succeeds() {
    let record = test_signed_record("test-svc", true);
    let res = validate_publication("test-svc", Some(WitVisibility::Internal), Some(&record));
    assert_eq!(res, Ok(AppVisibility::Internal));
}

#[test]
fn validate_publication_internal_without_certificate_fails() {
    let res = validate_publication("test-svc", Some(WitVisibility::Internal), None);
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("no registry certificate was supplied"));
}

#[test]
fn validate_publication_internal_with_is_private_false_fails() {
    let record = test_signed_record("test-svc", false);
    let res = validate_publication("test-svc", Some(WitVisibility::Internal), Some(&record));
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("is_private=false"));
}

#[test]
fn validate_publication_private_without_certificate_succeeds() {
    let res1 = validate_publication("test-svc", Some(WitVisibility::Private), None);
    assert_eq!(res1, Ok(AppVisibility::Private));
    let res2 = validate_publication("test-svc", None, None);
    assert_eq!(res2, Ok(AppVisibility::Private));
}

#[test]
fn validate_publication_private_with_certificate_fails() {
    let record = test_signed_record("test-svc", false);
    let res = validate_publication("test-svc", Some(WitVisibility::Private), Some(&record));
    assert!(res.is_err());
    assert!(
        res.unwrap_err()
            .contains("declares visibility 'private' but a registry certificate was supplied")
    );
}

#[test]
fn validate_publication_malformed_certificate_fails() {
    let res = validate_publication("test-svc", Some(WitVisibility::Public), Some("not valid json"));
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("does not parse"));
}

/// The guard both `deploy_with_context` and `undeploy_impl` call before
/// joining `service_id` into a stored-record filename. A real DID never
/// trips any of the rejected shapes.
#[test]
fn is_safe_service_id_for_path_rejects_traversal_and_admits_a_real_did() {
    assert!(!is_safe_service_id_for_path(""));
    assert!(!is_safe_service_id_for_path("../escaped"));
    assert!(!is_safe_service_id_for_path("a/b"));
    assert!(!is_safe_service_id_for_path("a\\b"));
    assert!(is_safe_service_id_for_path("did:key:z6MkExample"));
}

/// A public service redeployed as `private` clears
/// its stored record file -- otherwise the substrate keeps republishing
/// the old record on every heartbeat sweep for up to `not_after`.
#[tokio::test]
async fn a_private_redeploy_removes_the_stored_endpoint_record_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = service_for_inline_tests(temp_dir.path()).await;

    let mut public_manifest = tcp_manifest_with(9, None);
    public_manifest.config.visibility = Some(WitVisibility::Public);
    public_manifest.registry_certificate = Some(test_signed_record("stale-record-svc", false));
    service
        .deploy("stale-record-svc".to_string(), public_manifest, &node_wide_caller("owner"))
        .await
        .unwrap();

    let cert_path = temp_dir.path().join("stale-record-svc.json");
    assert!(cert_path.exists(), "the record file must exist after a public deploy");

    let mut private_manifest = tcp_manifest_with(9, None);
    private_manifest.config.visibility = Some(WitVisibility::Private);
    service
        .deploy("stale-record-svc".to_string(), private_manifest, &node_wide_caller("owner"))
        .await
        .unwrap();

    assert!(
        !cert_path.exists(),
        "redeploying as private must remove the stale record file, or the heartbeat sweep keeps \
         republishing it"
    );
}

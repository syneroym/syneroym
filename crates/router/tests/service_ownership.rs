#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! M04A Slice B7a: substrate & service ownership attribution. Drives
//! `ControlPlaneService::dispatch` (the public `NativeService` trait --
//! `deploy`/`undeploy`/`list` themselves are behind a crate-private trait,
//! matching `native_dispatch_identity.rs`'s dispatch-level style) to prove:
//!
//! 1. A caller holding node-wide orchestrator authority (a real substrate owner
//!    -- a verified `ControllerAgreement` controller) sees every deployed app
//!    regardless of who deployed it.
//! 2. An ordinary caller with no node-wide capability sees only the apps they
//!    themselves deployed; an app deployed by someone else -- or deployed
//!    before B7a and therefore unattributed -- is hidden.
//! 3. `deploy` rejects a redeploy from a different DID than the recorded owner
//!    unless the caller holds node-wide authority (F7); `undeploy` rejects a
//!    non-owner, non-node-wide caller the same way (§2.3).
//!
//! `build_caller` (`crates/router/src/route_handler/io.rs`) is what actually
//! issues that grant on a real connection; its own unit tests cover that
//! wiring. This file constructs `CallerContext`s by hand (as
//! `native_dispatch_identity.rs` does) to exercise `ControlPlaneService`'s
//! own ownership logic directly, independent of the router.

use std::sync::Arc;

use dashmap::DashMap;
use serde_json::{Value, json};
use syneroym_control_plane::{
    ControlPlaneService,
    dummy_sandbox::{AppSandboxEngine, ContainerEngine},
};
use syneroym_core::{
    config::SubstrateConfig,
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::{AppInstanceManagement, EndpointStorage, MockStorage},
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{
    Ability, AuthLevel, CallerContext, Capability, NativeDispatchRegistry, NativeInvocation,
    NativeResponse, NativeService, ResourceUri, RpcResult, SessionContext,
};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    DeployManifest, DeployedService, NetworkEndpoint, ServiceConfig, ServiceType, TcpManifest,
};

const NODE_DID: &str = "did:key:zNodeUnderTest";

/// An ordinary verified caller with no capabilities at all -- used where a
/// caller must be rejected outright (no admission grant, no ownership) or
/// where only `list`'s ownership-filter behavior is under test (`list` is
/// not gated on any ability -- §2.4/§3.2). **Not** used for `deploy`/
/// `undeploy` setup calls any more: M04A Slice B7b adds a Tier-1 admission
/// gate requiring an explicit `orchestrator/{deploy,undeploy}` capability
/// (`orchestration.rs`'s new checks), which this caller by construction
/// never holds -- see `app_grantee` for that.
fn plain_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext::default(),
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// M04A Slice B7b: a caller holding an app-scoped `orchestrator/{deploy,
/// undeploy}` grant for exactly `service_id` -- the shape a real B7b deploy
/// grant produces (§3.2's `substrate:<node>/app/<name>` selector), as
/// opposed to `node_wide_caller`'s bare, node-wide form. Used for every
/// `deploy`/`undeploy` setup call in this file so B7b's new admission gate
/// does not mask what these tests actually exercise (ownership/list
/// filtering, not admission) -- `plain_caller` alone no longer clears that
/// gate.
fn app_grantee(did: &str, service_id: &str) -> CallerContext {
    let resource = ResourceUri(format!("substrate:{NODE_DID}/app/{service_id}"));
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: did.to_string(),
            capabilities: vec![
                Capability {
                    with: resource.clone(),
                    can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
                    caveats: None,
                },
                Capability {
                    with: resource,
                    can: Ability(Ability::ORCHESTRATOR_UNDEPLOY.to_string()),
                    caveats: None,
                },
            ],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// A caller holding node-wide orchestrator authority on `NODE_DID` -- what a
/// real substrate owner's `substrate/admin` satisfies via `Ability::entails`'s
/// short-circuit (`build_caller` issues that shape for a verified
/// `ControllerAgreement` controller). Holds all three
/// `orchestrator/*` abilities together, so it passes
/// `ControlPlaneService::has_node_wide_ability` regardless of which specific
/// ability a given call site checks. Used here to drive that predicate
/// directly without going through the router.
fn node_wide_caller(did: &str) -> CallerContext {
    let resource = ResourceUri::substrate(NODE_DID);
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: did.to_string(),
            capabilities: vec![
                Capability {
                    with: resource.clone(),
                    can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
                    caveats: None,
                },
                Capability {
                    with: resource.clone(),
                    can: Ability(Ability::ORCHESTRATOR_UNDEPLOY.to_string()),
                    caveats: None,
                },
                Capability {
                    with: resource,
                    can: Ability(Ability::ORCHESTRATOR_STATUS.to_string()),
                    caveats: None,
                },
            ],
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// A storage decorator used only by
/// `failed_remove_owner_blocks_a_different_callers_later_redeploy` (CC2): it
/// delegates everything to an inner `MockStorage` except `remove_owner`,
/// which always fails, simulating a storage-layer error during undeploy's
/// best-effort owner cleanup.
struct RemoveOwnerFailingStorage {
    inner: MockStorage,
}

#[async_trait::async_trait]
impl EndpointStorage for RemoveOwnerFailingStorage {
    async fn load_all(&self) -> anyhow::Result<Vec<(String, String, SubstrateEndpoint)>> {
        self.inner.load_all().await
    }
    async fn save(
        &self,
        service_id: &str,
        interface_name: &str,
        endpoint: &SubstrateEndpoint,
    ) -> anyhow::Result<()> {
        self.inner.save(service_id, interface_name, endpoint).await
    }
    async fn remove(&self, service_id: &str, interface_name: &str) -> anyhow::Result<()> {
        self.inner.remove(service_id, interface_name).await
    }
    async fn load_all_owners(&self) -> anyhow::Result<Vec<(String, String)>> {
        self.inner.load_all_owners().await
    }
    async fn save_owner(&self, service_id: &str, owner_did: &str) -> anyhow::Result<()> {
        self.inner.save_owner(service_id, owner_did).await
    }
    async fn remove_owner(&self, _service_id: &str) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("simulated storage failure removing owner"))
    }
    async fn load_all_certs(&self) -> anyhow::Result<Vec<(String, String)>> {
        self.inner.load_all_certs().await
    }
    async fn save_cert(&self, service_id: &str, certificate_json: &str) -> anyhow::Result<()> {
        self.inner.save_cert(service_id, certificate_json).await
    }
    async fn remove_cert(&self, service_id: &str) -> anyhow::Result<()> {
        self.inner.remove_cert(service_id).await
    }
    async fn load_all_deploy_facts(
        &self,
    ) -> anyhow::Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>> {
        self.inner.load_all_deploy_facts().await
    }
    async fn save_deploy_facts(
        &self,
        service_id: &str,
        service_type: &str,
        health_check_json: Option<&str>,
        manifest_hash: Option<&str>,
        visibility: Option<&str>,
    ) -> anyhow::Result<()> {
        self.inner
            .save_deploy_facts(
                service_id,
                service_type,
                health_check_json,
                manifest_hash,
                visibility,
            )
            .await
    }
    async fn remove_deploy_facts(&self, service_id: &str) -> anyhow::Result<()> {
        self.inner.remove_deploy_facts(service_id).await
    }
    async fn load_all_app_contexts(&self) -> anyhow::Result<Vec<(String, String, String)>> {
        self.inner.load_all_app_contexts().await
    }
    async fn save_app_context(
        &self,
        service_id: &str,
        app_instance_id: &str,
        service_name: &str,
    ) -> anyhow::Result<()> {
        self.inner.save_app_context(service_id, app_instance_id, service_name).await
    }
    async fn remove_app_context(&self, service_id: &str) -> anyhow::Result<()> {
        self.inner.remove_app_context(service_id).await
    }
    async fn load_all_bindings(&self) -> anyhow::Result<Vec<(String, String, String, String)>> {
        self.inner.load_all_bindings().await
    }
    async fn save_binding(
        &self,
        service_id: &str,
        app_instance_id: &str,
        dependency_name: &str,
        topology_entry_json: &str,
    ) -> anyhow::Result<()> {
        self.inner
            .save_binding(service_id, app_instance_id, dependency_name, topology_entry_json)
            .await
    }
    async fn load_binding(
        &self,
        service_id: &str,
        dependency_name: &str,
    ) -> anyhow::Result<Option<String>> {
        self.inner.load_binding(service_id, dependency_name).await
    }
    async fn load_bindings_for(&self, service_id: &str) -> anyhow::Result<Vec<(String, String)>> {
        self.inner.load_bindings_for(service_id).await
    }
    async fn load_all_app_instance_management(
        &self,
    ) -> anyhow::Result<Vec<(String, AppInstanceManagement)>> {
        self.inner.load_all_app_instance_management().await
    }
    async fn save_app_instance_management(
        &self,
        app_instance_id: &str,
        management: &AppInstanceManagement,
    ) -> anyhow::Result<()> {
        self.inner.save_app_instance_management(app_instance_id, management).await
    }
    async fn remove_app_instance_management(&self, app_instance_id: &str) -> anyhow::Result<()> {
        self.inner.remove_app_instance_management(app_instance_id).await
    }
}

/// A TCP manifest with one real endpoint -- `list()` only surfaces a
/// service that has at least one *non*-native-capability interface
/// registered (the native data-layer/vault/etc. channels every deploy also
/// gets are filtered out), so an empty `endpoints: vec![]` manifest would
/// silently make every service invisible to `list` regardless of ownership.
fn tcp_manifest(port: u16) -> DeployManifest {
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
        service_type: ServiceType::Tcp(TcpManifest {
            endpoints: vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port,
            }],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

async fn test_service(temp_dir: &std::path::Path) -> ControlPlaneService {
    test_service_with_registry(temp_dir, EndpointRegistry::new_mock(Arc::new(MockStorage::new())))
        .await
}

async fn test_service_with_registry(
    temp_dir: &std::path::Path,
    registry: EndpointRegistry,
) -> ControlPlaneService {
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir, false).unwrap());
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
            registry.clone(),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let container_engine = Arc::new(ContainerEngine::new("podman".to_string(), temp_dir, None));
    let native_dispatch = NativeDispatchRegistry::default();
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());

    ControlPlaneService::init(
        "orchestrator".to_string(),
        NODE_DID.to_string(),
        app_sandbox,
        container_engine,
        registry,
        temp_dir.join("hosted_apps"),
        key_store,
        storage_provider,
        blob_provider,
        messaging_broker,
        native_dispatch,
        http_routes,
        Arc::new(DashMap::new()),
        Arc::new(syneroym_identity::Identity::generate().unwrap()),
        syneroym_app_orchestration::empty_resolver(),
    )
    .await
    .unwrap()
}

async fn deploy(service: &ControlPlaneService, service_id: &str, caller: &CallerContext) {
    deploy_result(service, service_id, caller).await.unwrap();
}

async fn deploy_result(
    service: &ControlPlaneService,
    service_id: &str,
    caller: &CallerContext,
) -> RpcResult<NativeResponse> {
    let invocation = NativeInvocation {
        interface: "orchestrator".to_string(),
        method: "deploy".to_string(),
        params: json!((service_id, tcp_manifest(9000))),
        caller: caller.clone(),
    };
    service.dispatch(invocation).await
}

async fn undeploy_result(
    service: &ControlPlaneService,
    service_id: &str,
    caller: &CallerContext,
) -> RpcResult<NativeResponse> {
    let invocation = NativeInvocation {
        interface: "orchestrator".to_string(),
        method: "undeploy".to_string(),
        params: json!((service_id, 0u64)),
        caller: caller.clone(),
    };
    service.dispatch(invocation).await
}

async fn list(service: &ControlPlaneService, caller: &CallerContext) -> Vec<DeployedService> {
    let invocation = NativeInvocation {
        interface: "orchestrator".to_string(),
        method: "list".to_string(),
        params: Value::Null,
        caller: caller.clone(),
    };
    let response = service.dispatch(invocation).await.unwrap();
    serde_json::from_value(response.payload).unwrap()
}

/// A caller holding node-wide orchestrator authority (a real substrate
/// owner) sees every deployed app, regardless of who deployed it --
/// long-standing behavior, preserved and now asserted directly rather
/// than assumed.
#[tokio::test]
async fn node_wide_authority_lists_every_app() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let alice = app_grantee("did:key:zAlice", "svc-a");
    let bob = app_grantee("did:key:zBob", "svc-b");
    let onlooker = node_wide_caller("did:key:zOnlooker");

    deploy(&service, "svc-a", &alice).await;
    deploy(&service, "svc-b", &bob).await;

    let seen = list(&service, &onlooker).await;
    let mut ids: Vec<_> = seen.into_iter().map(|s| s.service_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["svc-a".to_string(), "svc-b".to_string()]);
}

/// The substrate owner (or any node-wide-authority caller) sees every app,
/// even ones they did not themselves deploy.
#[tokio::test]
async fn owned_substrate_owner_sees_every_app() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let alice = app_grantee("did:key:zAlice", "alice-svc");
    let bob = app_grantee("did:key:zBob", "bob-svc");
    let owner = node_wide_caller("did:key:zOwner");

    deploy(&service, "alice-svc", &alice).await;
    deploy(&service, "bob-svc", &bob).await;

    let owner_view = list(&service, &owner).await;
    let mut ids: Vec<_> = owner_view.into_iter().map(|s| s.service_id).collect();
    ids.sort();
    assert_eq!(ids, vec!["alice-svc".to_string(), "bob-svc".to_string()]);
}

/// An ordinary caller with no node-wide capability sees only their own
/// deployed apps.
#[tokio::test]
async fn owned_substrate_service_owner_sees_only_own_apps() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let alice = app_grantee("did:key:zAlice", "alice-svc");
    let bob = app_grantee("did:key:zBob", "bob-svc");

    deploy(&service, "alice-svc", &alice).await;
    deploy(&service, "bob-svc", &bob).await;

    let alice_view = list(&service, &alice).await;
    assert_eq!(alice_view.len(), 1);
    assert_eq!(alice_view[0].service_id, "alice-svc");

    let bob_view = list(&service, &bob).await;
    assert_eq!(bob_view.len(), 1);
    assert_eq!(bob_view[0].service_id, "bob-svc");
}

#[tokio::test]
async fn unattributed_app_is_hidden_from_non_owners() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let alice = app_grantee("did:key:zAlice", "alice-svc");
    let bob = plain_caller("did:key:zBob");

    deploy(&service, "alice-svc", &alice).await;
    // `bob` never deployed anything of his own -- must see nothing, not
    // alice's unrelated app.
    let bob_view = list(&service, &bob).await;
    assert!(bob_view.is_empty());
}

/// F7: redeploying an existing `service_id` from a different, non-node-wide
/// DID than its recorded owner is a hostile takeover and must fail closed.
#[tokio::test]
async fn redeploy_by_a_different_did_is_rejected() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let alice = app_grantee("did:key:zAlice", "contested-svc");
    let mallory = plain_caller("did:key:zMallory");

    deploy(&service, "contested-svc", &alice).await;

    let result = deploy_result(&service, "contested-svc", &mallory).await;
    assert!(result.is_err(), "redeploy from a non-owner DID must be rejected");

    // The original owner is unchanged.
    let alice_view = list(&service, &alice).await;
    assert_eq!(alice_view.len(), 1);
    assert_eq!(alice_view[0].service_id, "contested-svc");
}

#[tokio::test]
async fn undeploy_by_a_non_owner_is_rejected() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let alice = app_grantee("did:key:zAlice", "guarded-svc");
    let mallory = plain_caller("did:key:zMallory");

    deploy(&service, "guarded-svc", &alice).await;

    let result = undeploy_result(&service, "guarded-svc", &mallory).await;
    assert!(result.is_err(), "undeploy from a non-owner DID must be rejected");

    let alice_view = list(&service, &alice).await;
    assert_eq!(
        alice_view.len(),
        1,
        "the service must still be deployed after the rejected undeploy"
    );
}

/// Post-commit review: every existing gate test asserted only *rejection*.
/// This is the positive-path counterpart of `redeploy_by_a_different_did_is_
/// rejected` -- the legitimate owner redeploying their own service must
/// still succeed (the `existing != caller.caller_did` branch never fires for
/// them). An over-strict gate that locked out the real owner would pass
/// every previously-existing test in this file.
#[tokio::test]
async fn owner_can_redeploy_their_own_service() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let alice = app_grantee("did:key:zAlice", "alice-svc");

    deploy(&service, "alice-svc", &alice).await;
    let result = deploy_result(&service, "alice-svc", &alice).await;
    assert!(result.is_ok(), "the owner must be able to redeploy their own service: {result:?}");

    let alice_view = list(&service, &alice).await;
    assert_eq!(alice_view.len(), 1);
    assert_eq!(alice_view[0].service_id, "alice-svc");
}

/// Positive-path counterpart of the takeover-rejection tests: a caller
/// holding node-wide orchestrator authority (a real substrate owner) may
/// redeploy over -- and thereby take ownership of -- a service someone
/// else owns. Exercises the
/// `!has_node_wide_ability(caller, ORCHESTRATOR_DEPLOY)` branch at
/// `orchestration.rs`'s takeover check, which `redeploy_by_a_different_did_
/// is_rejected` cannot reach (its `mallory` caller holds no capabilities at
/// all).
#[tokio::test]
async fn node_wide_caller_can_redeploy_over_a_foreign_owner() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let alice = app_grantee("did:key:zAlice", "alice-svc");
    let owner = node_wide_caller("did:key:zOwner");

    deploy(&service, "alice-svc", &alice).await;
    let result = deploy_result(&service, "alice-svc", &owner).await;
    assert!(
        result.is_ok(),
        "a node-wide caller must be able to override a foreign owner: {result:?}"
    );

    // Redeploying reassigns ownership to the overriding caller -- `set_owner`
    // unconditionally records `caller.caller_did` on every successful
    // deploy, authorized or not.
    let owner_view = list(&service, &owner).await;
    assert_eq!(owner_view.len(), 1);
    assert_eq!(owner_view[0].service_id, "alice-svc");
    let alice_view = list(&service, &alice).await;
    assert!(alice_view.is_empty(), "ownership must have transferred away from alice");
}

/// Positive-path counterpart of `undeploy_by_a_non_owner_is_rejected`: a
/// node-wide caller may undeploy someone else's service. Exercises the
/// `!has_node_wide_ability(caller, ORCHESTRATOR_UNDEPLOY)` branch at
/// `orchestration.rs`'s undeploy gate.
#[tokio::test]
async fn node_wide_caller_can_undeploy_a_foreign_owners_service() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let alice = app_grantee("did:key:zAlice", "alice-svc");
    let admin = node_wide_caller("did:key:zAdmin");

    deploy(&service, "alice-svc", &alice).await;
    let result = undeploy_result(&service, "alice-svc", &admin).await;
    assert!(
        result.is_ok(),
        "a node-wide caller must be able to undeploy a foreign owner's service: {result:?}"
    );

    let admin_view = list(&service, &admin).await;
    assert!(admin_view.is_empty(), "the service must be gone after the authorized undeploy");
}

/// Post-commit review (CC2): `undeploy`'s `remove_owner` is best-effort
/// (warn-not-fail, matching every other teardown step). If the storage
/// write fails, the owner row survives a fully-undeployed service and blocks
/// a *different* caller's later redeploy of that `service_id` via the
/// takeover check -- "ID squatting". This bites an *ordinary*,
/// non-node-wide caller on any substrate a `ControllerAgreement` has
/// claimed or that already has an app-scoped grantee;
/// a node-wide caller overrides the stale row anyway (see
/// `node_wide_caller_can_redeploy_over_a_foreign_owner`) and so never hits
/// it. Pinned here, not fixed -- a real fix needs either a
/// retryable/idempotent teardown or a recovery path, both out of this
/// slice's scope.
#[tokio::test]
async fn failed_remove_owner_blocks_a_different_callers_later_redeploy() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RemoveOwnerFailingStorage { inner: MockStorage::new() });
    let registry = EndpointRegistry::new(storage).await.unwrap();
    let service = test_service_with_registry(temp_dir.path(), registry).await;
    let alice = app_grantee("did:key:zAlice", "squat-svc");
    let bob = plain_caller("did:key:zBob");

    deploy(&service, "squat-svc", &alice).await;
    let undeploy_outcome = undeploy_result(&service, "squat-svc", &alice).await;
    assert!(
        undeploy_outcome.is_ok(),
        "undeploy itself must still succeed despite remove_owner failing (warn-not-fail): \
         {undeploy_outcome:?}"
    );

    // The stale owner row still says alice, even though the service is
    // fully torn down -- bob's attempt to deploy the same service_id is
    // rejected by the takeover check.
    let result = deploy_result(&service, "squat-svc", &bob).await;
    assert!(
        result.is_err(),
        "documents the known limitation: a stale owner row from a failed remove_owner blocks a \
         different caller's redeploy of the same service_id"
    );
}

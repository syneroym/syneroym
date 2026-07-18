#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! M04A Slice B7b: the deploy grant itself (task.md item 1 + F3's
//! `orchestrator` half). `service_ownership.rs` (B7a) proves ownership
//! *attribution* and *visibility*; this file proves *admission* -- the
//! Tier-1 `orchestrator/{deploy,undeploy,status}` capability check
//! `ControlPlaneService::deploy`/`undeploy`/`readyz` now enforce (plan
//! §3.2, §2.4.1), independent of ownership:
//!
//! 1. A caller holding no grant at all is denied `deploy` outright, even for a
//!    brand-new `service_id` nobody owns yet (the takeover check alone would
//!    let them through; the new admission gate must not).
//! 2. An app-scoped grantee (`substrate:<node>/app/<name>`) may deploy their
//!    own app but not a different one (§3.2's table, rows 3-4).
//! 3. An app-scoped grantee does not thereby gain node-wide visibility on
//!    `list` -- the selector excludes them from `has_node_wide_ability` (§2.2's
//!    predicate).
//! 4. Per-service `readyz` is gated the same way (§2.4.1); the empty-
//!    `service_id` liveness ping stays open regardless (the `wait_for_ready`
//!    regression guard).
//!
//! ADR-0015 A6 (owner-rooted trust)/A7 (revocation)/A4 (`can_delegate`)
//! themselves are proven end to end through real signed `CapabilityToken`s
//! in `crates/router/src/route_handler/io.rs`'s own test module (where
//! `build_caller`'s `is_root` closure lives) -- this file drives
//! `ControlPlaneService` directly with hand-built `CallerContext`s, matching
//! `service_ownership.rs`'s style, since admission itself does not depend on
//! how the capability was verified. The two `..._real_signed_token_...` /
//! `..._real_token_...` tests near the end (post-commit review, F3) are the
//! exception: they join both halves -- a real signed, real-registry-owner-
//! rooted `CapabilityToken` verified with `syneroym_ucan::verify_chain` (the
//! same code `build_caller` calls), fed into this file's real
//! `ControlPlaneService::deploy` gate -- closing the gap task.md item 1's
//! "exercised end to end ... not just hand-built `CallerContext`s" claim
//! otherwise left half-proven.

use std::{
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;
use serde_json::{Value, json};
use syneroym_control_plane::{
    ControlPlaneService,
    dummy_sandbox::{AppSandboxEngine, ContainerEngine},
};
use syneroym_core::{
    config::SubstrateConfig, http_routes::HttpRouteRegistry, local_registry::EndpointRegistry,
    storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{
    Ability, AuthLevel, CallerContext, Capability, CapabilityToken, ChainVerifyOpts,
    NativeDispatchRegistry, NativeInvocation, NativeResponse, NativeService, ResourceUri,
    RpcResult, SessionContext,
};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    DeployManifest, DeployedService, NetworkEndpoint, ServiceConfig, ServiceType, TcpManifest,
};

const NODE_DID: &str = "did:key:zNodeUnderTest";

/// A verified caller with no capabilities at all -- must be denied every
/// admission-gated call.
fn plain_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext::default(),
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// An app-scoped grantee holding `orchestrator/{deploy,undeploy,status}` on
/// exactly `substrate:<NODE_DID>/app/<service_id>` -- the shape a real B7b
/// deploy grant produces (plan §3.2's grant JSON).
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

fn tcp_manifest(port: u16) -> DeployManifest {
    DeployManifest {
        config: ServiceConfig {
            env: vec![],
            args: vec![],
            custom_config: None,
            quota: None,
            schema_path: None,
            rotation_policy: None,
        },
        service_type: ServiceType::Tcp(TcpManifest {
            endpoints: vec![NetworkEndpoint {
                interface_name: "default".to_string(),
                host: "127.0.0.1".to_string(),
                port,
            }],
        }),
        registry_certificate: None,
    }
}

async fn test_service(temp_dir: &Path) -> ControlPlaneService {
    test_service_with_registry(temp_dir, EndpointRegistry::new_mock(Arc::new(MockStorage::new())))
        .await
}

/// Like `test_service`, but takes the `EndpointRegistry` from the caller
/// instead of creating one internally -- lets a test keep a handle to query
/// `owner_of` after a deploy records ownership (B7a), the way a real
/// owner-rooted `is_trusted_root` predicate does (ADR-0015 A6).
async fn test_service_with_registry(
    temp_dir: &Path,
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
    )
    .await
    .unwrap()
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

async fn deploy(service: &ControlPlaneService, service_id: &str, caller: &CallerContext) {
    deploy_result(service, service_id, caller).await.unwrap();
}

async fn readyz_result(
    service: &ControlPlaneService,
    service_id: &str,
    caller: &CallerContext,
) -> RpcResult<NativeResponse> {
    let invocation = NativeInvocation {
        interface: "orchestrator".to_string(),
        method: "readyz".to_string(),
        params: json!((service_id,)),
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

/// §3.2's table, row 5: an ordinary caller holding no grant at all is
/// denied `deploy`, even for a brand-new `service_id` nobody owns yet --
/// the takeover check (F7) alone would let them through (no existing owner
/// to conflict with); the new Tier-1 admission gate must reject them
/// independently.
#[tokio::test]
async fn deploy_denied_without_an_orchestrator_grant() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let mallory = plain_caller("did:key:zMallory");

    let result = deploy_result(&service, "brand-new-svc", &mallory).await;
    assert!(result.is_err(), "a caller with no orchestrator/deploy grant must be denied deploy");
}

/// §3.2's table, row 4: an app-scoped grantee's capability is prefix-covered
/// to their own app selector -- it does not cover a *different* app.
#[tokio::test]
async fn app_scoped_grantee_cannot_deploy_a_different_app() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let grantee = app_grantee("did:key:zGrantee", "allowed-app");

    deploy(&service, "allowed-app", &grantee).await;
    let result = deploy_result(&service, "other-app", &grantee).await;
    assert!(result.is_err(), "a grant scoped to one app must not cover a different app");
}

/// §3.2's table, row 3, positive path: the same grantee may deploy the app
/// their grant actually names.
#[tokio::test]
async fn app_scoped_grantee_can_deploy_their_own_app() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let grantee = app_grantee("did:key:zGrantee", "allowed-app");

    let result = deploy_result(&service, "allowed-app", &grantee).await;
    assert!(result.is_ok(), "a grant scoped to this app must admit its own deploy: {result:?}");
}

/// §2.2's predicate: a selector-bearing capability is not `is_substrate_scope`
/// (M04A Slice B7b/F2), so an app-scoped grantee does not gain the node-wide
/// visibility bar `list` grants a bare `substrate:<node>` holder -- they see
/// only their own app, exactly like an ordinary (unscoped) owner.
#[tokio::test]
async fn app_scoped_grantee_does_not_see_every_app() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let grantee = app_grantee("did:key:zGrantee", "my-app");
    let other = app_grantee("did:key:zOther", "other-app");

    deploy(&service, "my-app", &grantee).await;
    deploy(&service, "other-app", &other).await;

    let seen = list(&service, &grantee).await;
    assert_eq!(seen.len(), 1, "an app-scoped grantee must not see every app");
    assert_eq!(seen[0].service_id, "my-app");
}

/// §2.4.1: the per-service form of `readyz` (a non-empty `service_id`) is
/// gated on `orchestrator/status`, exactly like `deploy`/`undeploy` gate on
/// their own abilities.
#[tokio::test]
async fn per_service_readyz_denied_without_orchestrator_status() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let owner = app_grantee("did:key:zOwner", "svc-a");
    let onlooker = plain_caller("did:key:zOnlooker");

    deploy(&service, "svc-a", &owner).await;

    let result = readyz_result(&service, "svc-a", &onlooker).await;
    assert!(
        result.is_err(),
        "a caller with no orchestrator/status grant for this app must be denied readyz"
    );
}

/// The `wait_for_ready` regression guard (§2.4.1): the empty-`service_id`
/// liveness ping must stay open even for a caller holding no capabilities at
/// all -- every `roymctl`/SDK client calls it pre-capability during
/// `connect()`, so gating it would break connect for every ordinary client.
#[tokio::test]
async fn empty_readyz_stays_open_regardless_of_capabilities() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let nobody = plain_caller("did:key:zNobody");

    let invocation = NativeInvocation {
        interface: "orchestrator".to_string(),
        method: "readyz".to_string(),
        params: json!(("",)),
        caller: nobody,
    };
    let result = service.dispatch(invocation).await;
    assert!(result.is_ok(), "the empty-service_id liveness ping must stay open: {result:?}");
}

/// A grantee holding a valid app-scoped grant clears the admission gate for
/// their own app's per-service readyz -- proven by the *error changing*, not
/// by a bare `Ok`: this environment has no real podman/WASM runtime to
/// finish the underlying readiness check (a TCP-manifest deploy makes
/// `readyz` also probe a container engine after the gate), so asserting
/// `is_ok()` here would make the test depend on podman being installed,
/// which is not what B7b's admission gate is about. What must hold is that
/// the *admission* error (`"holds no orchestrator/status grant"`) is gone.
#[tokio::test]
async fn per_service_readyz_admitted_with_orchestrator_status() {
    let temp_dir = tempfile::tempdir().unwrap();
    let service = test_service(temp_dir.path()).await;
    let owner = app_grantee("did:key:zOwner", "svc-a");

    deploy(&service, "svc-a", &owner).await;

    let result = readyz_result(&service, "svc-a", &owner).await;
    if let Err(e) = &result {
        assert!(
            !format!("{e:?}").contains("orchestrator/status"),
            "the owner's own grant must clear the admission gate, whatever the underlying \
             readiness engine then reports: {e:?}"
        );
    }
}

/// A verified `CapabilityToken` chain, owner-rooted per ADR-0015 A6 (trusted
/// iff the issuer is `registry`'s recorded owner of `service_id`), resolved
/// with the real `syneroym_ucan::verify_chain` -- the same call `build_caller`
/// makes, not a hand-built `SessionContext`. Mirrors `build_caller`'s
/// owner-rooted half of `is_root` (`crates/router/src/route_handler/io.rs`).
fn owner_rooted_caller_from_real_token(
    registry: &EndpointRegistry,
    service_id: &str,
    audience_did: &str,
    token: &CapabilityToken,
) -> CallerContext {
    let registry = registry.clone();
    let service_id = service_id.to_string();
    let is_root =
        move |iss: &str, _cap: &Capability| registry.owner_of(&service_id).as_deref() == Some(iss);
    let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let opts = ChainVerifyOpts {
        expected_audience_did: audience_did,
        is_trusted_root: &is_root,
        now_secs,
    };
    let session = SessionContext::from_verified_chain(token, &opts)
        .expect("a structurally valid chain must verify (even if it admits nothing)");
    CallerContext {
        caller_did: audience_did.to_string(),
        app_instance: None,
        session,
        auth: AuthLevel::Ucan,
        proof: None,
    }
}

/// Post-commit review (F3): joins the two halves task.md item 1 claims are
/// both covered -- a real signed, real-registry-owner-rooted `CapabilityToken`
/// (ADR-0015 A6), verified through the real `syneroym_ucan` chain-verification
/// code `build_caller` also calls (not a hand-built `SessionContext`), admits
/// at the real `ControlPlaneService::deploy` gate. Positive case: the owner
/// self-issues a real token to redeploy their own already-existing service --
/// the one owner-rooted-grant shape that clears *both* the Tier-1 capability
/// gate and F7's takeover-protection check (which requires `caller_did` to
/// equal the recorded owner, or node-wide authority -- see the negative case
/// below for what a non-owner delegate's real grant runs into).
#[tokio::test]
async fn owner_self_issued_real_token_admits_redeploy_of_their_own_service() {
    let temp_dir = tempfile::tempdir().unwrap();
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let service = test_service_with_registry(temp_dir.path(), registry.clone()).await;

    // The initial deploy establishes `owner_of("svc-a")` (B7a); a hand-built
    // grantee caller (`owner_did` as its `caller_did`) stands in for whatever
    // authenticated that first deploy -- this file's other tests establish
    // that admission is unaffected by how the capability was verified.
    let owner_identity = Identity::generate().unwrap();
    let owner_did = derive_did_key(&owner_identity.public_key());
    deploy(&service, "svc-a", &app_grantee(&owner_did, "svc-a")).await;
    assert_eq!(registry.owner_of("svc-a").as_deref(), Some(owner_did.as_str()));

    let resource = ResourceUri(format!("substrate:{NODE_DID}/app/svc-a"));
    let token = CapabilityToken::issue(
        &owner_identity,
        &owner_did,
        vec![Capability {
            with: resource,
            can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let caller = owner_rooted_caller_from_real_token(&registry, "svc-a", &owner_did, &token);
    assert_eq!(caller.auth, AuthLevel::Ucan);
    assert!(
        !caller.session.capabilities.is_empty(),
        "the owner-rooted chain must admit the deploy capability"
    );

    let result = deploy_result(&service, "svc-a", &caller).await;
    assert!(
        result.is_ok(),
        "the owner's own real signed, owner-rooted grant must admit redeploy at the real gate: \
         {result:?}"
    );
}

/// The negative half of F3's join: a *different* party holding a real
/// signed, owner-rooted grant for someone else's service is admitted at the
/// Tier-1 capability gate but still rejected by F7's takeover-protection
/// check, which binds on `caller_did == recorded owner` (or node-wide
/// authority), not on capability possession. Confirms capability delegation
/// cannot be used to bypass takeover protection -- the two checks in
/// `orchestration.rs::deploy` are independent, and a real verified chain
/// does not change that.
#[tokio::test]
async fn owner_rooted_grant_to_a_different_caller_does_not_bypass_takeover_protection() {
    let temp_dir = tempfile::tempdir().unwrap();
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let service = test_service_with_registry(temp_dir.path(), registry.clone()).await;

    let owner_identity = Identity::generate().unwrap();
    let owner_did = derive_did_key(&owner_identity.public_key());
    deploy(&service, "svc-a", &app_grantee(&owner_did, "svc-a")).await;

    let client_identity = Identity::generate().unwrap();
    let client_did = derive_did_key(&client_identity.public_key());
    let resource = ResourceUri(format!("substrate:{NODE_DID}/app/svc-a"));
    let token = CapabilityToken::issue(
        &owner_identity,
        &client_did,
        vec![Capability {
            with: resource,
            can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let caller = owner_rooted_caller_from_real_token(&registry, "svc-a", &client_did, &token);
    assert!(
        !caller.session.capabilities.is_empty(),
        "the Tier-1 capability gate admits the delegate's real grant"
    );

    let result = deploy_result(&service, "svc-a", &caller).await;
    assert!(
        result.is_err(),
        "a delegate holding a real owner-rooted grant must still be rejected by takeover \
         protection, since they are not the recorded owner: {result:?}"
    );
}

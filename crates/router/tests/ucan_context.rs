#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Slice B1 (M04A): UCAN context extraction and normalization -- reference
//! scenario step 21 ("A client presents a UCAN; the gateway verifies the
//! chain and normalizes claims/capabilities into a SessionContext").
//!
//! Router-internal chain verification/revocation wiring (`build_caller`) is
//! unit-tested directly in `crates/router/src/route_handler/io.rs` (it is a
//! private function, not reachable from an external test crate). This file
//! proves the other half of step 21 -- that a capability produced by real
//! `syneroym_ucan` chain verification (the same verification `build_caller`
//! performs) is admitted all the way through to native dispatch, using the
//! same harness style as `native_dispatch_identity.rs`'s
//! `execute_ddl_allowed_for_admin_ucan_root_native_caller`.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;
use serde_json::{Value, json};
use syneroym_control_plane::SynSvcNativeService;
use syneroym_core::{
    config::SubstrateConfig, http_routes::HttpRouteRegistry, local_registry::EndpointRegistry,
    storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{
    AdaptationStage, EncryptionStage, RouteHandler, RouteHandlerDeps, RoutePipeline, RoutePreamble,
    ServiceStage, TransportStage,
};
use syneroym_rpc::{
    Ability, AuthLevel, CallerContext, Capability, CapabilityToken, ChainVerifyOpts,
    NativeDispatchRegistry, NativeInvocation, NativeResponse, NativeService, ResourceUri,
    RpcResult, SessionContext,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tempfile::tempdir;

#[derive(Debug, Default)]
struct NoopControlPlane;

#[async_trait::async_trait]
impl NativeService for NoopControlPlane {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        Ok(NativeResponse { payload: Value::Null })
    }
}

fn json_rpc_body(method: &str, params: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": method, "params": params, "id": 1}))
        .unwrap()
}

fn raw_pipeline(service_id: &str) -> RoutePipeline {
    RoutePipeline {
        encryption: EncryptionStage::None,
        transport: TransportStage::Binary,
        adaptation: AdaptationStage::None,
        service: ServiceStage::NativeService { service_id: service_id.to_string() },
    }
}

/// Builds a minimal `RouteHandler` (mirroring `native_dispatch_identity.rs`'s
/// own helper) with a single `data-layer`-capable native service registered.
async fn test_route_handler(service_id: &str) -> RouteHandler {
    let temp_dir = tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let app_sandbox_engine = Arc::new(
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
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());

    let data_service = Arc::new(SynSvcNativeService::new(
        service_id.to_string(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider.clone(),
        messaging_broker.clone(),
        None,
    ));

    let deps = RouteHandlerDeps {
        key_store,
        storage_provider,
        app_sandbox_engine,
        messaging_broker,
        native_dispatch: NativeDispatchRegistry::default(),
        http_routes,
        control_plane_service: Arc::new(NoopControlPlane),
    };

    let route_handler = RouteHandler::init(
        "test-orchestrator".to_string(),
        &config,
        registry,
        [11u8; 32],
        None,
        deps,
    )
    .await
    .unwrap();

    route_handler.register_native_service(service_id.to_string(), data_service);
    route_handler
}

/// A verified `CapabilityToken` chain, rooted at `admin_root`, resolves into
/// exactly the `CallerContext` `build_caller` would construct for the same
/// input -- this is the step 21 "normalizes ... into a SessionContext"
/// claim, proven against the real `syneroym_ucan` verification code (not a
/// hand-built `SessionContext`).
fn caller_from_verified_chain(
    client_did: &str,
    admin_root: &str,
    token: &CapabilityToken,
) -> CallerContext {
    let is_root = |iss: &str, _cap: &Capability| iss == admin_root;
    let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let opts =
        ChainVerifyOpts { expected_audience_did: client_did, is_trusted_root: &is_root, now_secs };
    let session = SessionContext::from_verified_chain(token, &opts)
        .expect("chain rooted at admin_root, addressed to client, must verify");

    CallerContext {
        caller_did: client_did.to_string(),
        app_instance: None,
        session,
        auth: AuthLevel::Ucan,
        proof: None,
    }
}

/// Step 21, positive: `owner` (the configured admin root) issues a
/// `data-layer/admin` capability to `client`; the resulting `CallerContext`
/// admits `execute-ddl` on the target service -- the same admission
/// `execute_ddl_allowed_for_admin_ucan_root_native_caller`
/// (`native_dispatch_identity.rs`) proves for a hand-built admin caller, here
/// proven for a caller built from a *verified UCAN chain* instead.
#[tokio::test]
async fn verified_ucan_capability_reaches_native_dispatch() {
    let owner = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let client_did = derive_did_key(&client.public_key());

    let service_id = "ucan-context-svc";
    let route_handler = test_route_handler(service_id).await;

    // The `execute-ddl` gate checks `data-layer/admin` on
    // `ResourceUri::service(app_instance.unwrap_or(service_id), service_id)`
    // (`synsvc_native.rs`); our test caller has no `app_instance`, so this
    // resolves to `ResourceUri::service(service_id, service_id)`.
    let resource = ResourceUri::service(service_id, service_id);
    let token = CapabilityToken::issue(
        &owner,
        &client_did,
        vec![Capability {
            with: resource,
            can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let caller = caller_from_verified_chain(&client_did, &admin_root, &token);
    assert_eq!(caller.auth, AuthLevel::Ucan);
    assert_eq!(caller.session.subject_did, client_did);

    let pipeline = raw_pipeline(service_id);
    let preamble = RoutePreamble::binary_json_rpc(service_id, "data-layer");
    let body = json_rpc_body("execute-ddl", json!({"sql": "CREATE TABLE x (id TEXT)"}));

    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert!(
        resp.get("error").is_none(),
        "a caller holding a verified data-layer/admin UCAN capability must be admitted to \
         execute-ddl: {resp:?}"
    );
}

/// Step 21, negative: a chain rooted at a non-admin issuer grants nothing,
/// so the same `execute-ddl` call is denied -- B1 has exactly one trust
/// root (the node admin), not an arbitrary issuer.
#[tokio::test]
async fn ucan_capability_from_untrusted_root_does_not_reach_native_dispatch() {
    let owner = Identity::generate().unwrap();
    let alice = Identity::generate().unwrap();
    let client = Identity::generate().unwrap();
    let admin_root = derive_did_key(&owner.public_key());
    let client_did = derive_did_key(&client.public_key());

    let service_id = "ucan-context-untrusted-svc";
    let route_handler = test_route_handler(service_id).await;

    let resource = ResourceUri::service(service_id, service_id);
    // Issued by `alice`, not the admin root.
    let token = CapabilityToken::issue(
        &alice,
        &client_did,
        vec![Capability {
            with: resource,
            can: Ability(Ability::DATA_LAYER_ADMIN.to_string()),
            caveats: None,
        }],
        serde_json::Map::new(),
        3600,
        vec![],
    )
    .unwrap();

    let caller = caller_from_verified_chain(&client_did, &admin_root, &token);
    assert!(
        caller.session.capabilities.is_empty(),
        "a chain rooted at a non-admin issuer must grant nothing"
    );

    let pipeline = raw_pipeline(service_id);
    let preamble = RoutePreamble::binary_json_rpc(service_id, "data-layer");
    let body = json_rpc_body("execute-ddl", json!({"sql": "CREATE TABLE x (id TEXT)"}));

    let resp = route_handler
        .dispatch_json_rpc_once(&pipeline, &preamble, Some(&caller), &body)
        .await
        .unwrap();
    let resp: Value = serde_json::from_slice(&resp).unwrap();
    assert_eq!(
        resp["error"]["code"], -32010,
        "a caller with no admitted capability must be denied execute-ddl: {resp:?}"
    );
}

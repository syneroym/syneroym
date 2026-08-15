#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! M06A A2 integration tests: `syneroym:http/incoming-handler#handle-request`
//! driven end to end through `AppSandboxEngine::handle_guest_http_request`,
//! bypassing the router/HTTP-bridge layer (covered separately by
//! `crates/router/src/route_handler/http.rs`'s own tests and
//! `crates/substrate/tests/guest_http_e2e.rs`) so these tests focus on the
//! Wasmtime/dynamic-invocation boundary this slice adds.

use std::{fs, path::Path, sync::Arc};

use syneroym_core::{
    config::SubstrateConfig,
    guest_http::{GuestCallerAuth, GuestCallerIdentity, GuestHttpRequest},
    local_registry::EndpointRegistry,
    storage::MockStorage,
    test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{AuthLevel, CallerContext, JsonRpcRequest, SessionContext};
use syneroym_sandbox_wasm::{AppSandboxEngine, GuestHttpFailure, GuestHttpOutcome};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};

const TEST_DRIVER_INTERFACE: &str = test_constants::HTTP_GUEST_TEST_DRIVER_INTERFACE;
const SERVICE_ID: &str = "http-guest-svc";

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
        },
        service_type: ServiceType::Wasm(WasmManifest {
            source: ArtifactSource::Binary(bytes),
            hash: None,
            interfaces: vec![TEST_DRIVER_INTERFACE.to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

fn read_http_guest_test_wasm() -> Option<Vec<u8>> {
    fs::read(test_constants::http_guest_test_wasm_path()).ok()
}

macro_rules! skip_if_missing {
    ($test_name:literal) => {
        match read_http_guest_test_wasm() {
            Some(bytes) => bytes,
            None => {
                eprintln!(
                    "Skipping {}: http-guest-test WASM artifact not found (build \
                     test-components/http-guest-test with `cargo component build --release \
                     --target wasm32-wasip2`)",
                    $test_name
                );
                return;
            }
        }
    };
}

fn request(path: &str) -> GuestHttpRequest {
    GuestHttpRequest {
        method: "GET".to_string(),
        path: path.to_string(),
        query: String::new(),
        route: path.to_string(),
        path_params: vec![],
        headers: vec![],
        body: vec![],
        caller: None,
    }
}

async fn last_request_json(engine: &AppSandboxEngine, service_id: &str) -> serde_json::Value {
    let req = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "last-request".to_string(),
        params: serde_json::Value::Array(vec![]),
        id: None,
        idempotency_key: None,
    };
    let raw = engine.execute_wasm(service_id, TEST_DRIVER_INTERFACE, &req).await.unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[tokio::test]
async fn echo_round_trips_every_request_field() {
    let wasm_bytes = skip_if_missing!("echo_round_trips_every_request_field");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_ID, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let mut req = request("/echo");
    req.query = "a=b".to_string();
    req.headers = vec![("x-test".to_string(), "1".to_string())];
    req.body = vec![1, 2, 3];

    let outcome = engine.handle_guest_http_request(SERVICE_ID, &req, None).await.unwrap();
    let GuestHttpOutcome::Response(response) = outcome else { panic!("expected a response") };
    assert_eq!(response.status, 200);
    let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["method"], "GET");
    assert_eq!(body["path"], "/echo");
    assert_eq!(body["query"], "a=b");
    assert_eq!(body["header_count"], 1);
    assert_eq!(body["body_len"], 3);
    assert_eq!(body["caller"], serde_json::Value::Null);
}

#[tokio::test]
async fn last_request_persists_across_a_fresh_instantiation() {
    let wasm_bytes = skip_if_missing!("last_request_persists_across_a_fresh_instantiation");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_ID, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let mut req = request("/items/42");
    req.route = "/items/{id}".to_string();
    req.path_params = vec![("id".to_string(), "42".to_string())];
    engine.handle_guest_http_request(SERVICE_ID, &req, None).await.unwrap();

    // `last-request` runs in an entirely separate `handle_guest_http_request`-
    // style instantiation (via `execute_wasm`) -- if this reads back what
    // `/items/42` recorded, persistence survives the fresh instance every
    // call gets, proving the fixture (and the boundary it exercises)
    // doesn't rely on in-guest memory.
    let recorded = last_request_json(&engine, SERVICE_ID).await;
    assert_eq!(recorded["path"], "/items/42");
    assert_eq!(recorded["route"], "/items/{id}");
}

#[tokio::test]
async fn reject_returns_the_guests_own_status_and_message() {
    let wasm_bytes = skip_if_missing!("reject_returns_the_guests_own_status_and_message");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_ID, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let outcome =
        engine.handle_guest_http_request(SERVICE_ID, &request("/reject"), None).await.unwrap();
    let GuestHttpOutcome::Response(response) = outcome else { panic!("expected a response") };
    assert_eq!(response.status, 422);
    assert_eq!(response.body, b"comment is empty");
}

#[tokio::test]
async fn fail_is_declined_with_the_guests_message() {
    let wasm_bytes = skip_if_missing!("fail_is_declined_with_the_guests_message");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_ID, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    let outcome =
        engine.handle_guest_http_request(SERVICE_ID, &request("/fail"), None).await.unwrap();
    assert!(
        matches!(outcome, GuestHttpOutcome::Failed(GuestHttpFailure::Declined(ref m)) if m == "handler blew up")
    );
}

#[tokio::test]
async fn whoami_reflects_the_forwarded_caller_identity() {
    let wasm_bytes = skip_if_missing!("whoami_reflects_the_forwarded_caller_identity");
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    engine.deploy_wasm(SERVICE_ID, &wasm_deploy_manifest(wasm_bytes)).await.unwrap();

    // Anonymous.
    let outcome =
        engine.handle_guest_http_request(SERVICE_ID, &request("/whoami"), None).await.unwrap();
    let GuestHttpOutcome::Response(response) = outcome else { panic!("expected a response") };
    assert_eq!(response.body, b"anonymous");

    // A delegated caller: both the marshalled `GuestHttpRequest.caller`
    // (what the guest sees) and the raw `CallerContext` (what `HostState`
    // gets) are supplied, matching how the router builds both from the same
    // connection.
    let mut req = request("/whoami");
    req.caller = Some(GuestCallerIdentity {
        did: "did:key:caller123".to_string(),
        auth: GuestCallerAuth::Delegated,
        app_instance: None,
    });
    let caller_context = CallerContext {
        caller_did: "did:key:caller123".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:caller123".to_string(),
            ..Default::default()
        },
        auth: AuthLevel::Delegated,
        proof: None,
    };
    let outcome =
        engine.handle_guest_http_request(SERVICE_ID, &req, Some(caller_context)).await.unwrap();
    let GuestHttpOutcome::Response(response) = outcome else { panic!("expected a response") };
    assert_eq!(response.body, b"delegated:did:key:caller123".to_vec());
}

#[tokio::test]
async fn a_component_with_no_handler_export_fails_cleanly() {
    let wasm_bytes = fs::read(test_constants::greeter_wasm_path()).ok();
    let Some(wasm_bytes) = wasm_bytes else {
        eprintln!(
            "Skipping a_component_with_no_handler_export_fails_cleanly: greeter WASM artifact not \
             found"
        );
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let engine = make_engine(dir.path()).await;
    let mut manifest = wasm_deploy_manifest(wasm_bytes);
    let ServiceType::Wasm(ref mut wasm_manifest) = manifest.service_type else { unreachable!() };
    wasm_manifest.interfaces = vec![test_constants::GREETER_INTERFACE_NAME.to_string()];
    engine.deploy_wasm(SERVICE_ID, &manifest).await.unwrap();

    let outcome =
        engine.handle_guest_http_request(SERVICE_ID, &request("/echo"), None).await.unwrap();
    assert!(matches!(outcome, GuestHttpOutcome::Failed(GuestHttpFailure::NoHandler)));
}

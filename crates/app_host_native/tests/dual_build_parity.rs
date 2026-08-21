#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! One integration suite driving the dual-build-shim fixture through both
//! builds -- the real `wasm32-wasip2` component via `AppSandboxEngine`, and
//! the same source linked in via `syneroym-app-host-native` -- and
//! asserting the results are identical. A test that passes on one build and
//! fails on the other is a bug in the shim, not in the test.

use std::{
    fs,
    path::Path,
    sync::{Arc, Weak},
    time::Duration,
};

use serde_json::{Value, json};
use syneroym_app_host_native::{ConversationSink, MessageSink, NativeHostFactory};
use syneroym_async_queue::QueueConfig;
use syneroym_conversation::ConversationService;
use syneroym_core::{
    config::{RetryPolicy, SubstrateConfig},
    local_registry::EndpointRegistry,
    storage::MockStorage,
    test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{AuthLevel, CallerContext, JsonRpcRequest, NativeInvocation, SessionContext};
use syneroym_sandbox_wasm::AppSandboxEngine;
use syneroym_test_dual_build_fixture::native::{FIXTURE_INTERFACE, NativeFixture};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};

/// The two builds must share one service id, and must therefore share
/// nothing else -- it is the store namespace, the broker topic namespace,
/// and the `data-layer/admin` gate resource, all at once.
const SERVICE_ID: &str = "dual-build-fixture-parity";

/// Both builds run under a real, identical, non-anonymous caller -- the
/// router's own distinct treatment of an anonymous caller per interface
/// kind is a router concern, out of scope for this shim-parity suite.
fn caller() -> CallerContext {
    CallerContext {
        caller_did: "did:key:zParityTestCaller".to_string(),
        app_instance: None,
        session: SessionContext {
            subject_did: "did:key:zParityTestCaller".to_string(),
            ..Default::default()
        },
        auth: AuthLevel::Ucan,
        proof: None,
    }
}

trait Driver {
    async fn run(&self, request: &str) -> Result<String, String>;
}

/// Drives the component through the real sandbox engine.
struct WasmDriver {
    engine: Arc<AppSandboxEngine>,
}

impl Driver for WasmDriver {
    async fn run(&self, request: &str) -> Result<String, String> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "run".to_string(),
            params: json!([request]),
            id: None,
            idempotency_key: None,
        };
        let result = self
            .engine
            .execute_wasm_json(SERVICE_ID, FIXTURE_INTERFACE, &req, Some(caller()))
            .await
            .map_err(|e| e.to_string())?;
        match result {
            Value::String(s) => Ok(s),
            other => Err(format!("expected a string result, got {other:?}")),
        }
    }
}

/// Drives the same source, linked in, through the shim.
struct NativeDriver {
    fixture: Arc<NativeFixture<syneroym_app_host_native::NativeAppHost>>,
}

impl Driver for NativeDriver {
    async fn run(&self, request: &str) -> Result<String, String> {
        use syneroym_rpc::NativeService;
        let inv = NativeInvocation {
            interface: "test-driver".to_string(),
            method: "run".to_string(),
            params: json!([request]),
            caller: caller(),
        };
        let response = self.fixture.dispatch(inv).await.map_err(|e| e.to_string())?;
        match response.payload {
            Value::String(s) => Ok(s),
            other => Err(format!("expected a string result, got {other:?}")),
        }
    }
}

/// Wraps another driver and corrupts one field of its result. Exists purely
/// to prove the parity comparison detects a divergence -- if
/// `the_parity_comparison_detects_a_divergence` ever passes with this
/// removed, `both_builds_produce_identical_results` is not comparing
/// anything.
struct Mutant<'a, D>(&'a D);

impl<D: Driver> Driver for Mutant<'_, D> {
    async fn run(&self, request: &str) -> Result<String, String> {
        self.0.run(request).await.map(|s| s.replace("\"written\"", "\"wrote\""))
    }
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
            interfaces: vec![FIXTURE_INTERFACE.to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
    }
}

/// Everything one full harness setup produces, for tests that need to poke
/// past the `Driver` abstraction (e.g. asserting on persisted storage
/// state).
struct Harness {
    wasm: WasmDriver,
    native: NativeDriver,
    wasm_engine: Arc<AppSandboxEngine>,
    native_factory: Arc<NativeHostFactory>,
    native_storage_provider: Arc<dyn StorageProvider>,
    /// M06B slice B4: each stack's own `ConversationService`, for tests
    /// that drive the peer-facing side (`prekey_bundle`/`peer_deliver`)
    /// directly rather than through the guest `run()` surface.
    wasm_conversation: Arc<ConversationService>,
    native_conversation: Arc<ConversationService>,
    // Dropped last (declaration order), after everything that might still
    // have files open under them.
    _wasm_dir: tempfile::TempDir,
    _native_dir: tempfile::TempDir,
}

/// Tears the native stack down the way a real embedder would when a linked
/// app is undeployed -- this is `NativeHostFactory::shutdown`'s only caller.
impl Drop for Harness {
    fn drop(&mut self) {
        self.native_factory.shutdown();
    }
}

/// Two fully independent host stacks, sharing one `SERVICE_ID`. Panics if
/// the wasm component artifact hasn't been built -- this suite is the
/// milestone's evidence for exit criterion 2 (dual-build parity), so a run
/// that silently skipped every test would be worse than a build failure,
/// not equivalent to one. Build it with `mise run build:test-components`.
async fn harness() -> Harness {
    let wasm_bytes = fs::read(test_constants::dual_build_fixture_wasm_path()).unwrap_or_else(|e| {
        panic!(
            "dual_build_parity: WASM artifact not found ({e}) -- run `mise run \
             build:test-components`, or `cargo component build --release --target wasm32-wasip2 \
             -p syneroym-test-dual-build-fixture`"
        )
    });

    let wasm_dir = tempfile::tempdir().unwrap();
    let native_dir = tempfile::tempdir().unwrap();

    let (wasm_engine, wasm_conversation) = build_wasm_stack(wasm_dir.path(), &wasm_bytes).await;
    let (native_fixture, native_factory, native_storage_provider, native_conversation) =
        build_native_stack(native_dir.path());

    Harness {
        wasm: WasmDriver { engine: wasm_engine.clone() },
        native: NativeDriver { fixture: native_fixture },
        wasm_engine,
        native_factory,
        native_storage_provider,
        wasm_conversation,
        native_conversation,
        _wasm_dir: wasm_dir,
        _native_dir: native_dir,
    }
}

fn test_conversation_service(
    storage_provider: Arc<dyn StorageProvider>,
    key_store: Arc<KeyStore>,
    registry: EndpointRegistry,
) -> Arc<ConversationService> {
    ConversationService::new(
        storage_provider,
        key_store,
        registry,
        QueueConfig {
            retry: RetryPolicy {
                max_attempts: 5,
                initial_backoff_ms: 10,
                backoff_multiplier: 2.0,
                max_backoff_ms: 1000,
            },
            visibility_timeout_ms: 5000,
            dlq_max_rows: 100,
            max_pending_rows: 1000,
        },
        syneroym_conversation::ConversationConfig::default(),
    )
    .unwrap()
}

async fn build_wasm_stack(
    dir: &Path,
    wasm_bytes: &[u8],
) -> (Arc<AppSandboxEngine>, Arc<ConversationService>) {
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
    // `MqttBroker::new` opens no listener, so a second in-process instance
    // per stack costs nothing and binds no port.
    let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let conversation =
        test_conversation_service(storage_provider.clone(), key_store.clone(), registry.clone());

    let engine = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store,
            storage_provider,
            blob_provider,
            broker,
            registry,
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    engine.self_weak.set(Arc::downgrade(&engine)).expect("self_weak set once");
    engine
        .conversation
        .set(Arc::downgrade(&conversation) as std::sync::Weak<dyn syneroym_rpc::ConversationHost>)
        .expect("conversation set once");
    conversation.set_notifier(
        Arc::downgrade(&engine) as std::sync::Weak<dyn syneroym_rpc::ConversationNotifier>
    );
    engine.deploy_wasm(SERVICE_ID, &wasm_deploy_manifest(wasm_bytes.to_vec())).await.unwrap();
    (engine, conversation)
}

type NativeStack = (
    Arc<NativeFixture<syneroym_app_host_native::NativeAppHost>>,
    Arc<NativeHostFactory>,
    Arc<dyn StorageProvider>,
    Arc<ConversationService>,
);

fn build_native_stack(dir: &Path) -> NativeStack {
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(dir.join("data"), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let endpoint_registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let conversation = test_conversation_service(
        storage_provider.clone(),
        key_store.clone(),
        endpoint_registry.clone(),
    );

    let factory = NativeHostFactory::new(
        SERVICE_ID.to_string(),
        key_store,
        storage_provider.clone(),
        blob_provider,
        broker,
        endpoint_registry,
        syneroym_app_orchestration::empty_resolver(),
        conversation.clone(),
    );
    let f = factory.clone();
    let fixture =
        Arc::new(NativeFixture::new(SERVICE_ID.to_string(), move |caller| f.host_for(caller)));
    factory.set_sink(Arc::downgrade(&fixture) as Weak<dyn MessageSink>);
    factory.set_conversation_sink(Arc::downgrade(&fixture) as Weak<dyn ConversationSink>);
    (fixture, factory, storage_provider, conversation)
}

/// Sequential-body scenarios only: everything here completes within one
/// `run()` call with no background delivery task involved. The messaging
/// scenario (subscribe/publish/read-inbox) needs a settle step per build and
/// is its own dedicated test below, not part of this table.
const SCENARIOS: &[(&str, &str)] = &[
    ("store-messages", r#"{"op":"store-messages","count":5}"#),
    ("read-messages", r#"{"op":"read-messages","limit":100}"#),
    ("admin-ddl", r#"{"op":"admin-ddl","sql":"DROP TABLE messages"}"#),
    ("get-missing", r#"{"op":"get-missing","id":"does-not-exist"}"#),
    ("put-blob", r#"{"op":"put-blob","body":"hello dual-build shim"}"#),
    ("stream-blob", r#"{"op":"stream-blob","chunks":["ab","cd","ef"],"read_chunk":2}"#),
    ("unsubscribe", r#"{"op":"unsubscribe","topic":"scratch-topic"}"#),
    ("patch", r#"{"op":"patch","id":"p1"}"#),
    ("batch-mutate", r#"{"op":"batch-mutate","id_a":"b1","id_b":"b2"}"#),
    ("delete-many", r#"{"op":"delete-many","id":"dm1"}"#),
    ("drop-collection", r#"{"op":"drop-collection"}"#),
    ("delete-blob", r#"{"op":"delete-blob","body":"blob to delete"}"#),
    ("abort-upload", r#"{"op":"abort-upload","chunks":["ab","cd"]}"#),
    // M06B slice B4: `open-direct`'s id is derived from `(SERVICE_ID,
    // peer_address)` alone -- deterministic, so unlike `send-message`
    // (whose message id includes a random nonce) it belongs in this
    // byte-comparison table.
    ("open-conversation", r#"{"op":"open-conversation","peer_address":"peer-parity-scenario"}"#),
    // `retry`/`delivery-status`/`read-history` against an id that was never
    // created are deterministic error shapes too.
    ("retry-unknown", r#"{"op":"retry-message","message":"msg:does-not-exist"}"#),
    ("delivery-status-unknown", r#"{"op":"delivery-status","message":"msg:does-not-exist"}"#),
    (
        "read-history-unknown-conversation",
        r#"{"op":"read-history","conversation":"conv:does-not-exist","limit":10}"#,
    ),
    ("read-outbox-empty", r#"{"op":"read-outbox"}"#),
];

async fn scenarios<D: Driver>(d: &D) -> Vec<(&'static str, String)> {
    let mut out = Vec::with_capacity(SCENARIOS.len());
    for (name, request) in SCENARIOS {
        out.push((*name, d.run(request).await.unwrap_or_else(|e| format!("ERR:{e}"))));
    }
    out
}

#[tokio::test]
async fn both_builds_produce_identical_results() {
    let h = harness().await;
    let wasm_results = scenarios(&h.wasm).await;
    let native_results = scenarios(&h.native).await;
    assert!(!wasm_results.is_empty(), "the scenario table must not be empty");
    assert_eq!(wasm_results, native_results);
}

/// A passing `both_builds_produce_identical_results` is not evidence of
/// anything unless the comparison is known to detect a real divergence.
#[tokio::test]
async fn the_parity_comparison_detects_a_divergence() {
    let h = harness().await;
    let wasm_results = scenarios(&h.wasm).await;
    let mutant_results = scenarios(&Mutant(&h.native)).await;
    assert!(!wasm_results.is_empty());
    assert_ne!(wasm_results, mutant_results);
}

/// Named per-build positive assertions: the `assert_eq!` above tells you
/// *that* the builds differ, not which is wrong. A failure here names a
/// build.
#[tokio::test]
async fn wasm_build_store_and_read_round_trip() {
    let h = harness().await;
    let result = h.wasm.run(r#"{"op":"store-messages","count":5}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["ok"]["written"], 5);
    assert_eq!(v["ok"]["read"], 5);
}

#[tokio::test]
async fn native_build_store_and_read_round_trip() {
    let h = harness().await;
    let result = h.native.run(r#"{"op":"store-messages","count":5}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["ok"]["written"], 5);
    assert_eq!(v["ok"]["read"], 5);
}

#[tokio::test]
async fn wasm_build_stream_blob_round_trips_the_body() {
    let h = harness().await;
    let result = h
        .wasm
        .run(r#"{"op":"stream-blob","chunks":["ab","cd","ef"],"read_chunk":2}"#)
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["ok"]["body"], "abcdef");
}

#[tokio::test]
async fn native_build_stream_blob_round_trips_the_body() {
    let h = harness().await;
    let result = h
        .native
        .run(r#"{"op":"stream-blob","chunks":["ab","cd","ef"],"read_chunk":2}"#)
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["ok"]["body"], "abcdef");
}

#[tokio::test]
async fn wasm_build_admin_ddl_is_denied() {
    let h = harness().await;
    let result = h.wasm.run(r#"{"op":"admin-ddl","sql":"DROP TABLE messages"}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert!(v.get("err").is_some(), "expected admin-ddl to be denied, got {v}");
}

#[tokio::test]
async fn native_build_admin_ddl_is_denied() {
    let h = harness().await;
    let result = h.native.run(r#"{"op":"admin-ddl","sql":"DROP TABLE messages"}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert!(v.get("err").is_some(), "expected admin-ddl to be denied, got {v}");
}

/// `app::run`'s `serde_json::from_str` failure is the fixture's only WIT
/// `Err` path (as opposed to a WIT-level `Ok` carrying a JSON `"err"`
/// field, like the two tests above). Both builds surface it as an error at
/// this layer: `NativeFixture::dispatch`'s own comment notes its
/// `RpcError::InternalError` "mirrors the WASM `Err` arm's -32603" -- that
/// numeric code is a `syneroym-router` JSON-RPC-framing property
/// (`RpcError::code`, `crates/rpc/src/lib.rs`) neither driver here goes
/// through, so it is out of this suite's reach to assert directly.
#[tokio::test]
async fn malformed_request_json_errors_on_both_builds() {
    let h = harness().await;
    assert!(h.wasm.run(r#"{"op":"#).await.is_err());
    assert!(h.native.run(r#"{"op":"#).await.is_err());
}

/// `extract_request_param`'s `InvalidParams` arm needs a malformed *frame*
/// (no `request` field to find), which `Driver::run` can never produce --
/// it always builds a well-shaped `params: [<json>]`. Pinned here directly
/// against the native fixture, bypassing `Driver`. No WASM equivalent:
/// `WasmDriver` doesn't go through `NativeService::dispatch` either, so
/// there is nothing to compare against.
#[tokio::test]
async fn malformed_params_frame_is_invalid_params_not_internal_error() {
    use syneroym_rpc::{NativeService, RpcError};

    let h = harness().await;
    let inv = NativeInvocation {
        interface: "test-driver".to_string(),
        method: "run".to_string(),
        params: json!({}), // no "request" key
        caller: caller(),
    };
    let err = h.native.fixture.dispatch(inv).await.unwrap_err();
    assert!(matches!(err, RpcError::InvalidParams(_)), "got {err:?}");
}

/// Messaging round trip, with a settle step per build (publish is
/// fire-and-forget; delivery happens on a background task on both builds).
async fn poll_inbox_nonempty<D: Driver>(d: &D) -> Value {
    for _ in 0..50 {
        let result = d.run(r#"{"op":"read-inbox"}"#).await.unwrap();
        let v: Value = serde_json::from_str(&result).unwrap();
        if v["ok"]["entries"].as_array().is_some_and(|a| !a.is_empty()) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("inbox never became non-empty");
}

#[tokio::test]
async fn both_builds_deliver_a_published_message_to_their_own_inbox() {
    let h = harness().await;

    h.wasm.run(r#"{"op":"subscribe-topic","topic":"chat"}"#).await.unwrap();
    h.native.run(r#"{"op":"subscribe-topic","topic":"chat"}"#).await.unwrap();

    h.wasm.run(r#"{"op":"publish-topic","topic":"chat","payload":"hi from wasm"}"#).await.unwrap();
    h.native
        .run(r#"{"op":"publish-topic","topic":"chat","payload":"hi from native"}"#)
        .await
        .unwrap();

    let wasm_inbox = poll_inbox_nonempty(&h.wasm).await;
    let native_inbox = poll_inbox_nonempty(&h.native).await;

    let wasm_topic = wasm_inbox["ok"]["entries"][0]["topic"].as_str().unwrap();
    let native_topic = native_inbox["ok"]["entries"][0]["topic"].as_str().unwrap();
    // Both namespace to `svc/<SERVICE_ID>/chat` -- byte-identical since both
    // stacks share one service id.
    assert_eq!(wasm_topic, native_topic);
    assert_eq!(wasm_topic, format!("svc/{SERVICE_ID}/chat"));
}

// -- M06B slice B4: conversation scenarios not covered by the
// byte-comparison SCENARIOS table (a message id includes a random nonce,
// so `send-message`'s exact output cannot be compared verbatim across
// builds -- these assert on structure instead). --

#[tokio::test]
async fn open_direct_is_idempotent_on_both_builds() {
    let h = harness().await;
    let wasm_id_1 = open_conversation(&h.wasm, "peer-idempotent").await;
    let wasm_id_2 = open_conversation(&h.wasm, "peer-idempotent").await;
    assert_eq!(wasm_id_1, wasm_id_2, "wasm build: a second open-direct must return the same id");

    let native_id_1 = open_conversation(&h.native, "peer-idempotent").await;
    let native_id_2 = open_conversation(&h.native, "peer-idempotent").await;
    assert_eq!(
        native_id_1, native_id_2,
        "native build: a second open-direct must return the same id"
    );

    assert_eq!(wasm_id_1, native_id_1, "both builds must derive the same id for the same peer");
}

async fn open_conversation<D: Driver>(d: &D, peer_address: &str) -> String {
    let result = d
        .run(&format!(r#"{{"op":"open-conversation","peer_address":"{peer_address}"}}"#))
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    v["ok"]["conversation"].as_str().unwrap().to_string()
}

async fn assert_send_writes_pending_and_appears_in_the_outbox<D: Driver>(name: &str, driver: &D) {
    let conv = open_conversation(driver, "peer-send-pending").await;
    let send_result = driver
        .run(&format!(r#"{{"op":"send-message","conversation":"{conv}","body":"hello"}}"#))
        .await
        .unwrap();
    let send_v: Value = serde_json::from_str(&send_result).unwrap();
    let message_id = send_v["ok"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("{name}: send-message did not return a message id: {send_v}"));

    let status_result = driver
        .run(&format!(r#"{{"op":"delivery-status","message":"{message_id}"}}"#))
        .await
        .unwrap();
    let status_v: Value = serde_json::from_str(&status_result).unwrap();
    assert_eq!(
        status_v["ok"]["state"], "pending",
        "{name}: a freshly sent message must be pending"
    );

    let outbox_result = driver.run(r#"{"op":"read-outbox"}"#).await.unwrap();
    let outbox_v: Value = serde_json::from_str(&outbox_result).unwrap();
    let entries = outbox_v["ok"]["outbox"].as_array().unwrap();
    assert!(
        entries.iter().any(|e| e["id"] == message_id),
        "{name}: the outbox must list the just-sent message"
    );
}

#[tokio::test]
async fn send_writes_pending_and_appears_in_the_outbox_on_both_builds() {
    let h = harness().await;
    assert_send_writes_pending_and_appears_in_the_outbox("wasm", &h.wasm).await;
    assert_send_writes_pending_and_appears_in_the_outbox("native", &h.native).await;
}

async fn assert_oversized_body_is_refused<D: Driver>(name: &str, driver: &D, oversized: &str) {
    let conv = open_conversation(driver, "peer-quota").await;
    let result = driver
        .run(&format!(r#"{{"op":"send-message","conversation":"{conv}","body":"{oversized}"}}"#))
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert!(v["err"].is_string(), "{name}: an oversized body must be refused, got {v}");
}

#[tokio::test]
async fn a_body_over_the_configured_limit_is_refused_on_both_builds() {
    let h = harness().await;
    let oversized = "x".repeat(300_000); // > conversation_max_body_bytes (262_144)
    assert_oversized_body_is_refused("wasm", &h.wasm, &oversized).await;
    assert_oversized_body_is_refused("native", &h.native, &oversized).await;
}

async fn assert_retry_on_pending_is_refused<D: Driver>(name: &str, driver: &D) {
    let conv = open_conversation(driver, "peer-retry-pending").await;
    let send_result = driver
        .run(&format!(r#"{{"op":"send-message","conversation":"{conv}","body":"hi"}}"#))
        .await
        .unwrap();
    let send_v: Value = serde_json::from_str(&send_result).unwrap();
    let message_id = send_v["ok"]["message"].as_str().unwrap();

    let retry_result =
        driver.run(&format!(r#"{{"op":"retry-message","message":"{message_id}"}}"#)).await.unwrap();
    let retry_v: Value = serde_json::from_str(&retry_result).unwrap();
    assert!(
        retry_v["err"].is_string(),
        "{name}: retrying a pending (not failed) message must be refused, got {retry_v}"
    );
}

#[tokio::test]
async fn retry_on_a_pending_message_is_invalid_argument_on_both_builds() {
    let h = harness().await;
    assert_retry_on_pending_is_refused("wasm", &h.wasm).await;
    assert_retry_on_pending_is_refused("native", &h.native).await;
}

/// Drives a full prekey-bundle -> X3DH session -> sign -> encrypt
/// -> `peer_deliver` exchange from an independent third `ConversationService`
/// (standing in for a real peer substrate) into each build's own
/// `ConversationService`, and confirms the delivered message lands in that
/// build's own store, `verified: true`, and reaches the guest/native app's
/// `on-message` export (read back through `read-conversation-inbox`) --
/// the host -> app direction neither the SCENARIOS table nor `run()` alone
/// can exercise, since `peer_deliver` is reachable only through the
/// peer-facing native-capability dispatch arm, not the guest surface.
const SENDER_ADDRESS: &str = "external-peer-address";

async fn assert_signed_delivery_is_verified_and_notifies_the_app<D: Driver>(
    name: &str,
    target_conversation: &syneroym_conversation::ConversationService,
    driver: &D,
) {
    use syneroym_conversation::{
        crypto::{PrekeyBundle, SessionCrypto, X3dhDoubleRatchetCrypto, generate_identity_bytes},
        envelope::{self, DeliveryPayload},
        ids, store,
    };
    use syneroym_rpc::ConversationHost;

    {
        // The sender's own store -- an independent `ConversationStore`
        // standing in for a real peer substrate. Built directly (not
        // through a second `ConversationService`) since only session
        // establishment (`begin_session`/`encrypt`/`commit`) is needed.
        let sender_dir = tempfile::tempdir().unwrap();
        let sender_store = store::ConversationStore::open_encrypted(
            sender_dir.path(),
            None,
            QueueConfig {
                retry: RetryPolicy::default(),
                visibility_timeout_ms: 120_000,
                dlq_max_rows: 100,
                max_pending_rows: 1000,
            },
            store::ConversationConfig::default(),
        )
        .unwrap();
        let crypto = X3dhDoubleRatchetCrypto::new();

        let bundle_bytes =
            target_conversation.prekey_bundle(SERVICE_ID, SENDER_ADDRESS).await.unwrap();
        let bundle: PrekeyBundle = serde_json::from_slice(&bundle_bytes).unwrap();
        let mut session =
            crypto.begin_session(&sender_store, SENDER_ADDRESS, SERVICE_ID, &bundle).await.unwrap();

        let conversation_id = ids::derive_conversation_id(SENDER_ADDRESS, SERVICE_ID);
        let message_id = ids::derive_message_id(
            SENDER_ADDRESS,
            &conversation_id,
            1_000,
            "text/plain",
            b"hello from a peer",
            &[7u8; 16],
        );
        let identity = sender_store.local_identity_or_generate(generate_identity_bytes).unwrap();
        let sig_bytes: [u8; 32] = identity.sig_secret.as_slice().try_into().unwrap();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&sig_bytes);
        let signature = envelope::sign(
            &signing_key,
            &message_id,
            &conversation_id,
            SENDER_ADDRESS,
            1_000,
            "text/plain",
            b"hello from a peer",
        );
        let payload = DeliveryPayload {
            message_id: message_id.clone(),
            conversation_id: conversation_id.clone(),
            author: SENDER_ADDRESS.to_string(),
            sender_timestamp_ms: 1_000,
            content_type: "text/plain".to_string(),
            body: b"hello from a peer".to_vec(),
            signature,
        };
        let env = crypto.encrypt(&mut session, &payload).unwrap();
        crypto.commit(&sender_store, &session).await.unwrap();

        let env_bytes = serde_json::to_vec(&env).unwrap();
        let _ack_bytes = target_conversation
            .peer_deliver(SERVICE_ID, SENDER_ADDRESS, env_bytes)
            .await
            .unwrap_or_else(|e| panic!("{name}: peer_deliver failed: {e:?}"));
        // Both sides must derive the same conversation id — the receiver's
        // own value, computed independently, must match the sender's.
        let receiver_conv_id = ids::derive_conversation_id(SERVICE_ID, SENDER_ADDRESS);
        assert_eq!(receiver_conv_id, conversation_id);

        let history_result = driver
            .run(&format!(
                r#"{{"op":"read-history","conversation":"{receiver_conv_id}","limit":10}}"#
            ))
            .await
            .unwrap();
        let history_v: Value = serde_json::from_str(&history_result).unwrap();
        let messages = history_v["ok"]["messages"].as_array().unwrap();
        let delivered = messages.iter().find(|m| m["id"] == message_id).unwrap_or_else(|| {
            panic!("{name}: delivered message not found in history: {history_v}")
        });
        assert_eq!(
            delivered["verified"], true,
            "{name}: a validly signed delivery must be verified"
        );
        assert_eq!(
            delivered["state"], "delivered",
            "{name}: an inbound message is delivered on arrival"
        );

        // The app's own `on-message` export was called: the fixture
        // persists it through `data-layer`, read back here.
        let inbox_result = driver.run(r#"{"op":"read-conversation-inbox"}"#).await.unwrap();
        let inbox_v: Value = serde_json::from_str(&inbox_result).unwrap();
        let inbox_entries = inbox_v["ok"]["entries"].as_array().unwrap_or_else(|| {
            panic!("{name}: unexpected read-conversation-inbox response: {inbox_v}")
        });
        assert!(
            inbox_entries.iter().any(|e| e["id"] == message_id),
            "{name}: on-message must have notified the app, got {inbox_v}"
        );
    }
}

#[tokio::test]
async fn a_signed_delivery_from_an_external_peer_is_verified_and_notifies_the_app_on_both_builds() {
    let h = harness().await;
    assert_signed_delivery_is_verified_and_notifies_the_app("wasm", &h.wasm_conversation, &h.wasm)
        .await;
    assert_signed_delivery_is_verified_and_notifies_the_app(
        "native",
        &h.native_conversation,
        &h.native,
    )
    .await;
}

/// Permitted differences between the two builds, asserted explicitly rather
/// than left latent.
mod permitted_differences {
    use super::*;

    /// Resource lifetime: a fresh `HostState` (and therefore a fresh
    /// `ResourceTable`) is built per invocation on the native build, exactly
    /// as the sandbox builds a fresh `Store` per guest call. The
    /// WASM side cannot even express this (the fixture has one verb, and a
    /// resource never crosses a `run` call); asserted natively, where
    /// `NativeHostFactory::host_for` is separately reachable per call. Two
    /// independent invocations opening an upload each land at table index 0
    /// (`rep` 0) in their own fresh table -- if invocations shared one
    /// table, the second `open_upload` would land at index 1.
    #[tokio::test]
    async fn each_native_invocation_gets_a_fresh_resource_table() {
        let dir = tempfile::tempdir().unwrap();
        let (_, factory, _, _) = build_native_stack(dir.path());
        use syneroym_app_host::{AppBlobStore, AppBlobWriter};

        let host_a = factory.host_for(caller());
        let writer_a = host_a.open_upload().await.unwrap();
        assert_eq!(
            writer_a.rep(),
            0,
            "invocation a's writer should be the first entry in its own fresh table"
        );
        let hash_a = {
            let mut w = writer_a;
            w.write(b"invocation a".to_vec()).await.unwrap();
            w.finish().await.unwrap()
        };

        let host_b = factory.host_for(caller());
        let writer_b = host_b.open_upload().await.unwrap();
        assert_eq!(
            writer_b.rep(),
            0,
            "invocation b's writer should also be index 0 -- a shared table would put it at 1"
        );
        let hash_b = {
            let mut w = writer_b;
            w.write(b"invocation b".to_vec()).await.unwrap();
            w.finish().await.unwrap()
        };

        // Belt and suspenders: both blobs are also separately retrievable
        // afterward, proving neither invocation's table state leaked into
        // or clobbered the other's.
        assert_ne!(hash_a, hash_b);
        assert_eq!(host_a.get_blob(hash_a).await.unwrap(), b"invocation a");
        assert_eq!(host_b.get_blob(hash_b).await.unwrap(), b"invocation b");
    }

    /// Subscription persistence: the WASM build's subscription is written
    /// to `messaging_subscriptions` and replayed at boot; the native build
    /// deliberately writes nothing (see `NativeHostFactory::subscribe`'s own
    /// doc comment for why). Asserted on `StorageProvider` state directly,
    /// not via a restart simulation --
    /// `replay_persisted_subscriptions` is private to `syneroym-substrate`.
    /// Tracked in the deferred backlog as the native build's known restart
    /// gap.
    #[tokio::test]
    async fn only_the_wasm_stacks_subscription_is_persisted() {
        let h = harness().await;
        h.wasm.run(r#"{"op":"subscribe-topic","topic":"persisted"}"#).await.unwrap();
        h.native.run(r#"{"op":"subscribe-topic","topic":"persisted"}"#).await.unwrap();

        let wasm_rows =
            h.wasm_engine.storage_provider.list_all_messaging_subscriptions().await.unwrap();
        assert!(wasm_rows.iter().any(|(sid, _)| sid == SERVICE_ID));

        let native_rows =
            h.native_storage_provider.list_all_messaging_subscriptions().await.unwrap();
        assert!(native_rows.is_empty());
    }
}

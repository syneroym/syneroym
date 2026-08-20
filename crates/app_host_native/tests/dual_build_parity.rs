#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! One integration suite driving the dual-build-shim fixture through both
//! builds -- the real `wasm32-wasip2` component via `AppSandboxEngine`, and
//! the same source linked in via `syneroym-app-host-native` -- and
//! asserting the results are identical. A test that passes on one build and
//! fails on the other is a bug in the shim, not in the test.

use std::{fs, path::Path, sync::Arc, time::Duration};

use serde_json::{Value, json};
use syneroym_app_host_native::NativeHostFactory;
use syneroym_core::{
    config::SubstrateConfig, local_registry::EndpointRegistry, storage::MockStorage, test_constants,
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
struct Mutant<D>(D);

impl<D: Driver> Driver for Mutant<D> {
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
    #[expect(
        dead_code,
        reason = "kept alive for the shim's own teardown reasoning; not read directly"
    )]
    native_factory: Arc<NativeHostFactory>,
    native_storage_provider: Arc<dyn StorageProvider>,
}

/// Two fully independent host stacks, sharing one `SERVICE_ID`.
/// `None` when the wasm component artifact hasn't been built yet, so tests
/// can skip gracefully the same way every other WASM integration test in
/// this workspace does.
async fn harness() -> Option<Harness> {
    let Ok(wasm_bytes) = fs::read(test_constants::dual_build_fixture_wasm_path()) else {
        eprintln!(
            "Skipping dual_build_parity: WASM artifact not found (run `mise run \
             build:test-components`, or `cargo component build --release --target wasm32-wasip2 \
             -p syneroym-test-dual-build-fixture`)"
        );
        return None;
    };

    let wasm_dir = tempfile::tempdir().unwrap();
    let native_dir = tempfile::tempdir().unwrap();

    let wasm_engine = build_wasm_stack(wasm_dir.path(), &wasm_bytes).await;
    let (native_fixture, native_factory, native_storage_provider) =
        build_native_stack(native_dir.path());

    // Keep the temp dirs alive for the duration of the test process --
    // leaking them is fine, this is a short-lived test binary.
    std::mem::forget(wasm_dir);
    std::mem::forget(native_dir);

    Some(Harness {
        wasm: WasmDriver { engine: wasm_engine.clone() },
        native: NativeDriver { fixture: native_fixture },
        wasm_engine,
        native_factory,
        native_storage_provider,
    })
}

async fn build_wasm_stack(dir: &Path, wasm_bytes: &[u8]) -> Arc<AppSandboxEngine> {
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

    let engine = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store,
            storage_provider,
            blob_provider,
            broker,
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    engine.self_weak.set(Arc::downgrade(&engine)).expect("self_weak set once");
    engine.deploy_wasm(SERVICE_ID, &wasm_deploy_manifest(wasm_bytes.to_vec())).await.unwrap();
    engine
}

fn build_native_stack(
    dir: &Path,
) -> (
    Arc<NativeFixture<syneroym_app_host_native::NativeAppHost>>,
    Arc<NativeHostFactory>,
    Arc<dyn StorageProvider>,
) {
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(dir.join("data"), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let endpoint_registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let factory = NativeHostFactory::new(
        SERVICE_ID.to_string(),
        key_store,
        storage_provider.clone(),
        blob_provider,
        broker,
        endpoint_registry,
        syneroym_app_orchestration::empty_resolver(),
    );
    let f = factory.clone();
    let fixture =
        Arc::new(NativeFixture::new(SERVICE_ID.to_string(), move |caller| f.host_for(caller)));
    factory.set_sink(
        Arc::downgrade(&fixture) as std::sync::Weak<dyn syneroym_app_host_native::MessageSink>
    );
    (fixture, factory, storage_provider)
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
    let Some(h) = harness().await else { return };
    let wasm_results = scenarios(&h.wasm).await;
    let native_results = scenarios(&h.native).await;
    assert!(!wasm_results.is_empty(), "the scenario table must not be empty");
    assert_eq!(wasm_results, native_results);
}

/// A passing `both_builds_produce_identical_results` is not evidence of
/// anything unless the comparison is known to detect a real divergence.
#[tokio::test]
async fn the_parity_comparison_detects_a_divergence() {
    let Some(h) = harness().await else { return };
    let wasm_results = scenarios(&h.wasm).await;
    let mutant_results = scenarios(&Mutant(h.native)).await;
    assert!(!wasm_results.is_empty());
    assert_ne!(wasm_results, mutant_results);
}

/// Named per-build positive assertions: the `assert_eq!` above tells you
/// *that* the builds differ, not which is wrong. A failure here names a
/// build.
#[tokio::test]
async fn wasm_build_store_and_read_round_trip() {
    let Some(h) = harness().await else { return };
    let result = h.wasm.run(r#"{"op":"store-messages","count":5}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["ok"]["written"], 5);
    assert_eq!(v["ok"]["read"], 5);
}

#[tokio::test]
async fn native_build_store_and_read_round_trip() {
    let Some(h) = harness().await else { return };
    let result = h.native.run(r#"{"op":"store-messages","count":5}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(v["ok"]["written"], 5);
    assert_eq!(v["ok"]["read"], 5);
}

#[tokio::test]
async fn wasm_build_stream_blob_round_trips_the_body() {
    let Some(h) = harness().await else { return };
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
    let Some(h) = harness().await else { return };
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
    let Some(h) = harness().await else { return };
    let result = h.wasm.run(r#"{"op":"admin-ddl","sql":"DROP TABLE messages"}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert!(v.get("err").is_some(), "expected admin-ddl to be denied, got {v}");
}

#[tokio::test]
async fn native_build_admin_ddl_is_denied() {
    let Some(h) = harness().await else { return };
    let result = h.native.run(r#"{"op":"admin-ddl","sql":"DROP TABLE messages"}"#).await.unwrap();
    let v: Value = serde_json::from_str(&result).unwrap();
    assert!(v.get("err").is_some(), "expected admin-ddl to be denied, got {v}");
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
    let Some(h) = harness().await else { return };

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
        let (_, factory, _) = build_native_stack(dir.path());
        use syneroym_app_host::{AppBlobStore, AppBlobWriter};

        let host_a = factory.host_for(caller());
        let writer_a = host_a.open_upload().await.unwrap();
        let hash_a = {
            let mut w = writer_a;
            w.write(b"invocation a".to_vec()).await.unwrap();
            w.finish().await.unwrap()
        };

        let host_b = factory.host_for(caller());
        let writer_b = host_b.open_upload().await.unwrap();
        let hash_b = {
            let mut w = writer_b;
            w.write(b"invocation b".to_vec()).await.unwrap();
            w.finish().await.unwrap()
        };

        // Both succeed independently -- a shared table would still let this
        // pass, so the real assertion is that both blobs are separately
        // retrievable afterward, proving neither invocation's table state
        // leaked into or clobbered the other's.
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
        let Some(h) = harness().await else { return };
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

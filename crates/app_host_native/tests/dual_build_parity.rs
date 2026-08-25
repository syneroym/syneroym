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
use syneroym_app_host::types::http::{CallerAuth, CallerIdentity, HttpRequest, HttpResponse};
use syneroym_app_host_native::{
    ConversationSink, HttpSink, MessageSink, NativeHostFactory, NativeHttpAdapter, WebSocketSink,
};
use syneroym_app_orchestration::{
    AppInstanceId, AppRegistry, LogicalResolver, LogicalServiceName, ServiceId, StaticInventory,
    TopologyEntry, TopologyEpoch, TopologyKey, TopologyMode,
};
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
use syneroym_rpc::{
    AuthLevel, CallerContext, ConversationError, ConversationHost, JsonRpcRequest,
    NativeHttpService, NativeInvocation, ProxyError, ProxyRequest, ServiceProxy, SessionContext,
    WebSocketSenders,
};
use syneroym_sandbox_wasm::{AppSandboxEngine, GuestHttpOutcome};
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

/// The identity the *other* stack's `ConversationService` answers under
/// when it is standing in as a peer for `SERVICE_ID` (see `PeerProxy`).
/// Must differ from `SERVICE_ID`: both builds share that constant, and a
/// service cannot be its own group peer -- `peer_deliver_impl`'s
/// self-injection guard (`author == svc`) and `group_push_impl`/
/// `group_sync_impl`'s equivalent (`req.from.address == svc`) both refuse
/// on purpose if the two ever collide.
const PEER_SERVICE_ID: &str = "dual-build-fixture-parity-peer";

/// A minimal `ServiceProxy`: every outbound conversation call one build's
/// `ConversationService` makes is answered by calling the matching method
/// directly on the *other* build's `ConversationService` object, addressed
/// as `PEER_SERVICE_ID` rather than `SERVICE_ID` (see that constant's own
/// doc). `target_service`/`interface` are not consulted -- there is only
/// ever one peer relationship in this harness, so no routing table is
/// needed, matching how `synsvc_native.rs`'s `dispatch_conversation` maps
/// these same four methods for a real peer call.
struct PeerProxy {
    target: Arc<ConversationService>,
}

impl std::fmt::Debug for PeerProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerProxy").field("target", &PEER_SERVICE_ID).finish()
    }
}

fn conversation_error_to_proxy_error(e: ConversationError) -> ProxyError {
    match e {
        ConversationError::PermissionDenied => ProxyError::PermissionDenied(e.to_string()),
        _ => ProxyError::Callee { code: -1, message: e.to_string(), data: None },
    }
}

#[async_trait::async_trait]
impl ServiceProxy for PeerProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        let requester_did = request.caller.caller_did.as_str();
        let result_bytes = match request.method.as_str() {
            "prekey-bundle" => self
                .target
                .prekey_bundle(PEER_SERVICE_ID, requester_did)
                .await
                .map_err(conversation_error_to_proxy_error)?,
            "deliver" => {
                let envelope = serde_json::to_vec(&request.params)
                    .map_err(|e| ProxyError::Internal(e.to_string()))?;
                self.target
                    .peer_deliver(PEER_SERVICE_ID, requester_did, envelope)
                    .await
                    .map_err(conversation_error_to_proxy_error)?
            }
            "group-push" => {
                let payload = serde_json::to_vec(&request.params)
                    .map_err(|e| ProxyError::Internal(e.to_string()))?;
                self.target
                    .group_push(PEER_SERVICE_ID, requester_did, payload)
                    .await
                    .map_err(conversation_error_to_proxy_error)?
            }
            "group-sync" => {
                let payload = serde_json::to_vec(&request.params)
                    .map_err(|e| ProxyError::Internal(e.to_string()))?;
                self.target
                    .group_sync(PEER_SERVICE_ID, requester_did, payload)
                    .await
                    .map_err(conversation_error_to_proxy_error)?
            }
            other => {
                return Err(ProxyError::UnsupportedTarget(format!(
                    "PeerProxy has no stub for method {other}"
                )));
            }
        };
        serde_json::from_slice(&result_bytes).map_err(|e| ProxyError::Internal(e.to_string()))
    }
}

#[derive(Debug)]
struct StubProxy;

#[async_trait::async_trait]
impl ServiceProxy for StubProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        if request.interface == "greeter" && request.method == "greet" {
            Ok(json!({"greeting": "hello from stub"}))
        } else {
            Err(ProxyError::UnsupportedTarget(format!(
                "StubProxy has no handler for {}.{}",
                request.interface, request.method
            )))
        }
    }
}

trait HttpDriver {
    async fn get(&self, path: &str, caller: Option<CallerContext>) -> HttpResponse;
}

struct WasmHttpDriver {
    engine: Arc<AppSandboxEngine>,
}

impl HttpDriver for WasmHttpDriver {
    async fn get(&self, path: &str, caller: Option<CallerContext>) -> HttpResponse {
        let req = HttpRequest {
            method: "GET".to_string(),
            path: path.to_string(),
            query: String::new(),
            route: path.to_string(),
            path_params: vec![],
            headers: vec![],
            body: vec![],
            caller: caller.as_ref().map(|c| CallerIdentity {
                did: c.caller_did.clone(),
                auth: if matches!(c.auth, AuthLevel::Ucan) {
                    CallerAuth::Ucan
                } else {
                    CallerAuth::SelfAsserted
                },
                app_instance: c.app_instance.clone(),
            }),
        };
        match self.engine.handle_guest_http_request(SERVICE_ID, &req, caller).await {
            Ok(GuestHttpOutcome::Response(r)) => r,
            Ok(GuestHttpOutcome::Failed(f)) => panic!("wasm http driver failure: {f:?}"),
            Err(e) => panic!("wasm http driver error: {e:?}"),
        }
    }
}

struct NativeHttpDriver {
    adapter: Arc<NativeHttpAdapter>,
}

impl HttpDriver for NativeHttpDriver {
    async fn get(&self, path: &str, caller: Option<CallerContext>) -> HttpResponse {
        let req = HttpRequest {
            method: "GET".to_string(),
            path: path.to_string(),
            query: String::new(),
            route: path.to_string(),
            path_params: vec![],
            headers: vec![],
            body: vec![],
            caller: caller.as_ref().map(|c| CallerIdentity {
                did: c.caller_did.clone(),
                auth: if matches!(c.auth, AuthLevel::Ucan) {
                    CallerAuth::Ucan
                } else {
                    CallerAuth::SelfAsserted
                },
                app_instance: c.app_instance.clone(),
            }),
        };
        self.adapter.handle_request(req, caller).await.expect("native http driver")
    }
}

/// Everything one full harness setup produces, for tests that need to poke
/// past the `Driver` abstraction (e.g. asserting on persisted storage
/// state).
struct Harness {
    wasm: WasmDriver,
    native: NativeDriver,
    wasm_http: WasmHttpDriver,
    native_http: NativeHttpDriver,
    wasm_engine: Arc<AppSandboxEngine>,
    native_factory: Arc<NativeHostFactory>,
    native_storage_provider: Arc<dyn StorageProvider>,
    wasm_ws_senders: Arc<WebSocketSenders>,
    native_ws_senders: Arc<WebSocketSenders>,
    /// Each stack's own `ConversationService`, for tests
    /// that drive the peer-facing side (`prekey_bundle`/`peer_deliver`)
    /// directly rather than through the guest `run()` surface.
    wasm_conversation: Arc<ConversationService>,
    native_conversation: Arc<ConversationService>,
    /// Kept alive so the `Weak<dyn ServiceProxy>` each `ConversationService`
    /// holds (via `set_service_proxy`) does not dangle -- `wasm_conversation`
    /// calls out through `_wasm_peer_proxy` and reaches `native_conversation`
    /// (as `PEER_SERVICE_ID`), and vice versa.
    _wasm_peer_proxy: Arc<PeerProxy>,
    _native_peer_proxy: Arc<PeerProxy>,
    _stub_proxy: Arc<StubProxy>,
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

    let stub_proxy = Arc::new(StubProxy);
    let (wasm_engine, wasm_conversation, wasm_ws_senders) =
        build_wasm_stack(wasm_dir.path(), &wasm_bytes, &stub_proxy).await;
    let (
        native_fixture,
        native_factory,
        native_storage_provider,
        native_conversation,
        native_http_adapter,
        native_ws_senders,
    ) = build_native_stack(native_dir.path(), &stub_proxy).await;

    // Each stack calls out through a proxy that reaches straight into the
    // *other* stack's own `ConversationService` -- see `PeerProxy`.
    let wasm_peer_proxy = Arc::new(PeerProxy { target: native_conversation.clone() });
    let native_peer_proxy = Arc::new(PeerProxy { target: wasm_conversation.clone() });
    wasm_conversation.set_service_proxy(Arc::downgrade(&wasm_peer_proxy) as Weak<dyn ServiceProxy>);
    native_conversation
        .set_service_proxy(Arc::downgrade(&native_peer_proxy) as Weak<dyn ServiceProxy>);

    Harness {
        wasm: WasmDriver { engine: wasm_engine.clone() },
        native: NativeDriver { fixture: native_fixture },
        wasm_http: WasmHttpDriver { engine: wasm_engine.clone() },
        native_http: NativeHttpDriver { adapter: native_http_adapter },
        wasm_engine,
        native_factory,
        native_storage_provider,
        wasm_ws_senders,
        native_ws_senders,
        wasm_conversation,
        native_conversation,
        _wasm_peer_proxy: wasm_peer_proxy,
        _native_peer_proxy: native_peer_proxy,
        _stub_proxy: stub_proxy,
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

/// `call_peer`'s `check_outbound_identity` refuses up front unless the
/// caller holds both an instance certificate and a recorded owner for
/// `service_id` -- real requirements for presenting that service's own
/// identity to a peer, not exercised by any test before this one since
/// every existing group op either targets `self` (refused earlier) or a
/// group with no other member (skipped before any outbound call).
async fn install_outbound_identity(registry: &EndpointRegistry, service_id: &str) {
    let master = syneroym_identity::Identity::generate().unwrap();
    let instance = syneroym_identity::Identity::generate().unwrap();
    let mut cert = syneroym_identity::DelegationCertificate::issue(
        &master,
        instance.public_key(),
        3600,
        syneroym_identity::delegation::SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    cert.temporary_did = service_id.to_string();
    registry.set_instance_cert(service_id.to_string(), cert).await.unwrap();
    registry
        .set_owner(
            service_id.to_string(),
            syneroym_identity::substrate::derive_did_key(&master.public_key()),
        )
        .await
        .unwrap();
}

async fn build_wasm_stack(
    dir: &Path,
    wasm_bytes: &[u8],
    stub_proxy: &Arc<StubProxy>,
) -> (Arc<AppSandboxEngine>, Arc<ConversationService>, Arc<WebSocketSenders>) {
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
    key_store.inject_kek([0x42; 32]).expect("inject kek");
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(&config.storage.db_dir, true).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    // `MqttBroker::new` opens no listener, so a second in-process instance
    // per stack costs nothing and binds no port.
    let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    install_outbound_identity(&registry, SERVICE_ID).await;

    let app_instance = AppInstanceId::new("test-app");
    let sibling_name = LogicalServiceName::new("sibling");
    let inventory = Arc::new(StaticInventory::new());
    inventory.register(
        TopologyKey::local(app_instance.clone(), sibling_name),
        TopologyEntry {
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new("did:key:zSiblingMember")],
            sharding_strategy: None,
            epoch: TopologyEpoch(1),
            cache_ttl: Duration::from_secs(60),
            not_after: None,
        },
    );
    let resolver = Arc::new(LogicalResolver::new(inventory));
    registry
        .set_app_context(SERVICE_ID.to_string(), app_instance.to_string(), "self".to_string())
        .await
        .unwrap();

    storage_provider
        .save_config_generation(
            SERVICE_ID,
            r#"{"greeting":"hello","db.host":"x","db.port":"5432"}"#,
        )
        .await
        .unwrap();
    storage_provider
        .open_service_db(SERVICE_ID, &key_store)
        .await
        .unwrap()
        .write_secret("known", b"top-secret")
        .await
        .unwrap();

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
            resolver,
        )
        .await
        .unwrap(),
    );
    engine.self_weak.set(Arc::downgrade(&engine)).expect("self_weak set once");
    let ws_senders = WebSocketSenders::new();
    engine.websocket_senders.set(ws_senders.clone()).expect("set ws senders");
    engine
        .service_proxy
        .set(Arc::downgrade(stub_proxy) as Weak<dyn ServiceProxy>)
        .expect("set service proxy");
    engine
        .conversation
        .set(Arc::downgrade(&conversation) as std::sync::Weak<dyn syneroym_rpc::ConversationHost>)
        .expect("conversation set once");
    conversation.set_notifier(
        Arc::downgrade(&engine) as std::sync::Weak<dyn syneroym_rpc::ConversationNotifier>
    );
    engine.deploy_wasm(SERVICE_ID, &wasm_deploy_manifest(wasm_bytes.to_vec())).await.unwrap();
    (engine, conversation, ws_senders)
}

type NativeStack = (
    Arc<NativeFixture<syneroym_app_host_native::NativeAppHost>>,
    Arc<NativeHostFactory>,
    Arc<dyn StorageProvider>,
    Arc<ConversationService>,
    Arc<NativeHttpAdapter>,
    Arc<WebSocketSenders>,
);

async fn build_native_stack(dir: &Path, stub_proxy: &Arc<StubProxy>) -> NativeStack {
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([0x42; 32]).expect("inject kek");
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(dir.join("data"), true).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let endpoint_registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    install_outbound_identity(&endpoint_registry, SERVICE_ID).await;

    let app_instance = AppInstanceId::new("test-app");
    let sibling_name = LogicalServiceName::new("sibling");
    let inventory = Arc::new(StaticInventory::new());
    inventory.register(
        TopologyKey::local(app_instance.clone(), sibling_name),
        TopologyEntry {
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new("did:key:zSiblingMember")],
            sharding_strategy: None,
            epoch: TopologyEpoch(1),
            cache_ttl: Duration::from_secs(60),
            not_after: None,
        },
    );
    let resolver = Arc::new(LogicalResolver::new(inventory));
    endpoint_registry
        .set_app_context(SERVICE_ID.to_string(), app_instance.to_string(), "self".to_string())
        .await
        .unwrap();

    storage_provider
        .save_config_generation(
            SERVICE_ID,
            r#"{"greeting":"hello","db.host":"x","db.port":"5432"}"#,
        )
        .await
        .unwrap();
    storage_provider
        .open_service_db(SERVICE_ID, &key_store)
        .await
        .unwrap()
        .write_secret("known", b"top-secret")
        .await
        .unwrap();

    let conversation = test_conversation_service(
        storage_provider.clone(),
        key_store.clone(),
        endpoint_registry.clone(),
    );

    let ws_senders = WebSocketSenders::new();
    let factory = NativeHostFactory::new(
        SERVICE_ID.to_string(),
        key_store,
        storage_provider.clone(),
        blob_provider,
        broker,
        endpoint_registry,
        resolver,
        conversation.clone(),
        ws_senders.clone(),
    );
    let f = factory.clone();
    let fixture =
        Arc::new(NativeFixture::new(SERVICE_ID.to_string(), move |caller| f.host_for(caller)));
    factory.set_service_proxy(Arc::downgrade(stub_proxy) as Weak<dyn ServiceProxy>);
    factory.set_sink(Arc::downgrade(&fixture) as Weak<dyn MessageSink>);
    factory.set_conversation_sink(Arc::downgrade(&fixture) as Weak<dyn ConversationSink>);
    factory.set_http_sink(Arc::downgrade(&fixture) as Weak<dyn HttpSink>);
    factory.set_websocket_sink(Arc::downgrade(&fixture) as Weak<dyn WebSocketSink>);

    let adapter = Arc::new(NativeHttpAdapter::new(
        factory.clone(),
        Arc::downgrade(&fixture) as Weak<dyn HttpSink>,
        Arc::downgrade(&fixture) as Weak<dyn WebSocketSink>,
    ));

    (fixture, factory, storage_provider, conversation, adapter, ws_senders)
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
    // `open-direct`'s id is derived from `(SERVICE_ID,
    // peer_address)` alone -- deterministic, so unlike `send-message`
    // (whose message id includes a random nonce) it belongs in this
    // byte-comparison table.
    ("list-conversations", r#"{"op":"list-conversations"}"#),
    ("open-conversation", r#"{"op":"open-conversation","peer_address":"peer-parity-scenario"}"#),
    // `retry`/`delivery-status`/`read-history` against an id that was never
    // created are deterministic error shapes too.
    ("retry-unknown", r#"{"op":"retry-message","message":"msg:does-not-exist"}"#),
    ("delivery-status-unknown", r#"{"op":"delivery-status","message":"msg:does-not-exist"}"#),
    (
        "read-history-unknown-conversation",
        r#"{"op":"read-history","conversation":"conv:does-not-exist","limit":10}"#,
    ),
    ("members-unknown", r#"{"op":"members","conversation":"conv:does-not-exist"}"#),
    (
        "membership-history-unknown",
        r#"{"op":"membership-history","conversation":"conv:does-not-exist"}"#,
    ),
    ("sync-now-unknown", r#"{"op":"sync-now","conversation":"conv:does-not-exist"}"#),
    ("read-outbox-empty", r#"{"op":"read-outbox"}"#),
    (
        "proxy-call-self",
        r#"{"op":"proxy-call-self","service_id":"dual-build-fixture-parity","interface":"greeter","method":"greet","params":"{}"}"#,
    ),
    (
        "proxy-call-dependency",
        r#"{"op":"proxy-call-dependency","name":"sibling","interface":"greeter","method":"greet","params":"{}"}"#,
    ),
    ("proxy-unbound-dependency", r#"{"op":"proxy-call-unbound-dependency","name":"nope"}"#),
    ("proxy-enqueue-no-key", r#"{"op":"proxy-enqueue-no-key","name":"sibling"}"#),
    ("proxy-enqueue-empty-key", r#"{"op":"proxy-enqueue-empty-key","name":"sibling"}"#),
    ("read-config", r#"{"op":"read-config","key":"greeting"}"#),
    ("read-config-missing", r#"{"op":"read-config","key":"absent"}"#),
    ("read-config-section", r#"{"op":"read-config-section","prefix":"db"}"#),
    ("reveal-secret", r#"{"op":"reveal-secret","key":"known"}"#),
    ("reveal-secret-missing", r#"{"op":"reveal-secret","key":"absent"}"#),
    ("ws-send-unknown-conn", r#"{"op":"ws-send","conn":"nope","body":"hi"}"#),
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

// -- Conversation scenarios not covered by the
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

async fn assert_create_group<D: Driver>(name: &str, driver: &D) {
    let create_res = driver.run(r#"{"op":"create-group"}"#).await.unwrap();
    let create_v: Value = serde_json::from_str(&create_res).unwrap();
    let conv_id = create_v["ok"]["conversation"].as_str().unwrap();
    assert!(conv_id.starts_with("conv:"), "{name}: group id must start with conv:");

    let members_res =
        driver.run(&format!(r#"{{"op":"members","conversation":"{conv_id}"}}"#)).await.unwrap();
    let members_v: Value = serde_json::from_str(&members_res).unwrap();
    let members = members_v["ok"]["members"].as_array().unwrap();
    assert_eq!(members, &vec![json!(SERVICE_ID)], "{name}: owner must be the first member");

    let history_res = driver
        .run(&format!(r#"{{"op":"membership-history","conversation":"{conv_id}"}}"#))
        .await
        .unwrap();
    let history_v: Value = serde_json::from_str(&history_res).unwrap();
    let events = history_v["ok"]["history"].as_array().unwrap();
    assert_eq!(events.len(), 1, "{name}: genesis membership entry must exist");
    assert_eq!(events[0]["action"], "add");
    assert_eq!(events[0]["subject"], SERVICE_ID);
    assert_eq!(events[0]["epoch"], 1);

    // Test add_member on self is invalid argument
    let add_self = driver
        .run(&format!(
            r#"{{"op":"add-member","conversation":"{conv_id}","member_address":"{SERVICE_ID}"}}"#
        ))
        .await
        .unwrap();
    let add_self_v: Value = serde_json::from_str(&add_self).unwrap();
    assert!(
        add_self_v["err"].is_string(),
        "{name}: add_member for owner must return err response: {add_self}"
    );

    // Test remove_member for non-existent member is a no-op / success
    let rem_nonmember = driver
        .run(&format!(
            r#"{{"op":"remove-member","conversation":"{conv_id}","member_address":"peer-none"}}"#
        ))
        .await
        .unwrap();
    let rem_v: Value = serde_json::from_str(&rem_nonmember).unwrap();
    assert_eq!(rem_v["ok"]["removed"], true, "{name}: remove non-member succeeds as no-op");

    // Test sync_now on existing group succeeds
    let sync_res =
        driver.run(&format!(r#"{{"op":"sync-now","conversation":"{conv_id}"}}"#)).await.unwrap();
    let sync_v: Value = serde_json::from_str(&sync_res).unwrap();
    assert_eq!(sync_v["ok"]["synced"], true, "{name}: sync_now succeeds");
}

#[tokio::test]
async fn create_group_initializes_membership_and_epoch_on_both_builds() {
    let h = harness().await;
    assert_create_group("wasm", &h.wasm).await;
    assert_create_group("native", &h.native).await;
}

/// Drives `add-member` to a real second party (routed through `PeerProxy`
/// straight into the *other* stack's own `ConversationService`, so this is
/// a genuine `prekey-bundle` round trip -- an X3DH handshake, not a stub
/// response), then a group `send`, then `membership-history` -- exercising
/// exactly the group paths `assert_create_group` above could not reach
/// (its own `add-member` case is deliberately the *refused* one, adding the
/// owner to itself). `add-member`'s own network step (`fetch_prekey_bundle`)
/// is awaited inline by `change_membership_impl`, so no background worker
/// or settle delay is needed for these three assertions -- unlike group-key
/// distribution and DAG-entry relay, which are enqueued for the (unstarted,
/// in this harness) delivery worker and are not observed here.
async fn assert_group_add_member_send_and_history_on_a_populated_group<D: Driver>(
    name: &str,
    driver: &D,
) {
    let create_res = driver.run(r#"{"op":"create-group"}"#).await.unwrap();
    let create_v: Value = serde_json::from_str(&create_res).unwrap();
    let conv_id = create_v["ok"]["conversation"].as_str().unwrap().to_string();

    let add_res = driver
        .run(&format!(
            r#"{{"op":"add-member","conversation":"{conv_id}","member_address":"{PEER_SERVICE_ID}"}}"#
        ))
        .await
        .unwrap();
    let add_v: Value = serde_json::from_str(&add_res).unwrap();
    assert_eq!(
        add_v["ok"]["added"], true,
        "{name}: add-member on a real peer must succeed: {add_res}"
    );

    let members_res =
        driver.run(&format!(r#"{{"op":"members","conversation":"{conv_id}"}}"#)).await.unwrap();
    let members_v: Value = serde_json::from_str(&members_res).unwrap();
    let mut members: Vec<String> = members_v["ok"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap().to_string())
        .collect();
    members.sort();
    let mut expected = vec![SERVICE_ID.to_string(), PEER_SERVICE_ID.to_string()];
    expected.sort();
    assert_eq!(members, expected, "{name}: group must have both members after add-member");

    let send_res = driver
        .run(&format!(r#"{{"op":"send-message","conversation":"{conv_id}","body":"hello group"}}"#))
        .await
        .unwrap();
    let send_v: Value = serde_json::from_str(&send_res).unwrap();
    assert!(
        send_v["ok"]["message"].is_string(),
        "{name}: send-message on a populated group must succeed: {send_res}"
    );

    let history_res = driver
        .run(&format!(r#"{{"op":"membership-history","conversation":"{conv_id}"}}"#))
        .await
        .unwrap();
    let history_v: Value = serde_json::from_str(&history_res).unwrap();
    let events = history_v["ok"]["history"].as_array().unwrap();
    assert_eq!(
        events.len(),
        2,
        "{name}: membership history must hold the genesis and add-member events: {history_v:?}"
    );
    assert_eq!(events[0]["action"], "add");
    assert_eq!(events[0]["subject"], SERVICE_ID);
    assert_eq!(events[1]["action"], "add");
    assert_eq!(events[1]["subject"], PEER_SERVICE_ID);
}

#[tokio::test]
async fn group_add_member_send_and_history_are_identical_on_both_builds() {
    let h = harness().await;
    assert_group_add_member_send_and_history_on_a_populated_group("wasm", &h.wasm).await;
    assert_group_add_member_send_and_history_on_a_populated_group("native", &h.native).await;
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

#[tokio::test]
async fn both_builds_answer_an_http_request_identically() {
    let h = harness().await;
    let wasm_resp = h.wasm_http.get("/echo", Some(caller())).await;
    let native_resp = h.native_http.get("/echo", Some(caller())).await;
    assert_eq!(wasm_resp.status, 200);
    assert_eq!(native_resp.status, 200);
    assert_eq!(wasm_resp.body, native_resp.body);
    assert_eq!(wasm_resp.headers, native_resp.headers);
}

#[tokio::test]
async fn both_builds_render_the_same_caller_for_a_delegated_request() {
    let h = harness().await;
    let wasm_resp = h.wasm_http.get("/whoami", Some(caller())).await;
    let native_resp = h.native_http.get("/whoami", Some(caller())).await;
    assert_eq!(wasm_resp.status, 200);
    assert_eq!(native_resp.status, 200);
    assert_eq!(wasm_resp.body, native_resp.body);
}

#[tokio::test]
async fn both_builds_substitute_the_service_itself_for_an_anonymous_public_request() {
    let h = harness().await;
    let wasm_resp = h.wasm_http.get("/whoami", None).await;
    let native_resp = h.native_http.get("/whoami", None).await;
    assert_eq!(wasm_resp.status, 200);
    assert_eq!(native_resp.status, 200);
    assert_eq!(wasm_resp.body, native_resp.body);
    assert_eq!(String::from_utf8_lossy(&wasm_resp.body), "anonymous");
}

#[tokio::test]
async fn a_guest_rejection_is_an_ok_with_a_4xx_on_both_builds() {
    let h = harness().await;
    let wasm_resp = h.wasm_http.get("/reject", Some(caller())).await;
    let native_resp = h.native_http.get("/reject", Some(caller())).await;
    assert_eq!(wasm_resp.status, 403);
    assert_eq!(native_resp.status, 403);
    assert_eq!(wasm_resp.body, native_resp.body);
}

#[tokio::test]
async fn a_handler_failure_is_an_err_on_both_builds() {
    let h = harness().await;
    let req = HttpRequest {
        method: "GET".to_string(),
        path: "/fail".to_string(),
        query: String::new(),
        route: "/fail".to_string(),
        path_params: vec![],
        headers: vec![],
        body: vec![],
        caller: Some(CallerIdentity {
            did: caller().caller_did,
            auth: CallerAuth::Delegated,
            app_instance: None,
        }),
    };
    let wasm_res = h.wasm_engine.handle_guest_http_request(SERVICE_ID, &req, Some(caller())).await;
    assert!(matches!(wasm_res, Ok(GuestHttpOutcome::Failed(_)) | Err(_)));
    let native_res = h.native_http.adapter.handle_request(req, Some(caller())).await;
    assert!(native_res.is_err());
}

#[tokio::test]
async fn both_builds_deliver_websocket_frames_to_the_app() {
    let h = harness().await;
    // WASM
    h.wasm_engine.handle_websocket_on_open(SERVICE_ID, "ws-c1", Some(caller())).await;
    h.wasm_engine
        .handle_websocket_on_message(
            SERVICE_ID,
            "ws-c1",
            b"frame1".to_vec(),
            syneroym_sandbox_wasm::FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.wasm_engine
        .handle_websocket_on_message(
            SERVICE_ID,
            "ws-c1",
            b"frame1".to_vec(),
            syneroym_sandbox_wasm::FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.wasm_engine
        .handle_websocket_on_message(
            SERVICE_ID,
            "ws-c1",
            b"frame2".to_vec(),
            syneroym_sandbox_wasm::FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.wasm_engine.handle_websocket_on_close(SERVICE_ID, "ws-c1", Some(caller())).await;
    let wasm_log = h.wasm.run(r#"{"op":"read-ws-log"}"#).await.unwrap();

    // Native
    h.native_http.adapter.on_websocket_open("ws-c1".to_string(), Some(caller())).await;
    h.native_http
        .adapter
        .on_websocket_message(
            "ws-c1".to_string(),
            b"frame1".to_vec(),
            syneroym_sandbox_wasm::FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.native_http
        .adapter
        .on_websocket_message(
            "ws-c1".to_string(),
            b"frame1".to_vec(),
            syneroym_sandbox_wasm::FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.native_http
        .adapter
        .on_websocket_message(
            "ws-c1".to_string(),
            b"frame2".to_vec(),
            syneroym_sandbox_wasm::FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.native_http.adapter.on_websocket_close("ws-c1".to_string(), Some(caller())).await;
    let native_log = h.native.run(r#"{"op":"read-ws-log"}"#).await.unwrap();

    assert_eq!(wasm_log, native_log);
    let parsed: serde_json::Value = serde_json::from_str(&wasm_log).expect("parse wasm_log");
    let events = parsed["ok"]["events"].as_array().expect("events array");
    assert_eq!(events.len(), 5); // open + 3 messages + close
}

#[tokio::test]
async fn both_builds_push_a_frame_to_a_live_connection() {
    let h = harness().await;
    let mut rx_wasm = h.wasm_ws_senders.register(SERVICE_ID, "live-conn");
    let mut rx_native = h.native_ws_senders.register(SERVICE_ID, "live-conn");

    let wasm_res =
        h.wasm.run(r#"{"op":"ws-send","conn":"live-conn","body":"msg-to-live"}"#).await.unwrap();
    let native_res =
        h.native.run(r#"{"op":"ws-send","conn":"live-conn","body":"msg-to-live"}"#).await.unwrap();
    assert_eq!(wasm_res, native_res);

    let msg_wasm = rx_wasm.recv().await.unwrap();
    let msg_native = rx_native.recv().await.unwrap();
    assert_eq!(msg_wasm, msg_native);
}

#[tokio::test]
async fn a_dependency_resolves_to_the_same_target_on_both_builds() {
    let h = harness().await;
    let wasm_res = h
        .wasm
        .run(r#"{"op":"proxy-call-dependency","name":"sibling","interface":"greeter","method":"greet","params":"{}"}"#)
        .await
        .unwrap();
    let native_res = h
        .native
        .run(r#"{"op":"proxy-call-dependency","name":"sibling","interface":"greeter","method":"greet","params":"{}"}"#)
        .await
        .unwrap();
    assert_eq!(wasm_res, native_res);
}

#[tokio::test]
async fn an_enqueue_without_an_idempotency_key_is_refused_identically() {
    let h = harness().await;
    let wasm_res = h.wasm.run(r#"{"op":"proxy-enqueue-no-key","name":"sibling"}"#).await.unwrap();
    let native_res =
        h.native.run(r#"{"op":"proxy-enqueue-no-key","name":"sibling"}"#).await.unwrap();
    assert_eq!(wasm_res, native_res);
}

#[tokio::test]
async fn both_builds_read_the_same_config_generation() {
    let h = harness().await;
    let wasm_res1 = h.wasm.run(r#"{"op":"read-config","key":"greeting"}"#).await.unwrap();
    let native_res1 = h.native.run(r#"{"op":"read-config","key":"greeting"}"#).await.unwrap();
    assert_eq!(wasm_res1, native_res1);

    h.wasm_engine
        .storage_provider
        .save_config_generation(SERVICE_ID, r#"{"greeting":"hello generation 2"}"#)
        .await
        .unwrap();
    h.native_storage_provider
        .save_config_generation(SERVICE_ID, r#"{"greeting":"hello generation 2"}"#)
        .await
        .unwrap();

    let wasm_res2 = h.wasm.run(r#"{"op":"read-config","key":"greeting"}"#).await.unwrap();
    let native_res2 = h.native.run(r#"{"op":"read-config","key":"greeting"}"#).await.unwrap();
    assert_eq!(wasm_res2, native_res2);
    let v: Value = serde_json::from_str(&wasm_res2).unwrap();
    assert_eq!(v["ok"]["value"], "hello generation 2");
}

/// Permitted differences between the two builds, asserted explicitly rather
/// than left latent.
mod permitted_differences {
    use syneroym_app_host::{
        AppBlobStore, AppBlobWriter, AppDataLayer, types::data_layer::RecordWriteValue,
    };

    use super::*;

    fn abac_policy() -> String {
        r#"{
            "version": "fdae/v1",
            "definitions": {
                "profiles": {
                    "table": "profiles",
                    "principal_column": "creator_uuid",
                    "permissions": {
                        "view": {
                            "allows": ["data-layer/read"],
                            "paths": [["caller"]],
                            "authorize_rows": true
                        }
                    }
                }
            }
        }"#
        .to_string()
    }

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
        let stub = Arc::new(StubProxy);
        let (_, factory, _, _, _, _) = build_native_stack(dir.path(), &stub).await;

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

    #[tokio::test]
    async fn a_policy_with_abac_permissions_fails_closed_on_the_native_build() {
        let dir = tempfile::tempdir().unwrap();
        let stub = Arc::new(StubProxy);
        let (_, factory, storage_provider, _, _, _) = build_native_stack(dir.path(), &stub).await;
        storage_provider.save_fdae_policy(SERVICE_ID, &abac_policy()).await.unwrap();

        let host = factory.host_for(caller());
        use syneroym_app_host::AppDataLayer;
        let res = host.get("profiles".to_string(), "owned-by-alice".to_string()).await;
        assert!(res.is_err(), "native build should fail closed on ABAC policies");
    }

    struct TransientFdaeStorage {
        inner: Arc<dyn StorageProvider>,
        failed_once: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    impl StorageProvider for TransientFdaeStorage {
        async fn open_service_db(
            &self,
            service_id: &str,
            key_store: &Arc<KeyStore>,
        ) -> anyhow::Result<Box<dyn syneroym_data_db::traits::ServiceStore>> {
            self.inner.open_service_db(service_id, key_store).await
        }

        async fn rotate_kek(
            &self,
            key_store: &Arc<KeyStore>,
            new_kek: [u8; 32],
        ) -> anyhow::Result<()> {
            self.inner.rotate_kek(key_store, new_kek).await
        }

        async fn load_service_dek(
            &self,
            service_id: &str,
            key_store: &Arc<KeyStore>,
        ) -> anyhow::Result<Option<zeroize::Zeroizing<[u8; 32]>>> {
            self.inner.load_service_dek(service_id, key_store).await
        }

        async fn service_exists(&self, service_id: &str) -> anyhow::Result<bool> {
            self.inner.service_exists(service_id).await
        }

        async fn save_config_generation(
            &self,
            service_id: &str,
            config_blob: &str,
        ) -> anyhow::Result<u64> {
            self.inner.save_config_generation(service_id, config_blob).await
        }

        async fn delete_config_generation(
            &self,
            service_id: &str,
            generation: u64,
        ) -> anyhow::Result<()> {
            self.inner.delete_config_generation(service_id, generation).await
        }

        async fn get_config_generation(
            &self,
            service_id: &str,
            generation: u64,
        ) -> anyhow::Result<Option<String>> {
            self.inner.get_config_generation(service_id, generation).await
        }

        async fn get_latest_config_generation(
            &self,
            service_id: &str,
        ) -> anyhow::Result<Option<(u64, String)>> {
            self.inner.get_latest_config_generation(service_id).await
        }

        async fn save_messaging_subscription(
            &self,
            service_id: &str,
            topic: &str,
        ) -> anyhow::Result<()> {
            self.inner.save_messaging_subscription(service_id, topic).await
        }

        async fn delete_messaging_subscription(
            &self,
            service_id: &str,
            topic: &str,
        ) -> anyhow::Result<()> {
            self.inner.delete_messaging_subscription(service_id, topic).await
        }

        async fn delete_all_messaging_subscriptions_for_service(
            &self,
            service_id: &str,
        ) -> anyhow::Result<()> {
            self.inner.delete_all_messaging_subscriptions_for_service(service_id).await
        }

        async fn list_all_messaging_subscriptions(&self) -> anyhow::Result<Vec<(String, String)>> {
            self.inner.list_all_messaging_subscriptions().await
        }

        async fn save_fdae_policy(
            &self,
            service_id: &str,
            policy_json: &str,
        ) -> anyhow::Result<()> {
            self.inner.save_fdae_policy(service_id, policy_json).await
        }

        async fn load_fdae_policy(&self, service_id: &str) -> anyhow::Result<Option<String>> {
            if self.failed_once.fetch_and(false, std::sync::atomic::Ordering::SeqCst) {
                anyhow::bail!("transient storage error");
            }
            self.inner.load_fdae_policy(service_id).await
        }

        async fn delete_fdae_policy(&self, service_id: &str) -> anyhow::Result<()> {
            self.inner.delete_fdae_policy(service_id).await
        }
    }

    #[tokio::test]
    async fn a_transient_fdae_policy_load_failure_is_not_memoized() {
        let dir = tempfile::tempdir().unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([0x42; 32]).expect("inject kek");
        let raw_storage =
            Arc::new(SqliteStorageProvider::new(dir.path().join("data"), true).unwrap());
        {
            let service_store = raw_storage.open_service_db(SERVICE_ID, &key_store).await.unwrap();
            service_store
                .create_collection(&syneroym_data_db::host_store::CollectionSchema {
                    name: "profiles".to_string(),
                    indexes: vec![],
                })
                .await
                .unwrap();
            service_store
                .put(
                    "profiles",
                    &syneroym_data_db::host_store::RecordWriteValue {
                        id: "owned-by-alice".to_string(),
                        payload: br#"{"creator_uuid":"did:key:zParityTestCaller"}"#.to_vec(),
                    },
                    &caller().caller_did,
                    None,
                )
                .await
                .unwrap();
        }
        raw_storage.save_fdae_policy(SERVICE_ID, &abac_policy()).await.unwrap();

        let storage_provider: Arc<dyn StorageProvider> = Arc::new(TransientFdaeStorage {
            inner: raw_storage,
            failed_once: std::sync::atomic::AtomicBool::new(true),
        });

        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let endpoint_registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
        install_outbound_identity(&endpoint_registry, SERVICE_ID).await;

        let app_instance = AppInstanceId::new("test-app");
        let sibling_name = LogicalServiceName::new("sibling");
        let inventory = Arc::new(StaticInventory::new());
        inventory.register(
            TopologyKey::local(app_instance.clone(), sibling_name),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![ServiceId::new("did:key:zSiblingMember")],
                sharding_strategy: None,
                epoch: TopologyEpoch(1),
                cache_ttl: Duration::from_secs(60),
                not_after: None,
            },
        );
        let resolver = Arc::new(LogicalResolver::new(inventory));
        endpoint_registry
            .set_app_context(SERVICE_ID.to_string(), app_instance.to_string(), "self".to_string())
            .await
            .unwrap();

        let conversation = test_conversation_service(
            storage_provider.clone(),
            key_store.clone(),
            endpoint_registry.clone(),
        );
        let stub = Arc::new(StubProxy);
        let factory = NativeHostFactory::new(
            SERVICE_ID.to_string(),
            key_store,
            storage_provider,
            blob_provider,
            broker,
            endpoint_registry,
            resolver,
            conversation,
            WebSocketSenders::new(),
        );
        factory.set_service_proxy(Arc::downgrade(&stub) as Weak<dyn ServiceProxy>);

        let host1 = factory.host_for(caller());
        // First load attempts to read FDAE policy, but TransientFdaeStorage returns an
        // Err. The error is not memoized, so host1 sees absent policy and put
        // succeeds.
        let res1 = host1
            .put(
                "profiles".to_string(),
                RecordWriteValue { id: "p1".to_string(), payload: b"{}".to_vec() },
            )
            .await;
        assert!(res1.is_ok());

        let host2 = factory.host_for(caller());
        // Second load attempts to read FDAE policy and succeeds. The ABAC policy is
        // memoized and denies write permissions to caller.
        let res2 = host2
            .put(
                "profiles".to_string(),
                RecordWriteValue { id: "p2".to_string(), payload: b"{}".to_vec() },
            )
            .await;
        assert!(res2.is_err());

        let host3 = factory.host_for(caller());
        // Third load uses the memoized policy and also denies write permissions.
        let res3 = host3
            .put(
                "profiles".to_string(),
                RecordWriteValue { id: "p3".to_string(), payload: b"{}".to_vec() },
            )
            .await;
        assert!(res3.is_err());
    }

    #[tokio::test]
    async fn a_cached_fdae_policy_can_be_invalidated_and_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let key_store = Arc::new(KeyStore::new());
        key_store.inject_kek([0x42; 32]).expect("inject kek");
        let storage_provider: Arc<dyn StorageProvider> =
            Arc::new(SqliteStorageProvider::new(dir.path().join("data"), true).unwrap());
        {
            let service_store =
                storage_provider.open_service_db(SERVICE_ID, &key_store).await.unwrap();
            service_store
                .create_collection(&syneroym_data_db::host_store::CollectionSchema {
                    name: "profiles".to_string(),
                    indexes: vec![],
                })
                .await
                .unwrap();
        }
        // Initially no policy -> write succeeds
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let endpoint_registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
        install_outbound_identity(&endpoint_registry, SERVICE_ID).await;

        let app_instance = AppInstanceId::new("test-app");
        let sibling_name = LogicalServiceName::new("sibling");
        let inventory = Arc::new(StaticInventory::new());
        inventory.register(
            TopologyKey::local(app_instance.clone(), sibling_name),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![ServiceId::new("did:key:zSiblingMember")],
                sharding_strategy: None,
                epoch: TopologyEpoch(1),
                cache_ttl: Duration::from_secs(60),
                not_after: None,
            },
        );
        let resolver = Arc::new(LogicalResolver::new(inventory));
        endpoint_registry
            .set_app_context(SERVICE_ID.to_string(), app_instance.to_string(), "self".to_string())
            .await
            .unwrap();

        let conversation = test_conversation_service(
            storage_provider.clone(),
            key_store.clone(),
            endpoint_registry.clone(),
        );
        let stub = Arc::new(StubProxy);
        let factory = NativeHostFactory::new(
            SERVICE_ID.to_string(),
            key_store,
            storage_provider.clone(),
            blob_provider,
            broker,
            endpoint_registry,
            resolver,
            conversation,
            WebSocketSenders::new(),
        );
        factory.set_service_proxy(Arc::downgrade(&stub) as Weak<dyn ServiceProxy>);

        let host1 = factory.host_for(caller());
        let res1 = host1
            .put(
                "profiles".to_string(),
                RecordWriteValue { id: "p1".to_string(), payload: b"{}".to_vec() },
            )
            .await;
        assert!(res1.is_ok());

        // Now save an ABAC policy into storage. Because policy is cached as None,
        // it would still be None without invalidation.
        storage_provider.save_fdae_policy(SERVICE_ID, &abac_policy()).await.unwrap();

        // Invalidate FDAE policy cache and bump generation
        factory.invalidate_fdae_policy().await;

        // Fresh host should now reload policy from storage and deny write under ABAC
        let host2 = factory.host_for(caller());
        let res2 = host2
            .put(
                "profiles".to_string(),
                RecordWriteValue { id: "p2".to_string(), payload: b"{}".to_vec() },
            )
            .await;
        assert!(res2.is_err());
    }
}

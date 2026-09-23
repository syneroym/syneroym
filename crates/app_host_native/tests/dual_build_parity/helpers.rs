pub(crate) use std::{
    fs,
    path::Path,
    sync::{Arc, Weak, atomic::Ordering},
    time::Duration,
};

pub(crate) use serde_json::{Value, json};
pub(crate) use syneroym_app_host::{
    AppDataLayer,
    types::{
        data_layer::RecordWriteValue,
        http::{CallerAuth, CallerIdentity, FrameKind, HttpRequest, HttpResponse},
    },
};
pub(crate) use syneroym_app_host_native::{
    ConversationSink, HttpSink, MessageSink, NativeAppHost, NativeHostFactory, NativeHttpAdapter,
    WebSocketSink,
};
pub(crate) use syneroym_app_orchestration::{
    AppInstanceId, AppRegistry, LogicalResolver, LogicalServiceName, ServiceId, StaticInventory,
    TopologyEntry, TopologyEpoch, TopologyKey, TopologyMode,
};
pub(crate) use syneroym_async_queue::QueueConfig;
pub(crate) use syneroym_conversation::{ConversationConfig, ConversationService};
pub(crate) use syneroym_core::{
    config::{RetryPolicy, SubstrateConfig},
    local_registry::EndpointRegistry,
    record_signer::{NodeRecordSigner, RecordClock},
    storage::MockStorage,
    test_constants,
};
pub(crate) use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
pub(crate) use syneroym_data_db::{
    SqliteStorageProvider, StorageProvider,
    host_store::{CollectionSchema as DbCollectionSchema, RecordWriteValue as DbRecordWriteValue},
};
pub(crate) use syneroym_data_keystore::KeyStore;
pub(crate) use syneroym_identity::{
    DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE, substrate::derive_did_key,
};
pub(crate) use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
pub(crate) use syneroym_rpc::{
    AuthLevel, CallerContext, ConversationError, ConversationHost, ConversationNotifier,
    JsonRpcRequest, NativeHttpService, NativeInvocation, ProxyError, ProxyRequest, ServiceProxy,
    SessionContext, WebSocketSenders,
};
pub(crate) use syneroym_sandbox_wasm::{AppSandboxEngine, GuestHttpOutcome};
pub(crate) use syneroym_test_dual_build_fixture::native::{FIXTURE_INTERFACE, NativeFixture};
pub(crate) use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};

/// The two builds must share one service id, and must therefore share
/// nothing else -- it is the store namespace, the broker topic namespace,
/// and the `data-layer/admin` gate resource, all at once.
pub(crate) const SERVICE_ID: &str = "dual-build-fixture-parity";

/// Both builds run under a real, identical, non-anonymous caller -- the
/// router's own distinct treatment of an anonymous caller per interface
/// kind is a router concern, out of scope for this shim-parity suite.
pub(crate) fn caller() -> CallerContext {
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

pub(crate) trait Driver {
    async fn run(&self, request: &str) -> Result<String, String>;
}

pub(crate) fn caller_with_did(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext { subject_did: did.to_string(), ..Default::default() },
        auth: AuthLevel::Ucan,
        proof: None,
    }
}

/// Drives the component through the real sandbox engine.
pub(crate) struct WasmDriver {
    pub(crate) engine: Arc<AppSandboxEngine>,
}

impl WasmDriver {
    pub(crate) async fn run_with_caller(
        &self,
        request: &str,
        caller: CallerContext,
    ) -> Result<String, String> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "run".to_string(),
            params: json!([request]),
            id: None,
            idempotency_key: None,
        };
        let result = self
            .engine
            .execute_wasm_json(SERVICE_ID, FIXTURE_INTERFACE, &req, Some(caller))
            .await
            .map_err(|e| e.to_string())?;
        match result {
            Value::String(s) => Ok(s),
            other => Err(format!("expected a string result, got {other:?}")),
        }
    }
}

impl Driver for WasmDriver {
    async fn run(&self, request: &str) -> Result<String, String> {
        self.run_with_caller(request, caller()).await
    }
}

/// Drives the same source, linked in, through the shim.
pub(crate) struct NativeDriver {
    pub(crate) fixture: Arc<NativeFixture<NativeAppHost>>,
}

impl NativeDriver {
    pub(crate) async fn run_with_caller(
        &self,
        request: &str,
        caller: CallerContext,
    ) -> Result<String, String> {
        use syneroym_rpc::NativeService;
        let inv = NativeInvocation {
            interface: "test-driver".to_string(),
            method: "run".to_string(),
            params: json!([request]),
            caller,
        };
        let response = self.fixture.dispatch(inv).await.map_err(|e| e.to_string())?;
        match response.payload {
            Value::String(s) => Ok(s),
            other => Err(format!("expected a string result, got {other:?}")),
        }
    }
}

impl Driver for NativeDriver {
    async fn run(&self, request: &str) -> Result<String, String> {
        self.run_with_caller(request, caller()).await
    }
}

/// Wraps another driver and corrupts one field of its result. Exists purely
/// to prove the parity comparison detects a divergence -- if
/// `the_parity_comparison_detects_a_divergence` ever passes with this
/// removed, `both_builds_produce_identical_results` is not comparing
/// anything.
pub(crate) struct Mutant<'a, D>(pub(crate) &'a D);

impl<D: Driver> Driver for Mutant<'_, D> {
    async fn run(&self, request: &str) -> Result<String, String> {
        self.0.run(request).await.map(|s| s.replace("\"written\"", "\"wrote\""))
    }
}

pub(crate) fn wasm_deploy_manifest(bytes: Vec<u8>) -> DeployManifest {
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
pub(crate) const PEER_SERVICE_ID: &str = "dual-build-fixture-parity-peer";

/// A minimal `ServiceProxy`: every outbound conversation call one build's
/// `ConversationService` makes is answered by calling the matching method
/// directly on the *other* build's `ConversationService` object, addressed
/// as `PEER_SERVICE_ID` rather than `SERVICE_ID` (see that constant's own
/// doc). `target_service`/`interface` are not consulted -- there is only
/// ever one peer relationship in this harness, so no routing table is
/// needed, matching how `synsvc_native.rs`'s `dispatch_conversation` maps
/// these same four methods for a real peer call.
pub(crate) struct PeerProxy {
    pub(crate) target: Arc<ConversationService>,
}

impl std::fmt::Debug for PeerProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerProxy").field("target", &PEER_SERVICE_ID).finish()
    }
}

pub(crate) fn conversation_error_to_proxy_error(e: ConversationError) -> ProxyError {
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
pub(crate) struct StubProxy;

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

pub(crate) trait HttpDriver {
    async fn get(&self, path: &str, caller: Option<CallerContext>) -> HttpResponse;
    async fn post(
        &self,
        path_and_query: &str,
        body: Vec<u8>,
        caller: Option<CallerContext>,
    ) -> HttpResponse;
}

pub(crate) fn split_path_and_query(path_and_query: &str) -> (String, String) {
    if let Some((p, q)) = path_and_query.split_once('?') {
        (p.to_string(), q.to_string())
    } else {
        (path_and_query.to_string(), String::new())
    }
}

pub(crate) struct WasmHttpDriver {
    pub(crate) engine: Arc<AppSandboxEngine>,
}

impl HttpDriver for WasmHttpDriver {
    async fn get(&self, path: &str, caller: Option<CallerContext>) -> HttpResponse {
        let (path_str, query_str) = split_path_and_query(path);
        let req = HttpRequest {
            method: "GET".to_string(),
            path: path_str.clone(),
            query: query_str,
            route: path_str,
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

    async fn post(
        &self,
        path_and_query: &str,
        body: Vec<u8>,
        caller: Option<CallerContext>,
    ) -> HttpResponse {
        let (path_str, query_str) = split_path_and_query(path_and_query);
        let req = HttpRequest {
            method: "POST".to_string(),
            path: path_str.clone(),
            query: query_str,
            route: path_str,
            path_params: vec![],
            headers: vec![],
            body,
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

pub(crate) struct NativeHttpDriver {
    pub(crate) adapter: Arc<NativeHttpAdapter>,
}

impl HttpDriver for NativeHttpDriver {
    async fn get(&self, path: &str, caller: Option<CallerContext>) -> HttpResponse {
        let (path_str, query_str) = split_path_and_query(path);
        let req = HttpRequest {
            method: "GET".to_string(),
            path: path_str.clone(),
            query: query_str,
            route: path_str,
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

    async fn post(
        &self,
        path_and_query: &str,
        body: Vec<u8>,
        caller: Option<CallerContext>,
    ) -> HttpResponse {
        let (path_str, query_str) = split_path_and_query(path_and_query);
        let req = HttpRequest {
            method: "POST".to_string(),
            path: path_str.clone(),
            query: query_str,
            route: path_str,
            path_params: vec![],
            headers: vec![],
            body,
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
pub(crate) struct Harness {
    pub(crate) wasm: WasmDriver,
    pub(crate) native: NativeDriver,
    pub(crate) wasm_http: WasmHttpDriver,
    pub(crate) native_http: NativeHttpDriver,
    pub(crate) master_identity: Arc<Identity>,
    pub(crate) _node_identity: Arc<Identity>,
    pub(crate) wasm_engine: Arc<AppSandboxEngine>,
    pub(crate) native_factory: Arc<NativeHostFactory>,
    pub(crate) native_storage_provider: Arc<dyn StorageProvider>,
    pub(crate) wasm_ws_senders: Arc<WebSocketSenders>,
    pub(crate) native_ws_senders: Arc<WebSocketSenders>,
    /// Each stack's own `ConversationService`, for tests
    /// that drive the peer-facing side (`prekey_bundle`/`peer_deliver`)
    /// directly rather than through the guest `run()` surface.
    pub(crate) wasm_conversation: Arc<ConversationService>,
    pub(crate) native_conversation: Arc<ConversationService>,
    /// Kept alive so the `Weak<dyn ServiceProxy>` each `ConversationService`
    /// holds (via `set_service_proxy`) does not dangle -- `wasm_conversation`
    /// calls out through `_wasm_peer_proxy` and reaches `native_conversation`
    /// (as `PEER_SERVICE_ID`), and vice versa.
    pub(crate) _wasm_peer_proxy: Arc<PeerProxy>,
    pub(crate) _native_peer_proxy: Arc<PeerProxy>,
    pub(crate) _stub_proxy: Arc<StubProxy>,
    // Dropped last (declaration order), after everything that might still
    // have files open under them.
    pub(crate) _wasm_dir: tempfile::TempDir,
    pub(crate) _native_dir: tempfile::TempDir,
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
/// evidence for dual-build parity, so a run that silently skipped every
/// test would be worse than a build failure, not equivalent to one. Build
/// it with `mise run build:test-components`.
pub(crate) async fn harness() -> Harness {
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
    let node_identity = Arc::new(Identity::generate().unwrap());
    let master_identity = Arc::new(Identity::generate().unwrap());
    let clock = RecordClock::Fixed(1_800_000_000);

    let (wasm_engine, wasm_conversation, wasm_ws_senders) = build_wasm_stack(
        wasm_dir.path(),
        &wasm_bytes,
        &stub_proxy,
        node_identity.clone(),
        &master_identity,
        clock,
    )
    .await;
    let (
        native_fixture,
        native_factory,
        native_storage_provider,
        native_conversation,
        native_http_adapter,
        native_ws_senders,
    ) = build_native_stack(
        native_dir.path(),
        &stub_proxy,
        node_identity.clone(),
        &master_identity,
        clock,
    )
    .await;

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
        master_identity,
        _node_identity: node_identity,
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

pub(crate) fn test_conversation_service(
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
        ConversationConfig::default(),
    )
    .unwrap()
}

/// `call_peer`'s `check_outbound_identity` refuses up front unless the
/// caller holds both an instance certificate and a recorded owner for
/// `service_id` -- real requirements for presenting that service's own
/// identity to a peer, not exercised by any test before this one since
/// every existing group op either targets `self` (refused earlier) or a
/// group with no other member (skipped before any outbound call).
pub(crate) async fn install_outbound_identity(
    registry: &EndpointRegistry,
    service_id: &str,
    master: &Identity,
) {
    let instance = Identity::generate().unwrap();
    let mut cert = DelegationCertificate::issue(
        master,
        instance.public_key(),
        3600,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    cert.temporary_did = service_id.to_string();
    registry.set_instance_cert(service_id.to_string(), cert).await.unwrap();
    registry.set_owner(service_id.to_string(), derive_did_key(&master.public_key())).await.unwrap();
}

pub(crate) async fn build_wasm_stack(
    dir: &Path,
    wasm_bytes: &[u8],
    stub_proxy: &Arc<StubProxy>,
    node_identity: Arc<Identity>,
    master: &Identity,
    clock: RecordClock,
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
    install_outbound_identity(&registry, SERVICE_ID, master).await;

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
            registry.clone(),
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
        .set(Arc::downgrade(&conversation) as Weak<dyn ConversationHost>)
        .expect("conversation set once");
    conversation.set_notifier(Arc::downgrade(&engine) as Weak<dyn ConversationNotifier>);
    let record_signer = NodeRecordSigner::with_clock(node_identity, registry.clone(), clock);
    engine.record_signer.set(record_signer).expect("set record_signer");
    engine.deploy_wasm(SERVICE_ID, &wasm_deploy_manifest(wasm_bytes.to_vec())).await.unwrap();
    (engine, conversation, ws_senders)
}

pub(crate) type NativeStack = (
    Arc<NativeFixture<NativeAppHost>>,
    Arc<NativeHostFactory>,
    Arc<dyn StorageProvider>,
    Arc<ConversationService>,
    Arc<NativeHttpAdapter>,
    Arc<WebSocketSenders>,
);

pub(crate) async fn build_native_stack(
    dir: &Path,
    stub_proxy: &Arc<StubProxy>,
    node_identity: Arc<Identity>,
    master: &Identity,
    clock: RecordClock,
) -> NativeStack {
    let key_store = Arc::new(KeyStore::new());
    key_store.inject_kek([0x42; 32]).expect("inject kek");
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(dir.join("data"), true).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let endpoint_registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    install_outbound_identity(&endpoint_registry, SERVICE_ID, master).await;

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
        endpoint_registry.clone(),
        resolver,
        conversation.clone(),
        ws_senders.clone(),
    );
    let f = factory.clone();
    let f_http = factory.clone();
    let fixture = Arc::new(NativeFixture::new(
        SERVICE_ID.to_string(),
        move |caller| f.host_for(caller),
        move |caller| f_http.host_for_wire(caller),
    ));
    factory.set_service_proxy(Arc::downgrade(stub_proxy) as Weak<dyn ServiceProxy>);
    let record_signer =
        NodeRecordSigner::with_clock(node_identity, endpoint_registry.clone(), clock);
    factory.set_record_signer(record_signer);
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
pub(crate) const SCENARIOS: &[(&str, &str)] = &[
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
    // A local drive is `internal` on both builds, whatever caller it carries.
    ("caller-origin", r#"{"op":"caller-origin"}"#),
];

pub(crate) async fn scenarios<D: Driver>(d: &D) -> Vec<(&'static str, String)> {
    let mut out = Vec::with_capacity(SCENARIOS.len());
    for (name, request) in SCENARIOS {
        out.push((*name, d.run(request).await.unwrap_or_else(|e| format!("ERR:{e}"))));
    }
    out
}

pub(crate) fn abac_policy() -> String {
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

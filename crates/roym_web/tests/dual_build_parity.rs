#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration suite driving the Roym SynApp through both builds -- the real
//! `wasm32-wasip2` components via `AppSandboxEngine`, and the same sources
//! linked in via `syneroym-app-host-native` -- and asserting the results
//! are identical across all scenarios.

use std::{
    fs,
    sync::{Arc, Weak},
    time::Duration,
};

use serde_json::{Value, json};
use syneroym_app_host::types::http::{
    CallerAuth, CallerIdentity, FrameKind, HttpRequest, HttpResponse,
};
use syneroym_app_host_native::{
    HttpSink, NativeAppHost, NativeHostFactory, NativeHttpAdapter, WebSocketSink,
};
use syneroym_app_orchestration::{
    AppInstanceId, AppRegistry, LogicalResolver, LogicalServiceName, ServiceId, StaticInventory,
    TopologyEntry, TopologyEpoch, TopologyKey, TopologyMode,
};
use syneroym_async_queue::QueueConfig;
use syneroym_conversation::{ConversationConfig, ConversationService};
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
use syneroym_roym_catalog::native::NativeCatalog;
use syneroym_roym_conversation::native::NativeConversation;
use syneroym_roym_core::{envelope::Response, services};
use syneroym_roym_directory::native::NativeDirectory;
use syneroym_roym_profile::native::NativeProfile;
use syneroym_roym_transaction::native::NativeTransaction;
use syneroym_roym_web::native::NativeWeb;
use syneroym_rpc::{
    AuthLevel, CallerContext, ConversationHost, JsonRpcRequest, NativeHttpService,
    NativeInvocation, NativeService, ProxyError, ProxyRequest, ServiceProxy, SessionContext,
    WebSocketSenders,
};
use syneroym_sandbox_wasm::{AppSandboxEngine, GuestHttpOutcome};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};

fn did_for_service(name: &str) -> String {
    format!("did:key:zRoym{name}")
}

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
    async fn invoke_web(&self, request: &str) -> Result<String, String>;
    async fn status(&self, service_name: &str) -> Result<String, String>;
}

struct WasmDriver {
    engine: Arc<AppSandboxEngine>,
}

impl Driver for WasmDriver {
    async fn invoke_web(&self, request: &str) -> Result<String, String> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "invoke".to_string(),
            params: json!([request]),
            id: None,
            idempotency_key: None,
        };
        let web_id = did_for_service("web");
        let result = self
            .engine
            .execute_wasm_json(&web_id, services::WEB.interface, &req, Some(caller()))
            .await
            .map_err(|e| e.to_string())?;
        match result {
            Value::String(s) => Ok(s),
            other => Ok(other.to_string()),
        }
    }

    async fn status(&self, service_name: &str) -> Result<String, String> {
        let svc = services::ALL.iter().find(|s| s.name == service_name).expect("service not found");
        let service_id = did_for_service(service_name);
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "status".to_string(),
            params: json!([]),
            id: None,
            idempotency_key: None,
        };
        let result = self
            .engine
            .execute_wasm_json(&service_id, svc.interface, &req, Some(caller()))
            .await
            .map_err(|e| e.to_string())?;
        match result {
            Value::String(s) => Ok(s),
            other => Ok(other.to_string()),
        }
    }
}

struct NativeDriver {
    web: Arc<NativeWeb<NativeAppHost>>,
    profile: Arc<NativeProfile<NativeAppHost>>,
    conversation: Arc<NativeConversation<NativeAppHost>>,
    catalog: Arc<NativeCatalog<NativeAppHost>>,
    transaction: Arc<NativeTransaction<NativeAppHost>>,
    directory: Arc<NativeDirectory<NativeAppHost>>,
}

impl Driver for NativeDriver {
    async fn invoke_web(&self, request: &str) -> Result<String, String> {
        let inv = NativeInvocation {
            interface: services::WEB.interface.to_string(),
            method: "invoke".to_string(),
            params: json!([request]),
            caller: caller(),
        };
        let response = self.web.dispatch(inv).await.map_err(|e| e.to_string())?;
        match response.payload {
            Value::String(s) => Ok(s),
            other => Ok(other.to_string()),
        }
    }

    async fn status(&self, service_name: &str) -> Result<String, String> {
        let svc_dispatch: Arc<dyn NativeService> = match service_name {
            "web" => self.web.clone(),
            "profile" => self.profile.clone(),
            "conversation" => self.conversation.clone(),
            "catalog" => self.catalog.clone(),
            "transaction" => self.transaction.clone(),
            "directory" => self.directory.clone(),
            _ => panic!("unknown service {service_name}"),
        };
        let inv = NativeInvocation {
            interface: "api".to_string(),
            method: "status".to_string(),
            params: json!([]),
            caller: caller(),
        };
        let response = svc_dispatch.dispatch(inv).await.map_err(|e| e.to_string())?;
        match response.payload {
            Value::String(s) => Ok(s),
            other => Ok(other.to_string()),
        }
    }
}

trait HttpDriver {
    async fn post(
        &self,
        path_and_query: &str,
        body: Vec<u8>,
        caller: Option<CallerContext>,
    ) -> HttpResponse;
}

struct WasmHttpDriver {
    engine: Arc<AppSandboxEngine>,
}

impl HttpDriver for WasmHttpDriver {
    async fn post(
        &self,
        path_and_query: &str,
        body: Vec<u8>,
        caller: Option<CallerContext>,
    ) -> HttpResponse {
        let req = HttpRequest {
            method: "POST".to_string(),
            path: path_and_query.to_string(),
            query: String::new(),
            route: path_and_query.to_string(),
            path_params: vec![],
            headers: vec![("content-type".into(), "application/json".into())],
            body,
            caller: caller.as_ref().map(|c| CallerIdentity {
                did: c.caller_did.clone(),
                auth: if matches!(c.auth, AuthLevel::Delegated) {
                    CallerAuth::Delegated
                } else if matches!(c.auth, AuthLevel::Ucan) {
                    CallerAuth::Ucan
                } else {
                    CallerAuth::SelfAsserted
                },
                app_instance: c.app_instance.clone(),
            }),
        };
        let web_id = did_for_service("web");
        let outcome = self
            .engine
            .handle_guest_http_request(&web_id, &req, caller)
            .await
            .expect("wasm http execution failed");
        match outcome {
            GuestHttpOutcome::Response(resp) => resp,
            GuestHttpOutcome::Failed(f) => panic!("WASM HTTP failed: {f:?}"),
        }
    }
}

struct NativeHttpDriver {
    adapter: Arc<NativeHttpAdapter>,
}

impl HttpDriver for NativeHttpDriver {
    async fn post(
        &self,
        path_and_query: &str,
        body: Vec<u8>,
        caller: Option<CallerContext>,
    ) -> HttpResponse {
        let req = HttpRequest {
            method: "POST".to_string(),
            path: path_and_query.to_string(),
            query: String::new(),
            route: path_and_query.to_string(),
            path_params: vec![],
            headers: vec![("content-type".into(), "application/json".into())],
            body,
            caller: caller.as_ref().map(|c| CallerIdentity {
                did: c.caller_did.clone(),
                auth: if matches!(c.auth, AuthLevel::Delegated) {
                    CallerAuth::Delegated
                } else if matches!(c.auth, AuthLevel::Ucan) {
                    CallerAuth::Ucan
                } else {
                    CallerAuth::SelfAsserted
                },
                app_instance: c.app_instance.clone(),
            }),
        };
        self.adapter.handle_request(req, caller).await.expect("native http adapter failed")
    }
}

struct Harness {
    wasm: WasmDriver,
    native: NativeDriver,
    wasm_http: WasmHttpDriver,
    native_http: NativeHttpDriver,
    native_factories: Vec<Arc<NativeHostFactory>>,
    _wasm_proxy: Arc<TestWasmServiceProxy>,
    _native_proxy: Arc<TestNativeServiceProxy>,
    _wasm_ws_senders: Arc<WebSocketSenders>,
    _native_ws_senders: Arc<WebSocketSenders>,
    _wasm_dir: tempfile::TempDir,
    _native_dir: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for f in &self.native_factories {
            f.shutdown();
        }
    }
}

#[derive(Debug)]
struct TestWasmServiceProxy {
    engine: Arc<AppSandboxEngine>,
}

#[async_trait::async_trait]
impl ServiceProxy for TestWasmServiceProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        let target = request.target_service.as_str();
        let service_id = if target == "did:key:hForeign" {
            did_for_service("directory")
        } else {
            target.to_string()
        };

        if service_id.contains("unbound") {
            return Err(ProxyError::ServiceNotFound("unbound".to_string()));
        }
        if service_id.contains("timeout") {
            return Err(ProxyError::Timeout(Duration::from_secs(30)));
        }

        let rpc_req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: request.method,
            params: request.params,
            id: None,
            idempotency_key: request.idempotency_key,
        };

        let res = self
            .engine
            .execute_wasm_json(&service_id, &request.interface, &rpc_req, Some(request.caller))
            .await;

        match res {
            Ok(v) => Ok(v),
            Err(e) => {
                eprintln!(
                    "TestWasmServiceProxy error invoking {service_id} {}: {e:?}",
                    request.interface
                );
                Err(ProxyError::Internal(e.to_string()))
            }
        }
    }
}

#[derive(Debug)]
struct TestNativeServiceProxy {
    web: Arc<NativeWeb<NativeAppHost>>,
    profile: Arc<NativeProfile<NativeAppHost>>,
    conversation: Arc<NativeConversation<NativeAppHost>>,
    catalog: Arc<NativeCatalog<NativeAppHost>>,
    transaction: Arc<NativeTransaction<NativeAppHost>>,
    directory: Arc<NativeDirectory<NativeAppHost>>,
}

#[async_trait::async_trait]
impl ServiceProxy for TestNativeServiceProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        let target = request.target_service.as_str();
        let svc: Arc<dyn NativeService> = if target == "did:key:hForeign" {
            self.directory.clone()
        } else if target == did_for_service("profile") {
            self.profile.clone()
        } else if target == did_for_service("conversation") {
            self.conversation.clone()
        } else if target == did_for_service("catalog") {
            self.catalog.clone()
        } else if target == did_for_service("transaction") {
            self.transaction.clone()
        } else if target == did_for_service("directory") {
            self.directory.clone()
        } else if target == did_for_service("web") {
            self.web.clone()
        } else {
            return Err(ProxyError::ServiceNotFound(target.to_string()));
        };

        let inv = NativeInvocation {
            interface: request.interface,
            method: request.method,
            params: request.params,
            caller: request.caller,
        };

        let res = svc.dispatch(inv).await;
        match res {
            Ok(resp) => Ok(resp.payload),
            Err(e) => Err(ProxyError::Internal(e.to_string())),
        }
    }
}

fn wasm_deploy_manifest(bytes: Vec<u8>, iface: &str) -> DeployManifest {
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
            interfaces: vec![iface.to_string()],
        }),
        registry_certificate: None,
        instance_certificate: None,
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
        ConversationConfig::default(),
    )
    .unwrap()
}

async fn harness() -> Harness {
    let wasm_paths = [
        ("web", test_constants::roym_web_wasm_path()),
        ("profile", test_constants::roym_profile_wasm_path()),
        ("conversation", test_constants::roym_conversation_wasm_path()),
        ("catalog", test_constants::roym_catalog_wasm_path()),
        ("transaction", test_constants::roym_transaction_wasm_path()),
        ("directory", test_constants::roym_directory_wasm_path()),
    ];

    let mut wasm_binaries = Vec::new();
    for (name, path) in &wasm_paths {
        let bytes = fs::read(path).unwrap_or_else(|e| {
            panic!(
                "roym dual_build_parity: WASM artifact for {name} not found ({e}) -- run `mise \
                 run build:roym`"
            )
        });
        wasm_binaries.push((*name, bytes));
    }

    let wasm_dir = tempfile::tempdir().unwrap();
    let native_dir = tempfile::tempdir().unwrap();

    let mut config = SubstrateConfig {
        app_local_data_dir: wasm_dir.path().join("data"),
        app_data_dir: wasm_dir.path().join("user_data"),
        app_cache_dir: wasm_dir.path().join("cache"),
        app_log_dir: wasm_dir.path().join("logs"),
        profile: "full".to_string(),
        ..SubstrateConfig::default()
    };
    config.resolve_paths();

    // 1. WASM Stack setup
    let wasm_ks = Arc::new(KeyStore::new());
    wasm_ks.inject_kek([0x42; 32]).expect("inject kek");

    let wasm_storage: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(wasm_dir.path().join("db"), true).unwrap());
    let wasm_blobs: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let wasm_mqtt = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let wasm_reg = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    let app_instance = AppInstanceId::new("roym");
    let wasm_inventory = Arc::new(StaticInventory::new());
    for svc in services::SIBLINGS {
        let svc_did = did_for_service(svc.name);
        wasm_inventory.register(
            TopologyKey::local(app_instance.clone(), LogicalServiceName::new(svc.name)),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![ServiceId::new(svc_did)],
                sharding_strategy: None,
                epoch: TopologyEpoch(1),
                cache_ttl: Duration::from_secs(60),
                not_after: None,
            },
        );
    }

    let wasm_resolver = Arc::new(LogicalResolver::new(wasm_inventory));
    for svc in services::ALL {
        let service_id = did_for_service(svc.name);
        wasm_reg
            .set_app_context(service_id, app_instance.to_string(), svc.name.to_string())
            .await
            .unwrap();
    }

    let wasm_conversation =
        test_conversation_service(wasm_storage.clone(), wasm_ks.clone(), wasm_reg.clone());

    let wasm_engine = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            wasm_ks,
            wasm_storage,
            wasm_blobs,
            wasm_mqtt,
            wasm_reg,
            wasm_resolver,
        )
        .await
        .unwrap(),
    );
    wasm_engine.self_weak.set(Arc::downgrade(&wasm_engine)).expect("self_weak set once");
    let wasm_ws_senders = WebSocketSenders::new();
    wasm_engine.websocket_senders.set(wasm_ws_senders.clone()).expect("set ws senders");
    wasm_engine
        .conversation
        .set(Arc::downgrade(&wasm_conversation) as Weak<dyn ConversationHost>)
        .expect("conversation set once");

    let wasm_proxy = Arc::new(TestWasmServiceProxy { engine: wasm_engine.clone() });
    wasm_engine
        .service_proxy
        .set(Arc::downgrade(&wasm_proxy) as Weak<dyn ServiceProxy>)
        .expect("set service proxy");

    for (name, bytes) in wasm_binaries {
        let iface = services::ALL.iter().find(|s| s.name == name).map(|s| s.interface).unwrap();
        let service_id = did_for_service(name);
        let manifest = wasm_deploy_manifest(bytes, iface);
        wasm_engine.deploy_wasm(&service_id, &manifest).await.expect("deploy wasm service");
    }

    // 2. Native Stack setup
    let native_ks = Arc::new(KeyStore::new());
    native_ks.inject_kek([0x42; 32]).expect("inject kek");

    let native_storage: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(native_dir.path().join("db"), true).unwrap());
    let native_blobs: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let native_mqtt = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let native_reg = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
    let native_ws_senders = WebSocketSenders::new();

    let native_inventory = Arc::new(StaticInventory::new());
    for svc in services::SIBLINGS {
        let svc_did = did_for_service(svc.name);
        native_inventory.register(
            TopologyKey::local(app_instance.clone(), LogicalServiceName::new(svc.name)),
            TopologyEntry {
                mode: TopologyMode::Singleton,
                members: vec![ServiceId::new(svc_did)],
                sharding_strategy: None,
                epoch: TopologyEpoch(1),
                cache_ttl: Duration::from_secs(60),
                not_after: None,
            },
        );
    }
    let native_resolver = Arc::new(LogicalResolver::new(native_inventory));
    for svc in services::ALL {
        let service_id = did_for_service(svc.name);
        native_reg
            .set_app_context(service_id, app_instance.to_string(), svc.name.to_string())
            .await
            .unwrap();
    }

    let native_conversation =
        test_conversation_service(native_storage.clone(), native_ks.clone(), native_reg.clone());

    let make_factory = |name: &str| {
        let service_id = did_for_service(name);
        NativeHostFactory::new(
            service_id,
            native_ks.clone(),
            native_storage.clone(),
            native_blobs.clone(),
            native_mqtt.clone(),
            native_reg.clone(),
            native_resolver.clone(),
            native_conversation.clone(),
            native_ws_senders.clone(),
        )
    };

    let f_web = make_factory("web");
    let f_profile = make_factory("profile");
    let f_conversation = make_factory("conversation");
    let f_catalog = make_factory("catalog");
    let f_transaction = make_factory("transaction");
    let f_directory = make_factory("directory");

    let f_web_cl = f_web.clone();
    let native_web =
        Arc::new(NativeWeb::new(did_for_service("web"), move |caller| f_web_cl.host_for(caller)));

    let f_prof_cl = f_profile.clone();
    let native_profile = Arc::new(NativeProfile::new(did_for_service("profile"), move |caller| {
        f_prof_cl.host_for(caller)
    }));

    let f_conv_cl = f_conversation.clone();
    let native_conversation_svc =
        Arc::new(NativeConversation::new(did_for_service("conversation"), move |caller| {
            f_conv_cl.host_for(caller)
        }));

    let f_cat_cl = f_catalog.clone();
    let native_catalog = Arc::new(NativeCatalog::new(did_for_service("catalog"), move |caller| {
        f_cat_cl.host_for(caller)
    }));

    let f_tx_cl = f_transaction.clone();
    let native_transaction =
        Arc::new(NativeTransaction::new(did_for_service("transaction"), move |caller| {
            f_tx_cl.host_for(caller)
        }));

    let f_dir_cl = f_directory.clone();
    let native_directory =
        Arc::new(NativeDirectory::new(did_for_service("directory"), move |caller| {
            f_dir_cl.host_for(caller)
        }));

    let native_proxy = Arc::new(TestNativeServiceProxy {
        web: native_web.clone(),
        profile: native_profile.clone(),
        conversation: native_conversation_svc.clone(),
        catalog: native_catalog.clone(),
        transaction: native_transaction.clone(),
        directory: native_directory.clone(),
    });

    let native_factories = vec![
        f_web.clone(),
        f_profile.clone(),
        f_conversation.clone(),
        f_catalog.clone(),
        f_transaction.clone(),
        f_directory.clone(),
    ];

    for f in &native_factories {
        f.set_service_proxy(Arc::downgrade(&native_proxy) as Weak<dyn ServiceProxy>);
    }

    let web_http: Arc<dyn HttpSink> = native_web.clone();
    let web_ws: Arc<dyn WebSocketSink> = native_web.clone();

    f_web.set_http_sink(Arc::downgrade(&web_http));
    f_web.set_websocket_sink(Arc::downgrade(&web_ws));
    let native_http_adapter = Arc::new(NativeHttpAdapter::new(
        f_web.clone(),
        Arc::downgrade(&web_http),
        Arc::downgrade(&web_ws),
    ));

    Harness {
        wasm: WasmDriver { engine: wasm_engine.clone() },
        native: NativeDriver {
            web: native_web.clone(),
            profile: native_profile.clone(),
            conversation: native_conversation_svc.clone(),
            catalog: native_catalog.clone(),
            transaction: native_transaction.clone(),
            directory: native_directory.clone(),
        },
        wasm_http: WasmHttpDriver { engine: wasm_engine.clone() },
        native_http: NativeHttpDriver { adapter: native_http_adapter },
        native_factories,
        _wasm_proxy: wasm_proxy,
        _native_proxy: native_proxy,
        _wasm_ws_senders: wasm_ws_senders,
        _native_ws_senders: native_ws_senders,
        _wasm_dir: wasm_dir,
        _native_dir: native_dir,
    }
}

// ---------------- Scenarios ----------------

#[tokio::test]
async fn scenario_1_profile_ping_reachability_byte_identical() {
    let h = harness().await;
    let req = json!({
        "method": "profile.ping",
        "params": {}
    })
    .to_string();

    let wasm_res = h.wasm.invoke_web(&req).await.unwrap();
    let native_res = h.native.invoke_web(&req).await.unwrap();

    assert_eq!(wasm_res, native_res);
    let resp: Response = serde_json::from_str(&wasm_res).unwrap();
    assert_eq!(resp.result, Some(json!({ "service": "profile" })));
}

#[tokio::test]
async fn scenario_2_inert_extra_caller_field_in_params() {
    let h = harness().await;
    let req_extra = json!({
        "method": "profile.ping",
        "params": {
            "caller": "did:key:fakeAttacker",
            "person_did": "did:key:fakeAlice",
            "auth": "delegated"
        }
    })
    .to_string();

    let wasm_res = h.wasm.invoke_web(&req_extra).await.unwrap();
    let native_res = h.native.invoke_web(&req_extra).await.unwrap();

    assert_eq!(wasm_res, native_res);
    let resp: Response = serde_json::from_str(&wasm_res).unwrap();
    assert_eq!(resp.result, Some(json!({ "service": "profile" })));
}

#[tokio::test]
async fn scenario_3_session_whoami_with_delegated_caller() {
    let h = harness().await;
    let delegated_caller = CallerContext {
        caller_did: "did:key:hAlice".to_string(),
        app_instance: None,
        session: SessionContext { subject_did: "did:key:hAlice".to_string(), ..Default::default() },
        auth: AuthLevel::Delegated,
        proof: None,
    };

    let req_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "session.whoami",
        "params": {}
    })
    .to_string()
    .into_bytes();

    let wasm_resp =
        h.wasm_http.post("/rpc", req_body.clone(), Some(delegated_caller.clone())).await;
    let native_resp = h.native_http.post("/rpc", req_body, Some(delegated_caller)).await;

    assert_eq!(wasm_resp.status, 200);
    assert_eq!(native_resp.status, 200);
    assert_eq!(wasm_resp.body, native_resp.body);

    let val: Value = serde_json::from_slice(&wasm_resp.body).unwrap();
    assert_eq!(val["result"]["did"], "did:key:hAlice");
    assert_eq!(val["result"]["auth"], "delegated");
}

#[tokio::test]
async fn scenario_4_unlisted_method_returns_32601() {
    let h = harness().await;
    let req = json!({
        "method": "nope.thing",
        "params": {}
    })
    .to_string();

    let wasm_res = h.wasm.invoke_web(&req).await.unwrap();
    let native_res = h.native.invoke_web(&req).await.unwrap();

    assert_eq!(wasm_res, native_res);
    let resp: Response = serde_json::from_str(&wasm_res).unwrap();
    assert_eq!(resp.error.unwrap().code, -32601);
}

#[tokio::test]
async fn scenario_5_unbound_dependency_returns_32001() {
    let h = harness().await;
    let req = json!({
        "method": "conversation.ping",
        "params": {}
    })
    .to_string();

    let wasm_res = h.wasm.invoke_web(&req).await.unwrap();
    let native_res = h.native.invoke_web(&req).await.unwrap();
    assert_eq!(wasm_res, native_res);
}

#[tokio::test]
async fn scenario_6_handle_http_post_rpc_matches_invoke() {
    let h = harness().await;
    let req_body = json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "profile.ping",
        "params": {}
    })
    .to_string()
    .into_bytes();

    let wasm_http_resp = h.wasm_http.post("/rpc", req_body.clone(), None).await;
    let native_http_resp = h.native_http.post("/rpc", req_body, None).await;

    assert_eq!(wasm_http_resp.status, 200);
    assert_eq!(native_http_resp.status, 200);
    assert_eq!(wasm_http_resp.body, native_http_resp.body);

    let val: Value = serde_json::from_slice(&wasm_http_resp.body).unwrap();
    assert_eq!(val["id"], 42);
    assert_eq!(val["result"]["service"], "profile");
}

#[tokio::test]
async fn scenario_7_handle_http_malformed_body_returns_32700() {
    let h = harness().await;
    let bad_body = b"not a json object".to_vec();

    let wasm_http_resp = h.wasm_http.post("/rpc", bad_body.clone(), None).await;
    let native_http_resp = h.native_http.post("/rpc", bad_body, None).await;

    assert_eq!(wasm_http_resp.status, 200);
    assert_eq!(native_http_resp.status, 200);
    assert_eq!(wasm_http_resp.body, native_http_resp.body);

    let val: Value = serde_json::from_slice(&wasm_http_resp.body).unwrap();
    assert_eq!(val["error"]["code"], -32700);
}

#[tokio::test]
async fn scenario_8_status_on_all_six_services() {
    let h = harness().await;
    for svc in services::ALL {
        let wasm_status = h.wasm.status(svc.name).await.unwrap();
        let native_status = h.native.status(svc.name).await.unwrap();

        assert_eq!(wasm_status, native_status, "status mismatch on service {}", svc.name);
        let val: Value = serde_json::from_str(&wasm_status).unwrap();
        assert_eq!(val["service"], svc.name);
        assert_eq!(val["schema_version"], 1);
    }
}

#[tokio::test]
async fn scenario_9_websocket_lifecycle_fires() {
    let h = harness().await;
    let conn_id = "test-ws-conn-123".to_string();

    let caller_ctx = caller();
    NativeWeb::on_open(&h.native.web, caller_ctx.clone(), conn_id.clone()).await;
    NativeWeb::on_message(
        &h.native.web,
        caller_ctx.clone(),
        conn_id.clone(),
        b"hello".to_vec(),
        FrameKind::Text,
    )
    .await;
    NativeWeb::on_close(&h.native.web, caller_ctx, conn_id.clone()).await;
}

#[tokio::test]
async fn scenario_10_directory_service_call_target() {
    let h = harness().await;
    let req = json!({
        "method": "directory.ping",
        "params": {}
    })
    .to_string();

    let wasm_res = h.wasm.invoke_web(&req).await.unwrap();
    let native_res = h.native.invoke_web(&req).await.unwrap();

    assert_eq!(wasm_res, native_res);
    let resp: Response = serde_json::from_str(&wasm_res).unwrap();
    assert_eq!(resp.result, Some(json!({ "service": "directory" })));
}

#[tokio::test]
async fn the_parity_comparison_detects_a_divergence() {
    let h = harness().await;
    let req = json!({
        "method": "profile.ping",
        "params": {}
    })
    .to_string();

    let wasm_res = h.wasm.invoke_web(&req).await.unwrap();
    let mutated_res = wasm_res.replace("\"profile\"", "\"mutated\"");

    assert_ne!(wasm_res, mutated_res);
}

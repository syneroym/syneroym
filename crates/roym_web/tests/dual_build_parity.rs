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
use syneroym_app_host::{
    ConversationSink,
    types::http::{CallerAuth, CallerIdentity, FrameKind, HttpRequest, HttpResponse},
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
    config::{AppSandboxRole, RetryPolicy, SubstrateConfig},
    local_registry::EndpointRegistry,
    storage::MockStorage,
    test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider, host_store::QueryOptions};
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{
    Identity,
    delegation::{DelegationCertificate, SCOPE_RECORD_SIGNING},
    substrate::{derive_did_key, resolve_did_key},
};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_roym_catalog::native::NativeCatalog;
use syneroym_roym_conversation::native::NativeConversation;
use syneroym_roym_core::{envelope::Response, services};
use syneroym_roym_directory::native::NativeDirectory;
use syneroym_roym_profile::native::NativeProfile;
use syneroym_roym_transaction::native::NativeTransaction;
use syneroym_roym_web::native::NativeWeb;
use syneroym_rpc::{
    AuthLevel, CallerContext, ConversationDeliveryState, ConversationHost, ConversationMessage,
    ConversationNotifier, JsonRpcRequest, NativeHttpService, NativeInvocation, NativeService,
    ProxyError, ProxyRequest, ServiceProxy, SessionContext, WebSocketSenders,
};
use syneroym_sandbox_wasm::{AppSandboxEngine, GuestHttpOutcome};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, DeployManifest, ServiceConfig, ServiceType, WasmManifest,
};

fn did_for_service(name: &str) -> String {
    format!("did:key:zRoym{name}")
}

fn owner_identity() -> Identity {
    Identity::from_bytes(&[42; 32])
}

fn owner_did() -> String {
    derive_did_key(&owner_identity().public_key())
}

fn strip_volatile(val: &mut Value) {
    match val {
        Value::Object(map) => {
            map.remove("verified_at_secs");
            map.remove("added_at_secs");
            map.remove("at_secs");
            map.remove("since_secs");
            map.remove("produced_at_secs");
            // Rows Roym writes carry the host's own wall clock at write
            // time (stored/opened/updated/deleted seconds) and the host's
            // millisecond clock at send time (activity / sender timestamp),
            // neither of which is the pinned signing clock. The signed
            // listing envelope stays compared byte for byte -- its
            // timestamp is the pinned `RecordClock`.
            map.remove("stored_at_secs");
            map.remove("opened_at_secs");
            map.remove("updated_at_secs");
            map.remove("deleted_at_secs");
            map.remove("last_activity_ms");
            // A section digest folds in every row's bytes, including the
            // wall-clock fields removed above, so it is volatile too. The
            // raw bundle's own `check_integrity` runs before any strip.
            map.remove("digest");
            map.remove("sender_timestamp_ms");
            for (k, v) in map.iter_mut() {
                if k != "envelope" && k != "delegation" {
                    strip_volatile(v);
                }
            }
        }
        Value::Array(arr) => {
            for v in arr {
                strip_volatile(v);
            }
        }
        Value::String(s) => {
            if let Ok(mut parsed) = serde_json::from_str::<Value>(s)
                && (parsed.is_object() || parsed.is_array())
            {
                strip_volatile(&mut parsed);
                *s = parsed.to_string();
            }
        }
        _ => {}
    }
}

fn caller() -> CallerContext {
    custom_caller(&owner_did())
}

fn custom_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext { subject_did: did.to_string(), ..Default::default() },
        auth: AuthLevel::Delegated,
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
    owner: Identity,
    owner_did: String,
    wasm: WasmDriver,
    native: NativeDriver,
    wasm_http: WasmHttpDriver,
    native_http: NativeHttpDriver,
    native_factories: Vec<Arc<NativeHostFactory>>,
    wasm_proxy: Arc<TestWasmServiceProxy>,
    native_proxy: Arc<TestNativeServiceProxy>,
    /// The one factory whose conversation sink is wired, so a test can
    /// push an inbound message straight at Roym's own inbox on the native
    /// stack the same way `AppSandboxEngine` does on the wasm one.
    conv_factory: Arc<NativeHostFactory>,
    /// The shared host `ConversationService` per stack -- a test creates a
    /// group here (a kind the inbox refuses) or reads delivery state.
    wasm_conversation: Arc<ConversationService>,
    native_conversation: Arc<ConversationService>,
    /// `host_for_wire`-built native instances: the parity harness is the
    /// only caller of that constructor, and it is what makes the wire
    /// refusal a real two-build comparison rather than a wasm-only one.
    wire_native: Vec<(&'static str, Arc<dyn NativeService>)>,
    /// Storage + keystore per stack, so a test can read a collection no
    /// verb exposes (`refused_messages`).
    wasm_storage: Arc<dyn StorageProvider>,
    native_storage: Arc<dyn StorageProvider>,
    wasm_ks: Arc<KeyStore>,
    native_ks: Arc<KeyStore>,
    _wasm_ws_senders: Arc<WebSocketSenders>,
    _native_ws_senders: Arc<WebSocketSenders>,
    _wasm_dir: tempfile::TempDir,
    _native_dir: tempfile::TempDir,
}

impl Harness {
    fn caller(&self) -> CallerContext {
        custom_caller(&self.owner_did)
    }

    /// Drives one service's `invoke` as a call that arrived over the
    /// network, on both builds: wasm through `execute_wasm_json_from_wire`,
    /// native through the `host_for_wire` instance. Returns each build's
    /// parsed roym `Response`.
    async fn wire_invoke(&self, svc: services::Service, envelope: &Value) -> (Value, Value) {
        let env_str = envelope.to_string();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "invoke".to_string(),
            params: json!([env_str]),
            id: None,
            idempotency_key: None,
        };
        let wasm = self
            .wasm
            .engine
            .execute_wasm_json_from_wire(
                &did_for_service(svc.name),
                svc.interface,
                &req,
                Some(caller()),
            )
            .await
            .expect("wasm wire invoke");
        let native = self
            .wire_native
            .iter()
            .find(|(n, _)| *n == svc.name)
            .expect("wire instance")
            .1
            .dispatch(NativeInvocation {
                interface: svc.interface.to_string(),
                method: "invoke".to_string(),
                params: json!([env_str]),
                caller: caller(),
            })
            .await
            .expect("native wire invoke");
        (unwrap_payload(wasm), unwrap_payload(native.payload))
    }

    /// The same as `wire_invoke` but for the ungated `api.status` export.
    async fn wire_status(&self, svc: services::Service) -> (Value, Value) {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "status".to_string(),
            params: json!([]),
            id: None,
            idempotency_key: None,
        };
        let wasm = self
            .wasm
            .engine
            .execute_wasm_json_from_wire(
                &did_for_service(svc.name),
                svc.interface,
                &req,
                Some(caller()),
            )
            .await
            .expect("wasm wire status");
        let native = self
            .wire_native
            .iter()
            .find(|(n, _)| *n == svc.name)
            .expect("wire instance")
            .1
            .dispatch(NativeInvocation {
                interface: "api".to_string(),
                method: "status".to_string(),
                params: json!([]),
                caller: caller(),
            })
            .await
            .expect("native wire status");
        (unwrap_payload(wasm), unwrap_payload(native.payload))
    }

    /// Drives one service's `invoke` directly (not through `web`) as a
    /// local call carrying the verified delegated owner caller -- exactly
    /// what `WasmDriver`/`NativeDriver` already present, isolated here so a
    /// scenario can prove a local origin is admitted on both builds.
    async fn local_invoke(&self, svc: services::Service, envelope: &Value) -> (Value, Value) {
        let env_str = envelope.to_string();
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "invoke".to_string(),
            params: json!([env_str]),
            id: None,
            idempotency_key: None,
        };
        let wasm = self
            .wasm
            .engine
            .execute_wasm_json(&did_for_service(svc.name), svc.interface, &req, Some(caller()))
            .await
            .expect("wasm local invoke");
        let native_svc: Arc<dyn NativeService> = match svc.name {
            "web" => self.native.web.clone(),
            "profile" => self.native.profile.clone(),
            "conversation" => self.native.conversation.clone(),
            "catalog" => self.native.catalog.clone(),
            "transaction" => self.native.transaction.clone(),
            "directory" => self.native.directory.clone(),
            other => panic!("unknown service {other}"),
        };
        let native = native_svc
            .dispatch(NativeInvocation {
                interface: svc.interface.to_string(),
                method: "invoke".to_string(),
                params: json!([env_str]),
                caller: caller(),
            })
            .await
            .expect("native local invoke");
        (unwrap_payload(wasm), unwrap_payload(native.payload))
    }

    /// Pushes one inbound message at Roym's own inbox on the chosen stack,
    /// the same entry point `ConversationService`'s delivery worker uses.
    async fn deliver(&self, wasm: bool, msg: ConversationMessage) {
        let conv_id = did_for_service("conversation");
        if wasm {
            ConversationNotifier::notify_message(&*self.wasm.engine, &conv_id, msg).await;
        } else {
            ConversationNotifier::notify_message(&*self.conv_factory, &conv_id, msg).await;
        }
    }

    async fn notify_state(&self, wasm: bool, message_id: &str, state: ConversationDeliveryState) {
        let conv_id = did_for_service("conversation");
        if wasm {
            ConversationNotifier::notify_delivery_state(
                &*self.wasm.engine,
                &conv_id,
                message_id.to_string(),
                state,
            )
            .await;
        } else {
            ConversationNotifier::notify_delivery_state(
                &*self.conv_factory,
                &conv_id,
                message_id.to_string(),
                state,
            )
            .await;
        }
    }

    /// Reads every row of a `conversation`-service collection no verb
    /// exposes (`refused_messages`), on the chosen stack.
    async fn conv_rows(&self, wasm: bool, collection: &str) -> Vec<Value> {
        let (storage, ks) = if wasm {
            (&self.wasm_storage, &self.wasm_ks)
        } else {
            (&self.native_storage, &self.native_ks)
        };
        let db = storage
            .open_service_db(&did_for_service("conversation"), ks)
            .await
            .expect("open conversation db");
        let opts = QueryOptions { filter: None, limit: Some(500), cursor: None };
        let page = match db.query(collection, &opts, None).await {
            Ok(p) => p,
            // A collection the inbox never created yet is "no rows", not a
            // failure.
            Err(_) => return Vec::new(),
        };
        page.value
            .records
            .into_iter()
            .filter_map(|r| serde_json::from_slice::<Value>(&r.payload).ok())
            .collect()
    }
}

/// The roym services return their JSON-RPC response as a JSON string;
/// unwrap that one level so a scenario compares structured values.
fn unwrap_payload(v: Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
        other => other,
    }
}

/// Replaces every host message id with `<msg:N>` -- N being the row's
/// position once `messages` / `matches` rows are in their own sort order,
/// which the service already returns them in. Host message ids fold in a
/// random nonce, so they differ between the two stacks; a positional
/// rewrite keeps "two messages merged into one row" detectable where a
/// blanket strip would hide it. Returns the count of distinct ids mapped.
fn normalize_message_ids(val: &mut Value) -> usize {
    let mut order: Vec<String> = Vec::new();
    collect_ordered_ids(val, &mut order);
    let map: std::collections::HashMap<String, String> =
        order.iter().enumerate().map(|(i, id)| (id.clone(), format!("<msg:{i}>"))).collect();
    rewrite_ids(val, &map);
    map.len()
}

fn collect_ordered_ids(val: &Value, out: &mut Vec<String>) {
    match val {
        Value::Object(map) => {
            for (k, v) in map {
                if (k == "messages" || k == "matches")
                    && let Value::Array(rows) = v
                {
                    for row in rows {
                        if let Some(id) = row.get("id").and_then(Value::as_str)
                            && !out.iter().any(|e| e == id)
                        {
                            out.push(id.to_string());
                        }
                    }
                }
                collect_ordered_ids(v, out);
            }
        }
        Value::Array(arr) => arr.iter().for_each(|v| collect_ordered_ids(v, out)),
        _ => {}
    }
}

fn rewrite_ids(val: &mut Value, map: &std::collections::HashMap<String, String>) {
    match val {
        Value::Object(m) => m.values_mut().for_each(|v| rewrite_ids(v, map)),
        Value::Array(a) => a.iter_mut().for_each(|v| rewrite_ids(v, map)),
        Value::String(s) => {
            if let Some(replacement) = map.get(s) {
                *s = replacement.clone();
            }
        }
        _ => {}
    }
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
    invocations: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl ServiceProxy for TestWasmServiceProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        self.invocations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
    invocations: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl ServiceProxy for TestNativeServiceProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        self.invocations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

    // The default epoch budget (5s dispatch) is a wall-clock deadline, not a
    // CPU one: a guest that is ready to run but not scheduled still burns the
    // budget. Under a saturated CI host running every scenario in parallel,
    // legitimate profile-service work (cross-service calls plus delegation-
    // certificate crypto) blows past 5s and traps as `wasm trap: interrupt`.
    // Give the sandbox a generous budget so only a real hang fails the test.
    config.roles.app_sandbox = Some(AppSandboxRole {
        dispatch_epoch_timeout_secs: 120,
        lifecycle_hook_epoch_timeout_secs: 120,
        ..AppSandboxRole::default()
    });

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
    // `transaction` is left out of web's topology deliberately: scenario 5
    // needs a genuinely unbound dependency, and `conversation` is now bound
    // (the inbox sink and its own verbs). `transaction` has no verbs yet, so
    // nothing else needs it bound.
    for svc in services::SIBLINGS.into_iter().filter(|s| s.name != "transaction") {
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

    let owner = owner_identity();
    let owner_did = owner_did();
    let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
    // Pinned so the two stacks stamp every envelope with the same second
    // and compare byte for byte. The value has to clear two windows at
    // once: a delegated record must be dated at or after its certificate's
    // own `issued_at` (the certificate is minted a few seconds into each
    // scenario, after this harness is built), and on import a listing is
    // re-verified against the verifier's wall clock, which rejects a record
    // more than `max_clock_skew_secs` (300s) in the future. A small step
    // ahead of "now" satisfies both.
    let wall_now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let fixed_clock = syneroym_core::record_signer::RecordClock::Fixed(wall_now + 240);

    let wasm_resolver = Arc::new(LogicalResolver::new(wasm_inventory));
    for svc in services::ALL {
        let service_id = did_for_service(svc.name);
        wasm_reg
            .set_app_context(service_id.clone(), app_instance.to_string(), svc.name.to_string())
            .await
            .unwrap();
        wasm_reg.set_owner(service_id, owner_did.clone()).await.unwrap();
    }

    let wasm_conversation =
        test_conversation_service(wasm_storage.clone(), wasm_ks.clone(), wasm_reg.clone());

    let wasm_engine = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            wasm_ks.clone(),
            wasm_storage.clone(),
            wasm_blobs,
            wasm_mqtt,
            wasm_reg.clone(),
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
    // The wasm delivery path: `ConversationService` wakes the engine, which
    // invokes the deployed conversation component's `guest-api` export. The
    // native side registers its own notifier in `NativeHostFactory::new`.
    wasm_conversation.set_notifier(Arc::downgrade(&wasm_engine) as Weak<dyn ConversationNotifier>);

    let wasm_record_signer = syneroym_core::record_signer::NodeRecordSigner::with_clock(
        node_identity.clone(),
        wasm_reg,
        fixed_clock,
    );
    wasm_engine.record_signer.set(wasm_record_signer).expect("set wasm record_signer");

    let wasm_proxy = Arc::new(TestWasmServiceProxy {
        engine: wasm_engine.clone(),
        invocations: std::sync::atomic::AtomicUsize::new(0),
    });
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
    for svc in services::SIBLINGS.into_iter().filter(|s| s.name != "transaction") {
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
            .set_app_context(service_id.clone(), app_instance.to_string(), svc.name.to_string())
            .await
            .unwrap();
        native_reg.set_owner(service_id, owner_did.clone()).await.unwrap();
    }

    let native_conversation =
        test_conversation_service(native_storage.clone(), native_ks.clone(), native_reg.clone());

    let native_record_signer = syneroym_core::record_signer::NodeRecordSigner::with_clock(
        node_identity,
        native_reg.clone(),
        fixed_clock,
    );

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

    // The native inbox sink: `NativeHostFactory::new` already registered the
    // factory as this service's `ConversationNotifier`, so this is the one
    // line that points that notifier at the deployed conversation service.
    f_conversation.set_conversation_sink(
        Arc::downgrade(&native_conversation_svc) as Weak<dyn ConversationSink>
    );

    // `host_for_wire` instances -- one per service, sharing the fully wired
    // factories. Nothing in a real substrate reaches a natively linked roym
    // service over the wire; the parity harness is the only caller, and it
    // is what makes the wire-origin refusal a two-build comparison.
    let wire_native: Vec<(&'static str, Arc<dyn NativeService>)> = {
        let (fw, fp, fc, fca, ft, fd) = (
            f_web.clone(),
            f_profile.clone(),
            f_conversation.clone(),
            f_catalog.clone(),
            f_transaction.clone(),
            f_directory.clone(),
        );
        vec![
            (
                "web",
                Arc::new(NativeWeb::new(did_for_service("web"), move |c| fw.host_for_wire(c)))
                    as Arc<dyn NativeService>,
            ),
            (
                "profile",
                Arc::new(NativeProfile::new(did_for_service("profile"), move |c| {
                    fp.host_for_wire(c)
                })),
            ),
            (
                "conversation",
                Arc::new(NativeConversation::new(did_for_service("conversation"), move |c| {
                    fc.host_for_wire(c)
                })),
            ),
            (
                "catalog",
                Arc::new(NativeCatalog::new(did_for_service("catalog"), move |c| {
                    fca.host_for_wire(c)
                })),
            ),
            (
                "transaction",
                Arc::new(NativeTransaction::new(did_for_service("transaction"), move |c| {
                    ft.host_for_wire(c)
                })),
            ),
            (
                "directory",
                Arc::new(NativeDirectory::new(did_for_service("directory"), move |c| {
                    fd.host_for_wire(c)
                })),
            ),
        ]
    };

    let native_proxy = Arc::new(TestNativeServiceProxy {
        web: native_web.clone(),
        profile: native_profile.clone(),
        conversation: native_conversation_svc.clone(),
        catalog: native_catalog.clone(),
        transaction: native_transaction.clone(),
        directory: native_directory.clone(),
        invocations: std::sync::atomic::AtomicUsize::new(0),
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
        f.set_record_signer(native_record_signer.clone());
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
        owner,
        owner_did,
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
        wasm_proxy,
        native_proxy,
        conv_factory: f_conversation.clone(),
        wasm_conversation: wasm_conversation.clone(),
        native_conversation: native_conversation.clone(),
        wire_native,
        wasm_storage: wasm_storage.clone(),
        native_storage: native_storage.clone(),
        wasm_ks: wasm_ks.clone(),
        native_ks: native_ks.clone(),
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
        "method": "profile.policy",
        "params": {}
    })
    .to_string();

    let wasm_res = h.wasm.invoke_web(&req).await.unwrap();
    let native_res = h.native.invoke_web(&req).await.unwrap();

    assert_eq!(wasm_res, native_res);
    let resp: Response = serde_json::from_str(&wasm_res).unwrap();
    let result = resp.result.unwrap();
    assert!(result.get("statement").is_some());
}

#[tokio::test]
async fn scenario_2_inert_extra_caller_field_in_params() {
    let h = harness().await;
    let req_extra = json!({
        "method": "profile.policy",
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
    let result = resp.result.unwrap();
    assert!(result.get("statement").is_some());
}

#[tokio::test]
async fn scenario_3_session_whoami_with_delegated_caller() {
    let h = harness().await;
    let delegated_caller = caller();

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
    assert_eq!(val["result"]["did"], owner_did());
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

    // An unlisted method is rejected by web's own dispatch before it ever
    // reaches the proxy, so no sibling call should have been attempted.
    assert_eq!(h.wasm_proxy.invocations.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(h.native_proxy.invocations.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn scenario_5_unbound_dependency_returns_32001() {
    let h = harness().await;
    let req_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "receipt.ping",
        "params": {}
    })
    .to_string()
    .into_bytes();

    let wasm_resp = h.wasm_http.post("/rpc", req_body.clone(), Some(caller())).await;
    let native_resp = h.native_http.post("/rpc", req_body, Some(caller())).await;
    assert_eq!(wasm_resp.body, native_resp.body);

    // transaction is a declared dependency of web (see roym.toml), but the
    // topology-registration loops above deliberately filter it out (`s.name
    // != "transaction"`), leaving it unbound: the call must be refused with
    // -32001, and the refusal must not repeat the dependency's DID back to
    // the caller.
    let val: Value = serde_json::from_slice(&wasm_resp.body).unwrap();
    let err_code = val["error"]["code"].as_i64().unwrap();
    let err_msg = val["error"]["message"].as_str().unwrap();
    assert_eq!(err_code, -32001);
    assert!(!err_msg.contains("did:"), "error message must not leak a DID: {}", err_msg);
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

    let wasm_http_resp = h.wasm_http.post("/rpc", req_body.clone(), Some(caller())).await;
    let native_http_resp = h.native_http.post("/rpc", req_body, Some(caller())).await;

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
        // profile, catalog and conversation carry real state now.
        let expected_schema_version =
            if matches!(svc.name, "profile" | "catalog" | "conversation") { 2 } else { 1 };
        assert_eq!(val["schema_version"], expected_schema_version);
    }
}

// The web app keeps no per-connection state (CON-1): with `WS_CONNS`
// dropped, there is nothing left to assert registered/removed, so this
// only checks the lifecycle calls are callable and return without error
// on both drivers.
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

    let web_id = did_for_service("web");
    h.wasm.engine.handle_websocket_on_open(&web_id, &conn_id, Some(caller())).await;
    h.wasm
        .engine
        .handle_websocket_on_message(
            &web_id,
            &conn_id,
            b"hello".to_vec(),
            FrameKind::Text,
            Some(caller()),
        )
        .await;
    h.wasm.engine.handle_websocket_on_close(&web_id, &conn_id, Some(caller())).await;
}

#[tokio::test]
async fn scenario_10_directory_service_call_target() {
    let h = harness().await;
    let req_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "directory.ping",
        "params": {}
    })
    .to_string()
    .into_bytes();

    let wasm_resp = h.wasm_http.post("/rpc", req_body.clone(), Some(caller())).await;
    let native_resp = h.native_http.post("/rpc", req_body, Some(caller())).await;

    assert_eq!(wasm_resp.body, native_resp.body);
    let val: Value = serde_json::from_slice(&wasm_resp.body).unwrap();
    assert_eq!(val["result"]["service"], "directory");
}

#[tokio::test]
async fn scenario_11_profile_signing_status_unenrolled_parity() {
    let h = harness().await;
    let req = json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(h.caller())).await;
    let native_res = h.native_http.post("/rpc", req, Some(h.caller())).await;

    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_eq!(val["result"]["certificate"]["state"], "missing");
}

#[tokio::test]
async fn scenario_12_profile_set_without_enrolment_fails_parity() {
    let h = harness().await;
    let req = json!({
        "method": "profile.set",
        "params": { "display_name": "Alice" }
    })
    .to_string()
    .into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(h.caller())).await;
    let native_res = h.native_http.post("/rpc", req, Some(h.caller())).await;

    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_eq!(val["error"]["code"], -32602);
}

#[tokio::test]
async fn scenario_13_profile_install_certificate_and_signing_status_parity() {
    let h = harness().await;
    let caller_identity = h.caller();

    let status_req =
        json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();
    let status_res =
        h.wasm_http.post("/rpc", status_req.clone(), Some(caller_identity.clone())).await;
    let status_val: Value = serde_json::from_slice(&status_res.body).unwrap();
    let signing_did = status_val["result"]["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();

    let cert = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();

    let install_req = json!({
        "method": "profile.install-signing-certificate",
        "params": { "certificate": cert.to_json().unwrap() }
    })
    .to_string()
    .into_bytes();

    let wasm_res =
        h.wasm_http.post("/rpc", install_req.clone(), Some(caller_identity.clone())).await;
    let native_res = h.native_http.post("/rpc", install_req, Some(caller_identity.clone())).await;
    assert_eq!(wasm_res.body, native_res.body);

    let wasm_status =
        h.wasm_http.post("/rpc", status_req.clone(), Some(caller_identity.clone())).await;
    let native_status = h.native_http.post("/rpc", status_req, Some(caller_identity)).await;
    assert_eq!(wasm_status.body, native_status.body);
}

#[tokio::test]
async fn scenario_14_profile_set_and_get_envelope_parity() {
    let h = harness().await;
    let caller_identity = h.caller();

    let status_req =
        json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();
    let status_res = h.wasm_http.post("/rpc", status_req, Some(caller_identity.clone())).await;
    let status_val: Value = serde_json::from_slice(&status_res.body).unwrap();
    let signing_did = status_val["result"]["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();

    let cert = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let install_req = json!({
        "method": "profile.install-signing-certificate",
        "params": { "certificate": cert.to_json().unwrap() }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", install_req.clone(), Some(caller_identity.clone())).await;
    h.native_http.post("/rpc", install_req, Some(caller_identity.clone())).await;

    let set_req = json!({
        "method": "profile.set",
        "params": { "display_name": "Parity Alice", "conversation_address": "syneroym://addr/1" }
    })
    .to_string()
    .into_bytes();

    let wasm_set = h.wasm_http.post("/rpc", set_req.clone(), Some(caller_identity.clone())).await;
    let native_set = h.native_http.post("/rpc", set_req, Some(caller_identity.clone())).await;
    let mut wasm_set_val: Value = serde_json::from_slice(&wasm_set.body).unwrap();
    let mut native_set_val: Value = serde_json::from_slice(&native_set.body).unwrap();
    strip_volatile(&mut wasm_set_val);
    strip_volatile(&mut native_set_val);
    assert_eq!(wasm_set_val, native_set_val);

    let get_req = json!({ "method": "profile.get", "params": {} }).to_string().into_bytes();
    let wasm_get = h.wasm_http.post("/rpc", get_req.clone(), Some(caller_identity.clone())).await;
    let native_get = h.native_http.post("/rpc", get_req, Some(caller_identity)).await;
    let mut wasm_get_val: Value = serde_json::from_slice(&wasm_get.body).unwrap();
    let mut native_get_val: Value = serde_json::from_slice(&native_get.body).unwrap();
    strip_volatile(&mut wasm_get_val);
    strip_volatile(&mut native_get_val);
    assert_eq!(wasm_get_val, native_get_val);
}

#[tokio::test]
async fn scenario_15_contacts_resolve_address_and_remove_parity() {
    let h = harness().await;

    let upsert_req = json!({
        "method": "contacts.upsert",
        "params": {
            "person_did": "did:key:zFriend15",
            "display_name": "Friend 15",
            "conversation_address": "syneroym://friend15"
        }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", upsert_req.clone(), Some(caller())).await;
    h.native_http.post("/rpc", upsert_req, Some(caller())).await;

    let resolve_req = json!({
        "method": "contacts.resolve-address",
        "params": { "person_did": "did:key:zFriend15" }
    })
    .to_string()
    .into_bytes();
    let wasm_resolve = h.wasm_http.post("/rpc", resolve_req.clone(), Some(caller())).await;
    let native_resolve = h.native_http.post("/rpc", resolve_req, Some(caller())).await;
    assert_eq!(wasm_resolve.body, native_resolve.body);
    let val: Value = serde_json::from_slice(&wasm_resolve.body).unwrap();
    assert_ne!(val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));
    assert_eq!(val["result"]["conversation_address"], "syneroym://friend15");

    let remove_req = json!({
        "method": "contacts.remove",
        "params": { "person_did": "did:key:zFriend15" }
    })
    .to_string()
    .into_bytes();
    let wasm_rem = h.wasm_http.post("/rpc", remove_req.clone(), Some(caller())).await;
    let native_rem = h.native_http.post("/rpc", remove_req, Some(caller())).await;
    assert_eq!(wasm_rem.body, native_rem.body);
}

#[tokio::test]
async fn scenario_16_profile_policy_public_parity() {
    let h = harness().await;
    let req = json!({ "method": "profile.policy", "params": {} }).to_string().into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), None).await;
    let native_res = h.native_http.post("/rpc", req, None).await;
    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_ne!(val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));
    let expected = "A blocked sender's messages are refused at this node's inbox. They are never \
                    shown in any conversation, never fire a notification, and are never counted. \
                    Block is enforced locally by this installation's own Conversation service.";
    assert_eq!(val["result"]["statement"], expected);
}

#[tokio::test]
async fn scenario_17_contacts_upsert_and_list_parity() {
    let h = harness().await;

    let upsert_req = json!({
        "method": "contacts.upsert",
        "params": { "person_did": "did:key:zFriend17", "favourite": true }
    })
    .to_string()
    .into_bytes();

    let wasm_upsert = h.wasm_http.post("/rpc", upsert_req.clone(), Some(caller())).await;
    let native_upsert = h.native_http.post("/rpc", upsert_req, Some(caller())).await;
    assert_eq!(wasm_upsert.body, native_upsert.body);

    let list_req = json!({ "method": "contacts.list", "params": {} }).to_string().into_bytes();
    let wasm_list = h.wasm_http.post("/rpc", list_req.clone(), Some(caller())).await;
    let native_list = h.native_http.post("/rpc", list_req, Some(caller())).await;
    let mut wasm_list_val: Value = serde_json::from_slice(&wasm_list.body).unwrap();
    let mut native_list_val: Value = serde_json::from_slice(&native_list.body).unwrap();
    strip_volatile(&mut wasm_list_val);
    strip_volatile(&mut native_list_val);
    assert_eq!(wasm_list_val, native_list_val);
}

#[tokio::test]
async fn scenario_18_contacts_get_and_remove_parity() {
    let h = harness().await;

    let upsert_req = json!({
        "method": "contacts.upsert",
        "params": { "person_did": "did:key:zFriend18" }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", upsert_req.clone(), Some(caller())).await;
    h.native_http.post("/rpc", upsert_req, Some(caller())).await;

    let get_req = json!({
        "method": "contacts.get",
        "params": { "person_did": "did:key:zFriend18" }
    })
    .to_string()
    .into_bytes();
    let wasm_get = h.wasm_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let native_get = h.native_http.post("/rpc", get_req, Some(caller())).await;
    let mut wasm_get_val: Value = serde_json::from_slice(&wasm_get.body).unwrap();
    let mut native_get_val: Value = serde_json::from_slice(&native_get.body).unwrap();
    strip_volatile(&mut wasm_get_val);
    strip_volatile(&mut native_get_val);
    assert_eq!(wasm_get_val, native_get_val);

    let remove_req = json!({
        "method": "contacts.remove",
        "params": { "person_did": "did:key:zFriend18" }
    })
    .to_string()
    .into_bytes();
    let wasm_remove = h.wasm_http.post("/rpc", remove_req.clone(), Some(caller())).await;
    let native_remove = h.native_http.post("/rpc", remove_req, Some(caller())).await;
    assert_eq!(wasm_remove.body, native_remove.body);
}

#[tokio::test]
async fn scenario_19_contacts_favourite_parity() {
    let h = harness().await;

    let upsert_req = json!({
        "method": "contacts.upsert",
        "params": {
            "person_did": "did:key:zFriend19",
            "conversation_address": "syneroym://friend19",
            "favourite": true
        }
    })
    .to_string()
    .into_bytes();
    let wasm_up = h.wasm_http.post("/rpc", upsert_req.clone(), Some(caller())).await;
    let native_up = h.native_http.post("/rpc", upsert_req, Some(caller())).await;
    assert_eq!(wasm_up.body, native_up.body);

    let get_req = json!({
        "method": "contacts.get",
        "params": { "person_did": "did:key:zFriend19" }
    })
    .to_string()
    .into_bytes();
    let wasm_get = h.wasm_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let native_get = h.native_http.post("/rpc", get_req, Some(caller())).await;
    assert_eq!(wasm_get.body, native_get.body);
    let val: Value = serde_json::from_slice(&wasm_get.body).unwrap();
    assert_ne!(val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));
    assert_eq!(val["result"]["favourite"], true);
}

#[tokio::test]
async fn scenario_20_block_add_and_list_parity() {
    let h = harness().await;

    let add_req = json!({
        "method": "block.add",
        "params": { "person_did": "did:key:zSpammer20", "reason": "spam" }
    })
    .to_string()
    .into_bytes();
    let wasm_add = h.wasm_http.post("/rpc", add_req.clone(), Some(caller())).await;
    let native_add = h.native_http.post("/rpc", add_req, Some(caller())).await;
    assert_eq!(wasm_add.body, native_add.body);

    let list_req = json!({ "method": "block.list", "params": {} }).to_string().into_bytes();
    let wasm_list = h.wasm_http.post("/rpc", list_req.clone(), Some(caller())).await;
    let native_list = h.native_http.post("/rpc", list_req, Some(caller())).await;
    let mut wasm_list_val: Value = serde_json::from_slice(&wasm_list.body).unwrap();
    let mut native_list_val: Value = serde_json::from_slice(&native_list.body).unwrap();
    strip_volatile(&mut wasm_list_val);
    strip_volatile(&mut native_list_val);
    assert_eq!(wasm_list_val, native_list_val);
}

#[tokio::test]
async fn scenario_21_block_check_and_remove_parity() {
    let h = harness().await;

    let add_req = json!({
        "method": "block.add",
        "params": { "person_did": "did:key:zSpammer21" }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", add_req.clone(), Some(caller())).await;
    h.native_http.post("/rpc", add_req, Some(caller())).await;

    let check_req = json!({
        "method": "block.check",
        "params": { "person_did": "did:key:zSpammer21" }
    })
    .to_string()
    .into_bytes();
    let wasm_check = h.wasm_http.post("/rpc", check_req.clone(), Some(caller())).await;
    let native_check = h.native_http.post("/rpc", check_req, Some(caller())).await;
    let mut wasm_check_val: Value = serde_json::from_slice(&wasm_check.body).unwrap();
    let mut native_check_val: Value = serde_json::from_slice(&native_check.body).unwrap();
    strip_volatile(&mut wasm_check_val);
    strip_volatile(&mut native_check_val);
    assert_eq!(wasm_check_val, native_check_val);

    let remove_req = json!({
        "method": "block.remove",
        "params": { "person_did": "did:key:zSpammer21" }
    })
    .to_string()
    .into_bytes();
    let wasm_remove = h.wasm_http.post("/rpc", remove_req.clone(), Some(caller())).await;
    let native_remove = h.native_http.post("/rpc", remove_req, Some(caller())).await;
    assert_eq!(wasm_remove.body, native_remove.body);
}

#[tokio::test]
async fn scenario_22_report_create_get_withdraw_and_refile_refusal_parity() {
    let h = harness().await;

    let submit_req = json!({
        "method": "report.create",
        "params": { "subject_kind": "person", "subject_id": "did:key:zOffender22", "category": "harassment", "details": "bad" }
    })
    .to_string()
    .into_bytes();
    let wasm_sub = h.wasm_http.post("/rpc", submit_req.clone(), Some(caller())).await;
    let native_sub = h.native_http.post("/rpc", submit_req.clone(), Some(caller())).await;
    assert_eq!(wasm_sub.body, native_sub.body);
    let sub_val: Value = serde_json::from_slice(&wasm_sub.body).unwrap();
    assert_ne!(sub_val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));
    let report_id = sub_val["result"]["report_id"].as_str().unwrap();

    let get_req = json!({ "method": "report.get", "params": { "report_id": report_id } })
        .to_string()
        .into_bytes();
    let wasm_get = h.wasm_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let native_get = h.native_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let mut wasm_val: Value = serde_json::from_slice(&wasm_get.body).unwrap();
    let mut native_val: Value = serde_json::from_slice(&native_get.body).unwrap();
    strip_volatile(&mut wasm_val);
    strip_volatile(&mut native_val);
    assert_eq!(wasm_val, native_val);

    // Withdraw the report
    let withdraw_req = json!({ "method": "report.withdraw", "params": { "report_id": report_id } })
        .to_string()
        .into_bytes();
    let wasm_with = h.wasm_http.post("/rpc", withdraw_req.clone(), Some(caller())).await;
    let native_with = h.native_http.post("/rpc", withdraw_req, Some(caller())).await;
    assert_eq!(wasm_with.body, native_with.body);
    let with_val: Value = serde_json::from_slice(&wasm_with.body).unwrap();
    assert_eq!(with_val["result"]["status"], "withdrawn");

    // Verify report.get reflects status "withdrawn"
    let wasm_get2 = h.wasm_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let native_get2 = h.native_http.post("/rpc", get_req, Some(caller())).await;
    assert_eq!(wasm_get2.body, native_get2.body);
    let get2_val: Value = serde_json::from_slice(&wasm_get2.body).unwrap();
    assert_eq!(get2_val["result"]["status"], "withdrawn");

    // Attempting to re-file a withdrawn report refuses on both builds
    let wasm_refile = h.wasm_http.post("/rpc", submit_req.clone(), Some(caller())).await;
    let native_refile = h.native_http.post("/rpc", submit_req, Some(caller())).await;
    assert_eq!(wasm_refile.body, native_refile.body);
    let refile_val: Value = serde_json::from_slice(&wasm_refile.body).unwrap();
    assert!(refile_val.get("error").is_some());
    assert!(refile_val["error"]["message"].as_str().unwrap().contains("withdrawn"));
}

#[tokio::test]
async fn scenario_23_profile_export_and_import_parity() {
    let h = harness().await;

    let exp_req = json!({ "method": "profile.export", "params": {} }).to_string().into_bytes();
    let wasm_exp = h.wasm_http.post("/rpc", exp_req.clone(), Some(caller())).await;
    let native_exp = h.native_http.post("/rpc", exp_req, Some(caller())).await;
    let mut wasm_exp_val: Value = serde_json::from_slice(&wasm_exp.body).unwrap();
    let mut native_exp_val: Value = serde_json::from_slice(&native_exp.body).unwrap();
    strip_volatile(&mut wasm_exp_val);
    strip_volatile(&mut native_exp_val);
    assert_eq!(wasm_exp_val, native_exp_val);

    let exp_val: Value = serde_json::from_slice(&wasm_exp.body).unwrap();
    let bundle = &exp_val["result"];

    let imp_req = json!({
        "method": "profile.import",
        "params": { "bundle": bundle }
    })
    .to_string()
    .into_bytes();
    let wasm_imp = h.wasm_http.post("/rpc", imp_req.clone(), Some(caller())).await;
    let native_imp = h.native_http.post("/rpc", imp_req, Some(caller())).await;
    assert_eq!(wasm_imp.body, native_imp.body);
}

#[tokio::test]
async fn scenario_24_contacts_limits_and_set_limits_parity() {
    let h = harness().await;

    let get_req = json!({ "method": "contacts.limits", "params": {} }).to_string().into_bytes();
    let wasm_get = h.wasm_http.post("/rpc", get_req.clone(), Some(caller())).await;
    let native_get = h.native_http.post("/rpc", get_req, Some(caller())).await;
    assert_eq!(wasm_get.body, native_get.body);
    let val: Value = serde_json::from_slice(&wasm_get.body).unwrap();
    assert_ne!(val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));

    let set_req = json!({
        "method": "contacts.set-limits",
        "params": { "window_secs": 7200, "max_per_window": 5 }
    })
    .to_string()
    .into_bytes();
    let wasm_set = h.wasm_http.post("/rpc", set_req.clone(), Some(caller())).await;
    let native_set = h.native_http.post("/rpc", set_req, Some(caller())).await;
    assert_eq!(wasm_set.body, native_set.body);

    let check_req = json!({ "method": "contacts.limits", "params": {} }).to_string().into_bytes();
    let wasm_check = h.wasm_http.post("/rpc", check_req.clone(), Some(caller())).await;
    let native_check = h.native_http.post("/rpc", check_req, Some(caller())).await;
    assert_eq!(wasm_check.body, native_check.body);
    let check_val: Value = serde_json::from_slice(&wasm_check.body).unwrap();
    assert_eq!(check_val["result"]["window_secs"], 7200);
    assert_eq!(check_val["result"]["max_per_window"], 5);
}

#[tokio::test]
async fn scenario_25_unauthenticated_owner_methods_refused_parity() {
    let h = harness().await;
    let methods = ["profile.get", "contacts.list", "block.list", "report.list"];

    for method in methods {
        let req = json!({ "method": method, "params": {} }).to_string().into_bytes();
        let wasm_res = h.wasm_http.post("/rpc", req.clone(), None).await;
        let native_res = h.native_http.post("/rpc", req, None).await;
        assert_eq!(wasm_res.body, native_res.body);
        let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
        assert_eq!(val["error"]["code"], -32010);
    }
}

#[tokio::test]
async fn scenario_26_stranger_owner_methods_refused_parity() {
    let h = harness().await;
    let stranger_identity = custom_caller("did:key:zStranger26");
    let methods = ["profile.get", "contacts.list", "block.list", "report.list"];

    for method in methods {
        let req = json!({ "method": method, "params": {} }).to_string().into_bytes();
        let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(stranger_identity.clone())).await;
        let native_res = h.native_http.post("/rpc", req, Some(stranger_identity.clone())).await;
        assert_eq!(wasm_res.body, native_res.body);
        let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
        assert_eq!(val["error"]["code"], -32011);
    }
}

#[tokio::test]
async fn scenario_27_stranger_cannot_install_certificate_parity() {
    let h = harness().await;
    let stranger = Identity::generate().unwrap();
    let stranger_did = derive_did_key(&stranger.public_key());
    let stranger_identity = custom_caller(&stranger_did);

    let status_req =
        json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();
    let status_res = h.wasm_http.post("/rpc", status_req, Some(h.caller())).await;
    let status_val: Value = serde_json::from_slice(&status_res.body).unwrap();
    let signing_did = status_val["result"]["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();

    let cert = DelegationCertificate::issue(
        &stranger,
        signing_pubkey,
        3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let install_req = json!({
        "method": "profile.install-signing-certificate",
        "params": { "certificate": cert.to_json().unwrap() }
    })
    .to_string()
    .into_bytes();

    let wasm_res =
        h.wasm_http.post("/rpc", install_req.clone(), Some(stranger_identity.clone())).await;
    let native_res = h.native_http.post("/rpc", install_req, Some(stranger_identity)).await;
    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_eq!(val["error"]["code"], -32011);
}

#[tokio::test]
async fn scenario_28_expired_certificate_refuses_signing_parity() {
    let h = harness().await;
    let caller_identity = h.caller();

    let status_req =
        json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();
    let status_res = h.wasm_http.post("/rpc", status_req, Some(caller_identity.clone())).await;
    let status_val: Value = serde_json::from_slice(&status_res.body).unwrap();
    let signing_did = status_val["result"]["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();

    let mut cert = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        100,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    cert.issued_at_secs = 1_000_000_000;
    cert.expires_at_secs = 1_000_000_100;

    let install_req = json!({
        "method": "profile.install-signing-certificate",
        "params": { "certificate": cert.to_json().unwrap() }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", install_req.clone(), Some(caller_identity.clone())).await;
    h.native_http.post("/rpc", install_req, Some(caller_identity.clone())).await;

    let set_req = json!({
        "method": "profile.set",
        "params": { "display_name": "Expired Alice" }
    })
    .to_string()
    .into_bytes();
    let wasm_set = h.wasm_http.post("/rpc", set_req.clone(), Some(caller_identity.clone())).await;
    let native_set = h.native_http.post("/rpc", set_req, Some(caller_identity)).await;
    assert_eq!(wasm_set.body, native_set.body);
    let val: Value = serde_json::from_slice(&wasm_set.body).unwrap();
    assert_eq!(val["error"]["code"], -32602);
}

#[tokio::test]
async fn scenario_29_stale_certificate_refuses_signing_parity() {
    let h = harness().await;
    let caller_identity = h.caller();

    let status_req =
        json!({ "method": "profile.signing-status", "params": {} }).to_string().into_bytes();
    let status_res = h.wasm_http.post("/rpc", status_req, Some(caller_identity.clone())).await;
    let status_val: Value = serde_json::from_slice(&status_res.body).unwrap();
    let signing_did = status_val["result"]["signing_did"].as_str().unwrap();
    let signing_pubkey = resolve_did_key(signing_did).unwrap();

    let cert1 = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    let install1 = json!({
        "method": "profile.install-signing-certificate",
        "params": { "certificate": cert1.to_json().unwrap() }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", install1.clone(), Some(caller_identity.clone())).await;
    h.native_http.post("/rpc", install1, Some(caller_identity.clone())).await;

    let set_req = json!({ "method": "profile.set", "params": { "display_name": "Alice 1" } })
        .to_string()
        .into_bytes();
    let wasm_set1 = h.wasm_http.post("/rpc", set_req.clone(), Some(caller_identity.clone())).await;
    let native_set1 = h.native_http.post("/rpc", set_req, Some(caller_identity)).await;
    assert_eq!(wasm_set1.body, native_set1.body);
}

#[tokio::test]
async fn scenario_30_contacts_list_pagination_parity() {
    let h = harness().await;

    for i in 1..=5 {
        let req = json!({
            "method": "contacts.upsert",
            "params": { "person_did": format!("did:key:zFriend30_{i}") }
        })
        .to_string()
        .into_bytes();
        h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
        h.native_http.post("/rpc", req, Some(caller())).await;
    }

    let page_req = json!({
        "method": "contacts.list",
        "params": { "limit": 2, "offset": 1 }
    })
    .to_string()
    .into_bytes();
    let wasm_list = h.wasm_http.post("/rpc", page_req.clone(), Some(caller())).await;
    let native_list = h.native_http.post("/rpc", page_req, Some(caller())).await;
    assert_eq!(wasm_list.body, native_list.body);
}

#[tokio::test]
async fn scenario_31_block_list_pagination_parity() {
    let h = harness().await;

    for i in 1..=5 {
        let req = json!({
            "method": "block.add",
            "params": { "person_did": format!("did:key:zSpammer31_{i}") }
        })
        .to_string()
        .into_bytes();
        h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
        h.native_http.post("/rpc", req, Some(caller())).await;
    }

    let page_req = json!({
        "method": "block.list",
        "params": { "limit": 2, "offset": 1 }
    })
    .to_string()
    .into_bytes();
    let wasm_list = h.wasm_http.post("/rpc", page_req.clone(), Some(caller())).await;
    let native_list = h.native_http.post("/rpc", page_req, Some(caller())).await;
    let mut wasm_val: Value = serde_json::from_slice(&wasm_list.body).unwrap();
    let mut native_val: Value = serde_json::from_slice(&native_list.body).unwrap();
    strip_volatile(&mut wasm_val);
    strip_volatile(&mut native_val);
    assert_eq!(wasm_val, native_val);
    assert_eq!(wasm_val["result"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn scenario_32_report_list_pagination_parity() {
    let h = harness().await;

    for i in 1..=5 {
        let req = json!({
            "method": "report.create",
            "params": {
                "subject_kind": "person",
                "subject_id": format!("did:key:zOffender32_{i}"),
                "category": "harassment",
                "details": format!("abuse {i}")
            }
        })
        .to_string()
        .into_bytes();
        h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
        h.native_http.post("/rpc", req, Some(caller())).await;
    }

    let page_req = json!({
        "method": "report.list",
        "params": { "limit": 2, "offset": 1 }
    })
    .to_string()
    .into_bytes();
    let wasm_list = h.wasm_http.post("/rpc", page_req.clone(), Some(caller())).await;
    let native_list = h.native_http.post("/rpc", page_req, Some(caller())).await;
    let mut wasm_val: Value = serde_json::from_slice(&wasm_list.body).unwrap();
    let mut native_val: Value = serde_json::from_slice(&native_list.body).unwrap();
    strip_volatile(&mut wasm_val);
    strip_volatile(&mut native_val);
    assert_eq!(wasm_val, native_val);
    assert_eq!(wasm_val["result"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn scenario_33_contacts_admit_first_contact_parity() {
    let h = harness().await;

    // 1. Unblocked stranger within limits -> allow
    let req = json!({
        "method": "contacts.admit-first-contact",
        "params": {
            "sender_address": "syneroym://stranger33",
            "sender_person_did": "did:key:zStranger33"
        }
    })
    .to_string()
    .into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
    let native_res = h.native_http.post("/rpc", req, Some(caller())).await;
    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_ne!(val.get("error").and_then(|e| e.get("code")), Some(&json!(-32601)));
    assert_eq!(val["result"]["admission"], "allow");

    // 2. Blocked sender -> blocked
    let block_req = json!({
        "method": "block.add",
        "params": {
            "person_did": "did:key:zBlocked33",
            "address": "syneroym://blocked33"
        }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", block_req.clone(), Some(caller())).await;
    h.native_http.post("/rpc", block_req, Some(caller())).await;

    let admit_blocked_req = json!({
        "method": "contacts.admit-first-contact",
        "params": {
            "sender_address": "syneroym://blocked33",
            "sender_person_did": "did:key:zBlocked33"
        }
    })
    .to_string()
    .into_bytes();
    let wasm_blk = h.wasm_http.post("/rpc", admit_blocked_req.clone(), Some(caller())).await;
    let native_blk = h.native_http.post("/rpc", admit_blocked_req, Some(caller())).await;
    assert_eq!(wasm_blk.body, native_blk.body);
    let blk_val: Value = serde_json::from_slice(&wasm_blk.body).unwrap();
    assert_eq!(blk_val["result"]["admission"], "blocked");

    // 3. Sender hitting rate limit -> rate-limited
    let set_limits_req = json!({
        "method": "contacts.set-limits",
        "params": {
            "max_per_window": 1,
            "window_secs": 3600
        }
    })
    .to_string()
    .into_bytes();
    h.wasm_http.post("/rpc", set_limits_req.clone(), Some(caller())).await;
    h.native_http.post("/rpc", set_limits_req, Some(caller())).await;

    // First attempt for ratelimited sender -> allow
    let rate_req = json!({
        "method": "contacts.admit-first-contact",
        "params": {
            "sender_address": "syneroym://ratelimited33",
            "sender_person_did": "did:key:zRateLimited33"
        }
    })
    .to_string()
    .into_bytes();
    let wasm_r1 = h.wasm_http.post("/rpc", rate_req.clone(), Some(caller())).await;
    let native_r1 = h.native_http.post("/rpc", rate_req.clone(), Some(caller())).await;
    assert_eq!(wasm_r1.body, native_r1.body);
    let r1_val: Value = serde_json::from_slice(&wasm_r1.body).unwrap();
    assert_eq!(r1_val["result"]["admission"], "allow");

    // Second attempt within window -> rate-limited
    let wasm_r2 = h.wasm_http.post("/rpc", rate_req.clone(), Some(caller())).await;
    let native_r2 = h.native_http.post("/rpc", rate_req, Some(caller())).await;
    let mut wasm_r2_val: Value = serde_json::from_slice(&wasm_r2.body).unwrap();
    let mut native_r2_val: Value = serde_json::from_slice(&native_r2.body).unwrap();
    strip_volatile(&mut wasm_r2_val);
    strip_volatile(&mut native_r2_val);
    assert_eq!(wasm_r2_val, native_r2_val);
    assert_eq!(wasm_r2_val["result"]["admission"], "rate-limited");
}

#[tokio::test]
async fn scenario_34_unknown_method_parity() {
    let h = harness().await;
    let req =
        json!({ "method": "profile.nonexistent_method", "params": {} }).to_string().into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
    let native_res = h.native_http.post("/rpc", req, Some(caller())).await;
    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_eq!(val["error"]["code"], -32601);
}

#[tokio::test]
async fn scenario_35_invalid_params_parity() {
    let h = harness().await;
    let req = json!({ "method": "contacts.upsert", "params": {} }).to_string().into_bytes();

    let wasm_res = h.wasm_http.post("/rpc", req.clone(), Some(caller())).await;
    let native_res = h.native_http.post("/rpc", req, Some(caller())).await;
    assert_eq!(wasm_res.body, native_res.body);
    let val: Value = serde_json::from_slice(&wasm_res.body).unwrap();
    assert_eq!(val["error"]["code"], -32602);
}

struct Mutant<'a, D>(&'a D);

impl<D: Driver> Driver for Mutant<'_, D> {
    async fn invoke_web(&self, request: &str) -> Result<String, String> {
        self.0.invoke_web(request).await.map(|s| s.replace("\"result\"", "\"mutated_result\""))
    }
    async fn status(&self, service_name: &str) -> Result<String, String> {
        self.0.status(service_name).await.map(|s| s.replace("\"service\"", "\"mutated_service\""))
    }
}

/// A passing parity comparison is not evidence of anything unless the
/// comparison is known to detect a real divergence.
#[tokio::test]
async fn the_parity_comparison_detects_a_divergence() {
    let h = harness().await;
    // Public methods that return a result object:
    let bound_methods = ["profile.policy"];
    let mutant = Mutant(&h.native);

    for method in bound_methods {
        let req = json!({
            "method": method,
            "params": {}
        })
        .to_string();
        let wasm_res = h.wasm.invoke_web(&req).await.unwrap();
        let mutant_res = mutant.invoke_web(&req).await.unwrap();
        assert_ne!(wasm_res, mutant_res, "invoke_web mutant divergence failed for {method}");
    }

    let all_services = [
        services::WEB.name,
        services::PROFILE.name,
        services::CONVERSATION.name,
        services::CATALOG.name,
        services::TRANSACTION.name,
        services::DIRECTORY.name,
    ];
    for svc_name in all_services {
        let wasm_status = h.wasm.status(svc_name).await.unwrap();
        let mutant_status = mutant.status(svc_name).await.unwrap();
        assert_ne!(wasm_status, mutant_status, "status mutant divergence failed for {svc_name}");
    }
}

// ---------------- catalog and conversation ----------------

/// POSTs one JSON-RPC method to both stacks' `/rpc` with the owner session
/// and returns each build's parsed response.
async fn both_rpc(h: &Harness, method: &str, params: Value) -> (Value, Value) {
    let req = json!({ "method": method, "params": params }).to_string().into_bytes();
    let w = h.wasm_http.post("/rpc", req.clone(), Some(h.caller())).await;
    let n = h.native_http.post("/rpc", req, Some(h.caller())).await;
    (serde_json::from_slice(&w.body).unwrap(), serde_json::from_slice(&n.body).unwrap())
}

/// One JSON-RPC method to a single stack -- for the cases where the two
/// builds are driven with different arguments (a per-stack message id).
async fn one_rpc(h: &Harness, wasm: bool, method: &str, params: Value) -> Value {
    let req = json!({ "method": method, "params": params }).to_string().into_bytes();
    let r = if wasm {
        h.wasm_http.post("/rpc", req, Some(h.caller())).await
    } else {
        h.native_http.post("/rpc", req, Some(h.caller())).await
    };
    serde_json::from_slice(&r.body).unwrap()
}

/// Mints one delegation certificate against `service`'s own signing key and
/// installs it on both stacks, so `<service>` can sign records.
async fn enrol_signing(h: &Harness, service: &str) {
    let status_m = format!("{service}.signing-status");
    let install_m = format!("{service}.install-signing-certificate");
    let (w, _) = both_rpc(h, &status_m, json!({})).await;
    let signing_did = w["result"]["signing_did"]
        .as_str()
        .unwrap_or_else(|| panic!("no signing_did from {status_m}: {w}"));
    let signing_pubkey = resolve_did_key(signing_did).unwrap();
    // The signing host verifies the certificate against the pinned
    // `RecordClock` (ahead of wall-clock time), so the certificate must
    // still be valid then -- yet under the 5-year lifetime ceiling the
    // install path enforces. A default short-lived one signs nothing here.
    let cert = DelegationCertificate::issue(
        &h.owner,
        signing_pubkey,
        86_400 * 365 * 4,
        SCOPE_RECORD_SIGNING.to_string(),
    )
    .unwrap();
    both_rpc(h, &install_m, json!({ "certificate": cert.to_json().unwrap() })).await;
}

/// A `listing.set` params object with every one of the seven optional
/// blocks filled and no non-integer number anywhere.
fn full_listing_params(slug: &str, title: &str) -> Value {
    json!({
        "slug": slug,
        "title": title,
        "summary": "Neat hedges, fortnightly.",
        "categories": ["gardening", "outdoor"],
        "conversation_address": "did:key:zProviderConv",
        "booking": {
            "mode": "slots",
            "lead_time_secs": 3600,
            "cancellation_window_secs": 86400,
            "max_per_booking": 2
        },
        "payment": {
            "currency": "EUR",
            "model": "per-hour",
            "amount_minor": 3500,
            "tax_included": true,
            "methods": ["cash"],
            "payee": "A. Gardener"
        },
        "product": { "unit": "hour", "pack_size": 1, "condition": "new", "sku": "HT-1" },
        "service": { "duration_secs": 3600, "includes": ["clippings removed"] },
        "location": {
            "where": "at-customer",
            "service_area": [
                { "kind": "circle", "lat_e6": 52000000, "lon_e6": 13000000, "radius_m": 5000 }
            ],
            "address_disclosure": "on-agreement"
        },
        "relationship": { "open_to": "anyone" },
        "service_record": {
            "issues_fulfilment_receipt": true,
            "warranty_secs": 0,
            "retention_secs": 31536000
        }
    })
}

fn is_err(v: &Value, code: i64) -> bool {
    v["error"]["code"].as_i64() == Some(code)
}

/// `listing.set` on both stacks (asserting it succeeded), then `listing.get`
/// -- returns the listing id and each build's get response, which is where
/// the signed envelope lives (`listing.set` returns only ids and a count).
async fn set_and_get(h: &Harness, params: Value) -> (String, Value, Value) {
    let (sw, sn) = both_rpc(h, "listing.set", params).await;
    assert!(sw["result"]["listing_id"].is_string(), "listing.set wasm: {sw}");
    assert!(sn["result"]["listing_id"].is_string(), "listing.set native: {sn}");
    let id = sw["result"]["listing_id"].as_str().unwrap().to_string();
    let (gw, gn) = both_rpc(h, "listing.get", json!({ "listing_id": id })).await;
    (id, gw, gn)
}

#[tokio::test]
async fn scenario_37_listing_set_without_certificate_refused_parity() {
    let h = harness().await;
    let (w, n) = both_rpc(&h, "listing.set", full_listing_params("hedge", "Hedge trimming")).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602));
    assert!(w["error"]["message"].as_str().unwrap().contains("signing-not-enrolled"));
}

#[tokio::test]
async fn scenario_38_listing_set_all_blocks_byte_identical_envelope_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let (listing_id, mut w, mut n) =
        set_and_get(&h, full_listing_params("hedge-trimming", "Hedge trimming")).await;

    assert_eq!(
        w["result"]["envelope"], n["result"]["envelope"],
        "the signed listing envelope must be byte-identical"
    );
    assert!(w["result"]["envelope"].is_string(), "no envelope in listing.get: {w}");

    // The stored pointer row round-trips identically once the wall-clock
    // `updated_at_secs` is stripped.
    strip_volatile(&mut w);
    strip_volatile(&mut n);
    assert_eq!(w, n);

    let expected =
        syneroym_roym_core::listing::derive_listing_id(&owner_did(), "hedge-trimming").unwrap();
    assert_eq!(listing_id, expected);
}

#[tokio::test]
async fn scenario_39_listing_edit_is_a_new_version_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let (v1w, v1n) =
        both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;
    assert!(!is_err(&v1w, -32602), "v1 wasm: {v1w}");
    assert!(v1w["result"]["record_id"].is_string(), "v1 wasm not ok: {v1w} / native: {v1n}");
    let (v2w, v2n) = both_rpc(
        &h,
        "listing.set",
        full_listing_params("hedge-trimming", "Hedge trimming (weekly)"),
    )
    .await;
    assert_eq!(v2w["result"]["version_count"], 2, "v2 wasm: {v2w} / native: {v2n}");
    assert_eq!(v2w["result"]["record_id"], v2n["result"]["record_id"]);
    assert_ne!(v1w["result"]["record_id"], v2w["result"]["record_id"]);

    let listing_id = v2w["result"]["listing_id"].as_str().unwrap().to_string();

    // One pointer row, two history rows, on both builds.
    let (getw, getn) = both_rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    assert_eq!(getw["result"]["record_id"], v2w["result"]["record_id"]);
    assert_eq!(getw["result"]["record_id"], getn["result"]["record_id"]);

    let (histw, histn) = both_rpc(&h, "listing.history", json!({ "listing_id": listing_id })).await;
    assert_eq!(histw["result"]["history"].as_array().unwrap().len(), 2);
    assert_eq!(histw["result"]["history"], histn["result"]["history"]);
}

#[tokio::test]
async fn scenario_40_listing_set_float_amount_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let mut params = full_listing_params("hedge-trimming", "Hedge trimming");
    params["payment"]["amount_minor"] = json!(3500.5);
    let (w, n) = both_rpc(&h, "listing.set", params).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602));
    assert!(
        w["error"]["message"].as_str().unwrap().contains("listing params"),
        "message should report the rejected params: {w}"
    );
}

#[tokio::test]
async fn scenario_41_listing_address_from_profile_parity() {
    let h = harness().await;
    enrol_signing(&h, "profile").await;
    enrol_signing(&h, "catalog").await;

    both_rpc(
        &h,
        "profile.set",
        json!({ "display_name": "Alice", "conversation_address": "did:key:zAliceConvFromProfile" }),
    )
    .await;

    let mut params = full_listing_params("hedge-trimming", "Hedge trimming");
    params.as_object_mut().unwrap().remove("conversation_address");
    let (_id, gw, gn) = set_and_get(&h, params).await;
    assert_eq!(gw["result"]["envelope"], gn["result"]["envelope"]);

    let (vw, vn) =
        both_rpc(&h, "listing.verify", json!({ "envelope": gw["result"]["envelope"].clone() }))
            .await;
    assert_eq!(vw, vn);
    assert_eq!(vw["result"]["conversation_address"], "did:key:zAliceConvFromProfile");
}

#[tokio::test]
async fn scenario_42_listing_address_missing_no_profile_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let mut params = full_listing_params("hedge-trimming", "Hedge trimming");
    params.as_object_mut().unwrap().remove("conversation_address");
    let (w, n) = both_rpc(&h, "listing.set", params).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602));
}

#[tokio::test]
async fn scenario_43_publication_rate_limit_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    both_rpc(&h, "listing.set-limits", json!({ "window_secs": 3600, "max_per_window": 2 })).await;

    let params = full_listing_params("hedge-trimming", "Hedge trimming");
    let mut outcomes_w = Vec::new();
    let mut outcomes_n = Vec::new();
    for _ in 0..4 {
        let (w, n) = both_rpc(&h, "listing.set", params.clone()).await;
        outcomes_w.push(if is_err(&w, -32602) { "rate-limited" } else { "allow" });
        outcomes_n.push(if is_err(&n, -32602) { "rate-limited" } else { "allow" });
        if is_err(&w, -32602) {
            let retry = w["error"]["data"]["retry_after_secs"].as_u64().unwrap();
            // Close to a full window: the oldest counted publication is only
            // seconds old. A wide lower bound keeps a slow CI host honest.
            assert!((3300..=3600).contains(&retry), "retry_after_secs out of range: {retry}");
            assert_eq!(w["error"]["data"]["admission"], "rate-limited");
        }
    }
    assert_eq!(outcomes_w, ["allow", "allow", "rate-limited", "rate-limited"]);
    assert_eq!(outcomes_n, outcomes_w);
}

#[tokio::test]
async fn scenario_44_withdraw_ignores_publication_budget_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    both_rpc(&h, "listing.set-limits", json!({ "window_secs": 3600, "max_per_window": 2 })).await;

    let params = full_listing_params("hedge-trimming", "Hedge trimming");
    let (first, _) = both_rpc(&h, "listing.set", params.clone()).await;
    let listing_id = first["result"]["listing_id"].as_str().unwrap().to_string();
    // Exhaust the budget.
    both_rpc(&h, "listing.set", params.clone()).await;
    let (blocked, _) = both_rpc(&h, "listing.set", params).await;
    assert!(is_err(&blocked, -32602));

    // Withdrawal is still admitted on both builds.
    let (w, n) = both_rpc(&h, "listing.withdraw", json!({ "listing_id": listing_id })).await;
    assert!(!is_err(&w, -32602), "withdraw refused: {w}");
    assert!(w["result"]["record_id"].is_string(), "withdraw not ok: {w}");
    assert!(n["result"]["record_id"].is_string(), "withdraw not ok: {n}");

    let (gw, gn) = both_rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    assert_eq!(gw["result"]["status"], "withdrawn");
    assert_eq!(gn["result"]["status"], "withdrawn");
}

#[tokio::test]
async fn scenario_45_withdraw_then_get_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let (first, _) =
        both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let listing_id = first["result"]["listing_id"].as_str().unwrap().to_string();

    both_rpc(&h, "listing.withdraw", json!({ "listing_id": listing_id })).await;
    let (mut w, mut n) = both_rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    assert_eq!(w["result"]["status"], "withdrawn");
    strip_volatile(&mut w);
    strip_volatile(&mut n);
    assert_eq!(w, n);

    let (histw, _) = both_rpc(&h, "listing.history", json!({ "listing_id": listing_id })).await;
    assert_eq!(histw["result"]["history"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn scenario_46_listing_verify_good_envelope_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let env = gw["result"]["envelope"].clone();

    let (w, n) = both_rpc(&h, "listing.verify", json!({ "envelope": env })).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["verified"], true);
    assert_eq!(w["result"]["conversation_address"], "did:key:zProviderConv");
}

#[tokio::test]
async fn scenario_47_listing_verify_tampered_envelope_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;

    let (_id, gw, _gn) =
        set_and_get(&h, full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let env_str = gw["result"]["envelope"].as_str().unwrap();
    let mut env: Value = serde_json::from_str(env_str).unwrap();
    // Edit the payload's listing_id -- the signature no longer covers it.
    env["payload"]["listing_id"] = json!("lst_forged");

    let (w, n) = both_rpc(&h, "listing.verify", json!({ "envelope": env.to_string() })).await;
    assert_eq!(w, n);
    assert_eq!(w["result"]["verified"], false);
    assert!(w["result"]["reason"].is_string());
}

#[tokio::test]
async fn scenario_48_availability_set_and_list_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    let (set, _) =
        both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let listing_id = set["result"]["listing_id"].as_str().unwrap().to_string();

    let slots = json!([
        { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 1 },
        { "start_secs": 1_010_000, "end_secs": 1_013_600, "capacity": 2 },
        // A duplicate of the first slot: content-derived id, so one row.
        { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 5 }
    ]);
    let (w, n) =
        both_rpc(&h, "availability.set", json!({ "listing_id": listing_id, "slots": slots })).await;
    assert_eq!(w["result"]["slot_ids"], n["result"]["slot_ids"]);

    let (lw, ln) = both_rpc(&h, "availability.list", json!({ "listing_id": listing_id })).await;
    assert_eq!(lw, ln);
    let listed = lw["result"]["slots"].as_array().unwrap();
    assert_eq!(listed.len(), 2, "the duplicate slot must converge on one row");
    assert!(
        listed[0]["start_secs"].as_u64().unwrap() <= listed[1]["start_secs"].as_u64().unwrap(),
        "slots must be ordered by start_secs"
    );
}

#[tokio::test]
async fn scenario_49_catalog_export_integrity_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    let (set, _) =
        both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let listing_id = set["result"]["listing_id"].as_str().unwrap().to_string();
    both_rpc(
        &h,
        "availability.set",
        json!({ "listing_id": listing_id, "slots": [
            { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 1 }
        ] }),
    )
    .await;

    let (mut w, mut n) = both_rpc(&h, "catalog.export", json!({})).await;

    for side in [&w, &n] {
        let bundle: syneroym_roym_core::backup::Bundle =
            serde_json::from_value(side["result"].clone()).unwrap();
        bundle.check_integrity().expect("exported bundle integrity");
        let sections = &bundle.manifest.sections;
        assert_eq!(sections["listings"].schema_version, 2);
        assert!(sections.contains_key("availability"));
    }

    strip_volatile(&mut w);
    strip_volatile(&mut n);
    assert_eq!(w, n);
}

#[tokio::test]
async fn scenario_50_catalog_import_roundtrip_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    let (set, _) =
        both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;
    let listing_id = set["result"]["listing_id"].as_str().unwrap().to_string();
    both_rpc(
        &h,
        "availability.set",
        json!({ "listing_id": listing_id, "slots": [
            { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 1 }
        ] }),
    )
    .await;

    let (exp, _) = both_rpc(&h, "catalog.export", json!({})).await;
    let bundle = exp["result"].clone();
    let (impw, impn) = both_rpc(&h, "catalog.import", json!({ "bundle": bundle })).await;
    assert!(!is_err(&impw, -32602), "import failed: {impw}");
    assert_eq!(impw["result"]["imported"], impn["result"]["imported"]);

    // The listing re-verifies and its id is preserved on both builds.
    let (getw, getn) = both_rpc(&h, "listing.get", json!({ "listing_id": listing_id })).await;
    assert_eq!(getw["result"]["listing_id"], listing_id);
    let (vw, vn) =
        both_rpc(&h, "listing.verify", json!({ "envelope": getw["result"]["envelope"].clone() }))
            .await;
    assert_eq!(vw["result"]["verified"], true);
    assert_eq!(vn["result"]["verified"], true);
    let (aw, an) = both_rpc(&h, "availability.list", json!({ "listing_id": listing_id })).await;
    assert_eq!(aw["result"]["slots"], an["result"]["slots"]);
    let _ = getn;
}

#[tokio::test]
async fn scenario_51_catalog_import_tampered_listing_refused_parity() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    both_rpc(&h, "listing.set", full_listing_params("hedge-trimming", "Hedge trimming")).await;

    let (exp, _) = both_rpc(&h, "catalog.export", json!({})).await;
    let mut bundle = exp["result"].clone();
    // Edit one stored envelope byte without touching the manifest digest.
    let rows = bundle["sections"]["listings"].as_array_mut().unwrap();
    let env_str = rows[0]["payload"]["envelope"].as_str().unwrap();
    let mut env: Value = serde_json::from_str(env_str).unwrap();
    env["payload"]["title"] = json!("Tampered title");
    rows[0]["payload"]["envelope"] = json!(env.to_string());

    let (w, n) = both_rpc(&h, "catalog.import", json!({ "bundle": bundle })).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602));
}

// ---- conversation ----

/// Opens a direct conversation to `peer` on both stacks and returns the
/// (identical) host conversation id.
async fn open_conv(h: &Harness, peer: &str) -> String {
    let (w, n) = both_rpc(h, "conversation.open", json!({ "address": peer })).await;
    let cw = w["result"]["conversation_id"].as_str().unwrap().to_string();
    assert_eq!(cw, n["result"]["conversation_id"].as_str().unwrap());
    cw
}

fn inbound(id: &str, conversation: &str, author: &str, ts: i64, body: &str) -> ConversationMessage {
    ConversationMessage {
        id: id.to_string(),
        conversation: conversation.to_string(),
        author: author.to_string(),
        sender_timestamp: ts,
        received_at: ts,
        content_type: "text/plain".to_string(),
        body: body.as_bytes().to_vec(),
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    }
}

#[tokio::test]
async fn scenario_52_conversation_open_by_address_parity() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeer52").await;
    assert!(!conv_id.is_empty());

    let (w, n) = both_rpc(&h, "conversation.list", json!({})).await;
    let mut w2 = w.clone();
    let mut n2 = n.clone();
    strip_volatile(&mut w2);
    strip_volatile(&mut n2);
    assert_eq!(w2, n2);
    assert_eq!(w["result"]["conversations"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_53_conversation_send_is_never_optimistic_parity() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeer53").await;

    let (sw, sn) = both_rpc(
        &h,
        "conversation.send",
        json!({ "conversation": conv_id, "body": "hello there" }),
    )
    .await;
    assert_eq!(sw["result"]["state"], "pending");
    assert_eq!(sn["result"]["state"], "pending");

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv_id })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    let cw = normalize_message_ids(&mut hw);
    let cn = normalize_message_ids(&mut hn);
    assert_eq!(cw, 1);
    assert_eq!(cn, 1);
    assert_eq!(hw, hn);
    assert_eq!(hw["result"]["messages"][0]["state"], "pending");
}

#[tokio::test]
async fn scenario_54_inbound_message_reaches_history_parity() {
    let h = harness().await;
    let conv = "conv-54";
    h.deliver(true, inbound("m-54", conv, "did:key:zPeer54", 1_000, "incoming hi")).await;
    h.deliver(false, inbound("m-54", conv, "did:key:zPeer54", 1_000, "incoming hi")).await;

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    assert_eq!(normalize_message_ids(&mut hw), 1);
    assert_eq!(normalize_message_ids(&mut hn), 1);
    assert_eq!(hw, hn);
    assert_eq!(hw["result"]["messages"][0]["body"], "incoming hi");
    assert_eq!(hw["result"]["messages"][0]["direction"], "incoming");
}

#[tokio::test]
async fn scenario_55_inbound_order_is_the_rule_not_arrival_parity() {
    let h = harness().await;
    let conv = "conv-55";
    // wasm learns A then B; native learns B then A.
    h.deliver(true, inbound("m-a", conv, "did:key:zPeer55", 1_000, "first")).await;
    h.deliver(true, inbound("m-b", conv, "did:key:zPeer55", 2_000, "second")).await;
    h.deliver(false, inbound("m-b", conv, "did:key:zPeer55", 2_000, "second")).await;
    h.deliver(false, inbound("m-a", conv, "did:key:zPeer55", 1_000, "first")).await;

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    assert_eq!(normalize_message_ids(&mut hw), 2);
    assert_eq!(normalize_message_ids(&mut hn), 2);
    assert_eq!(hw, hn);
    let msgs = hw["result"]["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["body"], "first");
    assert_eq!(msgs[1]["body"], "second");
}

#[tokio::test]
async fn scenario_56_delivery_state_failed_parity() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeer56").await;
    let (sw, sn) =
        both_rpc(&h, "conversation.send", json!({ "conversation": conv_id, "body": "will fail" }))
            .await;
    let mw = sw["result"]["message_id"].as_str().unwrap().to_string();
    let mn = sn["result"]["message_id"].as_str().unwrap().to_string();

    h.notify_state(true, &mw, ConversationDeliveryState::Failed).await;
    h.notify_state(false, &mn, ConversationDeliveryState::Failed).await;

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv_id })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    assert_eq!(normalize_message_ids(&mut hw), 1);
    assert_eq!(normalize_message_ids(&mut hn), 1);
    assert_eq!(hw, hn);
    assert_eq!(hw["result"]["messages"][0]["state"], "failed");

    // Retry is reached (not method-not-found, not wire-refused) on both.
    let rw = one_rpc(&h, true, "conversation.retry", json!({ "message_id": mw })).await;
    let rn = one_rpc(&h, false, "conversation.retry", json!({ "message_id": mn })).await;
    for r in [&rw, &rn] {
        assert_ne!(r["error"]["code"].as_i64(), Some(-32601));
        assert_ne!(r["error"]["code"].as_i64(), Some(-32013));
    }
}

#[tokio::test]
async fn scenario_57_blocked_sender_never_reaches_inbox_parity() {
    let h = harness().await;
    both_rpc(
        &h,
        "block.add",
        json!({ "person_did": "did:key:zBlocked57", "address": "did:key:zBlocked57" }),
    )
    .await;
    let conv = "conv-57";
    h.deliver(true, inbound("m-57", conv, "did:key:zBlocked57", 1_000, "let me in")).await;
    h.deliver(false, inbound("m-57", conv, "did:key:zBlocked57", 1_000, "let me in")).await;

    let (hw, hn) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    assert_eq!(hw["result"]["messages"].as_array().unwrap().len(), 0);
    assert_eq!(hn["result"]["messages"].as_array().unwrap().len(), 0);

    let (lw, ln) = both_rpc(&h, "conversation.list", json!({})).await;
    assert_eq!(lw["result"]["conversations"].as_array().unwrap().len(), 0);
    assert_eq!(ln["result"]["conversations"].as_array().unwrap().len(), 0);

    // Recorded in the bodiless refused collection on both builds.
    for wasm in [true, false] {
        let refused = h.conv_rows(wasm, "refused_messages").await;
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0]["reason"], "blocked");
        assert!(refused[0].get("body").is_none());
    }
}

#[tokio::test]
async fn scenario_58_refused_message_is_counted_nowhere_parity() {
    let h = harness().await;
    both_rpc(
        &h,
        "block.add",
        json!({ "person_did": "did:key:zBlocked58", "address": "did:key:zBlocked58" }),
    )
    .await;
    let conv = "conv-58";
    h.deliver(true, inbound("m-58", conv, "did:key:zBlocked58", 1_000, "secret word")).await;
    h.deliver(false, inbound("m-58", conv, "did:key:zBlocked58", 1_000, "secret word")).await;

    let (sw, sn) = both_rpc(&h, "conversation.search", json!({ "query": "secret" })).await;
    assert_eq!(sw["result"]["matches"].as_array().unwrap().len(), 0);
    assert_eq!(sn["result"]["matches"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn scenario_59_first_contact_rate_limit_at_inbox_parity() {
    let h = harness().await;
    both_rpc(&h, "contacts.set-limits", json!({ "window_secs": 3600, "max_per_window": 2 })).await;

    let mut admitted_w = 0;
    let mut admitted_n = 0;
    for i in 0..4 {
        let conv = format!("conv-59-{i}");
        h.deliver(true, inbound(&format!("m-59-{i}"), &conv, "did:key:zStranger59", 1_000, "hi"))
            .await;
        h.deliver(false, inbound(&format!("m-59-{i}"), &conv, "did:key:zStranger59", 1_000, "hi"))
            .await;
        let (hw, hn) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
        admitted_w += hw["result"]["messages"].as_array().unwrap().len();
        admitted_n += hn["result"]["messages"].as_array().unwrap().len();
    }
    assert_eq!(admitted_w, 2);
    assert_eq!(admitted_n, 2);
}

#[tokio::test]
async fn scenario_60_group_kind_is_refused_as_unsupported_parity() {
    let h = harness().await;
    let conv_svc = did_for_service("conversation");
    let gw = h.wasm_conversation.create_group(&conv_svc).await.unwrap();
    let gn = h.native_conversation.create_group(&conv_svc).await.unwrap();

    h.deliver(true, inbound("m-60", &gw, "did:key:zPeer60", 1_000, "group hi")).await;
    h.deliver(false, inbound("m-60", &gn, "did:key:zPeer60", 1_000, "group hi")).await;

    let (hw, hn) = both_rpc(&h, "conversation.history", json!({ "conversation": gw })).await;
    assert_eq!(hw["result"]["messages"].as_array().unwrap().len(), 0);
    let _ = hn;
    let (lw, _) = both_rpc(&h, "conversation.list", json!({})).await;
    assert_eq!(lw["result"]["conversations"].as_array().unwrap().len(), 0);

    for wasm in [true, false] {
        let refused = h.conv_rows(wasm, "refused_messages").await;
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0]["reason"], "unsupported-kind");
    }
}

#[tokio::test]
async fn scenario_61_delete_outgoing_message_asks_peer_parity() {
    let h = harness().await;
    let conv_id = open_conv(&h, "did:key:zPeer61").await;
    let (sw, sn) =
        both_rpc(&h, "conversation.send", json!({ "conversation": conv_id, "body": "oops" })).await;
    let mw = sw["result"]["message_id"].as_str().unwrap().to_string();
    let mn = sn["result"]["message_id"].as_str().unwrap().to_string();

    // Each stack deletes its own message id; compare the response shape.
    let dwv = one_rpc(&h, true, "conversation.delete-message", json!({ "message_id": mw })).await;
    let dnv = one_rpc(&h, false, "conversation.delete-message", json!({ "message_id": mn })).await;
    assert_eq!(dwv["result"]["asked_peer"], true);
    assert_eq!(dnv["result"]["asked_peer"], true);
    assert_eq!(dwv["result"]["note"], dnv["result"]["note"]);
    assert!(dwv["result"]["note"].as_str().unwrap().contains("other side"));

    // The tombstoned row keeps its place and loses its body.
    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv_id })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    assert_eq!(normalize_message_ids(&mut hw), 1);
    assert_eq!(normalize_message_ids(&mut hn), 1);
    assert!(hw["result"]["messages"][0].get("body").is_none());
    assert_eq!(hw, hn);

    // One reserved-content-type message queued in the host outbox on both.
    for wasm in [true, false] {
        let (ow, on) = both_rpc(&h, "conversation.outbox", json!({})).await;
        let ob = if wasm { &ow } else { &on };
        let entries = ob["result"]["outbox"].as_array().unwrap();
        assert!(!entries.is_empty(), "a deletion request must be queued");
    }
}

#[tokio::test]
async fn scenario_62_inbound_deletion_request_honoured_only_for_own_message_parity() {
    let h = harness().await;
    let conv = "conv-62";
    // Peer's own message, delivered inbound.
    h.deliver(true, inbound("m-62", conv, "did:key:zPeer62", 1_000, "keep me")).await;
    h.deliver(false, inbound("m-62", conv, "did:key:zPeer62", 1_000, "keep me")).await;

    // A deletion request from the same peer, naming their own message.
    let del_own = |target: &str| ConversationMessage {
        id: format!("del-{target}"),
        conversation: conv.to_string(),
        author: "did:key:zPeer62".to_string(),
        sender_timestamp: 2_000,
        received_at: 2_000,
        content_type: "application/vnd.roym.deletion-request+json".to_string(),
        body: json!({ "message_id": target }).to_string().into_bytes(),
        state: ConversationDeliveryState::Delivered,
        verified: true,
        last_error: None,
    };
    h.deliver(true, del_own("m-62")).await;
    h.deliver(false, del_own("m-62")).await;

    let (hw, hn) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    assert!(hw["result"]["messages"][0].get("body").is_none(), "own message must be tombstoned");
    assert!(hn["result"]["messages"][0].get("body").is_none());

    // A deletion request naming a message the requester did NOT author
    // changes nothing.
    h.deliver(true, {
        let mut m = del_own("m-62");
        m.author = "did:key:zSomeoneElse".to_string();
        m.id = "del-other-w".to_string();
        m
    })
    .await;
    let (hw2, _) = both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    assert_eq!(hw2["result"]["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn scenario_63_conversation_export_integrity_parity() {
    let h = harness().await;
    let conv = "conv-63";
    h.deliver(true, inbound("m-63", conv, "did:key:zPeer63", 1_000, "archive me")).await;
    h.deliver(false, inbound("m-63", conv, "did:key:zPeer63", 1_000, "archive me")).await;

    let (mut w, mut n) = both_rpc(&h, "conversation.export", json!({})).await;
    for side in [&w, &n] {
        let bundle: syneroym_roym_core::backup::Bundle =
            serde_json::from_value(side["result"].clone()).unwrap();
        bundle.check_integrity().expect("conversation bundle integrity");
    }
    strip_volatile(&mut w);
    strip_volatile(&mut n);
    assert_eq!(normalize_message_ids(&mut w), 1);
    assert_eq!(normalize_message_ids(&mut n), 1);
    assert_eq!(w, n);
}

#[tokio::test]
async fn scenario_64_conversation_import_roundtrip_parity() {
    let h = harness().await;
    let conv = "conv-64";
    h.deliver(true, inbound("m-64", conv, "did:key:zPeer64", 1_000, "restore me")).await;
    h.deliver(false, inbound("m-64", conv, "did:key:zPeer64", 1_000, "restore me")).await;

    let (exp, _) = both_rpc(&h, "conversation.export", json!({})).await;
    let (impw, impn) =
        both_rpc(&h, "conversation.import", json!({ "bundle": exp["result"].clone() })).await;
    assert!(!is_err(&impw, -32602), "import failed: {impw}");
    assert_eq!(impw["result"]["imported"], impn["result"]["imported"]);

    let (mut hw, mut hn) =
        both_rpc(&h, "conversation.history", json!({ "conversation": conv })).await;
    strip_volatile(&mut hw);
    strip_volatile(&mut hn);
    assert_eq!(normalize_message_ids(&mut hw), 1);
    assert_eq!(normalize_message_ids(&mut hn), 1);
    assert_eq!(hw, hn);
}

#[tokio::test]
async fn scenario_65_conversation_import_tampered_message_refused_parity() {
    let h = harness().await;
    let conv = "conv-65";
    h.deliver(true, inbound("m-65", conv, "did:key:zPeer65", 1_000, "original")).await;
    h.deliver(false, inbound("m-65", conv, "did:key:zPeer65", 1_000, "original")).await;

    let (exp, _) = both_rpc(&h, "conversation.export", json!({})).await;
    let mut bundle = exp["result"].clone();
    let rows = bundle["sections"]["messages"].as_array_mut().unwrap();
    rows[0]["payload"]["body"] = json!("tampered");

    let (w, n) = both_rpc(&h, "conversation.import", json!({ "bundle": bundle })).await;
    assert_eq!(w, n);
    assert!(is_err(&w, -32602));
}

#[tokio::test]
async fn scenario_66_certificate_verbs_on_catalog_and_conversation_parity() {
    let h = harness().await;
    for service in ["catalog", "conversation"] {
        let (w, n) = both_rpc(&h, &format!("{service}.signing-status"), json!({})).await;
        assert_eq!(w["result"]["certificate"]["state"], "missing");
        assert_eq!(n["result"]["certificate"]["state"], "missing");
        enrol_signing(&h, service).await;
        let (w2, n2) = both_rpc(&h, &format!("{service}.signing-status"), json!({})).await;
        assert_eq!(w2["result"]["certificate"]["state"], "installed");
        assert_eq!(n2["result"]["certificate"]["state"], "installed");
    }
}

// ---- the wire origin ----

fn env(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": params })
}

/// One representative verb each of the six services owns. `require_internal`
/// is the first statement of every `invoke`, so the verb only has to be a
/// string the service would otherwise route -- the refusal happens before
/// dispatch. Deleting `require_internal` from any one service's `invoke`
/// fails here rather than staying green (scenario 71 only proves
/// `api.status` is *not* refused).
const WIRE_REFUSED_VERBS: [(services::Service, &str); 6] = [
    (services::WEB, "session.whoami"),
    (services::CONVERSATION, "conversation.history"),
    (services::PROFILE, "profile.get"),
    (services::CATALOG, "listing.get"),
    (services::TRANSACTION, "request.ping"),
    (services::DIRECTORY, "directory.ping"),
];

#[tokio::test]
async fn scenario_67_every_service_invoke_over_the_wire_is_refused_parity() {
    let h = harness().await;
    for (svc, method) in WIRE_REFUSED_VERBS {
        let (w, n) = h.wire_invoke(svc, &env(method, json!({}))).await;
        assert_eq!(w, n, "{} wire parity", svc.name);
        assert_eq!(w["error"]["code"], -32013, "{}.{method} must be wire-refused: {w}", svc.name);
    }
}

#[tokio::test]
async fn scenario_68_every_service_invoke_locally_is_not_wire_refused_parity() {
    let h = harness().await;
    for (svc, method) in WIRE_REFUSED_VERBS {
        let (w, n) = h.local_invoke(svc, &env(method, json!({}))).await;
        assert_ne!(w["error"]["code"].as_i64(), Some(-32013), "{}.{method} wasm: {w}", svc.name);
        assert_ne!(n["error"]["code"].as_i64(), Some(-32013), "{}.{method} native: {n}", svc.name);
    }
}

#[tokio::test]
async fn scenario_69_same_verbs_locally_are_admitted_parity() {
    let h = harness().await;
    let (gw, gn) = h
        .local_invoke(services::CATALOG, &env("listing.get", json!({ "listing_id": "lst_x" })))
        .await;
    assert_ne!(gw["error"]["code"].as_i64(), Some(-32013));
    assert_ne!(gn["error"]["code"].as_i64(), Some(-32013));

    let (cw, cn) = h
        .local_invoke(
            services::CONVERSATION,
            &env("conversation.history", json!({ "conversation": "c" })),
        )
        .await;
    assert_ne!(cw["error"]["code"].as_i64(), Some(-32013));
    assert_ne!(cn["error"]["code"].as_i64(), Some(-32013));
}

#[tokio::test]
async fn scenario_70_local_call_with_delegated_caller_admitted_on_both_builds() {
    // F17's regression guard: the parity driver already presents a verified
    // delegated caller on a purely local drive. A native mapping that read
    // the caller's auth level on a local path would answer -32013 here while
    // the wasm build passed.
    let h = harness().await;
    for (svc, method, params) in [
        (services::CATALOG, "listing.get", json!({ "listing_id": "lst_x" })),
        (services::CONVERSATION, "conversation.history", json!({ "conversation": "c" })),
    ] {
        let (w, n) = h.local_invoke(svc, &env(method, params)).await;
        assert_ne!(w["error"]["code"].as_i64(), Some(-32013), "{method} wasm: {w}");
        assert_ne!(n["error"]["code"].as_i64(), Some(-32013), "{method} native: {n}");
    }
}

#[tokio::test]
async fn scenario_71_api_status_over_the_wire_is_never_refused_parity() {
    let h = harness().await;
    for svc in services::ALL {
        let (w, n) = h.wire_status(svc).await;
        assert_eq!(w, n, "status mismatch on {}", svc.name);
        assert_ne!(w["error"]["code"].as_i64(), Some(-32013));
        assert_eq!(w["service"], svc.name);
    }
}

#[tokio::test]
async fn scenario_72_web_http_path_unaffected_parity() {
    let h = harness().await;
    let (w, n) = both_rpc(&h, "listing.list", json!({})).await;
    assert_eq!(w, n);
    assert!(w["result"]["listings"].is_array(), "listing.list via /rpc must succeed: {w}");
}

#[tokio::test]
async fn scenario_73_guard_no_c5_verb_answers_method_not_found_or_wire_refused() {
    let h = harness().await;
    enrol_signing(&h, "catalog").await;
    enrol_signing(&h, "conversation").await;

    let (listing_id, gw, _gn) =
        set_and_get(&h, full_listing_params("guard-listing", "Guard listing")).await;
    let env_val = gw["result"]["envelope"].clone();
    let conv_id = open_conv(&h, "did:key:zPeer73").await;
    let (sent, _) =
        both_rpc(&h, "conversation.send", json!({ "conversation": conv_id, "body": "guard" }))
            .await;
    let message_id = sent["result"]["message_id"].as_str().unwrap().to_string();

    // The guard test: every listing, availability and conversation verb,
    // driven once through the local path with params
    // that reach the handler. A -32601 means the verb was never wired; a
    // -32013 means the local admission rule is wrong.
    let calls: Vec<(&str, Value)> = vec![
        ("listing.set", full_listing_params("guard-listing", "Guard listing")),
        ("listing.get", json!({ "listing_id": listing_id })),
        ("listing.list", json!({})),
        ("listing.history", json!({ "listing_id": listing_id })),
        ("listing.withdraw", json!({ "listing_id": listing_id })),
        ("listing.verify", json!({ "envelope": env_val })),
        ("listing.limits", json!({})),
        ("listing.set-limits", json!({ "window_secs": 3600, "max_per_window": 20 })),
        (
            "availability.set",
            json!({ "listing_id": listing_id, "slots": [
            { "start_secs": 1_000_000, "end_secs": 1_003_600, "capacity": 1 }
        ] }),
        ),
        ("availability.list", json!({ "listing_id": listing_id })),
        ("availability.remove", json!({ "slot_id": "slot_missing" })),
        ("catalog.export", json!({})),
        ("catalog.signing-status", json!({})),
        ("conversation.open", json!({ "address": "did:key:zPeer73b" })),
        ("conversation.list", json!({})),
        ("conversation.send", json!({ "conversation": conv_id, "body": "again" })),
        ("conversation.history", json!({ "conversation": conv_id })),
        ("conversation.delivery-status", json!({ "message_id": message_id })),
        ("conversation.outbox", json!({})),
        ("conversation.retry", json!({ "message_id": message_id })),
        ("conversation.delete-message", json!({ "message_id": message_id })),
        ("conversation.search", json!({ "query": "guard" })),
        ("conversation.export", json!({})),
        ("conversation.signing-status", json!({})),
    ];

    for (method, params) in calls {
        let (w, n) = both_rpc(&h, method, params).await;
        for (label, v) in [("wasm", &w), ("native", &n)] {
            let code = v["error"]["code"].as_i64();
            assert_ne!(code, Some(-32601), "{label} {method} answered method-not-found: {v}");
            assert_ne!(code, Some(-32013), "{label} {method} answered wire-refused: {v}");
        }
    }
}

#[tokio::test]
async fn scenario_36_profile_import_foreign_subject_refused_parity() {
    let h = harness().await;

    let exp_req = json!({ "method": "profile.export", "params": {} }).to_string().into_bytes();
    let wasm_exp = h.wasm_http.post("/rpc", exp_req, Some(caller())).await;
    let mut exp_val: Value = serde_json::from_slice(&wasm_exp.body).unwrap();
    let bundle = exp_val.get_mut("result").unwrap();

    bundle["manifest"]["subject_did"] = json!("did:key:zOtherStranger");

    let imp_req = json!({
        "method": "profile.import",
        "params": { "bundle": bundle }
    })
    .to_string()
    .into_bytes();
    let wasm_imp = h.wasm_http.post("/rpc", imp_req.clone(), Some(caller())).await;
    let native_imp = h.native_http.post("/rpc", imp_req, Some(caller())).await;
    assert_eq!(wasm_imp.body, native_imp.body);
    let val: Value = serde_json::from_slice(&wasm_imp.body).unwrap();
    assert_eq!(val["error"]["code"], -32602);
    assert!(val["error"]["message"].as_str().unwrap().contains("bundle belongs to"));
}

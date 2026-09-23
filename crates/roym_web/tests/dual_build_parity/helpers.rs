use std::{
    collections::HashMap,
    fs,
    sync::{
        Arc, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use syneroym_app_host::{
    ConversationSink,
    types::http::{CallerAuth, CallerIdentity, HttpRequest, HttpResponse},
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
    record_signer::{NodeRecordSigner, RecordClock},
    storage::MockStorage,
    test_constants,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider, host_store::QueryOptions};
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_roym_catalog::native::NativeCatalog;
use syneroym_roym_conversation::native::NativeConversation;
use syneroym_roym_core::services;
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

use super::fixtures::*;

pub(crate) fn did_for_service(name: &str) -> String {
    format!("did:key:zRoym{name}")
}

pub(crate) fn owner_identity() -> Identity {
    Identity::from_bytes(&[42; 32])
}

pub(crate) fn owner_did() -> String {
    derive_did_key(&owner_identity().public_key())
}

/// `directory.publish`'s ledger row id is `<published_by>:<now_secs>:
/// <record_id>` -- `now_secs` is the one clock the two builds cannot share
/// a reading of, unlike the signed envelope's own pinned `issued_at_secs`.
/// A same-named field would already be caught by the removals below, but
/// here the volatile value is folded into a composite string used as a
/// bundle row's `id`, so it needs picking out by shape instead: the first
/// run of digits sandwiched between two colons. Returns `None` when the id
/// has no such run (every other collection's id -- a DID or a
/// content-derived record id -- never does).
pub(crate) fn normalize_ledger_row_id(id: &str) -> Option<String> {
    let colons: Vec<usize> = id.match_indices(':').map(|(i, _)| i).collect();
    for pair in colons.windows(2) {
        let (start, end) = (pair[0] + 1, pair[1]);
        if start < end && id.as_bytes()[start..end].iter().all(u8::is_ascii_digit) {
            return Some(format!("{}:TS:{}", &id[..pair[0]], &id[pair[1] + 1..]));
        }
    }
    None
}

/// The rate limiter's `retry_after_secs` (see the removal below) is a
/// difference of two unpinned wall-clock reads, and `directory.publish`
/// folds the same number into this human-readable error `message`, not
/// just the named field. Picks out `retry in <digits>s` and blanks the
/// digits; returns `None` for every other message, which is untouched.
pub(crate) fn normalize_retry_message(msg: &str) -> Option<String> {
    let marker = "retry in ";
    let start = msg.find(marker)? + marker.len();
    let digits_end = start + msg[start..].bytes().take_while(u8::is_ascii_digit).count();
    if digits_end == start || !msg[digits_end..].starts_with('s') {
        return None;
    }
    Some(format!("{}N{}", &msg[..start], &msg[digits_end..]))
}

pub(crate) fn strip_volatile(val: &mut Value) {
    match val {
        Value::Object(map) => {
            if let Some(normalized) =
                map.get("id").and_then(Value::as_str).and_then(normalize_ledger_row_id)
            {
                map.insert("id".to_string(), Value::String(normalized));
            }
            if let Some(normalized) =
                map.get("message").and_then(Value::as_str).and_then(normalize_retry_message)
            {
                map.insert("message".to_string(), Value::String(normalized));
            }
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
            // Host wall-clock written at the moment the decline row is
            // persisted — not derived from the pinned RecordClock, so it
            // can differ by a second between WASM and native builds.
            map.remove("declined_at_secs");
            map.remove("last_activity_ms");
            // A section digest folds in every row's bytes, including the
            // wall-clock fields removed above, so it is volatile too. The
            // raw bundle's own `check_integrity` runs before any strip.
            map.remove("digest");
            map.remove("sender_timestamp_ms");
            // The directory's own clock (`received_at_secs`,
            // `answered_at_secs`) and anything computed from a difference
            // of two wall-clock reads across the two builds
            // (`retry_after_secs`, `age_secs`) -- neither build's clock is
            // pinned, only the signed listing envelope's `issued_at_secs`
            // is, so these can legitimately differ by the odd second.
            //
            // `age_secs` could in principle be value-asserted (it must be
            // `consumer_now - issued_at_secs`, never the directory's own
            // `received_at_secs`), but not in this harness: the signing
            // clock is pinned 240 s in the *future* (for certificate
            // freshness), so `now - issued_at` saturates to 0 on both
            // builds and a refactor to `received_at` would also read ~0.
            // A past-clock signer fixture would be needed; tracked in the
            // deferred backlog.
            map.remove("received_at_secs");
            map.remove("answered_at_secs");
            map.remove("retry_after_secs");
            map.remove("age_secs");
            map.remove("last_ok_secs");
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

/// `strip_volatile` as a value-returning helper over a borrow, for a
/// comparison written as `assert_eq!(stripped(&w), stripped(&n))` without
/// giving up the original value for later use in the same test --
/// directory responses carry each build's own unsynchronized wall clock
/// (`received_at_secs`, `answered_at_secs`, `age_secs`) or a value derived
/// from two such reads (`retry_after_secs`), any of which can legitimately
/// differ by the odd second between the two builds' calls.
pub(crate) fn stripped(v: &Value) -> Value {
    let mut c = v.clone();
    strip_volatile(&mut c);
    c
}

pub(crate) fn caller() -> CallerContext {
    custom_caller(&owner_did())
}

pub(crate) fn custom_caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext { subject_did: did.to_string(), ..Default::default() },
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

/// The identity a fan-out `query-source` / `publish-to-source` call
/// presents to a *foreign* directory over the wire -- deliberately not
/// this node's own owner, so a directory's publication limiter (keyed on
/// the verified connection identity) sees a distinct party.
pub(crate) fn foreign_consumer_caller() -> CallerContext {
    custom_caller("did:key:zForeignConsumer")
}

/// A wire caller with no verified identity -- `AuthLevel::System` maps to
/// `CallerOrigin::Anonymous` (only `Delegated` / `Ucan` map to `Verified`).
pub(crate) fn anon_wire_caller() -> CallerContext {
    CallerContext {
        caller_did: String::new(),
        app_instance: None,
        session: SessionContext::default(),
        auth: AuthLevel::System,
        proof: None,
    }
}

/// Maps a fan-out source DID onto the directory instance it stands for and
/// whether the call arrives anonymously. `hForeignWire` / `hForeignAnon`
/// reach this node's own directory over a genuine wire round trip;
/// `hForeignWire2` reaches the second, independently-stored directory.
/// `hForeign` is intentionally absent: it keeps its existing local
/// routing so the earlier client-half scenarios that use it are
/// unchanged.
pub(crate) fn foreign_wire_route(target: &str) -> Option<(String, bool)> {
    match target {
        "did:key:hForeignWire" => Some((did_for_service("directory"), false)),
        "did:key:hForeignAnon" => Some((did_for_service("directory"), true)),
        "did:key:hForeignWire2" => Some((did_for_service("directory2"), false)),
        _ => None,
    }
}

/// How many forged hits a hostile fake source returns per `query-source`.
pub(crate) const HOSTILE_SOURCE_FORGERIES: usize = 15;

/// A few shapes a forged envelope can take, cycled across a hostile
/// source's hits so the consumer's verification is exercised past its
/// outermost JSON parse: not-an-object, an object with no signature, an
/// object with an unusable signature, a wrong record type, and an
/// issued-at far in the future.
pub(crate) const FORGED_ENVELOPE_SHAPES: &[&str] = &[
    "{\"not\":\"a signed listing envelope\"}",
    "\"just a string\"",
    "{\"payload\":{\"record_type\":\"listing\"},\"delegation\":null}",
    "{\"payload\":{\"record_type\":\"listing\"},\"signature\":\"!!not-base64!!\"}",
    "{\"payload\":{\"record_type\":\"profile\"},\"signature\":\"AAAA\"}",
    "{\"payload\":{\"record_type\":\"listing\",\"issued_at_secs\":9999999999},\"signature\":\"\
     AAAA\"}",
];

/// Canned response for the hostile fake sources `did:key:hForge1` /
/// `did:key:hForge2`, and the `did:key:hTrunc` source that answers with
/// no hits but `truncated: true`. A real second directory cannot serve
/// forgeries -- its own `directory.publish` verifies every envelope at
/// the door -- so a canned page is the only way to drive "a source that
/// returns nothing but forgeries" or "a source that had more matches
/// than it would return". The consumer's own verification in
/// `query-source`, not the source, is what must reject a forgery.
/// Returned as the `Value::String` shape both real directory calls
/// produce, so the two builds see byte-identical input.
pub(crate) fn hostile_source_response(target: &str) -> Option<Value> {
    if target == "did:key:hTrunc" {
        return Some(Value::String(
            json!({ "result": { "hits": [], "truncated": true } }).to_string(),
        ));
    }
    let tag = match target {
        "did:key:hForge1" => "a",
        "did:key:hForge2" => "b",
        _ => return None,
    };
    let hits: Vec<Value> = (0..HOSTILE_SOURCE_FORGERIES)
        .map(|i| {
            json!({
                "listing_id": format!("forged-{tag}-{i}"),
                "record_id": format!("forged-rec-{tag}-{i}"),
                "envelope": FORGED_ENVELOPE_SHAPES[i % FORGED_ENVELOPE_SHAPES.len()],
                "issued_at_secs": 4_000_000_000u64,
                "received_at_secs": 4_000_000_000u64,
                "area_match": { "kind": "not-queried" }
            })
        })
        .collect();
    Some(Value::String(json!({ "result": { "hits": hits } }).to_string()))
}

pub(crate) trait Driver {
    async fn invoke_web(&self, request: &str) -> Result<String, String>;
    async fn status(&self, service_name: &str) -> Result<String, String>;
}

pub(crate) struct WasmDriver {
    pub(crate) engine: Arc<AppSandboxEngine>,
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

pub(crate) struct NativeDriver {
    pub(crate) web: Arc<NativeWeb<NativeAppHost>>,
    pub(crate) profile: Arc<NativeProfile<NativeAppHost>>,
    pub(crate) conversation: Arc<NativeConversation<NativeAppHost>>,
    pub(crate) catalog: Arc<NativeCatalog<NativeAppHost>>,
    pub(crate) transaction: Arc<NativeTransaction<NativeAppHost>>,
    pub(crate) directory: Arc<NativeDirectory<NativeAppHost>>,
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

pub(crate) trait HttpDriver {
    async fn post(
        &self,
        path_and_query: &str,
        body: Vec<u8>,
        caller: Option<CallerContext>,
    ) -> HttpResponse;
}

pub(crate) struct WasmHttpDriver {
    pub(crate) engine: Arc<AppSandboxEngine>,
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

pub(crate) struct NativeHttpDriver {
    pub(crate) adapter: Arc<NativeHttpAdapter>,
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

pub(crate) struct Harness {
    pub(crate) owner: Identity,
    pub(crate) owner_did: String,
    pub(crate) wasm: WasmDriver,
    pub(crate) native: NativeDriver,
    pub(crate) wasm_http: WasmHttpDriver,
    pub(crate) native_http: NativeHttpDriver,
    /// The second directory's local (`host_for`) native instance -- used
    /// only by test setup helpers that must reach a local-only verb
    /// (`directory.set-settings`) or seed its store directly. The fan-out
    /// path reaches it over the wire through the proxies.
    pub(crate) native_directory2: Arc<NativeDirectory<NativeAppHost>>,
    pub(crate) native_factories: Vec<Arc<NativeHostFactory>>,
    pub(crate) wasm_proxy: Arc<TestWasmServiceProxy>,
    pub(crate) native_proxy: Arc<TestNativeServiceProxy>,
    /// The one factory whose conversation sink is wired, so a test can
    /// push an inbound message straight at Roym's own inbox on the native
    /// stack the same way `AppSandboxEngine` does on the wasm one.
    pub(crate) conv_factory: Arc<NativeHostFactory>,
    /// The shared host `ConversationService` per stack -- a test creates a
    /// group here (a kind the inbox refuses) or reads delivery state.
    pub(crate) wasm_conversation: Arc<ConversationService>,
    pub(crate) native_conversation: Arc<ConversationService>,
    /// `host_for_wire`-built native instances: the parity harness is the
    /// only caller of that constructor, and it is what makes the wire
    /// refusal a real two-build comparison rather than a wasm-only one.
    pub(crate) wire_native: Vec<(&'static str, Arc<dyn NativeService>)>,
    /// Storage + keystore per stack, so a test can read a collection no
    /// verb exposes (`refused_messages`).
    pub(crate) wasm_storage: Arc<dyn StorageProvider>,
    pub(crate) native_storage: Arc<dyn StorageProvider>,
    pub(crate) wasm_ks: Arc<KeyStore>,
    pub(crate) native_ks: Arc<KeyStore>,
    pub(crate) _wasm_ws_senders: Arc<WebSocketSenders>,
    pub(crate) _native_ws_senders: Arc<WebSocketSenders>,
    pub(crate) _wasm_dir: tempfile::TempDir,
    pub(crate) _native_dir: tempfile::TempDir,
}

impl Harness {
    pub(crate) fn caller(&self) -> CallerContext {
        custom_caller(&self.owner_did)
    }

    /// Drives one service's `invoke` as a call that arrived over the
    /// network, on both builds: wasm through `execute_wasm_json_from_wire`,
    /// native through the `host_for_wire` instance. Returns each build's
    /// parsed roym `Response`.
    pub(crate) async fn wire_invoke(
        &self,
        svc: services::Service,
        envelope: &Value,
    ) -> (Value, Value) {
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
    pub(crate) async fn wire_status(&self, svc: services::Service) -> (Value, Value) {
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
    pub(crate) async fn local_invoke(
        &self,
        svc: services::Service,
        envelope: &Value,
    ) -> (Value, Value) {
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

    /// Drives one verb against the **second** directory over a local
    /// (`host_for` / `execute_wasm_json`) dispatch, on both builds. Used by
    /// test setup to reach that directory's local-only verbs
    /// (`directory.set-settings`) and to seed its store, since the wire
    /// path -- the only one the fan-out uses -- refuses everything outside
    /// `WIRE_REACHABLE`.
    pub(crate) async fn dir2_local(&self, method: &str, params: Value) -> (Value, Value) {
        let env_str = env(method, params).to_string();
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
            .execute_wasm_json(
                &did_for_service("directory2"),
                services::DIRECTORY.interface,
                &req,
                Some(caller()),
            )
            .await
            .expect("wasm dir2 local invoke");
        let native = self
            .native_directory2
            .dispatch(NativeInvocation {
                interface: services::DIRECTORY.interface.to_string(),
                method: "invoke".to_string(),
                params: json!([env_str]),
                caller: caller(),
            })
            .await
            .expect("native dir2 local invoke");
        (unwrap_payload(wasm), unwrap_payload(native.payload))
    }

    /// Pushes one inbound message at Roym's own inbox on the chosen stack,
    /// the same entry point `ConversationService`'s delivery worker uses.
    pub(crate) async fn deliver(&self, wasm: bool, msg: ConversationMessage) {
        let conv_id = did_for_service("conversation");
        if wasm {
            ConversationNotifier::notify_message(&*self.wasm.engine, &conv_id, msg).await;
        } else {
            ConversationNotifier::notify_message(&*self.conv_factory, &conv_id, msg).await;
        }
    }

    pub(crate) async fn notify_state(
        &self,
        wasm: bool,
        message_id: &str,
        state: ConversationDeliveryState,
    ) {
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
    pub(crate) async fn conv_rows(&self, wasm: bool, collection: &str) -> Vec<Value> {
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
pub(crate) fn unwrap_payload(v: Value) -> Value {
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
pub(crate) fn normalize_message_ids(val: &mut Value) -> usize {
    let mut order: Vec<String> = Vec::new();
    collect_ordered_ids(val, &mut order);
    let map: HashMap<String, String> =
        order.iter().enumerate().map(|(i, id)| (id.clone(), format!("<msg:{i}>"))).collect();
    rewrite_ids(val, &map);
    map.len()
}

pub(crate) fn collect_ordered_ids(val: &Value, out: &mut Vec<String>) {
    match val {
        Value::Object(map) => {
            for (k, v) in map {
                if (k == "messages" || k == "matches" || k == "cards")
                    && let Value::Array(rows) = v
                {
                    for row in rows {
                        let maybe_id =
                            row.get("id").or_else(|| row.get("message_id")).and_then(Value::as_str);
                        if let Some(id) = maybe_id
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

pub(crate) fn rewrite_ids(val: &mut Value, map: &HashMap<String, String>) {
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
pub(crate) struct TestWasmServiceProxy {
    pub(crate) engine: Arc<AppSandboxEngine>,
    pub(crate) invocations: AtomicUsize,
}

#[async_trait::async_trait]
impl ServiceProxy for TestWasmServiceProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        let target = request.target_service.as_str();

        if let Some(canned) = hostile_source_response(target) {
            return Ok(canned);
        }

        if let Some((service_id, anon)) = foreign_wire_route(target) {
            let caller = if anon { anon_wire_caller() } else { foreign_consumer_caller() };
            let rpc_req = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                method: request.method,
                params: request.params,
                id: None,
                idempotency_key: request.idempotency_key,
            };
            return self
                .engine
                .execute_wasm_json_from_wire(
                    &service_id,
                    &request.interface,
                    &rpc_req,
                    Some(caller),
                )
                .await
                .map_err(|e| ProxyError::Internal(e.to_string()));
        }

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
pub(crate) struct TestNativeServiceProxy {
    pub(crate) web: Arc<NativeWeb<NativeAppHost>>,
    pub(crate) profile: Arc<NativeProfile<NativeAppHost>>,
    pub(crate) conversation: Arc<NativeConversation<NativeAppHost>>,
    pub(crate) catalog: Arc<NativeCatalog<NativeAppHost>>,
    pub(crate) transaction: Arc<NativeTransaction<NativeAppHost>>,
    pub(crate) directory: Arc<NativeDirectory<NativeAppHost>>,
    /// `host_for_wire`-built instances the foreign-wire targets route to,
    /// so a fan-out call is a real wire round trip on the native stack too.
    pub(crate) directory_wire: Arc<dyn NativeService>,
    pub(crate) directory2_wire: Arc<dyn NativeService>,
    pub(crate) invocations: AtomicUsize,
}

#[async_trait::async_trait]
impl ServiceProxy for TestNativeServiceProxy {
    async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
        self.invocations.fetch_add(1, Ordering::SeqCst);
        let target = request.target_service.as_str();

        if let Some(canned) = hostile_source_response(target) {
            return Ok(canned);
        }

        if let Some((_service_id, anon)) = foreign_wire_route(target) {
            let svc = if target == "did:key:hForeignWire2" {
                &self.directory2_wire
            } else {
                &self.directory_wire
            };
            let caller = if anon { anon_wire_caller() } else { foreign_consumer_caller() };
            let inv = NativeInvocation {
                interface: request.interface,
                method: request.method,
                params: request.params,
                caller,
            };
            return svc
                .dispatch(inv)
                .await
                .map(|r| r.payload)
                .map_err(|e| ProxyError::Internal(e.to_string()));
        }

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

pub(crate) fn wasm_deploy_manifest(bytes: Vec<u8>, iface: &str) -> DeployManifest {
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

pub(crate) async fn harness() -> Harness {
    harness_with_unbound(None).await
}

#[expect(clippy::too_many_lines, reason = "complex dual-build harness builder")]
pub(crate) async fn harness_with_unbound(skip: Option<&'static str>) -> Harness {
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
    for svc in services::SIBLINGS {
        if skip == Some(svc.name) {
            continue;
        }
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
    let node_identity = Arc::new(Identity::generate().unwrap());
    let wasm_resolver = Arc::new(LogicalResolver::new(wasm_inventory));
    for svc in services::ALL {
        let service_id = did_for_service(svc.name);
        wasm_reg
            .set_app_context(service_id.clone(), app_instance.to_string(), svc.name.to_string())
            .await
            .unwrap();
        wasm_reg.set_owner(service_id, owner_did.clone()).await.unwrap();
    }
    wasm_reg
        .set_app_context(
            did_for_service("directory2"),
            app_instance.to_string(),
            "directory".to_string(),
        )
        .await
        .unwrap();
    wasm_reg.set_owner(did_for_service("directory2"), owner_did.clone()).await.unwrap();

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

    let wasm_proxy = Arc::new(TestWasmServiceProxy {
        engine: wasm_engine.clone(),
        invocations: AtomicUsize::new(0),
    });
    wasm_engine
        .service_proxy
        .set(Arc::downgrade(&wasm_proxy) as Weak<dyn ServiceProxy>)
        .expect("set service proxy");

    // The directory component is deployed a second time under its own
    // service id, so the parity suite has a genuinely independent second
    // directory -- its own store, its own settings, its own publications --
    // to drive `versions_differ`, the per-source share, and refused-evidence
    // round-robining, none of which one directory can produce.
    let directory_wasm_bytes = wasm_binaries
        .iter()
        .find(|(n, _)| *n == "directory")
        .map(|(_, b)| b.clone())
        .expect("directory wasm bytes");

    for (name, bytes) in wasm_binaries {
        let iface = services::ALL.iter().find(|s| s.name == name).map(|s| s.interface).unwrap();
        let service_id = did_for_service(name);
        let manifest = wasm_deploy_manifest(bytes, iface);
        wasm_engine.deploy_wasm(&service_id, &manifest).await.expect("deploy wasm service");
    }
    wasm_engine
        .deploy_wasm(
            &did_for_service("directory2"),
            &wasm_deploy_manifest(directory_wasm_bytes, services::DIRECTORY.interface),
        )
        .await
        .expect("deploy second directory wasm service");

    // Pinned so the two stacks stamp every envelope with the same second
    // and compare byte for byte. The value has to clear two windows at
    // once: a delegated record must be dated at or after its certificate's
    // own `issued_at` (the certificate is minted a few seconds into each
    // scenario, after this harness is built), and on import a listing is
    // re-verified against the verifier's wall clock, which rejects a record
    // more than `max_clock_skew_secs` (300s) in the future. A step ahead of
    // "now" satisfies both.
    //
    // Read here rather than at the top of the harness: everything above
    // this line -- above all the wasm compile per deployed component --
    // would otherwise spend the same budget, and under a loaded run (every
    // scenario in this file builds its own harness, several at a time) that
    // setup alone can outlast it. Then the certificate minted later, at
    // real wall-clock time, is dated *after* the pinned signing clock and
    // every signature in the scenario fails with "Delegation certificate
    // issued_at is in the future". Below this line only the cheap native
    // half of the harness is left, so the budget covers the scenario body.
    let wall_now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let fixed_clock = RecordClock::Fixed(wall_now + 240);

    let wasm_record_signer =
        NodeRecordSigner::with_clock(node_identity.clone(), wasm_reg, fixed_clock);
    wasm_engine.record_signer.set(wasm_record_signer).expect("set wasm record_signer");

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
        if skip == Some(svc.name) {
            continue;
        }
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
    native_reg
        .set_app_context(
            did_for_service("directory2"),
            app_instance.to_string(),
            "directory".to_string(),
        )
        .await
        .unwrap();
    native_reg.set_owner(did_for_service("directory2"), owner_did.clone()).await.unwrap();

    let native_conversation =
        test_conversation_service(native_storage.clone(), native_ks.clone(), native_reg.clone());

    let native_record_signer =
        NodeRecordSigner::with_clock(node_identity, native_reg.clone(), fixed_clock);

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
    let f_directory2 = make_factory("directory2");

    let f_web_cl = f_web.clone();
    let f_web_http = f_web.clone();
    let native_web = Arc::new(NativeWeb::new(
        did_for_service("web"),
        move |caller| f_web_cl.host_for(caller),
        move |caller| f_web_http.host_for_wire(caller),
    ));

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

    // The second directory instance. A local `host_for` build is used for
    // test setup (writing its `settings`, publishing directly into its
    // store) because `directory.set-settings` is local-only and cannot be
    // reached over the wire; the fan-out path reaches it through a
    // `host_for_wire` build held by the proxies.
    let f_dir2_cl = f_directory2.clone();
    let native_directory2 =
        Arc::new(NativeDirectory::new(did_for_service("directory2"), move |caller| {
            f_dir2_cl.host_for(caller)
        }));
    let f_dir2_wire = f_directory2.clone();
    let native_directory2_wire: Arc<dyn NativeService> =
        Arc::new(NativeDirectory::new(did_for_service("directory2"), move |caller| {
            f_dir2_wire.host_for_wire(caller)
        }));
    let f_dir_wire = f_directory.clone();
    let native_directory_wire: Arc<dyn NativeService> =
        Arc::new(NativeDirectory::new(did_for_service("directory"), move |caller| {
            f_dir_wire.host_for_wire(caller)
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
        let fw_http = f_web.clone();
        vec![
            (
                "web",
                Arc::new(NativeWeb::new(
                    did_for_service("web"),
                    move |c| fw.host_for_wire(c),
                    move |c| fw_http.host_for_wire(c),
                )) as Arc<dyn NativeService>,
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
        directory_wire: native_directory_wire.clone(),
        directory2_wire: native_directory2_wire.clone(),
        invocations: AtomicUsize::new(0),
    });

    let native_factories = vec![
        f_web.clone(),
        f_profile.clone(),
        f_conversation.clone(),
        f_catalog.clone(),
        f_transaction.clone(),
        f_directory.clone(),
        f_directory2.clone(),
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
        native_directory2: native_directory2.clone(),
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

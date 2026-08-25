#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The receiver-side idempotency fence at the *wire* entry point
//! ([ADR-0023](../../../docs/decisions/0023-durable-async-primitives.md) §4).
//!
//! A call arriving from another node never passes through `ProxyRouter` --
//! it lands in the route handler's own `dispatch_json_rpc_once`, which has
//! its own native and WASM arms. A fence on only the proxy would be
//! bypassed by every remote caller, which is every caller that actually
//! needed a durable queue. These tests drive that second entry point
//! directly, against a real `RouteHandler::init` composition, so "one
//! guard, both entry points" is a fact rather than an intention.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use dashmap::DashMap;
use serde_json::{Value, json};
use syneroym_core::{
    config::SubstrateConfig,
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{
    AdaptationStage, EncryptionStage, RouteHandler, RouteHandlerDeps, RoutePipeline, RoutePreamble,
    RouteProtocol, RouteTransport, ServiceStage, TransportStage,
};
use syneroym_rpc::{
    AuthLevel, CallerContext, JsonRpcErrorResponse, JsonRpcResponse, NativeDispatchRegistry,
    NativeInvocation, NativeResponse, NativeService, RpcResult, SessionContext,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tempfile::{TempDir, tempdir};

const TARGET_SERVICE: &str = "svc-target";

#[derive(Debug, Default)]
struct NoopControlPlane;

#[async_trait::async_trait]
impl NativeService for NoopControlPlane {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        Ok(NativeResponse { payload: Value::Null })
    }
}

/// Counts how many times it actually ran, which is the only thing these
/// tests really need to observe.
#[derive(Debug, Default)]
struct CountingService {
    invoked: AtomicUsize,
}

#[async_trait::async_trait]
impl NativeService for CountingService {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let n = self.invoked.fetch_add(1, Ordering::SeqCst);
        Ok(NativeResponse { payload: json!({ "ran": n }) })
    }
}

struct Harness {
    handler: RouteHandler,
    service: Arc<CountingService>,
    dir: TempDir,
}

async fn harness() -> Harness {
    let dir = tempdir().unwrap();
    let config = SubstrateConfig::default();
    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(dir.path(), false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
    let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

    // The guard fences a key only for a target the endpoint registry knows
    // is deployed here, so the fixture has to register it the way a deploy
    // would. Deliberately no `state.db`: a service that has never touched
    // its own data layer has none, and must still be fenceable.
    registry
        .register(
            TARGET_SERVICE.to_string(),
            "greeter".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: TARGET_SERVICE.to_string() },
        )
        .await
        .unwrap();

    let app_sandbox_engine = Arc::new(
        AppSandboxEngine::init(
            &config,
            vec![],
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            registry.clone(),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap(),
    );
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
    let deps = RouteHandlerDeps {
        logical_resolver: syneroym_app_orchestration::empty_resolver(),
        key_store,
        storage_provider: storage_provider as Arc<dyn StorageProvider>,
        app_sandbox_engine,
        messaging_broker,
        native_dispatch: NativeDispatchRegistry::default(),
        native_http: Arc::new(DashMap::new()),
        websocket_senders: syneroym_rpc::WebSocketSenders::new(),
        http_routes,
        assets: Arc::new(DashMap::new()),
        sse_permits: Arc::new(DashMap::new()),
        control_plane_service: Arc::new(NoopControlPlane),
        control_plane: None,
    };
    let handler = RouteHandler::init(
        "test-orchestrator".to_string(),
        &config,
        registry,
        [11u8; 32],
        None,
        deps,
    )
    .await
    .unwrap();

    let service = Arc::new(CountingService::default());
    handler.register_native_service(TARGET_SERVICE.to_string(), service.clone());
    Harness { handler, service, dir }
}

fn pipeline() -> RoutePipeline {
    RoutePipeline {
        encryption: EncryptionStage::None,
        transport: TransportStage::Binary,
        adaptation: AdaptationStage::None,
        service: ServiceStage::NativeService { service_id: TARGET_SERVICE.to_string() },
    }
}

fn preamble() -> RoutePreamble {
    RoutePreamble {
        transport: RouteTransport::Binary,
        protocol: RouteProtocol::JsonRpc,
        interface: "greeter".to_string(),
        service_id: TARGET_SERVICE.to_string(),
        enc: None,
        pubkey: None,
        delegation: None,
        ucan: None,
        dir: None,
    }
}

fn caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext::default(),
        auth: AuthLevel::Delegated,
        proof: None,
    }
}

fn body(key: Option<&str>) -> Vec<u8> {
    let mut request = json!({
        "jsonrpc": "2.0",
        "method": "greet",
        "params": {},
        "id": 1,
    });
    if let Some(key) = key {
        request["idempotency_key"] = json!(key);
    }
    serde_json::to_vec(&request).unwrap()
}

fn result_of(frame: &[u8]) -> Value {
    serde_json::from_slice::<JsonRpcResponse>(frame)
        .unwrap_or_else(|_| panic!("not a success frame: {}", String::from_utf8_lossy(frame)))
        .result
}

fn error_of(frame: &[u8]) -> JsonRpcErrorResponse {
    serde_json::from_slice(frame)
        .unwrap_or_else(|_| panic!("not an error frame: {}", String::from_utf8_lossy(frame)))
}

/// The whole point of the second entry point: a retry that arrives over
/// the wire is answered from the first call's record, and the target does
/// not run again.
#[tokio::test]
async fn a_remote_call_is_deduplicated_at_the_receiving_node() {
    let h = harness().await;
    let who = caller("did:key:zRemoteCaller");

    let first = h
        .handler
        .dispatch_json_rpc_once(&pipeline(), &preamble(), Some(&who), &body(Some("k1")))
        .await
        .unwrap();
    assert_eq!(h.service.invoked.load(Ordering::SeqCst), 1);

    let repeat = h
        .handler
        .dispatch_json_rpc_once(&pipeline(), &preamble(), Some(&who), &body(Some("k1")))
        .await
        .unwrap();
    assert_eq!(
        result_of(&repeat),
        result_of(&first),
        "the duplicate must be answered from the first call's own result"
    );
    assert_eq!(
        h.service.invoked.load(Ordering::SeqCst),
        1,
        "the target must not run a second time"
    );
}

/// The record's identity is `(caller, key)`, so the same key string from a
/// different caller is a different call -- otherwise two callers would read
/// each other's results.
#[tokio::test]
async fn a_key_is_scoped_to_its_caller_at_the_wire_entry_point() {
    let h = harness().await;
    let one = caller("did:key:zCallerOne");
    let two = caller("did:key:zCallerTwo");

    h.handler
        .dispatch_json_rpc_once(&pipeline(), &preamble(), Some(&one), &body(Some("k1")))
        .await
        .unwrap();
    h.handler
        .dispatch_json_rpc_once(&pipeline(), &preamble(), Some(&two), &body(Some("k1")))
        .await
        .unwrap();
    assert_eq!(h.service.invoked.load(Ordering::SeqCst), 2);
}

/// An anonymous caller has no namespace at all: two of them sharing one
/// would read each other's stored results. Refused rather than filed under
/// a shared name.
#[tokio::test]
async fn a_keyed_call_from_an_unidentified_caller_is_refused_at_the_wire() {
    let h = harness().await;
    let frame = h
        .handler
        .dispatch_json_rpc_once(&pipeline(), &preamble(), None, &body(Some("k1")))
        .await
        .unwrap();
    assert_eq!(error_of(&frame).error.code, -32010, "expected a permission denial");
    assert_eq!(h.service.invoked.load(Ordering::SeqCst), 0);
}

/// The unchanged path: every call on the hot path today carries no key and
/// must behave exactly as it did before the fence existed.
#[tokio::test]
async fn an_unkeyed_remote_call_is_never_deduplicated() {
    let h = harness().await;
    let who = caller("did:key:zRemoteCaller");
    for _ in 0..3 {
        h.handler
            .dispatch_json_rpc_once(&pipeline(), &preamble(), Some(&who), &body(None))
            .await
            .unwrap();
    }
    assert_eq!(h.service.invoked.load(Ordering::SeqCst), 3);
    assert!(
        !h.dir.path().join("services").join(TARGET_SERVICE).join("async.db").exists(),
        "an unkeyed call must not open, or create, any dedup store"
    );
}

#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The receiver-side idempotency fence at the *wire* entry point (see
//! [ADR-0023](../../../docs/decisions/0023-durable-async-primitives.md),
//! section 4).
//!
//! A call arriving from another node never passes through `ProxyRouter` --
//! it lands in the route handler's own `dispatch_json_rpc_once`, which has
//! its own native and WASM arms. A fence on only the proxy would be
//! bypassed by every remote caller, which is every caller that actually
//! needed a durable queue. These tests drive that second entry point
//! directly, against a real `RouteHandler::init` composition, so "one
//! guard, both entry points" is a fact rather than an intention.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
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
use syneroym_identity::{DelegationCertificate, Identity, delegation::SCOPE_SERVICE_INSTANCE};
use syneroym_mailbox_format::MessageContent;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_router::{
    AdaptationStage, DeferredOutcome, EncryptionStage, RouteHandler, RouteHandlerDeps,
    RoutePipeline, RoutePreamble, ServiceStage, TransportStage,
};
use syneroym_rpc::{
    AuthLevel, CallerContext, CallerProof, JsonRpcErrorResponse, JsonRpcResponse,
    NativeDispatchRegistry, NativeInvocation, NativeResponse, NativeService, RpcError, RpcResult,
    SessionContext,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tempfile::{TempDir, tempdir};
use tokio::{sync::Notify, time};

const TARGET_SERVICE: &str = "svc-target";

#[derive(Debug, Default)]
struct NoopControlPlane;

#[async_trait::async_trait]
impl NativeService for NoopControlPlane {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        Ok(NativeResponse { payload: Value::Null })
    }
}

/// Counts how many times it actually ran, which is the only thing most of
/// these tests need to observe. It also keeps the caller it last saw, can be
/// told to answer an error, and can hold its first call until released.
#[derive(Debug, Default)]
struct CountingService {
    invoked: AtomicUsize,
    last_caller: Mutex<Option<CallerContext>>,
    fails: AtomicBool,
    hold_first: Mutex<Option<Arc<Notify>>>,
    entered: Notify,
}

#[async_trait::async_trait]
impl NativeService for CountingService {
    async fn dispatch(&self, invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        let n = self.invoked.fetch_add(1, Ordering::SeqCst);
        *self.last_caller.lock().unwrap() = Some(invocation.caller);
        let hold = self.hold_first.lock().unwrap().take();
        if let Some(release) = hold {
            self.entered.notify_one();
            release.notified().await;
        }
        if self.fails.load(Ordering::SeqCst) {
            return Err(RpcError::InvalidParams("no".to_string()));
        }
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
    let mut config = SubstrateConfig::default();
    config.storage.db_dir = dir.path().to_path_buf();
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
        session_revocation: None,
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
    RoutePreamble::binary_json_rpc(TARGET_SERVICE, "greeter")
}

fn caller(did: &str) -> CallerContext {
    CallerContext {
        caller_did: did.to_string(),
        app_instance: None,
        session: SessionContext::default(),
        auth: AuthLevel::Delegated,
        proof: None,
        on_behalf_did: None,
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

// -- Calls that waited at the service's mailbox
// --------------------------------

fn from_mailbox(key: &str) -> MessageContent {
    MessageContent {
        interface: "greeter".to_string(),
        method: "greet".to_string(),
        params: json!({}),
        idempotency_key: key.to_string(),
    }
}

/// A direct attempt and a later delivery from the mailbox are one call to
/// the fence, because both present the sender's master as the caller.
#[tokio::test]
async fn a_call_from_the_mailbox_after_the_same_direct_call_runs_once() {
    let h = harness().await;
    let who = caller("did:key:zSenderMaster");
    h.handler
        .dispatch_json_rpc_once(&pipeline(), &preamble(), Some(&who), &body(Some("k1")))
        .await
        .unwrap();

    let outcome =
        h.handler.dispatch_deferred(TARGET_SERVICE, "greeter", &from_mailbox("k1"), &who).await;

    assert_eq!(outcome, DeferredOutcome::Succeeded);
    assert_eq!(h.service.invoked.load(Ordering::SeqCst), 1);
}

/// This is what makes the unsigned `call_failed` receipt safe: the sender
/// goes back to direct attempts and gets the same error, without a second
/// run.
#[tokio::test]
async fn a_direct_retry_after_a_failed_mailbox_call_gets_the_same_error() {
    let h = harness().await;
    h.service.fails.store(true, Ordering::SeqCst);
    let who = caller("did:key:zSenderMaster");

    let outcome =
        h.handler.dispatch_deferred(TARGET_SERVICE, "greeter", &from_mailbox("k1"), &who).await;
    assert!(matches!(outcome, DeferredOutcome::Failed { code } if code != 0), "{outcome:?}");

    let retry = h
        .handler
        .dispatch_json_rpc_once(&pipeline(), &preamble(), Some(&who), &body(Some("k1")))
        .await
        .unwrap();
    let DeferredOutcome::Failed { code } = outcome else { unreachable!() };
    assert_eq!(error_of(&retry).error.code, code);
    assert_eq!(h.service.invoked.load(Ordering::SeqCst), 1);
}

/// The service sees the original sender, with the proof of the sealed
/// certificate, even when that certificate has expired since the send time.
#[tokio::test]
async fn a_call_from_the_mailbox_reaches_the_service_as_its_sender() {
    let h = harness().await;
    let (master, instance) = (Identity::generate().unwrap(), Identity::generate().unwrap());
    let cert = DelegationCertificate::issue(
        &master,
        instance.public_key(),
        1,
        SCOPE_SERVICE_INSTANCE.to_string(),
    )
    .unwrap();
    time::sleep(Duration::from_secs(2)).await;
    assert!(cert.is_expired(), "nothing on the path may check the certificate again");
    let mut who = caller(&cert.master_did);
    who.proof = Some(CallerProof {
        pubkey_hex: hex::encode(instance.public_key().to_bytes()),
        delegation_json: Some(cert.to_json().unwrap()),
        signed: None,
        on_behalf: None,
    });

    let outcome =
        h.handler.dispatch_deferred(TARGET_SERVICE, "greeter", &from_mailbox("k1"), &who).await;

    assert_eq!(outcome, DeferredOutcome::Succeeded);
    let seen = h.service.last_caller.lock().unwrap().clone().unwrap();
    assert_eq!(seen.caller_did, cert.master_did);
    let proof = seen.proof.expect("the sender's proof reaches the service");
    assert_eq!(proof.pubkey_hex, hex::encode(instance.public_key().to_bytes()));
    assert!(proof.signed.is_none());
}

#[tokio::test]
async fn a_refused_fence_is_not_run() {
    let h = harness().await;
    h.handler
        .registry()
        .register(
            TARGET_SERVICE.to_string(),
            "mailbox".to_string(),
            SubstrateEndpoint::NativeHostChannel { service_id: TARGET_SERVICE.to_string() },
        )
        .await
        .unwrap();
    let who = caller("did:key:zSenderMaster");
    let content = MessageContent { interface: "mailbox".to_string(), ..from_mailbox("k1") };

    let outcome = h.handler.dispatch_deferred(TARGET_SERVICE, "mailbox", &content, &who).await;

    assert!(matches!(outcome, DeferredOutcome::NotRun(_)), "{outcome:?}");
    assert_eq!(h.service.invoked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_running_call_is_not_run_again() {
    let h = harness().await;
    let release = Arc::new(Notify::new());
    *h.service.hold_first.lock().unwrap() = Some(release.clone());
    let who = caller("did:key:zSenderMaster");
    let first = {
        let (handler, who) = (h.handler.clone(), who.clone());
        tokio::spawn(async move {
            handler.dispatch_deferred(TARGET_SERVICE, "greeter", &from_mailbox("k1"), &who).await
        })
    };
    h.service.entered.notified().await;

    let second =
        h.handler.dispatch_deferred(TARGET_SERVICE, "greeter", &from_mailbox("k1"), &who).await;

    assert!(matches!(second, DeferredOutcome::NotRun(_)), "{second:?}");
    release.notify_one();
    assert_eq!(first.await.unwrap(), DeferredOutcome::Succeeded);
    assert_eq!(h.service.invoked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_tcp_service_cannot_take_a_call_from_the_mailbox() {
    let h = harness().await;
    h.handler
        .registry()
        .register(
            TARGET_SERVICE.to_string(),
            "raw".to_string(),
            SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 1 },
        )
        .await
        .unwrap();
    let who = caller("did:key:zSenderMaster");
    let content = MessageContent { interface: "raw".to_string(), ..from_mailbox("k1") };

    let outcome = h.handler.dispatch_deferred(TARGET_SERVICE, "raw", &content, &who).await;

    assert_eq!(outcome, DeferredOutcome::NotServed);
}

use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use dashmap::DashMap;
use http_body_util::BodyExt;
use hyper::{
    HeaderMap, StatusCode,
    header::{ACCEPT, RETRY_AFTER},
};
use syneroym_core::{
    config::SubstrateConfig,
    event_bus::EventBus,
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
    protocol_utils::{AUTH_SERVICE_ALIAS, SessionRevocationCheck},
    storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::SqliteStorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{
    AuthLevel, CallerContext, NativeDispatchRegistry, NativeInvocation, NativeResponse,
    NativeService, RpcResult, SessionContext,
};
use syneroym_sandbox_wasm::AppSandboxEngine;

use super::{FeedGuard, HttpHandler, SESSION_CALLER_MARKER};
use crate::{RouteHandler, RouteHandlerDeps, RoutePreamble, route_handler::http::HttpBody};

mod login;
mod session;

#[derive(Debug, Default)]
struct NoopControlPlane;

#[async_trait::async_trait]
impl NativeService for NoopControlPlane {
    async fn dispatch(&self, _invocation: NativeInvocation) -> RpcResult<NativeResponse> {
        Ok(NativeResponse { payload: serde_json::Value::Null })
    }
}

async fn build_test_router(
    temp_dir: &Path,
    registry: EndpointRegistry,
    bus: Option<Arc<EventBus>>,
    session_revocation: Option<Arc<dyn SessionRevocationCheck>>,
) -> RouteHandler {
    let mut config = SubstrateConfig::default();
    config.storage.db_dir = temp_dir.to_path_buf();

    let key_store = Arc::new(KeyStore::new());
    let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir, false).unwrap());
    let blob_provider: Arc<dyn BlobProvider> =
        Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
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
        storage_provider,
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
        session_revocation,
        event_bus: bus,
    };
    RouteHandler::init("test-node".to_string(), &config, registry, [11u8; 32], None, deps)
        .await
        .unwrap()
}

const OWNER_DID: &str = "did:key:z6MkuVv9OwnerDid";

struct TestFixture {
    _temp: tempfile::TempDir,
    route_handler: RouteHandler,
    bus: Arc<EventBus>,
    owner_did: String,
    service_id: String,
}

impl TestFixture {
    async fn new() -> Self {
        Self::with_revocation(None).await
    }

    async fn with_revocation(session_revocation: Option<Arc<dyn SessionRevocationCheck>>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
        let bus = EventBus::new(registry.clone());
        let owner_did = OWNER_DID.to_string();
        let service_id = "test-service".to_string();

        registry.set_owner(service_id.clone(), owner_did.clone()).await.unwrap();
        registry
            .set_app_context(service_id.clone(), "test-app".to_string(), "chat".to_string())
            .await
            .unwrap();

        let route_handler =
            build_test_router(temp.path(), registry, Some(bus.clone()), session_revocation).await;

        route_handler
            .inner
            .native_http
            .insert(AUTH_SERVICE_ALIAS.to_string(), Arc::new(login::FakeAuthService::new()));

        Self { _temp: temp, route_handler, bus, owner_did, service_id }
    }

    fn make_handler(
        &self,
        caller: Option<CallerContext>,
        service_override: Option<&str>,
    ) -> HttpHandler {
        let svc = service_override.unwrap_or(&self.service_id);
        let preamble = RoutePreamble::binary_json_rpc(svc, "http-native");
        let endpoint = SubstrateEndpoint::NativeHostChannel { service_id: svc.to_string() };
        let pipeline = self.route_handler.plan_pipeline(&preamble, &endpoint);

        HttpHandler { route_handler: self.route_handler.clone(), preamble, pipeline, caller }
    }

    fn session_caller(&self, caller_did: Option<&str>) -> CallerContext {
        let mut session = SessionContext::default();
        session.claims.insert(SESSION_CALLER_MARKER.to_string(), serde_json::Value::Bool(true));
        CallerContext {
            caller_did: caller_did.unwrap_or(&self.owner_did).to_string(),
            auth: AuthLevel::Delegated,
            app_instance: None,
            session,
            proof: None,
            on_behalf_did: None,
        }
    }
}

/// The headers of an owner's feed request: `Accept` and a valid session token.
fn sse_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(ACCEPT, "text/event-stream".parse().unwrap());
    let token = login::mint_token(OWNER_DID, 3600);
    h.insert(hyper::header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
    h
}

async fn read_frame_text(body: &mut HttpBody) -> Option<String> {
    let res = body.frame().await?;
    let frame = res.expect("infallible body stream");
    let data = frame.into_data().ok()?;
    Some(String::from_utf8_lossy(&data).to_string())
}

#[tokio::test]
async fn the_feed_needs_a_session() {
    let fix = TestFixture::new().await;
    let handler = fix.make_handler(None, None);
    let resp = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn another_person_gets_403() {
    let fix = TestFixture::new().await;
    let caller = fix.session_caller(Some("did:key:z6MkuVv9Stranger"));
    let handler = fix.make_handler(Some(caller), None);
    let resp = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_service_with_no_app_instance_gets_404() {
    let fix = TestFixture::new().await;
    let caller = fix.session_caller(None);
    let plain_svc = "plain-svc";
    fix.route_handler
        .inner
        .registry
        .set_owner(plain_svc.to_string(), fix.owner_did.clone())
        .await
        .unwrap();

    let handler = fix.make_handler(Some(caller), Some(plain_svc));
    let resp = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_request_without_event_stream_accept_gets_406() {
    let fix = TestFixture::new().await;
    let caller = fix.session_caller(None);
    let handler = fix.make_handler(Some(caller), None);
    let headers = HeaderMap::new();
    let resp = handler.handle_event_feed_parts(&headers, None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn the_owner_gets_ready_then_each_change() {
    let fix = TestFixture::new().await;
    let caller = fix.session_caller(None);
    let handler = fix.make_handler(Some(caller), None);

    let resp = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();

    let ready = read_frame_text(&mut body).await.unwrap();
    assert!(ready.starts_with("event: ready\n"), "got: {ready}");
    assert!(ready.contains("data: {\"cursor\":"), "got: {ready}");

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    fix.bus.record(&fix.service_id, "conversation.message", "conv1", Some("m1"), now_ms).unwrap();

    let change = read_frame_text(&mut body).await.unwrap();
    assert!(change.starts_with("event: change\n"), "got: {change}");
    assert!(change.contains("\"kind\":\"conversation.message\""), "got: {change}");
    assert!(change.contains("\"subject\":\"conv1\""), "got: {change}");
    assert!(change.contains("\"item\":\"m1\""), "got: {change}");
}

#[tokio::test]
async fn last_event_id_wins_over_the_after_parameter() {
    let fix = TestFixture::new().await;
    let caller = fix.session_caller(None);
    let handler = fix.make_handler(Some(caller), None);

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    let c1 = fix
        .bus
        .record(&fix.service_id, "conversation.message", "conv1", Some("m1"), now_ms)
        .unwrap();
    let c2 = fix
        .bus
        .record(&fix.service_id, "conversation.message", "conv1", Some("m2"), now_ms)
        .unwrap();

    let mut headers = sse_headers();
    headers.insert("Last-Event-ID", c1.to_string().parse().unwrap());
    let query = format!("after={}", c2);

    let resp = handler.handle_event_feed_parts(&headers, Some(&query)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();

    let change = read_frame_text(&mut body).await.unwrap();
    assert!(change.contains("\"item\":\"m2\""), "expected m2 after c1, got: {change}");
}

#[tokio::test]
async fn a_too_old_cursor_gets_reload_then_live_changes() {
    let fix = TestFixture::new().await;
    let caller = fix.session_caller(None);
    let handler = fix.make_handler(Some(caller), None);

    let mut headers = sse_headers();
    headers.insert("Last-Event-ID", "0000000000000000-1".parse().unwrap());

    let resp = handler.handle_event_feed_parts(&headers, None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();

    let reload = read_frame_text(&mut body).await.unwrap();
    assert!(reload.starts_with("event: reload\n"), "got: {reload}");

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    fix.bus
        .record(&fix.service_id, "conversation.message", "conv1", Some("live1"), now_ms)
        .unwrap();

    let change = read_frame_text(&mut body).await.unwrap();
    assert!(change.contains("\"item\":\"live1\""), "got: {change}");
}

#[tokio::test]
async fn the_feed_of_one_app_never_carries_another_apps_event() {
    let fix = TestFixture::new().await;
    let caller = fix.session_caller(None);

    let other_svc = "other-service";
    fix.route_handler
        .inner
        .registry
        .set_owner(other_svc.to_string(), fix.owner_did.clone())
        .await
        .unwrap();
    fix.route_handler
        .inner
        .registry
        .set_app_context(other_svc.to_string(), "other-app".to_string(), "other".to_string())
        .await
        .unwrap();

    let handler = fix.make_handler(Some(caller), None);
    let resp = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    let mut body = resp.into_body();
    let _ = read_frame_text(&mut body).await.unwrap();

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    fix.bus.record(other_svc, "other.event", "sub", None, now_ms).unwrap();

    let timeout_res =
        tokio::time::timeout(Duration::from_millis(60), read_frame_text(&mut body)).await;
    assert!(timeout_res.is_err(), "other app event should not reach this feed");

    fix.bus.record(&fix.service_id, "conversation.message", "conv1", Some("mine"), now_ms).unwrap();
    let change = read_frame_text(&mut body).await.unwrap();
    assert!(change.contains("\"item\":\"mine\""), "got: {change}");
}

#[tokio::test]
async fn the_seventeenth_feed_gets_503_and_a_closed_feed_frees_its_permit() {
    let fix = TestFixture::new().await;
    let caller = fix.session_caller(None);
    let handler = fix.make_handler(Some(caller), None);

    let mut bodies = Vec::new();
    for _ in 0..16 {
        let resp = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        bodies.push(resp.into_body());
    }

    let resp17 = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp17.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(resp17.headers().get(RETRY_AFTER).unwrap(), "1");

    bodies.pop();

    let resp_retry = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp_retry.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_quiet_feed_sends_a_keep_alive() {
    tokio::time::pause();

    let fix = TestFixture::new().await;
    let caller = fix.session_caller(None);
    let handler = fix.make_handler(Some(caller), None);

    let resp = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();

    let ready = read_frame_text(&mut body).await.unwrap();
    assert!(ready.starts_with("event: ready\n"), "got: {ready}");

    tokio::time::advance(Duration::from_secs(25)).await;

    let keep_alive = read_frame_text(&mut body).await.unwrap();
    assert_eq!(keep_alive, ": keep-alive\n\n");
}

#[tokio::test]
async fn a_peer_without_a_gateway_session_gets_401() {
    let fix = TestFixture::new().await;
    let peer_caller = CallerContext {
        caller_did: "did:key:z6MkuVv9Peer".to_string(),
        auth: AuthLevel::Delegated,
        app_instance: None,
        session: SessionContext::default(),
        proof: None,
        on_behalf_did: None,
    };
    let handler = fix.make_handler(Some(peer_caller), None);
    let resp = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_stranger_cannot_tell_an_app_service_from_a_plain_one() {
    let fix = TestFixture::new().await;
    let plain_svc = "plain-svc";
    fix.route_handler
        .inner
        .registry
        .set_owner(plain_svc.to_string(), fix.owner_did.clone())
        .await
        .unwrap();

    let handler_app = fix.make_handler(None, None);
    let resp_app = handler_app.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp_app.status(), StatusCode::UNAUTHORIZED);

    let handler_plain = fix.make_handler(None, Some(plain_svc));
    let resp_plain = handler_plain.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp_plain.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_reader_far_behind_gets_all_events_before_it_waits() {
    let fix = TestFixture::new().await;
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;

    let c0 = fix.bus.head("test-app");
    for i in 0..1200 {
        let item = format!("item-{i}");
        fix.bus
            .record(&fix.service_id, "conversation.message", "conv1", Some(&item), now_ms)
            .unwrap();
    }

    let caller = fix.session_caller(None);
    let handler = fix.make_handler(Some(caller), None);
    let mut headers = sse_headers();
    headers.insert("Last-Event-ID", c0.to_string().parse().unwrap());

    let resp = handler.handle_event_feed_parts(&headers, None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();

    let mut count = 0;
    while let Ok(Some(frame)) =
        tokio::time::timeout(Duration::from_millis(50), read_frame_text(&mut body)).await
    {
        assert!(frame.starts_with("event: change\n"));
        count += 1;
        if count == 1200 {
            break;
        }
    }
    assert_eq!(count, 1200, "all 1200 events should arrive without waiting for new records");
}

struct TestGauge(Arc<AtomicI64>);
impl metrics::GaugeFn for TestGauge {
    fn increment(&self, value: f64) {
        self.0.fetch_add(value as i64, Ordering::SeqCst);
    }
    fn decrement(&self, value: f64) {
        self.0.fetch_sub(value as i64, Ordering::SeqCst);
    }
    fn set(&self, value: f64) {
        self.0.store(value as i64, Ordering::SeqCst);
    }
}

struct TestRecorder(Arc<AtomicI64>);
impl metrics::Recorder for TestRecorder {
    fn describe_counter(
        &self,
        _: metrics::KeyName,
        _: Option<metrics::Unit>,
        _: metrics::SharedString,
    ) {
    }
    fn describe_gauge(
        &self,
        _: metrics::KeyName,
        _: Option<metrics::Unit>,
        _: metrics::SharedString,
    ) {
    }
    fn describe_histogram(
        &self,
        _: metrics::KeyName,
        _: Option<metrics::Unit>,
        _: metrics::SharedString,
    ) {
    }
    fn register_counter(&self, _: &metrics::Key, _: &metrics::Metadata<'_>) -> metrics::Counter {
        metrics::Counter::noop()
    }
    fn register_gauge(&self, _: &metrics::Key, _: &metrics::Metadata<'_>) -> metrics::Gauge {
        metrics::Gauge::from_arc(Arc::new(TestGauge(self.0.clone())))
    }
    fn register_histogram(
        &self,
        _: &metrics::Key,
        _: &metrics::Metadata<'_>,
    ) -> metrics::Histogram {
        metrics::Histogram::noop()
    }
}

#[test]
fn a_closed_feed_lowers_the_open_feeds_gauge() {
    let gauge_val = Arc::new(AtomicI64::new(0));
    let recorder = TestRecorder(gauge_val.clone());
    metrics::with_local_recorder(&recorder, || {
        assert_eq!(gauge_val.load(Ordering::SeqCst), 0);
        let guard = FeedGuard::new();
        assert_eq!(gauge_val.load(Ordering::SeqCst), 1);
        drop(guard);
        assert_eq!(gauge_val.load(Ordering::SeqCst), 0);
    });
}

struct DynamicRevocation(AtomicBool);
impl SessionRevocationCheck for DynamicRevocation {
    fn is_revoked(&self, _token: &str) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

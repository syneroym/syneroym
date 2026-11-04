//! How the feed ends a stream when the session behind it ends.

use syneroym_app_host::types::http::{FrameKind, HttpRequest, HttpResponse};
use syneroym_core::protocol_utils::AUTH_SERVICE_ALIAS;
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::NativeHttpService;
use syneroym_ucan::SessionToken;

use super::*;
use crate::route_handler::http::auth::resolve_effective_session_caller;

#[derive(Debug)]
struct FakeAuthService(String);

#[async_trait::async_trait]
impl NativeHttpService for FakeAuthService {
    async fn handle_request(
        &self,
        _request: HttpRequest,
        _caller: Option<CallerContext>,
    ) -> Result<HttpResponse, String> {
        Err("not used".to_string())
    }
    async fn on_websocket_open(&self, _conn: String, _caller: Option<CallerContext>) {}
    async fn on_websocket_message(
        &self,
        _conn: String,
        _frame: Vec<u8>,
        _kind: FrameKind,
        _caller: Option<CallerContext>,
    ) {
    }
    async fn on_websocket_close(&self, _conn: String, _caller: Option<CallerContext>) {}
    fn service_id(&self) -> Option<&str> {
        Some(&self.0)
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn bearer_headers(token: &str) -> HeaderMap {
    let mut h = sse_headers();
    h.insert(hyper::header::AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
    h
}

fn session_caller_expiring_at(fix: &TestFixture, expires_at_secs: u64) -> CallerContext {
    let mut caller = fix.session_caller(None);
    caller
        .session
        .claims
        .insert(SESSION_EXPIRES_AT_CLAIM.to_string(), serde_json::Value::from(expires_at_secs));
    caller
}

fn record_one_change(fix: &TestFixture) {
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    fix.bus.record(&fix.service_id, "conversation.message", "conv1", Some("m1"), now_ms).unwrap();
}

#[tokio::test]
async fn a_session_caller_carries_no_token_in_its_claims() {
    let fix = TestFixture::new().await;
    let auth_identity = Identity::generate().unwrap();
    let auth_did = substrate::derive_did_key(&auth_identity.public_key());
    fix.route_handler
        .inner
        .native_http
        .insert(AUTH_SERVICE_ALIAS.to_string(), Arc::new(FakeAuthService(auth_did)));

    let token = SessionToken::mint(&auth_identity, &fix.owner_did, "test", None, 3600)
        .unwrap()
        .to_token_string()
        .unwrap();
    let gateway_caller = CallerContext {
        caller_did: fix.route_handler.inner.node_did.clone(),
        auth: AuthLevel::Delegated,
        app_instance: None,
        session: SessionContext::default(),
        proof: None,
        on_behalf_did: None,
    };
    let preamble = RoutePreamble::binary_json_rpc(&fix.service_id, "http-native");

    let caller = resolve_effective_session_caller(
        &fix.route_handler,
        &preamble,
        Some(&gateway_caller),
        &bearer_headers(&token),
    )
    .expect("a valid session token authenticates");

    assert_eq!(caller.caller_did, fix.owner_did);
    assert!(caller.session.claims.contains_key(SESSION_EXPIRES_AT_CLAIM));
    let claims_json = serde_json::to_string(&caller.session.claims).unwrap();
    assert!(!claims_json.contains(&token), "guest code receives the claims: {claims_json}");
    assert!(!format!("{caller:?}").contains(&token), "a logged caller must not print the token");

    let handler = fix.make_handler(Some(caller), None);
    let resp = handler.handle_event_feed_parts(&bearer_headers(&token), None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_revoked_session_terminates_an_open_feed_stream() {
    let revocation = Arc::new(DynamicRevocation(AtomicBool::new(false)));
    let fix = TestFixture::with_revocation(Some(revocation.clone())).await;
    let handler = fix.make_handler(Some(fix.session_caller(None)), None);

    let resp = handler.handle_event_feed_parts(&bearer_headers("token-123"), None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();
    let ready = read_frame_text(&mut body).await.unwrap();
    assert!(ready.starts_with("event: ready\n"));

    revocation.0.store(true, Ordering::SeqCst);
    record_one_change(&fix);

    let next_frame = read_frame_text(&mut body).await;
    assert!(next_frame.is_none(), "the feed must end on revocation, got: {next_frame:?}");
}

#[tokio::test]
async fn a_session_that_has_expired_gets_401_at_connect() {
    let fix = TestFixture::new().await;
    let caller = session_caller_expiring_at(&fix, now_secs());
    let handler = fix.make_handler(Some(caller), None);

    let resp = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_open_feed_ends_on_the_first_change_after_its_session_expires() {
    let fix = TestFixture::new().await;
    let caller = session_caller_expiring_at(&fix, now_secs() + 1);
    let handler = fix.make_handler(Some(caller), None);

    let resp = handler.handle_event_feed_parts(&sse_headers(), None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();
    let ready = read_frame_text(&mut body).await.unwrap();
    assert!(ready.starts_with("event: ready\n"));

    tokio::time::sleep(Duration::from_millis(2100)).await;
    record_one_change(&fix);

    let next_frame = read_frame_text(&mut body).await;
    assert!(next_frame.is_none(), "the feed must end at expiry, got: {next_frame:?}");
}

#[tokio::test]
async fn a_revoked_session_ends_a_quiet_feed_at_the_keep_alive() {
    tokio::time::pause();
    let revocation = Arc::new(DynamicRevocation(AtomicBool::new(false)));
    let fix = TestFixture::with_revocation(Some(revocation.clone())).await;
    let handler = fix.make_handler(Some(fix.session_caller(None)), None);

    let resp = handler.handle_event_feed_parts(&bearer_headers("token-123"), None).await.unwrap();
    let mut body = resp.into_body();
    let ready = read_frame_text(&mut body).await.unwrap();
    assert!(ready.starts_with("event: ready\n"));

    revocation.0.store(true, Ordering::SeqCst);
    tokio::time::advance(super::super::FEED_KEEP_ALIVE_INTERVAL).await;

    let next_frame = read_frame_text(&mut body).await;
    assert!(next_frame.is_none(), "no event came, so the keep-alive must end it: {next_frame:?}");
}

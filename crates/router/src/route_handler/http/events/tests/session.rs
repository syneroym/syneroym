//! Which session token a request uses, and how the feed ends a stream when
//! the session behind it ends.

use std::collections::HashSet;

use hyper::header::COOKIE;

use super::{login::mint_token, *};
use crate::route_handler::http::auth::{resolve_effective_session_caller, select_session_token};

const OTHER_DID: &str = "did:key:z6MkOtherPerson";
const APP_BEARER: &str = "app_bearer_secret_xyz123";

/// Revokes exactly the tokens it was given.
struct RevokedTokens(HashSet<String>);
impl SessionRevocationCheck for RevokedTokens {
    fn is_revoked(&self, token: &str) -> bool {
        self.0.contains(token)
    }
}

/// Revokes one token, once told to.
struct RevokesLater {
    token: String,
    on: AtomicBool,
}
impl SessionRevocationCheck for RevokesLater {
    fn is_revoked(&self, token: &str) -> bool {
        self.on.load(Ordering::SeqCst) && token == self.token
    }
}

fn headers_with(cookie: Option<&str>, bearer: Option<&str>) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(ACCEPT, "text/event-stream".parse().unwrap());
    if let Some(token) = cookie {
        h.insert(COOKIE, format!("syneroym_session={token}").parse().unwrap());
    }
    if let Some(token) = bearer {
        h.insert(AUTHORIZATION, format!("Bearer {token}").parse().unwrap());
    }
    h
}

fn gateway_caller(fix: &TestFixture) -> CallerContext {
    CallerContext {
        caller_did: fix.route_handler.inner.node_did.clone(),
        auth: AuthLevel::Delegated,
        app_instance: None,
        session: SessionContext::default(),
        proof: None,
        on_behalf_did: None,
    }
}

fn resolve(fix: &TestFixture, headers: &HeaderMap) -> Option<CallerContext> {
    let preamble = RoutePreamble::binary_json_rpc(&fix.service_id, "http-native");
    resolve_effective_session_caller(
        &fix.route_handler,
        &preamble,
        Some(&gateway_caller(fix)),
        headers,
    )
}

fn record_one_change(fix: &TestFixture) {
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    fix.bus.record(&fix.service_id, "conversation.message", "conv1", Some("m1"), now_ms).unwrap();
}

async fn open_feed(fix: &TestFixture, headers: &HeaderMap) -> HttpBody {
    let handler = fix.make_handler(Some(fix.session_caller(None)), None);
    let resp = handler.handle_event_feed_parts(headers, None).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();
    let ready = read_frame_text(&mut body).await.unwrap();
    assert!(ready.starts_with("event: ready\n"), "got: {ready}");
    body
}

#[tokio::test]
async fn a_session_caller_carries_no_token_in_its_claims() {
    let fix = TestFixture::new().await;
    let token = mint_token(OWNER_DID, 3600);

    let caller = resolve(&fix, &headers_with(None, Some(&token))).expect("a valid token");

    assert_eq!(caller.caller_did, OWNER_DID);
    let claims_json = serde_json::to_string(&caller.session.claims).unwrap();
    assert!(!claims_json.contains(&token), "guest code receives the claims: {claims_json}");
    assert!(!format!("{caller:?}").contains(&token), "a logged caller must not print the token");
}

#[tokio::test]
async fn a_revoked_cookie_falls_through_to_a_valid_bearer() {
    let cookie = mint_token(OTHER_DID, 3600);
    let bearer = mint_token(OWNER_DID, 3600);
    let revoked = Arc::new(RevokedTokens(HashSet::from([cookie.clone()])));
    let fix = TestFixture::with_revocation(Some(revoked)).await;

    let caller = resolve(&fix, &headers_with(Some(&cookie), Some(&bearer))).unwrap();

    assert_eq!(caller.caller_did, OWNER_DID);
}

#[tokio::test]
async fn a_valid_cookie_wins_over_an_apps_own_bearer() {
    let fix = TestFixture::new().await;
    let cookie = mint_token(OWNER_DID, 3600);

    let caller = resolve(&fix, &headers_with(Some(&cookie), Some(APP_BEARER))).unwrap();

    assert_eq!(caller.caller_did, OWNER_DID);
}

#[tokio::test]
async fn a_request_with_no_valid_token_has_no_session() {
    let fix = TestFixture::new().await;

    assert!(resolve(&fix, &headers_with(Some("not-a-token"), Some(APP_BEARER))).is_none());
    assert!(select_session_token(&fix.route_handler, &headers_with(None, None)).is_none());
}

#[tokio::test]
async fn the_feed_watches_the_token_that_authenticated_it() {
    let cookie = mint_token(OWNER_DID, 3600);
    let revocation = Arc::new(RevokesLater { token: cookie.clone(), on: AtomicBool::new(false) });
    let fix = TestFixture::with_revocation(Some(revocation.clone())).await;
    let mut body = open_feed(&fix, &headers_with(Some(&cookie), Some(APP_BEARER))).await;

    revocation.on.store(true, Ordering::SeqCst);
    record_one_change(&fix);

    let next_frame = read_frame_text(&mut body).await;
    assert!(next_frame.is_none(), "the feed must end on revocation, got: {next_frame:?}");
}

#[tokio::test]
async fn a_revoked_session_terminates_an_open_feed_stream() {
    let token = mint_token(OWNER_DID, 3600);
    let revoked = Arc::new(DynamicRevocation(AtomicBool::new(false)));
    let fix = TestFixture::with_revocation(Some(revoked.clone())).await;
    let mut body = open_feed(&fix, &headers_with(None, Some(&token))).await;

    revoked.0.store(true, Ordering::SeqCst);
    record_one_change(&fix);

    let next_frame = read_frame_text(&mut body).await;
    assert!(next_frame.is_none(), "the feed must end on revocation, got: {next_frame:?}");
}

#[tokio::test]
async fn a_session_that_has_expired_gets_401_at_connect() {
    let fix = TestFixture::new().await;
    let expired = mint_token(OWNER_DID, 0);
    let handler = fix.make_handler(Some(fix.session_caller(None)), None);

    let resp =
        handler.handle_event_feed_parts(&headers_with(None, Some(&expired)), None).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_open_feed_ends_on_the_first_change_after_its_session_expires() {
    let fix = TestFixture::new().await;
    let token = mint_token(OWNER_DID, 2);
    let mut body = open_feed(&fix, &headers_with(None, Some(&token))).await;

    time::sleep(Duration::from_millis(2100)).await;
    record_one_change(&fix);

    let next_frame = read_frame_text(&mut body).await;
    assert!(next_frame.is_none(), "the feed must end at expiry, got: {next_frame:?}");
}

#[tokio::test]
async fn a_revoked_session_ends_a_quiet_feed_at_the_keep_alive() {
    time::pause();
    let token = mint_token(OWNER_DID, 3600);
    let revoked = Arc::new(DynamicRevocation(AtomicBool::new(false)));
    let fix = TestFixture::with_revocation(Some(revoked.clone())).await;
    let mut body = open_feed(&fix, &headers_with(None, Some(&token))).await;

    revoked.0.store(true, Ordering::SeqCst);
    time::advance(super::super::FEED_KEEP_ALIVE_INTERVAL).await;

    let next_frame = read_frame_text(&mut body).await;
    assert!(next_frame.is_none(), "no event came, so the keep-alive must end it: {next_frame:?}");
}

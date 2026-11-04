//! A local auth service for the feed tests, and session tokens it signs.

use std::sync::LazyLock;

use syneroym_app_host::types::http::{FrameKind, HttpRequest, HttpResponse};
use syneroym_identity::{Identity, substrate};
use syneroym_rpc::{CallerContext, NativeHttpService};
use syneroym_ucan::SessionToken;

static AUTH_IDENTITY: LazyLock<Identity> = LazyLock::new(|| Identity::generate().unwrap());

#[derive(Debug)]
pub(super) struct FakeAuthService(String);

impl FakeAuthService {
    pub(super) fn new() -> Self {
        Self(substrate::derive_did_key(&AUTH_IDENTITY.public_key()))
    }
}

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

/// A session token for `person_did` that the fake auth service signed.
pub(super) fn mint_token(person_did: &str, ttl_secs: u64) -> String {
    SessionToken::mint(&AUTH_IDENTITY, person_did, "test", None, ttl_secs)
        .unwrap()
        .to_token_string()
        .unwrap()
}

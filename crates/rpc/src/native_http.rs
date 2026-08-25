//! The inbound-HTTP counterpart to [`NativeService`]: how a natively
//! linked app receives a request the router matched against its
//! `http_routes` table, and the WebSocket lifecycle that goes with it.
//!
//! Separate from `NativeService` rather than a method on it: an HTTP route
//! is not a JSON-RPC method, `NativeInvocation` has no place to put a body
//! or headers, and most native services (the control plane, the
//! supervisor) answer JSON-RPC and no HTTP at all.

use std::{fmt::Debug, sync::Arc};

use dashmap::DashMap;
use syneroym_app_host::types::http::{FrameKind, HttpRequest, HttpResponse};

use crate::CallerContext;

/// A natively linked app's inbound HTTP surface. Mirrors
/// `syneroym:http/incoming-handler` and `syneroym:http/websocket-handler`.
///
/// `caller` is the router-verified context, forwarded exactly as
/// `AppSandboxEngine::handle_guest_http_request` forwards it into
/// `HostState.caller`. `None` reaches here only for a route the deploy
/// declared `public`, and the implementation must substitute
/// `CallerContext::service_system(service_id)` -- the same substitution
/// the WASM path makes, in the same place, so an anonymous request is the
/// same principal on both builds.
#[async_trait::async_trait]
pub trait NativeHttpService: Send + Sync + Debug {
    async fn handle_request(
        &self,
        request: HttpRequest,
        caller: Option<CallerContext>,
    ) -> Result<HttpResponse, String>;

    async fn on_websocket_open(&self, conn: String, caller: Option<CallerContext>);
    async fn on_websocket_message(
        &self,
        conn: String,
        frame: Vec<u8>,
        kind: FrameKind,
        caller: Option<CallerContext>,
    );
    async fn on_websocket_close(&self, conn: String, caller: Option<CallerContext>);
}

/// Shared registry of natively linked HTTP surfaces, keyed by `service_id`
/// -- the `guest`/`websocket` route targets' analogue of
/// [`NativeDispatchRegistry`](crate::NativeDispatchRegistry), which covers
/// the `data-layer`/`messaging`/`stream` targets.
pub type NativeHttpRegistry = Arc<DashMap<String, Arc<dyn NativeHttpService>>>;

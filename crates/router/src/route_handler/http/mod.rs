//! HTTP router request interception
//!
//! Handles incoming HTTP traffic: the original JSON-RPC-over-`POST` bridge
//! (unchanged), plus HTTP verb/path passthrough onto
//! `data-layer`/`blob-store`/`messaging`. `HttpRoute`/`HttpRouteRegistry`
//! live in `syneroym_core::http_routes`; entries are parsed and populated
//! by `syneroym_control_plane::http_routes` on deploy/undeploy.
//!
//! Route resolution order, per request:
//! 1. `GET /blobs/{hash}` -- always intercepted (fixed, self-authorizing via
//!    the signed-URL HMAC, not a per-service opt-in).
//! 2. Static assets: `GET`/`HEAD` only, exact path plus a trailing-slash
//!    directory index, served straight from blob storage without instantiating
//!    the component. Deploy-time collision detection means this and step 3
//!    below never actually contend for the same path.
//! 3. The connected service's `http_routes` table (method + path-with-
//!    `{param}` match) -- bridges onto `data-layer`/`messaging`/a registered
//!    stream protocol, or hands the request to the deployed component's own
//!    `syneroym:http/incoming-handler#handle-request` export.
//! 4. Fallthrough, unchanged: the original `POST`+`application/json` JSON-RPC
//!    bridge.

use std::{
    collections::HashMap,
    convert::Infallible,
    io, result,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use futures::{TryStreamExt, stream};
use http_body_util::{
    BodyExt, Full, LengthLimitError, Limited, StreamBody, combinators::UnsyncBoxBody,
};
use hyper::{
    HeaderMap, Method, Request, Response, StatusCode,
    body::{Frame, Incoming},
    header::{
        ACCEPT, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, HeaderName, HeaderValue,
        IF_NONE_MATCH, RETRY_AFTER, X_CONTENT_TYPE_OPTIONS,
    },
    service,
};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as AutoBuilder,
};
use serde_json::Value;
use syneroym_app_host::types::http::{CallerAuth, CallerIdentity, HttpRequest, HttpResponse};
use syneroym_core::{
    asset_manifest::AssetEntry,
    http_routes::{DEFAULT_MAX_SSE_SUBSCRIBERS_PER_SERVICE, HttpRoute, match_path, param_name},
    streaming::StreamDirection,
};
use syneroym_data_blob::{
    crypto,
    native_types::{OpenDownloadResponse, ReadChunkResponse},
};
use syneroym_mqtt_broker::namespace_topic;
use syneroym_rpc::{
    AuthLevel, CallerContext, JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest,
    PROXY_TRANSPORT_RPC_CODE, UNSUPPORTED_PROTOCOL_RPC_CODE, UNSUPPORTED_TARGET_RPC_CODE,
};
use syneroym_sandbox_wasm::{
    AppSandboxEngine, FrameKind, GuestHttpFailure, GuestHttpOutcome, StreamRequestOutcome,
};
use tokio::{
    io::{self as tokio_io, AsyncRead, AsyncReadExt, AsyncWrite},
    sync::{Semaphore, oneshot},
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        handshake::derive_accept_key,
        protocol::{Role, WebSocketConfig},
    },
};
use tokio_util::io::StreamReader;
use tracing::{debug, error, warn};
use uuid::Uuid;

use super::RouteHandler;
use crate::{
    preamble::RoutePreamble,
    routing::{RoutePipeline, ServiceStage},
};

/// Unified response body type for `HttpHandler`: the pre-Slice-7 JSON-RPC
/// bridge responses are wrapped in it unchanged (`Full<Bytes>` boxed), and
/// every new streaming route (blob `GET`, SSE) is built on it directly via
/// `StreamBody`. Replaces the old `Response<Full<Bytes>>` everywhere in this
/// file.
type HttpBody = UnsyncBoxBody<Bytes, Infallible>;

/// Small-body routes (`data-layer` `put`/`patch`, `messaging` `publish`)
/// share this guard; blob download and chunked-upload routes are exempt by
/// design (see the module doc).
const UNAUTHENTICATED_RPC_CODE: i32 = -32090;

const MAX_SMALL_BODY_BYTES: usize = 1024 * 1024;

/// Chunk size requested per `blob-store/read-chunk` native-dispatch call
/// while streaming a `GET /blobs/{hash}` response body.
const BLOB_CHUNK_BYTES: u32 = 64 * 1024;

/// Request-body ceiling for a `guest` route. Its own constant
/// rather than `MAX_SMALL_BODY_BYTES`: this body is additionally marshalled
/// into a `Vec<Val::U8>` for the component-model call, so the two limits
/// have different cost curves and may diverge.
const MAX_GUEST_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// Response-body ceiling for a `guest` route. Bounds what is **sent**, not
/// what is allocated: the guest's `list<u8>` is fully materialised in host
/// memory before it can be measured, and the allocation bound is the
/// guest's own `max_memory_bytes` store limiter.
const MAX_GUEST_RESPONSE_BODY_BYTES: usize = 1024 * 1024;

const MAX_GUEST_REQUEST_HEADERS: usize = 64;
const MAX_GUEST_RESPONSE_HEADERS: usize = 64;

/// Headers the host owns, never the guest: stripped from a guest response
/// and never forwarded from a request.
const HOST_OWNED_HEADERS: [&str; 8] = [
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "upgrade",
    "proxy-connection",
    "te",
    "trailer",
];

/// A handler for HTTP-based JSON-RPC requests.
///
/// It wraps a `RouteHandler`, a connection-level `RoutePreamble`, and the
/// planned `RoutePipeline`.
#[derive(Clone)]
pub struct HttpHandler {
    pub route_handler: RouteHandler,
    pub preamble: RoutePreamble,
    pub pipeline: RoutePipeline,
    pub caller: Option<CallerContext>,
}

mod auth;
mod blobs;
mod dispatch;
mod format;
mod guest;
mod parse;
mod streaming;
mod websocket;

#[cfg(test)]
mod tests;

use auth::*;
use dispatch::*;
pub use format::http_error;
use format::*;
use parse::*;

impl RouteHandler {
    /// Upgrades a raw stream to an HTTP server and handles incoming requests.
    ///
    /// This uses `hyper` to serve JSON-RPC over HTTP/1.1.
    pub async fn handle_http_stream<I>(
        self,
        io: TokioIo<I>,
        preamble: RoutePreamble,
        pipeline: RoutePipeline,
        caller: Option<CallerContext>,
    ) -> Result<()>
    where
        I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let handler = Arc::new(HttpHandler { route_handler: self, preamble, pipeline, caller });

        let mut builder = AutoBuilder::new(TokioExecutor::new());
        // Many real HTTP/1.1 clients (and every client this slice's own
        // e2e test needed to write) shut down their write side once
        // they've finished sending a request, without waiting for the
        // response -- entirely normal, especially paired with
        // `Connection: close`. Without this, hyper's h1 server treats
        // that as a fatal `IncompleteMessage` ("connection closed before
        // message completed") if the read-side EOF is observed while the
        // response is still being written, silently dropping the
        // connection before any response reaches the client. Discovered
        // via this slice's own e2e test (`http_passthrough_e2e.rs`) --
        // every bridged HTTP route needed this fix, not just streaming
        // ones.
        builder.http1().half_close(true);
        builder
            .serve_connection_with_upgrades(
                io,
                service::service_fn(move |req| {
                    let h = handler.clone();
                    async move { h.handle_http_request(req).await }
                }),
            )
            .await
            .map_err(|e| anyhow!("HTTP connection error: {e}"))
    }
}

/// Result of the shared small-body read: either the body bytes, or a
/// response to return immediately (size-limit or read failure) without
/// dispatching anything.
enum BodyRead {
    Ok(Bytes),
    Rejected(Response<HttpBody>),
}

/// Recognizes the fixed `GET /blobs/{hash}` prefix -- extracted as a pure
/// function so the "always intercepted before the per-service route table"
/// rule is unit-testable without a live `HttpHandler`.
impl HttpHandler {
    /// The entry point for a single HTTP request.
    ///
    /// This is called by `hyper` for every incoming request on the stream.
    pub async fn handle_http_request(
        &self,
        req: Request<Incoming>,
    ) -> result::Result<Response<HttpBody>, Infallible> {
        let response = self.try_handle_http_request(req).await.unwrap_or_else(|e| {
            error!("HTTP JSON-RPC handler error: {e}");
            http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        });
        Ok(response)
    }

    async fn try_handle_http_request(&self, req: Request<Incoming>) -> Result<Response<HttpBody>> {
        let effective_caller = resolve_effective_session_caller(
            &self.route_handler,
            &self.preamble,
            self.caller.as_ref(),
            req.headers(),
        )
        .or_else(|| self.caller.clone());

        let handler = Self {
            route_handler: self.route_handler.clone(),
            preamble: self.preamble.clone(),
            pipeline: self.pipeline.clone(),
            caller: effective_caller,
        };

        let method = req.method().clone();
        let path = req.uri().path().to_string();

        if method == Method::GET
            && let Some(hash) = blob_hash_from_path(&path)
        {
            let query = req.uri().query().unwrap_or("").to_string();
            return handler.handle_blob_get(hash, &query).await;
        }

        // Static assets, exact-path plus a trailing-slash
        // directory index. Placed before route resolution -- an asset path
        // colliding with a declared route pattern is refused at deploy,
        // so this ordering is never actually ambiguous at
        // request time; it exists to keep resolve_asset a cheap, sandbox-
        // free check ahead of the route table lookup.
        if let Some(resp) = handler.try_handle_asset(&method, &path, &req).await? {
            return Ok(resp);
        }

        if let Some((route, path_param)) = handler.resolve_route(&method, &path) {
            return handler.dispatch_route(&route, path_param, req).await;
        }

        handler.handle_json_rpc_bridge(req).await
    }
}

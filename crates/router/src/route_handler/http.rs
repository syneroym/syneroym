//! HTTP router request interception
//!
//! Handles incoming HTTP traffic: the original JSON-RPC-over-`POST` bridge
//! (unchanged), plus M3B Slice 7's HTTP verb/path passthrough onto
//! `data-layer`/`blob-store`/`messaging` -- see `task.md`'s "Slice 7: HTTP
//! Passthrough" section. `HttpRoute`/`HttpRouteRegistry` live in
//! `syneroym_core::http_routes`; entries are parsed and populated by
//! `syneroym_control_plane::http_routes` on deploy/undeploy.
//!
//! Route resolution order, per request:
//! 1. `GET /blobs/{hash}` -- always intercepted (fixed, self-authorizing via
//!    the signed-URL HMAC, not a per-service opt-in).
//! 2. M06A A1's static assets: `GET`/`HEAD` only, exact path plus a
//!    trailing-slash directory index, served straight from blob storage without
//!    instantiating the component. Deploy-time collision detection (D-A1-4)
//!    means this and step 3 below never actually contend for the same path.
//! 3. The connected service's `http_routes` table (method + path-with-
//!    `{param}` match) -- bridges onto `data-layer`/`messaging`/a registered
//!    stream protocol, or (M06A A2) hands the request to the deployed
//!    component's own `syneroym:http/incoming-handler#handle-request` export.
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
use syneroym_core::{
    asset_manifest::AssetEntry,
    guest_http::{GuestCallerAuth, GuestCallerIdentity, GuestHttpRequest, GuestHttpResponse},
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
use syneroym_sandbox_wasm::{FrameKind, GuestHttpFailure, GuestHttpOutcome, StreamRequestOutcome};
use tokio::io::{self as tokio_io, AsyncRead, AsyncReadExt, AsyncWrite};
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
const MAX_SMALL_BODY_BYTES: usize = 1024 * 1024;

/// Chunk size requested per `blob-store/read-chunk` native-dispatch call
/// while streaming a `GET /blobs/{hash}` response body.
const BLOB_CHUNK_BYTES: u32 = 64 * 1024;

/// Request-body ceiling for a `guest` route (M06A D-A2-8). Its own constant
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
/// and never forwarded from a request (M06A D-A2-5).
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
pub struct HttpHandler {
    pub route_handler: RouteHandler,
    pub preamble: RoutePreamble,
    pub pipeline: RoutePipeline,
    pub caller: Option<CallerContext>,
}

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

/// The outcome of bridging one JSON-RPC round trip through
/// `RouteHandler::dispatch_json_rpc_once` -- `dispatch_json_rpc_once` itself
/// never surfaces a native-service error as `Err`; it always returns
/// `Ok(bytes)` containing either a JSON-RPC `result` or `error` envelope, so
/// callers that want a real HTTP status code have to inspect the envelope.
enum DispatchOutcome {
    Success(Value),
    Error { code: i32, message: String },
}

/// Builds and dispatches one native JSON-RPC request through the existing,
/// unchanged `dispatch_json_rpc_once` path, with `preamble.interface`
/// overridden to whichever real native interface (`data-layer`/`blob-store`/
/// `messaging`) the resolved HTTP route implies -- decision 2 of the Slice 7
/// plan: a client connects once with `http://http-native|<service_id>`, and
/// `pipeline.service` (resolved once per connection from the `"http-native"`
/// native-capability interface) already points at the right `service_id`
/// regardless of which real interface a given request targets.
async fn dispatch_native(
    route_handler: &RouteHandler,
    pipeline: &RoutePipeline,
    preamble: &RoutePreamble,
    caller: Option<&CallerContext>,
    interface: &str,
    method: &str,
    params: Value,
) -> Result<DispatchOutcome> {
    // Every bridged data-layer/messaging route reaches native dispatch
    // through this shared fn, so one guard here covers them all and maps to
    // a clean 401 (ADR-0016 §3, ADR-0016 §4.4) -- rather than the 500 a raw
    // `dispatch_json_rpc_once` rejection would surface. Callers that are
    // already self-authorizing by another mechanism (the signed-URL blob
    // GET, see `handle_blob_get`) pass an explicit `service_system` caller,
    // never `None`, so they never hit this guard.
    if caller.is_none() {
        return Ok(DispatchOutcome::Error {
            code: UNAUTHENTICATED_RPC_CODE,
            message: format!("unauthenticated caller for native interface '{interface}'"),
        });
    }
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
        id: Some(Value::Number(1.into())),
        idempotency_key: None,
    };
    let body = serde_json::to_vec(&request)?;
    let synthetic = RoutePreamble { interface: interface.to_string(), ..preamble.clone() };
    let response_bytes =
        route_handler.dispatch_json_rpc_once(pipeline, &synthetic, caller, &body).await?;
    let response: Value = serde_json::from_slice(&response_bytes)
        .map_err(|e| anyhow!("malformed native-dispatch response: {e}"))?;
    if let Some(error) = response.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32603) as i32;
        let message =
            error.get("message").and_then(Value::as_str).unwrap_or("internal error").to_string();
        Ok(DispatchOutcome::Error { code, message })
    } else {
        Ok(DispatchOutcome::Success(response.get("result").cloned().unwrap_or(Value::Null)))
    }
}

/// Reserved JSON-RPC error code for "no verifiable caller identity" on a
/// bridged native-capability request (M04A Slice B0) -- never emitted by a
/// native service itself, only by the `dispatch_native` guard above, and
/// mapped to HTTP 401 below rather than the default 500.
const UNAUTHENTICATED_RPC_CODE: i32 = -32090;

/// The `data-layer`/`blob-store`/JSON-RPC error -> HTTP status mapping
/// table, defined once and reused by every bridged route (task.md's Slice 7
/// checklist). See `data_layer_error`/`blob_error` in
/// `crates/control_plane/src/synsvc_native.rs` for the code assignments.
fn status_for_rpc_error_code(code: i32) -> StatusCode {
    match code {
        -32001 => StatusCode::NOT_FOUND,         // blob not found
        -32002 => StatusCode::TOO_MANY_REQUESTS, // blob quota exceeded
        -32010 => StatusCode::FORBIDDEN,         // data-layer permission denied
        -32011 => StatusCode::NOT_FOUND,         // data-layer collection not found
        -32012 => StatusCode::BAD_REQUEST,       // data-layer schema violation
        -32013 => StatusCode::TOO_MANY_REQUESTS, // data-layer quota exceeded
        UNAUTHENTICATED_RPC_CODE => StatusCode::UNAUTHORIZED,
        -32602 => StatusCode::BAD_REQUEST, // JSON-RPC invalid params
        UNSUPPORTED_PROTOCOL_RPC_CODE | UNSUPPORTED_TARGET_RPC_CODE => StatusCode::NOT_IMPLEMENTED,
        PROXY_TRANSPORT_RPC_CODE => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
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
fn blob_hash_from_path(path: &str) -> Option<&str> {
    let hash = path.strip_prefix("/blobs/")?;
    if hash.is_empty() || hash.contains('/') { None } else { Some(hash) }
}

/// **Cache-Control is chosen by content type, not by path** (M06A A1
/// §5.2): `text/html`'s name is stable while its content changes every
/// deploy, so caching it immutably would pin a browser to a stale bundle
/// indefinitely. Everything else gets long-lived immutable caching, correct
/// for the bundler-hashed filenames a real asset pipeline produces.
fn cache_control_for(content_type: &str) -> &'static str {
    if content_type.starts_with("text/html") {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    }
}

/// Whether an `If-None-Match` header value matches `etag` (always a strong
/// validator here -- the manifest's own content hash). Per RFC 9110
/// §13.1.2: a bare `*` matches unconditionally (the entry was already
/// resolved by the caller, so a representation does currently exist), and
/// the header may otherwise carry a comma-separated list, each member
/// optionally weak (`W/"..."`) -- a weak comparison ignores that prefix,
/// same as a strong one, since this function is only ever asked "does the
/// client already have exactly this content", not "byte-for-byte
/// identical". A browser always echoes the token verbatim, so this is a
/// pure widening: a proxy or `fetch` caller sending a list or a weak
/// validator now gets a 304 instead of silently re-downloading the whole
/// body.
fn if_none_match_hits(header: Option<&HeaderValue>, etag: &str) -> bool {
    let Some(value) = header.and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let value = value.trim();
    value == "*"
        || value.split(',').any(|candidate| {
            let candidate = candidate.trim();
            candidate.strip_prefix("W/").unwrap_or(candidate) == etag
        })
}

/// Parses an HTTP query string (`k=v&k2=v2`) and percent-decodes keys and
/// values.
fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .map(|(k, v)| {
            let key = percent_encoding::percent_decode_str(k).decode_utf8_lossy().to_string();
            let val = percent_encoding::percent_decode_str(v).decode_utf8_lossy().to_string();
            (key, val)
        })
        .collect()
}

/// Maps a `GET`-with-query-string request onto `data-layer::query`'s
/// `query-options` (`filter`/`limit`/`cursor`). `limit` and `cursor` are
/// reserved keys mapped directly onto those fields; every other key becomes
/// an equality clause in the MongoDB-style filter document (`?status=open`
/// -> `{"status": "open"}`), matching `compile_filter`'s own `{field:
/// value}` equality shorthand (`crates/data_db/src/filter.rs`) -- string
/// values only, no operators (`$gt`, `$in`, ...) or type coercion. That
/// covers the common case this bridge is for; a route needing richer
/// filtering than plain-equality-AND can still be reached directly via the
/// JSON-RPC bridge, which takes a filter document verbatim. An absent or
/// empty query string maps to an unfiltered query (`filter: null`),
/// unchanged from before this mapping existed. A non-numeric `limit`
/// produces a `400`-worthy error message rather than being silently dropped.
fn query_opts_from_query_string(query: &str) -> result::Result<Value, String> {
    let mut params = parse_query(query);
    let limit = match params.remove("limit") {
        Some(raw) => {
            let n = raw
                .parse::<u32>()
                .map_err(|_| format!("invalid `limit` query parameter: {raw:?}"))?;
            Value::Number(n.into())
        }
        None => Value::Null,
    };
    let cursor = params.remove("cursor").map_or(Value::Null, Value::String);
    let filter = if params.is_empty() {
        Value::Null
    } else {
        let filter_doc: serde_json::Map<String, Value> =
            params.into_iter().map(|(k, v)| (k, Value::String(v))).collect();
        Value::String(serde_json::to_string(&filter_doc).map_err(|e| e.to_string())?)
    };
    Ok(serde_json::json!({"filter": filter, "limit": limit, "cursor": cursor}))
}

/// The `svc/<service-id>/` prefix `namespace_topic` adds is a substrate
/// implementation detail. An SSE subscriber names topics the way the route
/// table does, so the wire carries the service-relative name -- a browser
/// cannot subscribe by a name that embeds the deployment's own DID.
/// A topic that is not in this service's namespace (a cross-service
/// subscription) is passed through whole.
fn service_relative_topic<'a>(service_id: &str, topic: &'a str) -> &'a str {
    topic
        .strip_prefix("svc/")
        .and_then(|rest| rest.strip_prefix(service_id))
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(topic)
}

/// Formats one broker-delivered `(topic, payload)` message as an SSE frame.
///
/// Payload is treated as UTF-8 text (lossy) -- every fixture in this repo
/// only ever publishes UTF-8 text payloads (see `status.md`'s Slice 6A
/// notes), and SSE's `data:` framing is line-oriented, so a payload
/// containing newlines emits multiple `data:` lines.
///
/// Topic replaces `\r` and `\n` with spaces: publisher-supplied topics from
/// `MqttBroker` do not validate characters, and unescaped CR/LF in a
/// single-line `event:` field would allow a publisher to inject fabricated
/// `data:` or `event:` lines into another subscriber's stream.
fn format_sse_frame(topic: &str, payload: &[u8]) -> String {
    let safe_topic: String =
        topic.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).collect();
    let text = String::from_utf8_lossy(payload);
    let mut frame = format!("event: {safe_topic}\n");
    if text.is_empty() {
        frame.push_str("data: \n");
    } else {
        for line in text.lines() {
            frame.push_str("data: ");
            frame.push_str(line);
            frame.push('\n');
        }
    }
    frame.push('\n');
    frame
}

fn full_body(bytes: Bytes) -> HttpBody {
    Full::new(bytes).boxed_unsync()
}

fn json_response(status: StatusCode, value: &Value) -> Response<HttpBody> {
    let bytes = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(full_body(Bytes::from(bytes)))
        .unwrap_or_else(|_| Response::default())
}

fn structured_rpc_error(status: StatusCode, code: i32, message: String) -> Response<HttpBody> {
    let body = JsonRpcErrorResponse {
        jsonrpc: "2.0".to_string(),
        error: JsonRpcError { code, message, data: None },
        id: None,
    };
    json_response(status, &serde_json::to_value(&body).unwrap_or(Value::Null))
}

/// Request headers a guest sees (M06A D-A2-5, D-A2-8): lowercased, with
/// every `HOST_OWNED_HEADERS` entry removed (the host owns framing, not the
/// guest) and any non-UTF-8 value silently dropped rather than failing the
/// request. A free function so the filtering rule is unit-testable without
/// a live `HttpHandler`, same as `blob_hash_from_path`/`if_none_match_hits`.
/// Error is `(status, message)`, not a built `Response`, so this stays a
/// small `Result` -- the caller builds the response with `http_error`.
fn guest_request_headers(
    headers: &HeaderMap,
) -> result::Result<Vec<(String, String)>, (StatusCode, String)> {
    let mut out = Vec::new();
    for (name, value) in headers {
        let lower = name.as_str().to_ascii_lowercase();
        if HOST_OWNED_HEADERS.contains(&lower.as_str()) {
            continue;
        }
        let Ok(text) = value.to_str() else { continue };
        if out.len() == MAX_GUEST_REQUEST_HEADERS {
            return Err((
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                format!("request has more than {MAX_GUEST_REQUEST_HEADERS} headers"),
            ));
        }
        out.push((lower, text.to_string()));
    }
    Ok(out)
}

/// The router's view of `CallerContext` as a guest may see it (M06A
/// D-A2-12). `Err` on a substrate-injected `AuthLevel`, which cannot
/// legitimately reach an inbound HTTP request -- fail closed rather than
/// report a level that isn't true.
///
/// Takes `preamble` as well as `caller` because the two `auth` halves read
/// different sources: `CallerContext.auth` cannot distinguish a verified
/// certificate from an unchallenged pubkey (F5a) -- `AuthLevel::Delegated`
/// is assigned to *every* verified preamble, including the client gateway's
/// unchallenged node-DID pubkey -- while the preamble's own `delegation`
/// field can, since a malformed certificate is a hard reject before this
/// point. Conversely `preamble.ucan.is_some()` says only that a token was
/// *attached*, not that it verified (`build_caller` fails open on a bad
/// chain), while `CallerContext.auth == AuthLevel::Ucan` is set only on a
/// verified, unrevoked, capability-bearing chain. The two sources are
/// therefore mixed on purpose, one field from each -- collapsing this to a
/// single source would let a caller self-label the stronger `ucan` value
/// with a junk token.
fn guest_caller_identity(
    caller: Option<&CallerContext>,
    preamble: &RoutePreamble,
) -> result::Result<Option<GuestCallerIdentity>, String> {
    let Some(caller) = caller else { return Ok(None) };
    if matches!(
        caller.auth,
        AuthLevel::LocalElevated | AuthLevel::LocalReadOnly | AuthLevel::System
    ) {
        return Err("substrate-injected auth level on an inbound HTTP request".to_string());
    }
    let auth = if matches!(caller.auth, AuthLevel::Ucan) {
        GuestCallerAuth::Ucan
    } else if preamble.delegation.is_some() {
        GuestCallerAuth::Delegated
    } else {
        GuestCallerAuth::SelfAsserted
    };
    Ok(Some(GuestCallerIdentity {
        did: caller.caller_did.clone(),
        auth,
        app_instance: caller.app_instance.clone(),
    }))
}

/// Turns a guest's answer into an HTTP response, or into the 500 failure-
/// matrix row 6 requires (M06A D-A2-5). `Content-Length` is always the
/// host's computed one, never the guest's -- a mismatch would be a
/// connection desync -- and an invalid header **fails the whole response**
/// rather than being silently dropped: a guest that thought it set
/// `Content-Type: application/json` must not silently serve
/// `application/octet-stream`.
fn build_guest_response(response: GuestHttpResponse) -> Response<HttpBody> {
    if response.body.len() > MAX_GUEST_RESPONSE_BODY_BYTES {
        return http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest response body exceeds {MAX_GUEST_RESPONSE_BODY_BYTES} byte limit"),
        );
    }
    if response.headers.len() > MAX_GUEST_RESPONSE_HEADERS {
        return http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest response declares more than {MAX_GUEST_RESPONSE_HEADERS} headers"),
        );
    }
    // 200-599 only: the WIT doc caps this range (1xx is informational, not
    // a final response), narrower than `StatusCode::from_u16`'s own
    // 100..=999 acceptance.
    if !(200..600).contains(&response.status) {
        return http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest returned an out-of-range status: {}", response.status),
        );
    }
    let Ok(status) = StatusCode::from_u16(response.status) else {
        return http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest returned an out-of-range status: {}", response.status),
        );
    };

    let mut builder = Response::builder().status(status);
    let mut saw_content_type = false;
    let mut saw_nosniff = false;
    for (name, value) in response.headers {
        let lower = name.to_ascii_lowercase();
        if HOST_OWNED_HEADERS.contains(&lower.as_str()) {
            debug!(header = %lower, "guest response header stripped -- host owns framing (D-A2-5)");
            continue;
        }
        let Ok(header_name) = HeaderName::from_bytes(lower.as_bytes()) else {
            return http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("guest returned an invalid header name: {name:?}"),
            );
        };
        let Ok(header_value) = HeaderValue::from_str(&value) else {
            return http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("guest returned an invalid value for header {lower}"),
            );
        };
        saw_content_type |= lower == CONTENT_TYPE.as_str();
        saw_nosniff |= lower == X_CONTENT_TYPE_OPTIONS.as_str();
        builder = builder.header(header_name, header_value);
    }
    if !saw_content_type {
        builder = builder.header(CONTENT_TYPE, "application/octet-stream");
    }
    if !saw_nosniff {
        builder = builder.header(X_CONTENT_TYPE_OPTIONS, "nosniff");
    }
    builder = builder.header(CONTENT_LENGTH, response.body.len().to_string());

    match builder.body(full_body(Bytes::from(response.body))) {
        Ok(resp) => resp,
        Err(e) => http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to build guest response: {e}"),
        ),
    }
}

/// A short, safe-to-return sentence per `GuestHttpFailure` variant. The
/// guest's own string (in `Declined`/`BudgetExceeded`/`Trap`/`Malformed`) is
/// already truncated by the engine's `truncate_detail` before it reaches
/// here, so it is safe to include verbatim.
fn describe_guest_http_failure(failure: &GuestHttpFailure) -> String {
    match failure {
        GuestHttpFailure::NoHandler => {
            "deployed component does not export the guest HTTP handler".to_string()
        }
        GuestHttpFailure::Declined(detail) => format!("guest HTTP handler failed: {detail}"),
        GuestHttpFailure::BudgetExceeded(detail) => {
            format!("guest HTTP handler exceeded its budget: {detail}")
        }
        GuestHttpFailure::Trap(detail) => format!("guest HTTP handler trapped: {detail}"),
        GuestHttpFailure::Malformed(detail) => {
            format!("guest HTTP handler returned a malformed response: {detail}")
        }
        // Handled by its own 503 branch at the call site; kept here so the
        // match stays exhaustive if a new caller reuses this function.
        GuestHttpFailure::Unavailable(detail) => {
            format!("guest HTTP handler unavailable: {detail}")
        }
    }
}

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
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        if method == Method::GET
            && let Some(hash) = blob_hash_from_path(&path)
        {
            let query = req.uri().query().unwrap_or("").to_string();
            return self.handle_blob_get(hash, &query).await;
        }

        // M06A A1: static assets, exact-path plus a trailing-slash
        // directory index. Placed before route resolution -- an asset path
        // colliding with a declared route pattern is refused at deploy
        // (D-A1-4), so this ordering is never actually ambiguous at
        // request time; it exists to keep resolve_asset a cheap, sandbox-
        // free check (D-06A-1) ahead of the route table lookup.
        if let Some(resp) = self.try_handle_asset(&method, &path, &req).await? {
            return Ok(resp);
        }

        if let Some((route, path_param)) = self.resolve_route(&method, &path) {
            return self.dispatch_route(&route, path_param, req).await;
        }

        self.handle_json_rpc_bridge(req).await
    }

    /// The original `POST`+`application/json` JSON-RPC bridge, wrapped in the
    /// unified `HttpBody` type. An anonymous caller targeting a native
    /// service is rejected with 401 before dispatch (§5.3); a WASM-component
    /// target is unaffected, matching pre-Slice-7 behavior.
    async fn handle_json_rpc_bridge(&self, req: Request<Incoming>) -> Result<Response<HttpBody>> {
        if req.method() != Method::POST {
            // Also where a static-asset miss and a non-`public` bundle land
            // (D-A1-8, `try_handle_asset` returning `Ok(None)` for a `GET`/
            // `HEAD` falls all the way through to here): 405, not the
            // failure matrix's originally-planned 404, since this bridge
            // rejects every non-`POST` method uniformly, asset request or
            // not, and special-casing `GET`/`HEAD` here would change
            // behaviour for the ordinary JSON-RPC-bridge case too, not just
            // assets. The property the matrix actually cares about --
            // absence and refusal look identical from outside -- holds
            // regardless of which 4xx it is; task.md/status.md record 405
            // as the deliberate answer, not 404.
            return Ok(http_error(StatusCode::METHOD_NOT_ALLOWED, "Only POST is supported".into()));
        }

        let content_type =
            req.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");
        if !content_type.starts_with("application/json") {
            return Ok(http_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Content-Type must be application/json".into(),
            ));
        }

        let body_bytes =
            req.collect().await.map_err(|e| anyhow!("Failed to read HTTP body: {e}"))?.to_bytes();

        if body_bytes.is_empty() {
            return Ok(http_error(StatusCode::BAD_REQUEST, "Empty request body".into()));
        }

        // Mirrors `dispatch_native`'s guard (§5.3): only the native-service
        // arm of `dispatch_json_rpc_once` requires a caller, so only gate
        // here when the resolved pipeline targets one -- an anonymous
        // WASM-component call over this same fallthrough is unaffected.
        if matches!(self.pipeline.service, ServiceStage::NativeService { .. })
            && self.caller.is_none()
        {
            return Ok(structured_rpc_error(
                StatusCode::UNAUTHORIZED,
                UNAUTHENTICATED_RPC_CODE,
                format!(
                    "unauthenticated caller for native interface '{}'",
                    self.preamble.interface
                ),
            ));
        }

        match self
            .route_handler
            .dispatch_json_rpc_once(
                &self.pipeline,
                &self.preamble,
                self.caller.as_ref(),
                &body_bytes,
            )
            .await
        {
            Ok(payload) => {
                let res = Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/json")
                    .body(full_body(Bytes::from(payload)));
                Ok(res.unwrap_or_else(|_| Response::default()))
            }
            Err(e) => Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        }
    }

    async fn dispatch(
        &self,
        interface: &str,
        method: &str,
        params: Value,
    ) -> Result<DispatchOutcome> {
        dispatch_native(
            &self.route_handler,
            &self.pipeline,
            &self.preamble,
            self.caller.as_ref(),
            interface,
            method,
            params,
        )
        .await
    }

    /// Percent-decodes `path`, then does an exact-path lookup plus
    /// D-A1-11's one rewrite: a path ending in `/` resolves to
    /// `<path>index.html`. This function owns both the decoding and the
    /// rewrite -- callers pass the raw request path and do no
    /// normalisation of their own, so there is exactly one place either
    /// rule lives. Manifest keys come from raw archive entry names (never
    /// encoded), but every browser percent-encodes a request path (a file
    /// named `my file.js` is requested as `/my%20file.js`), so decoding
    /// here -- not at `resolve_route`, which keeps its existing
    /// non-decoding style for API routes -- is what makes such a file
    /// reachable at all. No history-fallback, no prefix rules (D-A1-4):
    /// `/api/comments` has no trailing slash, so it is never rewritten and
    /// always falls through to route resolution.
    ///
    /// `None` when the service has no bundle, its declared visibility is
    /// not `public`, `path` isn't valid percent-encoded UTF-8, or no entry
    /// matches the (possibly rewritten) path -- deliberately
    /// indistinguishable, so a miss and a non-public bundle both read as
    /// "not found" to the caller (D-A1-8).
    fn resolve_asset(&self, path: &str) -> Option<AssetEntry> {
        let service_assets = self.route_handler.inner.assets.get(&self.preamble.service_id)?;
        if !service_assets.public {
            return None;
        }
        let decoded = percent_encoding::percent_decode_str(path).decode_utf8().ok()?;
        let lookup_path = match decoded.strip_suffix('/') {
            Some(prefix) => format!("{prefix}/index.html"),
            None => decoded.into_owned(),
        };
        service_assets.manifest.entries.get(&lookup_path).cloned()
    }

    /// Serves one static asset (M06A A1). `Ok(None)` means "not an asset"
    /// and the caller falls through to route resolution unchanged.
    async fn try_handle_asset(
        &self,
        method: &Method,
        path: &str,
        req: &Request<Incoming>,
    ) -> Result<Option<Response<HttpBody>>> {
        if *method != Method::GET && *method != Method::HEAD {
            return Ok(None);
        }
        let Some(entry) = self.resolve_asset(path) else {
            return Ok(None);
        };

        let etag = format!("\"{}\"", entry.hash);
        let cache_control = cache_control_for(&entry.content_type);
        if if_none_match_hits(req.headers().get(IF_NONE_MATCH), &etag) {
            let resp = Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(ETAG, etag)
                .header(CACHE_CONTROL, cache_control)
                .body(full_body(Bytes::new()))
                .map_err(|e| anyhow!("failed to build 304 response: {e}"))?;
            return Ok(Some(resp));
        }

        // `mime_guess` (`crates/control_plane/src/assets.rs`) falls back to
        // `application/octet-stream` for an unrecognised extension --
        // `nosniff` stops a browser from content-sniffing that into
        // something it will execute or render unexpectedly. `text/html`
        // additionally needs an explicit charset: with none, encoding falls
        // back to the browser's default, which mangles non-ASCII pages.
        let content_type = if entry.content_type == "text/html" {
            "text/html; charset=utf-8".to_string()
        } else {
            entry.content_type.clone()
        };
        // `entry.len` is promised here, before the body has streamed a
        // single byte -- a mid-stream `read-chunk` error in
        // `blob_download_step` below ends the body short of this declared
        // length instead of surfacing as a clean error, so the client sees
        // an aborted connection rather than a graceful failure. Pre-existing
        // on the signed-URL blob-download path too, just not previously
        // paired with a `Content-Length` for the mismatch to be assertable
        // against.
        let builder = Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, content_type)
            .header(CONTENT_LENGTH, entry.len.to_string())
            .header(ETAG, etag)
            .header(CACHE_CONTROL, cache_control)
            .header(X_CONTENT_TYPE_OPTIONS, "nosniff");

        if *method == Method::HEAD {
            let resp = builder
                .body(full_body(Bytes::new()))
                .map_err(|e| anyhow!("failed to build HEAD response: {e}"))?;
            return Ok(Some(resp));
        }

        // F3: never instantiates the component -- the same
        // `blob-store/open-download`+`read-chunk` native-dispatch streaming
        // `handle_blob_get` uses, reached through the identical
        // `NativeService` arm (D-06A-1). Deliberately bypasses `self.
        // dispatch()` (bound to `self.caller`, which may be `None`): a
        // public asset's authorization is D-A1-1's declared `visibility`,
        // already checked in `resolve_asset`, not the connection's own
        // delegation.
        let system_caller = CallerContext::service_system(&self.preamble.service_id);
        let open_params = serde_json::json!({"hash": entry.hash, "offset": 0});
        let download_id = match dispatch_native(
            &self.route_handler,
            &self.pipeline,
            &self.preamble,
            Some(&system_caller),
            "blob-store",
            "open-download",
            open_params,
        )
        .await?
        {
            DispatchOutcome::Success(value) => {
                let resp: OpenDownloadResponse = serde_json::from_value(value)
                    .map_err(|e| anyhow!("malformed open-download response: {e}"))?;
                resp.download_id
            }
            DispatchOutcome::Error { code, message } => {
                return Ok(Some(structured_rpc_error(
                    status_for_rpc_error_code(code),
                    code,
                    message,
                )));
            }
        };

        let state = BlobDownloadState {
            route_handler: self.route_handler.clone(),
            pipeline: self.pipeline.clone(),
            preamble: RoutePreamble {
                interface: "blob-store".to_string(),
                ..self.preamble.clone()
            },
            caller: system_caller,
            download_id,
            closed: false,
        };
        let stream = stream::unfold(state, blob_download_step);
        let body = StreamBody::new(stream).boxed_unsync();
        let resp =
            builder.body(body).map_err(|e| anyhow!("failed to build asset response: {e}"))?;
        Ok(Some(resp))
    }

    fn resolve_route(&self, method: &Method, path: &str) -> Option<(HttpRoute, Option<String>)> {
        let routes = self.route_handler.inner.http_routes.get(&self.preamble.service_id)?;
        routes.iter().find_map(|route| {
            if !route.method.eq_ignore_ascii_case(method.as_str()) {
                return None;
            }
            match_path(&route.path, path).map(|param| (route.clone(), param))
        })
    }

    async fn read_small_body(&self, req: Request<Incoming>) -> Result<BodyRead> {
        let limited = Limited::new(req.into_body(), MAX_SMALL_BODY_BYTES);
        match limited.collect().await {
            Ok(collected) => Ok(BodyRead::Ok(collected.to_bytes())),
            Err(e) => {
                if e.downcast_ref::<LengthLimitError>().is_some() {
                    Ok(BodyRead::Rejected(http_error(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        format!("request body exceeds {MAX_SMALL_BODY_BYTES} byte limit"),
                    )))
                } else {
                    Ok(BodyRead::Rejected(http_error(
                        StatusCode::BAD_REQUEST,
                        format!("failed to read request body: {e}"),
                    )))
                }
            }
        }
    }

    /// `read_small_body` plus the JSON-validity check every small-body
    /// route (`put`/`patch`/`publish`) needs -- collapses each call site's
    /// repeated "read, then reject on non-JSON" block to one match.
    async fn read_small_json_body(
        &self,
        req: Request<Incoming>,
    ) -> Result<result::Result<Bytes, Response<HttpBody>>> {
        let body = match self.read_small_body(req).await? {
            BodyRead::Ok(bytes) => bytes,
            BodyRead::Rejected(resp) => return Ok(Err(resp)),
        };
        if serde_json::from_slice::<Value>(&body).is_err() {
            return Ok(Err(http_error(
                StatusCode::BAD_REQUEST,
                "request body must be valid JSON".into(),
            )));
        }
        Ok(Ok(body))
    }

    /// Dispatches one native request and maps its `DispatchOutcome` to an
    /// HTTP response, without special-casing a `null` success value --
    /// shared by every route whose success case is "return the result
    /// as-is" (`query`/`patch`/`publish`). `get` and `put`'s follow-up
    /// fetch-back need different `null` handling per call site (a 404 vs.
    /// an internal error) and use `dispatch_get_response` instead.
    async fn dispatch_response(
        &self,
        interface: &str,
        method: &str,
        params: Value,
        ok_status: StatusCode,
    ) -> Result<Response<HttpBody>> {
        Ok(match self.dispatch(interface, method, params).await? {
            DispatchOutcome::Success(value) => json_response(ok_status, &value),
            DispatchOutcome::Error { code, message } => {
                structured_rpc_error(status_for_rpc_error_code(code), code, message)
            }
        })
    }

    /// Dispatches a `data-layer::get` and maps a `null` result (no record
    /// with this id) to `not_found_status`/`not_found_message` -- shared by
    /// the plain `get` route (a genuine 404) and `put`'s follow-up
    /// fetch-back (the record we just wrote being gone is a 500, not a
    /// 404).
    async fn dispatch_get_response(
        &self,
        collection: &str,
        id: &str,
        ok_status: StatusCode,
        not_found_status: StatusCode,
        not_found_message: &str,
    ) -> Result<Response<HttpBody>> {
        Ok(
            match self
                .dispatch(
                    "data-layer",
                    "get",
                    serde_json::json!({"collection": collection, "id": id}),
                )
                .await?
            {
                DispatchOutcome::Success(value) if value.is_null() => {
                    http_error(not_found_status, not_found_message.into())
                }
                DispatchOutcome::Success(value) => json_response(ok_status, &value),
                DispatchOutcome::Error { code, message } => {
                    structured_rpc_error(status_for_rpc_error_code(code), code, message)
                }
            },
        )
    }

    async fn dispatch_route(
        &self,
        route: &HttpRoute,
        path_param: Option<String>,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        match route.target.as_str() {
            "data-layer" => self.handle_data_layer_route(route, path_param, req).await,
            "messaging" => self.handle_messaging_route(route, req).await,
            "stream" => self.handle_stream_route(route, path_param, req).await,
            "guest" => self.handle_guest_route(route, path_param, req).await,
            "websocket" => self.handle_websocket_route(route, req).await,
            other => Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("http_routes entry has unknown target: {other}"),
            )),
        }
    }

    // -- data-layer ---------------------------------------------------

    async fn handle_data_layer_route(
        &self,
        route: &HttpRoute,
        path_param: Option<String>,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        let collection = route.collection.clone().unwrap_or_default();
        match route.operation.as_str() {
            "get" => {
                let Some(id) = path_param else {
                    return Ok(http_error(
                        StatusCode::BAD_REQUEST,
                        "route requires a path parameter".into(),
                    ));
                };
                self.dispatch_get_response(
                    &collection,
                    &id,
                    StatusCode::OK,
                    StatusCode::NOT_FOUND,
                    "record not found",
                )
                .await
            }
            "query" => {
                let opts = match query_opts_from_query_string(req.uri().query().unwrap_or("")) {
                    Ok(opts) => opts,
                    Err(message) => return Ok(http_error(StatusCode::BAD_REQUEST, message)),
                };
                self.dispatch_response(
                    "data-layer",
                    "query",
                    serde_json::json!({"collection": collection, "opts": opts}),
                    StatusCode::OK,
                )
                .await
            }
            "put" => {
                let body = match self.read_small_json_body(req).await? {
                    Ok(bytes) => bytes,
                    Err(resp) => return Ok(resp),
                };
                // No `{id}` path segment (a plain `POST /collection`
                // create route) means the record id is server-generated --
                // `data-layer::put`'s WIT signature has no separate
                // create-vs-update distinction (it's an upsert), and
                // task.md's own route table only shows this shape without
                // an id in the path.
                let id = path_param.unwrap_or_else(|| Uuid::new_v4().to_string());
                let value = serde_json::json!({"id": id, "payload": body.to_vec()});
                match self
                    .dispatch(
                        "data-layer",
                        "put",
                        serde_json::json!({"collection": collection, "value": value}),
                    )
                    .await?
                {
                    DispatchOutcome::Error { code, message } => {
                        Ok(structured_rpc_error(status_for_rpc_error_code(code), code, message))
                    }
                    DispatchOutcome::Success(_) => {
                        // `put` itself returns `()` -- fetch the record back
                        // so the HTTP response can return it, per task.md's
                        // "POST /orders ... returns the resulting record".
                        // A `null` here means the record we just wrote is
                        // already gone (e.g. a concurrent delete raced this
                        // request) -- that's a 500, not the plain-`get`
                        // route's 404, since the write itself succeeded.
                        self.dispatch_get_response(
                            &collection,
                            &id,
                            StatusCode::CREATED,
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "record vanished immediately after being written",
                        )
                        .await
                    }
                }
            }
            "patch" => {
                let Some(id) = path_param else {
                    return Ok(http_error(
                        StatusCode::BAD_REQUEST,
                        "route requires a path parameter".into(),
                    ));
                };
                let body = match self.read_small_json_body(req).await? {
                    Ok(bytes) => bytes,
                    Err(resp) => return Ok(resp),
                };
                self.dispatch_response(
                    "data-layer",
                    "patch",
                    serde_json::json!({"collection": collection, "id": id, "patch_json": body.to_vec()}),
                    StatusCode::OK,
                )
                .await
            }
            other => Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported data-layer operation: {other}"),
            )),
        }
    }

    // -- messaging ------------------------------------------------------

    async fn handle_messaging_route(
        &self,
        route: &HttpRoute,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        match route.operation.as_str() {
            "publish" => self.handle_messaging_publish(route, req).await,
            "subscribe-sse" => self.handle_messaging_sse(route, req).await,
            other => Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported messaging operation: {other}"),
            )),
        }
    }

    async fn handle_messaging_publish(
        &self,
        route: &HttpRoute,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        let topic = route.topic.clone().unwrap_or_default();
        let body = match self.read_small_json_body(req).await? {
            Ok(bytes) => bytes,
            Err(resp) => return Ok(resp),
        };
        self.dispatch_response(
            "messaging",
            "publish",
            serde_json::json!({"topic": topic, "payload": body.to_vec()}),
            StatusCode::OK,
        )
        .await
    }

    /// `req` carries no body worth reading for a `GET`+SSE subscription;
    /// kept as a parameter for symmetry with the other route handlers
    /// (only the `Accept` header is inspected).
    async fn handle_messaging_sse(
        &self,
        route: &HttpRoute,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        let accepts_sse = req
            .headers()
            .get(ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/event-stream"));
        if !accepts_sse {
            return Ok(http_error(
                StatusCode::NOT_ACCEPTABLE,
                "Accept: text/event-stream is required for SSE subscription routes".into(),
            ));
        }

        let service_id = self.preamble.service_id.clone();
        let sse_service_id = self.preamble.service_id.clone();
        let max_subscribers = self
            .route_handler
            .inner
            .app_sandbox_engine
            .as_ref()
            .map(|e| e.max_sse_subscribers_per_service())
            .unwrap_or(DEFAULT_MAX_SSE_SUBSCRIBERS_PER_SERVICE);
        let permits = {
            let entry = self
                .route_handler
                .inner
                .sse_permits
                .entry(service_id)
                .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max_subscribers)));
            entry.value().clone()
        };
        let permit = match permits.try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                let mut resp = http_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service is at its SSE subscriber concurrency limit".into(),
                );
                resp.headers_mut().insert(RETRY_AFTER, HeaderValue::from_static("1"));
                return Ok(resp);
            }
        };

        let topic = route.topic.clone().unwrap_or_default();
        let namespaced = namespace_topic(&self.preamble.service_id, &topic);
        let (handle, receiver) = self
            .route_handler
            .inner
            .messaging_broker
            .subscribe(namespaced)
            .await
            .map_err(|e| anyhow!("SSE subscribe failed: {e}"))?;

        // Pull-based: each poll awaits the next broker message and formats
        // it as one SSE frame. `handle` (the `SubscriptionHandle`) and
        // `permit` are carried inside the stream's own state, so they -- and
        // the broker subscription they own -- are dropped the moment hyper
        // stops driving this response body, which is exactly what happens
        // when the client disconnects.
        let stream = stream::unfold(
            (receiver, handle, permit, sse_service_id),
            |(mut receiver, handle, permit, sid)| async move {
                let (topic, payload) = receiver.recv().await?;
                let name = service_relative_topic(&sid, &topic);
                let frame = Frame::data(Bytes::from(format_sse_frame(name, &payload)));
                Some((Ok::<_, Infallible>(frame), (receiver, handle, permit, sid)))
            },
        );

        let body = StreamBody::new(stream).boxed_unsync();
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream")
            .header(CACHE_CONTROL, "no-cache")
            .body(body)
            .map_err(|e| anyhow!("failed to build SSE response: {e}"))
    }

    // -- stream / chunked upload and download ----------------------------

    async fn handle_stream_route(
        &self,
        route: &HttpRoute,
        path_param: Option<String>,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        let Some(app_sandbox_engine) = self.route_handler.inner.app_sandbox_engine.clone() else {
            return Ok(http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "app sandbox engine not available (coordinator mode)".into(),
            ));
        };
        let Some(protocol) = route.protocol.clone() else {
            return Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "http_routes entry missing `protocol` for a stream route".into(),
            ));
        };
        // Mirrors `io.rs::handle_stream_protocol_request`'s
        // `UNKNOWN_PEER_ID` fallback for the raw-QUIC path -- an HTTP
        // caller carries the same optional `delegation` on its preamble.
        let peer_id = self
            .preamble
            .delegation
            .as_ref()
            .map(|d| d.master_did.clone())
            .unwrap_or_else(|| "unknown-peer".to_string());

        // `initial_payload` doubles as the guest's `metadata` parameter
        // (`accept-stream-upload(protocol, peer-id, metadata)`) or the
        // download request parameter (`handle-stream-request(protocol, peer-id,
        // request-data)`).
        let initial_payload = path_param
            .as_ref()
            .map(|p| percent_encoding::percent_decode_str(p).collect::<Vec<u8>>())
            .unwrap_or_else(|| {
                req.uri()
                    .query()
                    .and_then(|q| parse_query(q).remove("metadata"))
                    .map(String::into_bytes)
                    .unwrap_or_default()
            });

        match route.operation.as_str() {
            "accept-upload" => {
                let body_stream = req.into_body().into_data_stream().map_err(io::Error::other);
                let reader: Box<dyn AsyncRead + Unpin + Send> =
                    Box::new(StreamReader::new(body_stream));
                let writer: Box<dyn AsyncWrite + Unpin + Send> = Box::new(tokio_io::sink());

                match app_sandbox_engine
                    .handle_stream_protocol_request(
                        &self.preamble.service_id,
                        &protocol,
                        &peer_id,
                        StreamDirection::Upload,
                        initial_payload,
                        reader,
                        writer,
                    )
                    .await
                {
                    Ok(StreamRequestOutcome::Completed) => Ok(json_response(
                        StatusCode::OK,
                        &serde_json::json!({"status": "uploaded"}),
                    )),
                    Ok(StreamRequestOutcome::Declined) => {
                        Ok(http_error(StatusCode::FORBIDDEN, "upload declined by guest".into()))
                    }
                    Err(e) => {
                        error!(
                            service_id = %self.preamble.service_id,
                            protocol = %protocol,
                            error = %e,
                            "accept-upload failed"
                        );
                        Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
                    }
                }
            }
            "accept-download" => {
                let (duplex_writer, mut duplex_reader) = tokio_io::duplex(64 * 1024);
                let reader: Box<dyn AsyncRead + Unpin + Send> = Box::new(tokio_io::empty());
                let writer: Box<dyn AsyncWrite + Unpin + Send> = Box::new(duplex_writer);

                let service_id = self.preamble.service_id.clone();
                let engine = app_sandbox_engine.clone();
                let proto = protocol.clone();
                let peer = peer_id.clone();

                let join_handle = tokio::spawn(async move {
                    engine
                        .handle_stream_protocol_request(
                            &service_id,
                            &proto,
                            &peer,
                            StreamDirection::Download,
                            initial_payload,
                            reader,
                            writer,
                        )
                        .await
                });

                let mut first_buf = vec![0u8; 64 * 1024];
                let first_chunk = match duplex_reader.read(&mut first_buf).await {
                    Ok(0) => match join_handle.await {
                        Ok(Ok(StreamRequestOutcome::Declined)) => {
                            return Ok(http_error(
                                StatusCode::NOT_FOUND,
                                "stream download declined or file not found".into(),
                            ));
                        }
                        Ok(Err(e)) => {
                            return Ok(http_error(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                e.to_string(),
                            ));
                        }
                        _ => {
                            return Ok(http_error(
                                StatusCode::NOT_FOUND,
                                "stream download produced no data".into(),
                            ));
                        }
                    },
                    Ok(n) => {
                        first_buf.truncate(n);
                        Some(Bytes::from(first_buf))
                    }
                    Err(e) => {
                        return Ok(http_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("failed to read stream download: {e}"),
                        ));
                    }
                };

                let stream = stream::unfold(
                    (duplex_reader, first_chunk, Some(join_handle)),
                    |(mut reader, first, mut handle)| async move {
                        if let Some(chunk) = first {
                            return Some((
                                Ok::<_, Infallible>(Frame::data(chunk)),
                                (reader, None, handle),
                            ));
                        }
                        let mut buf = vec![0u8; 64 * 1024];
                        match reader.read(&mut buf).await {
                            Ok(0) => {
                                if let Some(h) = handle.take() {
                                    match h.await {
                                        Ok(Ok(StreamRequestOutcome::Completed)) => {}
                                        Ok(Ok(StreamRequestOutcome::Declined)) => {
                                            warn!(
                                                "stream download declined by guest after partial \
                                                 transfer"
                                            );
                                        }
                                        Ok(Err(e)) => {
                                            error!(
                                                "stream download task failed after partial \
                                                 transfer: {e}"
                                            );
                                        }
                                        Err(e) => {
                                            error!("stream download task panicked: {e}");
                                        }
                                    }
                                }
                                None
                            }
                            Ok(n) => {
                                buf.truncate(n);
                                Some((
                                    Ok::<_, Infallible>(Frame::data(Bytes::from(buf))),
                                    (reader, None, handle),
                                ))
                            }
                            Err(e) => {
                                error!("stream download read error: {e}");
                                None
                            }
                        }
                    },
                );

                let body = StreamBody::new(stream).boxed_unsync();
                let mut resp_builder = Response::builder().status(StatusCode::OK);
                if let Some(filename) = path_param.as_deref() {
                    let mime = mime_guess::from_path(filename).first_or_octet_stream();
                    resp_builder = resp_builder.header(CONTENT_TYPE, mime.as_ref());
                } else {
                    resp_builder = resp_builder.header(CONTENT_TYPE, "application/octet-stream");
                }
                resp_builder
                    .body(body)
                    .map_err(|e| anyhow!("failed to build download response: {e}"))
            }
            other => Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported stream operation: {other}"),
            )),
        }
    }

    // -- guest HTTP route target (M06A A2) --------------------------------

    /// The fourth `dispatch_route` target: hands the request to the
    /// deployed component's `syneroym:http/incoming-handler#handle-request`
    /// export and turns its answer into an HTTP response. Reaches the guest
    /// directly through `app_sandbox_engine`, mirroring
    /// `handle_stream_route` -- an `http-native` connection resolves to a
    /// `NativeService` pipeline (F2), so `dispatch_json_rpc_once` can never
    /// reach a guest, unlike `data-layer`/`messaging` above.
    async fn handle_guest_route(
        &self,
        route: &HttpRoute,
        path_param: Option<String>,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        if route.operation != "handle-request" {
            return Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported guest operation: {}", route.operation),
            ));
        }

        // D-A2-7, BEFORE any engine work: an anonymous caller on a
        // non-public route never instantiates anything. Same code and
        // status shape `dispatch_native` uses, so one 401 taxonomy covers
        // the whole bridge.
        if self.caller.is_none() && !route.public {
            return Ok(structured_rpc_error(
                StatusCode::UNAUTHORIZED,
                UNAUTHENTICATED_RPC_CODE,
                format!("unauthenticated caller for guest route {} {}", route.method, route.path),
            ));
        }

        let caller_identity = match guest_caller_identity(self.caller.as_ref(), &self.preamble) {
            Ok(identity) => identity,
            Err(reason) => {
                return Ok(http_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("unexpected caller context: {reason}"),
                ));
            }
        };

        let Some(app_sandbox_engine) = self.route_handler.inner.app_sandbox_engine.clone() else {
            return Ok(http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "app sandbox engine not available (coordinator mode)".into(),
            ));
        };
        if !app_sandbox_engine.is_deployed(&self.preamble.service_id) {
            return Ok(http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "service has no deployed WASM component".into(),
            ));
        }

        let (parts, body) = req.into_parts();
        let headers = match guest_request_headers(&parts.headers) {
            Ok(headers) => headers,
            Err((status, message)) => return Ok(http_error(status, message)),
        };
        // Every rejection above happens before any engine call, so each
        // costs zero instantiations.
        let limited = Limited::new(body, MAX_GUEST_REQUEST_BODY_BYTES);
        let body_bytes = match limited.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) if e.downcast_ref::<LengthLimitError>().is_some() => {
                return Ok(http_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!("request body exceeds {MAX_GUEST_REQUEST_BODY_BYTES} byte limit"),
                ));
            }
            Err(e) => {
                return Ok(http_error(
                    StatusCode::BAD_REQUEST,
                    format!("failed to read request body: {e}"),
                ));
            }
        };

        let path_params = match (param_name(&route.path), path_param) {
            (Some(name), Some(value)) => vec![(name.to_string(), value)],
            _ => vec![],
        };
        let request = GuestHttpRequest {
            method: parts.method.as_str().to_string(),
            path: parts.uri.path().to_string(),
            query: parts.uri.query().unwrap_or("").to_string(),
            route: route.path.clone(),
            path_params,
            headers,
            body: body_bytes.to_vec(),
            caller: caller_identity,
        };

        match app_sandbox_engine
            .handle_guest_http_request(&self.preamble.service_id, &request, self.caller.clone())
            .await
        {
            Ok(GuestHttpOutcome::Response(response)) => Ok(build_guest_response(response)),
            Ok(GuestHttpOutcome::Failed(GuestHttpFailure::Unavailable(detail))) => {
                let mut resp = http_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("service is at its guest HTTP concurrency limit: {detail}"),
                );
                resp.headers_mut().insert("retry-after", HeaderValue::from_static("1"));
                Ok(resp)
            }
            Ok(GuestHttpOutcome::Failed(failure)) => {
                // `Declined` is the guest's own `Err` return -- an ordinary
                // application-level outcome, and on a `public: true` route
                // one any anonymous caller can trigger at will. Logging it
                // at `error!` would make the node's error log a rate the
                // caller controls; every other variant is a genuine host or
                // component-shape problem and stays at `error!`.
                if matches!(failure, GuestHttpFailure::Declined(_)) {
                    warn!(
                        service_id = %self.preamble.service_id,
                        route = %route.path,
                        ?failure,
                        "guest HTTP handler declined the request"
                    );
                } else {
                    error!(
                        service_id = %self.preamble.service_id,
                        route = %route.path,
                        ?failure,
                        "guest HTTP handler failed"
                    );
                }
                Ok(http_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    describe_guest_http_failure(&failure),
                ))
            }
            Err(e) => Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        }
    }

    fn validate_websocket_upgrade_headers(headers: &HeaderMap) -> Result<&str, &'static str> {
        let is_upgrade = headers
            .get(hyper::header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
        let is_conn_upgrade = headers
            .get(hyper::header::CONNECTION)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.to_ascii_lowercase().contains("upgrade"));
        let has_version_13 = headers
            .get("sec-websocket-version")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == "13");
        let req_key =
            headers.get("sec-websocket-key").and_then(|v| v.to_str().ok()).unwrap_or_default();

        if !is_upgrade || !is_conn_upgrade || !has_version_13 || req_key.is_empty() {
            return Err("Invalid WebSocket upgrade request headers");
        }
        Ok(req_key)
    }

    async fn handle_websocket_route(
        &self,
        route: &HttpRoute,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        if self.caller.is_none() && !route.public {
            return Ok(http_error(StatusCode::UNAUTHORIZED, "Unauthorized".into()));
        }

        if route.operation != "handle-upgrade" {
            return Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported websocket operation: {}", route.operation),
            ));
        }

        let Some(app_sandbox_engine) = self.route_handler.inner.app_sandbox_engine.clone() else {
            return Ok(http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "app sandbox engine not available (coordinator mode)".into(),
            ));
        };
        if !app_sandbox_engine.is_deployed(&self.preamble.service_id) {
            return Ok(http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "service has no deployed WASM component".into(),
            ));
        }

        let req_key = match Self::validate_websocket_upgrade_headers(req.headers()) {
            Ok(k) => k,
            Err(msg) => {
                return Ok(http_error(StatusCode::BAD_REQUEST, msg.into()));
            }
        };

        let accept_key = derive_accept_key(req_key.as_bytes());
        let response = Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(hyper::header::UPGRADE, "websocket")
            .header(hyper::header::CONNECTION, "upgrade")
            .header("Sec-WebSocket-Accept", accept_key)
            .body(full_body(Bytes::new()));

        let response = match response {
            Ok(r) => r,
            Err(_) => {
                return Ok(http_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "failed to build response".into(),
                ));
            }
        };

        let service_id = self.preamble.service_id.clone();
        let mut topic_rx = None;
        let mut _sub_handle = None;
        if let Some(topic) = &route.topic {
            let namespaced = syneroym_mqtt_broker::namespace_topic(&service_id, topic);
            let (handle, rx_broadcast) =
                match self.route_handler.inner.messaging_broker.subscribe(namespaced).await {
                    Ok(res) => res,
                    Err(e) => {
                        return Ok(http_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("failed to subscribe: {e}"),
                        ));
                    }
                };
            topic_rx = Some(rx_broadcast);
            _sub_handle = Some(handle);
        }

        let permit = match app_sandbox_engine
            .acquire_websocket_permit(&service_id, Duration::from_secs(2))
            .await
        {
            Some(p) => p,
            None => {
                let mut resp = http_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "websocket concurrency limit reached".into(),
                );
                resp.headers_mut().insert(RETRY_AFTER, HeaderValue::from_static("1"));
                return Ok(resp);
            }
        };

        let conn_id = uuid::Uuid::new_v4().to_string();
        let mut rx_internal = app_sandbox_engine.register_websocket_sender(&service_id, &conn_id);
        let engine = app_sandbox_engine.clone();
        let caller = self.caller.clone();

        tokio::task::spawn(async move {
            let _keep_alive = _sub_handle;
            let _permit = permit;
            match hyper::upgrade::on(req).await {
                Ok(upgraded) => {
                    let io = hyper_util::rt::TokioIo::new(upgraded);
                    let mut ws_config = WebSocketConfig::default();
                    ws_config.max_message_size = Some(1024 * 1024);
                    ws_config.max_frame_size = Some(1024 * 1024);

                    let ws_stream =
                        WebSocketStream::from_raw_socket(io, Role::Server, Some(ws_config)).await;

                    use futures::{SinkExt, StreamExt};
                    let (mut ws_sink, mut ws_stream) = ws_stream.split();

                    let (writer_shutdown_tx, mut writer_shutdown_rx) =
                        tokio::sync::oneshot::channel::<()>();
                    let (session_stop_tx, mut session_stop_rx) =
                        tokio::sync::oneshot::channel::<()>();
                    let writer_task = tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                _ = &mut writer_shutdown_rx => break,
                                broadcast = async {
                                    if let Some(rx) = &mut topic_rx {
                                        rx.recv().await
                                    } else {
                                        futures::future::pending().await
                                    }
                                } => {
                                    match broadcast {
                                        Some((_, payload)) => {
                                            let msg = if let Ok(text) = String::from_utf8(payload.clone()) {
                                                Message::Text(text.into())
                                            } else {
                                                Message::Binary(payload.into())
                                            };
                                            if ws_sink.send(msg).await.is_err() {
                                                break;
                                            }
                                        }
                                        None => break,
                                    }
                                }
                                internal_msg = rx_internal.recv() => {
                                    match internal_msg {
                                        Some((frame, kind)) => {
                                            let msg = match kind {
                                                FrameKind::Text => {
                                                    let text = String::from_utf8_lossy(&frame).to_string();
                                                    Message::Text(text.into())
                                                }
                                                FrameKind::Binary => Message::Binary(frame.into()),
                                            };
                                            if ws_sink.send(msg).await.is_err() {
                                                break;
                                            }
                                        }
                                        None => break,
                                    }
                                }
                            }
                        }
                        let _ = session_stop_tx.send(());
                    });

                    // Sequential dispatch: await on-open before frame loop
                    engine.handle_websocket_on_open(&service_id, &conn_id, caller.clone()).await;

                    loop {
                        tokio::select! {
                            _ = &mut session_stop_rx => break,
                            msg_opt = ws_stream.next() => {
                                let Some(msg_res) = msg_opt else { break; };
                                match msg_res {
                                    Ok(Message::Text(txt)) => {
                                        engine
                                            .handle_websocket_on_message(
                                                &service_id,
                                                &conn_id,
                                                txt.as_bytes().to_vec(),
                                                FrameKind::Text,
                                                caller.clone(),
                                            )
                                            .await;
                                    }
                                    Ok(Message::Binary(bin)) => {
                                        engine
                                            .handle_websocket_on_message(
                                                &service_id,
                                                &conn_id,
                                                bin.to_vec(),
                                                FrameKind::Binary,
                                                caller.clone(),
                                            )
                                            .await;
                                    }
                                    Ok(Message::Close(_)) => break,
                                    Ok(Message::Ping(_)) => {}
                                    Ok(Message::Pong(_)) => {}
                                    Ok(Message::Frame(_)) => {}
                                    Err(e) => {
                                        debug!(service_id, conn_id, error = %e, "WebSocket stream error");
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    drop(ws_stream);
                    let _ = writer_shutdown_tx.send(());
                    let _ = writer_task.await;
                    engine.deregister_websocket_sender(&service_id, &conn_id);
                    engine.handle_websocket_on_close(&service_id, &conn_id, caller).await;
                }
                Err(e) => {
                    error!("WebSocket upgrade error: {}", e);
                    engine.deregister_websocket_sender(&service_id, &conn_id);
                }
            }
        });

        Ok(response)
    }

    // -- signed-URL blob GET ---------------------------------------------

    async fn handle_blob_get(&self, hash: &str, query: &str) -> Result<Response<HttpBody>> {
        let params = parse_query(query);
        let Some(svc) = params.get("svc") else {
            return Ok(http_error(StatusCode::BAD_REQUEST, "missing svc query parameter".into()));
        };
        // Decision 6: `svc` must equal the connection's own
        // `preamble.service_id` -- self-authorizing via the HMAC alone
        // doesn't extend to letting one connection serve another
        // service's blobs.
        if svc != &self.preamble.service_id {
            return Ok(http_error(
                StatusCode::FORBIDDEN,
                "svc query parameter must match the connected service".into(),
            ));
        }
        let Some(exp) = params.get("exp").and_then(|v| v.parse::<u64>().ok()) else {
            return Ok(http_error(
                StatusCode::BAD_REQUEST,
                "missing or invalid exp query parameter".into(),
            ));
        };
        let Some(sig) = params.get("sig") else {
            return Ok(http_error(StatusCode::BAD_REQUEST, "missing sig query parameter".into()));
        };

        let (Some(key_store), Some(storage_provider)) =
            (&self.route_handler.inner.key_store, &self.route_handler.inner.storage_provider)
        else {
            return Ok(http_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "blob serving is not available in this mode".into(),
            ));
        };
        let dek = storage_provider
            .load_service_dek(&self.preamble.service_id, key_store)
            .await
            .map_err(|e| anyhow!("failed to resolve service DEK: {e}"))?
            .unwrap_or_default();
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        if let Err(e) =
            crypto::verify_signed_url(&dek, &self.preamble.service_id, hash, exp, sig, now)
        {
            return Ok(http_error(
                StatusCode::FORBIDDEN,
                format!("invalid or expired signed URL: {e}"),
            ));
        }

        // TODO(M04B/FDAE): the signed-URL HMAC is the B0 authorization for
        // blob GET. Final policy (who may fetch which blob) is enforced by
        // FDAE (M04B) against the resolved caller; `service_system` is an
        // interim system identity.
        //
        // This bypasses `self.dispatch()` (bound to `self.caller`, which may
        // be `None` for an anonymous signed-URL request) deliberately -- the
        // HMAC verified above is this route's authorization, not the
        // connection's delegation.
        let system_caller = CallerContext::service_system(&self.preamble.service_id);
        let open_params = serde_json::json!({"hash": hash, "offset": 0});
        let download_id = match dispatch_native(
            &self.route_handler,
            &self.pipeline,
            &self.preamble,
            Some(&system_caller),
            "blob-store",
            "open-download",
            open_params,
        )
        .await?
        {
            DispatchOutcome::Success(value) => {
                let resp: OpenDownloadResponse = serde_json::from_value(value)
                    .map_err(|e| anyhow!("malformed open-download response: {e}"))?;
                resp.download_id
            }
            DispatchOutcome::Error { code, message } => {
                return Ok(structured_rpc_error(status_for_rpc_error_code(code), code, message));
            }
        };

        let state = BlobDownloadState {
            route_handler: self.route_handler.clone(),
            pipeline: self.pipeline.clone(),
            preamble: RoutePreamble {
                interface: "blob-store".to_string(),
                ..self.preamble.clone()
            },
            caller: system_caller,
            download_id,
            closed: false,
        };
        let stream = stream::unfold(state, blob_download_step);
        let body = StreamBody::new(stream).boxed_unsync();
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(body)
            .map_err(|e| anyhow!("failed to build blob response: {e}"))
    }
}

/// State carried across `blob_download_step`'s `stream::unfold` iterations:
/// everything needed to make another `blob-store/read-chunk` native-dispatch
/// call. No `blob_provider`/DEK access here -- streaming reuses the
/// existing `open-download`/`read-chunk` methods (which resolve the DEK
/// internally per call, same as every other native-dispatch blob-store
/// method), per decision 7 of the Slice 7 plan.
struct BlobDownloadState {
    route_handler: RouteHandler,
    pipeline: RoutePipeline,
    preamble: RoutePreamble,
    /// The `service_system` caller established in `handle_blob_get` -- reused
    /// here rather than `None`/`self.caller` so the per-chunk and cleanup
    /// dispatches stay self-authorizing regardless of the original
    /// connection's delegation.
    caller: CallerContext,
    download_id: String,
    /// Set once the server side is known to have already released
    /// `download_id` on its own (the EOF path in `dispatch_blob_store`'s
    /// `read-chunk` arm doesn't reinsert the session) -- `Drop` only issues
    /// a `close-download` cleanup call when this is still `false`, so a
    /// normally-completed download doesn't pay for a redundant round trip.
    closed: bool,
}

impl Drop for BlobDownloadState {
    /// An HTTP client that disconnects before the body reaches EOF (a
    /// routine tab close, a client timeout, or simply not reading the full
    /// response) makes hyper drop this state without polling
    /// `blob_download_step` again -- with no other cancellation signal,
    /// the server-side `download_sessions` entry would otherwise leak
    /// until process restart. Fires a best-effort, fire-and-forget
    /// `close-download` in that case (mirrors `abort-upload`'s cleanup for
    /// the symmetric upload-side case).
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let route_handler = self.route_handler.clone();
        let pipeline = self.pipeline.clone();
        let preamble = self.preamble.clone();
        let caller = self.caller.clone();
        let download_id = self.download_id.clone();
        tokio::spawn(async move {
            let _ = dispatch_native(
                &route_handler,
                &pipeline,
                &preamble,
                Some(&caller),
                "blob-store",
                "close-download",
                serde_json::json!({"download_id": download_id}),
            )
            .await;
        });
    }
}

/// Pull-based blob `GET` body: `stream::unfold` naturally drives "read next
/// chunk" lazily as the HTTP body is polled. A read-chunk error mid-stream
/// has no HTTP-status channel left to use (headers are already sent, and
/// chunked transfer-encoding has no structured mid-body error frame) --
/// ending the stream cleanly here is the same "peer observes a clean
/// failure, not a hang" outcome the raw-QUIC stream paths use.
async fn blob_download_step(
    mut state: BlobDownloadState,
) -> Option<(result::Result<Frame<Bytes>, Infallible>, BlobDownloadState)> {
    let params =
        serde_json::json!({"download_id": state.download_id, "max_bytes": BLOB_CHUNK_BYTES});
    let outcome = dispatch_native(
        &state.route_handler,
        &state.pipeline,
        &state.preamble,
        Some(&state.caller),
        "blob-store",
        "read-chunk",
        params,
    )
    .await
    .ok()?;
    let DispatchOutcome::Success(value) = outcome else {
        return None;
    };
    let resp: ReadChunkResponse = serde_json::from_value(value).ok()?;
    if resp.eof {
        // The server already dropped this download_id from its own
        // session map on the EOF path -- no cleanup call needed.
        state.closed = true;
        return None;
    }
    let frame = Frame::data(Bytes::from(resp.chunk));
    Some((Ok(frame), state))
}

/// Formats a JSON-RPC error response within an HTTP response, using the
/// generic `-32603` internal-error code -- callers with a real mapped RPC
/// error code use `structured_rpc_error` instead, to preserve it.
pub fn http_error(status: StatusCode, message: String) -> Response<HttpBody> {
    let body = JsonRpcErrorResponse {
        jsonrpc: "2.0".to_string(),
        error: JsonRpcError { code: -32603, message, data: None },
        id: None,
    };
    json_response(status, &serde_json::to_value(&body).unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use syneroym_identity::DelegationCertificate;
    use syneroym_rpc::SessionContext;
    use syneroym_ucan::CapabilityToken;

    use super::*;

    #[test]
    fn blob_hash_from_path_extracts_a_bare_hash() {
        assert_eq!(blob_hash_from_path("/blobs/deadbeef"), Some("deadbeef"));
    }

    #[test]
    fn blob_hash_from_path_rejects_non_blob_paths_and_nested_segments() {
        assert_eq!(blob_hash_from_path("/orders/abc"), None);
        assert_eq!(blob_hash_from_path("/blobs/"), None);
        assert_eq!(blob_hash_from_path("/blobs/a/b"), None);
    }

    #[test]
    fn if_none_match_hits_a_wildcard() {
        let value = HeaderValue::from_static("*");
        assert!(if_none_match_hits(Some(&value), "\"abc\""));
    }

    #[test]
    fn if_none_match_hits_an_exact_strong_etag() {
        let value = HeaderValue::from_static("\"abc\"");
        assert!(if_none_match_hits(Some(&value), "\"abc\""));
    }

    #[test]
    fn if_none_match_hits_one_entry_in_a_comma_separated_list() {
        let value = HeaderValue::from_static("\"nope\", \"abc\", \"also-nope\"");
        assert!(if_none_match_hits(Some(&value), "\"abc\""));
    }

    #[test]
    fn if_none_match_hits_a_weak_validator_by_stripping_the_prefix() {
        let value = HeaderValue::from_static("W/\"abc\"");
        assert!(if_none_match_hits(Some(&value), "\"abc\""));
    }

    #[test]
    fn if_none_match_misses_a_different_etag_or_a_missing_header() {
        let value = HeaderValue::from_static("\"different\"");
        assert!(!if_none_match_hits(Some(&value), "\"abc\""));
        assert!(!if_none_match_hits(None, "\"abc\""));
    }

    #[test]
    fn status_for_rpc_error_code_maps_every_known_code() {
        assert_eq!(status_for_rpc_error_code(-32001), StatusCode::NOT_FOUND);
        assert_eq!(status_for_rpc_error_code(-32002), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(status_for_rpc_error_code(-32010), StatusCode::FORBIDDEN);
        assert_eq!(status_for_rpc_error_code(-32011), StatusCode::NOT_FOUND);
        assert_eq!(status_for_rpc_error_code(-32012), StatusCode::BAD_REQUEST);
        assert_eq!(status_for_rpc_error_code(-32013), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(status_for_rpc_error_code(-32602), StatusCode::BAD_REQUEST);
        assert_eq!(status_for_rpc_error_code(-32603), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            status_for_rpc_error_code(UNSUPPORTED_PROTOCOL_RPC_CODE),
            StatusCode::NOT_IMPLEMENTED
        );
        assert_eq!(status_for_rpc_error_code(PROXY_TRANSPORT_RPC_CODE), StatusCode::BAD_GATEWAY);
        assert_eq!(
            status_for_rpc_error_code(UNSUPPORTED_TARGET_RPC_CODE),
            StatusCode::NOT_IMPLEMENTED
        );
        assert_eq!(status_for_rpc_error_code(-1), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn parse_query_parses_ampersand_separated_pairs_and_percent_decodes() {
        let parsed = parse_query("svc=abc&exp=123&sig=deadbeef&name=hello%20world&tag=a%26b");
        assert_eq!(parsed.get("svc"), Some(&"abc".to_string()));
        assert_eq!(parsed.get("exp"), Some(&"123".to_string()));
        assert_eq!(parsed.get("sig"), Some(&"deadbeef".to_string()));
        assert_eq!(parsed.get("name"), Some(&"hello world".to_string()));
        assert_eq!(parsed.get("tag"), Some(&"a&b".to_string()));
    }

    #[test]
    fn query_opts_from_query_string_maps_reserved_and_filter_keys() {
        let opts = query_opts_from_query_string("status=open&limit=5&cursor=abc").unwrap();
        assert_eq!(opts["limit"], serde_json::json!(5));
        assert_eq!(opts["cursor"], serde_json::json!("abc"));
        let filter: Value = serde_json::from_str(opts["filter"].as_str().unwrap()).unwrap();
        assert_eq!(filter, serde_json::json!({"status": "open"}));
    }

    #[test]
    fn query_opts_from_query_string_empty_query_is_unfiltered() {
        let opts = query_opts_from_query_string("").unwrap();
        assert_eq!(opts, serde_json::json!({"filter": null, "limit": null, "cursor": null}));
    }

    #[test]
    fn query_opts_from_query_string_rejects_non_numeric_limit() {
        assert!(query_opts_from_query_string("limit=notanumber").is_err());
    }

    #[test]
    fn format_sse_frame_includes_event_and_data_lines() {
        let frame = format_sse_frame("orders/new", b"hello");
        assert!(frame.starts_with("event: orders/new\n"));
        assert!(frame.contains("data: hello\n"));
        assert!(frame.ends_with("\n\n"));
    }

    #[test]
    fn format_sse_frame_strips_embedded_newlines_from_topic() {
        // A publisher-controlled topic containing CR/LF must not be able to
        // inject extra `data:`/`event:` lines into the frame -- a topic
        // string is exactly one MQTT topic, exactly one `event:` line.
        let malicious = "orders/new\ndata: {\"fake\":true}\n\nevent: spoofed";
        let frame = format_sse_frame(malicious, b"hello");
        let event_lines = frame.lines().filter(|l| l.starts_with("event:")).count();
        let data_lines = frame.lines().filter(|l| l.starts_with("data:")).count();
        assert_eq!(event_lines, 1, "exactly one event: line, frame was:\n{frame}");
        assert_eq!(data_lines, 1, "exactly one data: line, frame was:\n{frame}");
        assert!(!frame.contains('\r'), "no raw CR should survive into the frame");
    }

    #[test]
    fn service_relative_topic_strips_this_services_namespace() {
        assert_eq!(service_relative_topic("svc-a", "svc/svc-a/comment-updates"), "comment-updates");
    }

    #[test]
    fn service_relative_topic_leaves_a_foreign_or_unprefixed_topic_whole() {
        assert_eq!(service_relative_topic("svc-a", "svc/svc-b/x"), "svc/svc-b/x");
        assert_eq!(service_relative_topic("svc-a", "plain"), "plain");
    }

    // -- guest HTTP route target (M06A A2) -------------------------------

    fn caller_context(auth: AuthLevel) -> CallerContext {
        CallerContext {
            caller_did: "did:key:caller".to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: "did:key:caller".to_string(),
                ..Default::default()
            },
            auth,
            proof: None,
        }
    }

    fn preamble_with(delegation: Option<()>, ucan: Option<()>) -> RoutePreamble {
        let mut preamble = RoutePreamble::binary_json_rpc("svc", "http-native");
        if delegation.is_some() {
            preamble.delegation = Some(DelegationCertificate {
                master_did: "did:key:master".to_string(),
                temporary_did: "did:key:temp".to_string(),
                issued_at_secs: 0,
                expires_at_secs: u64::MAX,
                scope: "routing".to_string(),
                signature: "test-signature".to_string(),
            });
        }
        if ucan.is_some() {
            // Only `preamble.ucan.is_some()`-ness is exercised by these
            // tests (F5b: a rejected chain still leaves this set) -- the
            // token's own fields don't need to verify.
            preamble.ucan = Some(CapabilityToken {
                issuer_did: "did:key:issuer".to_string(),
                audience_did: "did:key:caller".to_string(),
                anchor_did: None,
                capabilities: vec![],
                facts: serde_json::Map::new(),
                not_before_secs: 0,
                expires_at_secs: u64::MAX,
                proofs: vec![],
                signature: "junk-signature".to_string(),
            });
        }
        preamble
    }

    #[test]
    fn guest_request_headers_lowercases_and_drops_host_owned() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Test", HeaderValue::from_static("1"));
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("100"));
        headers.insert("Connection", HeaderValue::from_static("keep-alive"));
        let result = guest_request_headers(&headers).unwrap();
        assert_eq!(result, vec![("x-test".to_string(), "1".to_string())]);
    }

    #[test]
    fn guest_request_headers_drops_non_utf8_values() {
        let mut headers = HeaderMap::new();
        headers.insert("x-binary", HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap());
        headers.insert("x-ok", HeaderValue::from_static("fine"));
        let result = guest_request_headers(&headers).unwrap();
        assert_eq!(result, vec![("x-ok".to_string(), "fine".to_string())]);
    }

    #[test]
    fn guest_request_headers_431s_past_the_count_cap() {
        let mut headers = HeaderMap::new();
        for i in 0..MAX_GUEST_REQUEST_HEADERS + 1 {
            headers.insert(
                HeaderName::from_bytes(format!("x-h{i}").as_bytes()).unwrap(),
                HeaderValue::from_static("v"),
            );
        }
        let (status, _) = guest_request_headers(&headers).unwrap_err();
        assert_eq!(status, StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE);
    }

    #[test]
    fn guest_caller_identity_none_stays_none() {
        let preamble = RoutePreamble::binary_json_rpc("svc", "http-native");
        assert_eq!(guest_caller_identity(None, &preamble).unwrap(), None);
    }

    #[test]
    fn guest_caller_identity_bare_pubkey_is_self_asserted_even_though_auth_says_delegated() {
        // F5a: `AuthLevel::Delegated` is assigned to every verified preamble,
        // including the client gateway's unchallenged pubkey -- so `auth`
        // must not be read straight off it.
        let caller = caller_context(AuthLevel::Delegated);
        let preamble = preamble_with(None, None);
        let identity = guest_caller_identity(Some(&caller), &preamble).unwrap().unwrap();
        assert_eq!(identity.auth, GuestCallerAuth::SelfAsserted);
    }

    #[test]
    fn guest_caller_identity_a_rejected_ucan_is_self_asserted_not_ucan() {
        // F5b: `build_caller` fails open on a bad UCAN chain, leaving
        // `preamble.ucan` set but `CallerContext.auth` at `Delegated` -- so
        // keying `ucan` off the preamble would let any caller self-label
        // the strongest value with a junk token.
        let caller = caller_context(AuthLevel::Delegated);
        let preamble = preamble_with(None, Some(()));
        let identity = guest_caller_identity(Some(&caller), &preamble).unwrap().unwrap();
        assert_eq!(identity.auth, GuestCallerAuth::SelfAsserted);
    }

    #[test]
    fn guest_caller_identity_verified_ucan_is_ucan() {
        let caller = caller_context(AuthLevel::Ucan);
        let preamble = preamble_with(None, Some(()));
        let identity = guest_caller_identity(Some(&caller), &preamble).unwrap().unwrap();
        assert_eq!(identity.auth, GuestCallerAuth::Ucan);
    }

    #[test]
    fn guest_caller_identity_delegation_present_is_delegated() {
        let caller = caller_context(AuthLevel::Delegated);
        let preamble = preamble_with(Some(()), None);
        let identity = guest_caller_identity(Some(&caller), &preamble).unwrap().unwrap();
        assert_eq!(identity.auth, GuestCallerAuth::Delegated);
    }

    #[test]
    fn guest_caller_identity_fails_closed_on_substrate_injected_levels() {
        let preamble = RoutePreamble::binary_json_rpc("svc", "http-native");
        for level in [AuthLevel::LocalElevated, AuthLevel::LocalReadOnly, AuthLevel::System] {
            let caller = caller_context(level);
            assert!(guest_caller_identity(Some(&caller), &preamble).is_err());
        }
    }

    #[test]
    fn http_route_with_no_public_key_deserializes_to_public_false() {
        let route: HttpRoute = serde_json::from_value(serde_json::json!({
            "method": "GET",
            "path": "/echo",
            "target": "guest",
            "operation": "handle-request"
        }))
        .unwrap();
        assert!(!route.public);
    }

    fn sample_guest_response(
        status: u16,
        headers: Vec<(&str, &str)>,
        body: Vec<u8>,
    ) -> GuestHttpResponse {
        GuestHttpResponse {
            status,
            headers: headers.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            body,
        }
    }

    #[test]
    fn build_guest_response_strips_host_owned_headers() {
        let response = sample_guest_response(
            200,
            vec![("content-length", "999"), ("connection", "close"), ("x-ok", "1")],
            b"hi".to_vec(),
        );
        let resp = build_guest_response(response);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get(CONTENT_LENGTH).unwrap(), "2");
        assert_eq!(resp.headers().get("x-ok").unwrap(), "1");
    }

    #[test]
    fn build_guest_response_rejects_invalid_header_value_with_500() {
        let response =
            sample_guest_response(200, vec![("x-bad", "line1\r\nline2")], b"hi".to_vec());
        let resp = build_guest_response(response);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn build_guest_response_rejects_invalid_header_name_with_500() {
        let response = sample_guest_response(200, vec![("x bad", "1")], b"hi".to_vec());
        let resp = build_guest_response(response);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn build_guest_response_rejects_out_of_range_status() {
        for status in [0u16, 99, 100, 600, 999] {
            let response = sample_guest_response(status, vec![], vec![]);
            let resp = build_guest_response(response);
            assert_eq!(
                resp.status(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "status {status} should have been rejected"
            );
        }
    }

    #[test]
    fn build_guest_response_rejects_over_cap_body() {
        let response =
            sample_guest_response(200, vec![], vec![0u8; MAX_GUEST_RESPONSE_BODY_BYTES + 1]);
        let resp = build_guest_response(response);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn build_guest_response_rejects_over_cap_header_count() {
        let headers = (0..MAX_GUEST_RESPONSE_HEADERS + 1)
            .map(|i| (format!("x-h{i}"), "v".to_string()))
            .collect();
        let response = GuestHttpResponse { status: 200, headers, body: vec![] };
        let resp = build_guest_response(response);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn build_guest_response_adds_nosniff_only_when_absent() {
        let response = sample_guest_response(200, vec![], vec![]);
        let resp = build_guest_response(response);
        assert_eq!(resp.headers().get(X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");

        let response =
            sample_guest_response(200, vec![("x-content-type-options", "custom")], vec![]);
        let resp = build_guest_response(response);
        assert_eq!(resp.headers().get(X_CONTENT_TYPE_OPTIONS).unwrap(), "custom");
    }

    #[test]
    fn build_guest_response_keeps_repeated_set_cookie_headers() {
        let response =
            sample_guest_response(200, vec![("set-cookie", "a=1"), ("set-cookie", "b=2")], vec![]);
        let resp = build_guest_response(response);
        let values: Vec<&str> =
            resp.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap()).collect();
        assert_eq!(values, vec!["a=1", "b=2"]);
    }

    #[test]
    fn websocket_upgrade_headers_validation_accepts_valid_and_rejects_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert("Upgrade", HeaderValue::from_static("websocket"));
        headers.insert("Connection", HeaderValue::from_static("Upgrade"));
        headers.insert("Sec-WebSocket-Version", HeaderValue::from_static("13"));
        headers.insert("Sec-WebSocket-Key", HeaderValue::from_static("dGhlIHNhbXBsZSBub25jZQ=="));
        assert_eq!(
            HttpHandler::validate_websocket_upgrade_headers(&headers),
            Ok("dGhlIHNhbXBsZSBub25jZQ==")
        );

        // Missing Sec-WebSocket-Key
        let mut bad_headers = headers.clone();
        bad_headers.remove("Sec-WebSocket-Key");
        assert!(HttpHandler::validate_websocket_upgrade_headers(&bad_headers).is_err());

        // Empty Sec-WebSocket-Key
        let mut empty_key = headers.clone();
        empty_key.insert("Sec-WebSocket-Key", HeaderValue::from_static(""));
        assert!(HttpHandler::validate_websocket_upgrade_headers(&empty_key).is_err());

        // Wrong version
        let mut bad_ver = headers.clone();
        bad_ver.insert("Sec-WebSocket-Version", HeaderValue::from_static("12"));
        assert!(HttpHandler::validate_websocket_upgrade_headers(&bad_ver).is_err());

        // Wrong connection
        let mut bad_conn = headers.clone();
        bad_conn.insert("Connection", HeaderValue::from_static("keep-alive"));
        assert!(HttpHandler::validate_websocket_upgrade_headers(&bad_conn).is_err());

        // Wrong upgrade
        let mut bad_up = headers.clone();
        bad_up.insert("Upgrade", HeaderValue::from_static("http2"));
        assert!(HttpHandler::validate_websocket_upgrade_headers(&bad_up).is_err());
    }
}

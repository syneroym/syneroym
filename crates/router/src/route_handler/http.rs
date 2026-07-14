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
//! 2. The connected service's `http_routes` table (method + path-with-
//!    `{param}` match) -- bridges onto `data-layer`/`messaging`/a registered
//!    stream protocol.
//! 3. Fallthrough, unchanged: the original `POST`+`application/json` JSON-RPC
//!    bridge.

use std::{
    collections::HashMap,
    convert::Infallible,
    io, result,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use futures::stream;
use http_body_util::{
    BodyExt, Full, LengthLimitError, Limited, StreamBody, combinators::UnsyncBoxBody,
};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Frame, Incoming},
    header::{ACCEPT, CACHE_CONTROL, CONTENT_TYPE},
    service,
};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as AutoBuilder,
};
use serde_json::Value;
use syneroym_core::{http_routes::HttpRoute, streaming::StreamDirection};
use syneroym_data_blob::{
    crypto,
    native_types::{OpenDownloadResponse, ReadChunkResponse},
};
use syneroym_mqtt_broker::namespace_topic;
use syneroym_rpc::{CallerContext, JsonRpcError, JsonRpcErrorResponse, JsonRpcRequest};
use syneroym_sandbox_wasm::StreamRequestOutcome;
use tokio::io::{self as tokio_io, AsyncRead, AsyncWrite};
use tokio_util::io::StreamReader;
use tracing::error;
use uuid::Uuid;

use super::RouteHandler;
use crate::{preamble::RoutePreamble, routing::RoutePipeline};

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
            .serve_connection(
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

/// Matches a single `{param}` path pattern (e.g. `/orders/{id}`) against a
/// request path. Returns `None` if the pattern doesn't match at all,
/// `Some(None)` if it matches with no captured parameter, `Some(Some(v))`
/// if it matches and captured `v`. Only a single `{param}` segment is
/// supported (sufficient for every route shape `task.md` specifies) -- no
/// general globbing/regex.
fn match_path(pattern: &str, path: &str) -> Option<Option<String>> {
    let pattern_segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path_segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if pattern_segs.len() != path_segs.len() {
        return None;
    }
    let mut captured = None;
    for (p, s) in pattern_segs.iter().zip(path_segs.iter()) {
        if p.starts_with('{') && p.ends_with('}') {
            captured = Some((*s).to_string());
        } else if p != s {
            return None;
        }
    }
    Some(captured)
}

/// Parses an HTTP query string (`k=v&k2=v2`) leniently, matching
/// `RoutePreamble::parse`'s own permissive, non-percent-decoding style
/// elsewhere in this crate.
fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
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

/// Formats one broker-delivered `(topic, payload)` message as an SSE frame.
/// Payload is treated as UTF-8 text (lossy) -- every fixture in this repo
/// only ever publishes UTF-8 text payloads (see `status.md`'s Slice 6A
/// notes), and SSE's `data:` framing is line-oriented, so a payload
/// containing embedded newlines is split across multiple `data:` lines per
/// the SSE spec rather than corrupting the frame.
///
/// `topic` is the publisher-supplied MQTT topic string, not a value this
/// route's own config controls -- `MqttBroker::publish` performs no
/// character validation on it, so embedded `\r`/`\n` are valid MQTT topic
/// bytes that would otherwise land verbatim in the single-line `event: `
/// field below and let a publisher inject fabricated `data:`/`event:` lines
/// into a subscriber's SSE stream (response-splitting). Stripped rather
/// than rejected: this frame is written directly into an already-open
/// streaming response with no HTTP-status channel left to reject through.
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

        if let Some((route, path_param)) = self.resolve_route(&method, &path) {
            return self.dispatch_route(&route, path_param, req).await;
        }

        self.handle_json_rpc_bridge(req).await
    }

    /// The original `POST`+`application/json` JSON-RPC bridge -- byte-for-
    /// byte the same behavior as before Slice 7, just wrapped in the new
    /// unified `HttpBody` type.
    async fn handle_json_rpc_bridge(&self, req: Request<Incoming>) -> Result<Response<HttpBody>> {
        if req.method() != Method::POST {
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
            "stream" => self.handle_stream_route(route, req).await,
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
        // it as one SSE frame. `handle` (the `SubscriptionHandle`) is
        // carried inside the stream's own state, so it -- and the broker
        // subscription it owns -- is dropped the moment hyper stops
        // driving this response body, which is exactly what happens when
        // the client disconnects (hyper's connection loop observes the
        // write failing and drops the in-flight response future).
        let stream = stream::unfold((receiver, handle), |(mut receiver, handle)| async move {
            let (topic, payload) = receiver.recv().await?;
            let frame = Frame::data(Bytes::from(format_sse_frame(&topic, &payload)));
            Some((Ok::<_, Infallible>(frame), (receiver, handle)))
        });

        let body = StreamBody::new(stream).boxed_unsync();
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/event-stream")
            .header(CACHE_CONTROL, "no-cache")
            .body(body)
            .map_err(|e| anyhow!("failed to build SSE response: {e}"))
    }

    // -- stream / chunked upload -----------------------------------------

    async fn handle_stream_route(
        &self,
        route: &HttpRoute,
        req: Request<Incoming>,
    ) -> Result<Response<HttpBody>> {
        if route.operation != "accept-upload" {
            return Ok(http_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unsupported stream operation: {}", route.operation),
            ));
        }
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
        // (`accept-stream-upload(protocol, peer-id, metadata)`) -- there is
        // no equivalent "first framed frame" concept in a plain chunked
        // HTTP body the way the raw-QUIC path has one, so it's taken from
        // the request's `metadata` query parameter instead (e.g. `PUT
        // /upload?metadata=x`), empty if absent. An HTTP-specific
        // simplification vs. the raw-QUIC path's framed initial payload,
        // but still lets a route's caller pass guest-meaningful metadata
        // rather than always sending nothing.
        let metadata = req
            .uri()
            .query()
            .and_then(|q| parse_query(q).remove("metadata"))
            .unwrap_or_default()
            .into_bytes();

        // Decision 5 of the Slice 7 plan: adapt the HTTP request body into
        // the `AsyncRead` `handle_stream_protocol_request` already accepts
        // (a trait object, no router-specific concrete type baked in).
        // `hyper::Error` doesn't implement `Into<io::Error>`, hence the
        // explicit `map_err`. The writer side is never used for data
        // transfer on the upload path (only `.shutdown()` is called), so a
        // plain sink suffices.
        use futures::TryStreamExt;
        let body_stream = req.into_body().into_data_stream().map_err(io::Error::other);
        let reader: Box<dyn AsyncRead + Unpin + Send> = Box::new(StreamReader::new(body_stream));
        let writer: Box<dyn AsyncWrite + Unpin + Send> = Box::new(tokio_io::sink());

        match app_sandbox_engine
            .handle_stream_protocol_request(
                &self.preamble.service_id,
                &protocol,
                &peer_id,
                StreamDirection::Upload,
                metadata,
                reader,
                writer,
            )
            .await
        {
            Ok(StreamRequestOutcome::Completed) => {
                Ok(json_response(StatusCode::OK, &serde_json::json!({"status": "uploaded"})))
            }
            Ok(StreamRequestOutcome::Declined) => {
                Ok(http_error(StatusCode::FORBIDDEN, "upload declined by guest".into()))
            }
            Err(e) => Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        }
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
    use super::*;

    #[test]
    fn match_path_captures_a_single_param() {
        assert_eq!(match_path("/orders/{id}", "/orders/abc123"), Some(Some("abc123".to_string())));
    }

    #[test]
    fn match_path_matches_exact_literal_with_no_param() {
        assert_eq!(match_path("/orders", "/orders"), Some(None));
    }

    #[test]
    fn match_path_rejects_different_segment_counts() {
        assert_eq!(match_path("/orders", "/orders/abc123"), None);
        assert_eq!(match_path("/orders/{id}", "/orders"), None);
    }

    #[test]
    fn match_path_rejects_mismatched_literal_segments() {
        assert_eq!(match_path("/orders/{id}", "/events/abc123"), None);
    }

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
    fn status_for_rpc_error_code_maps_every_known_code() {
        assert_eq!(status_for_rpc_error_code(-32001), StatusCode::NOT_FOUND);
        assert_eq!(status_for_rpc_error_code(-32002), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(status_for_rpc_error_code(-32010), StatusCode::FORBIDDEN);
        assert_eq!(status_for_rpc_error_code(-32011), StatusCode::NOT_FOUND);
        assert_eq!(status_for_rpc_error_code(-32012), StatusCode::BAD_REQUEST);
        assert_eq!(status_for_rpc_error_code(-32013), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(status_for_rpc_error_code(-32602), StatusCode::BAD_REQUEST);
        assert_eq!(status_for_rpc_error_code(-32603), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(status_for_rpc_error_code(-1), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn parse_query_parses_ampersand_separated_pairs() {
        let parsed = parse_query("svc=abc&exp=123&sig=deadbeef");
        assert_eq!(parsed.get("svc"), Some(&"abc".to_string()));
        assert_eq!(parsed.get("exp"), Some(&"123".to_string()));
        assert_eq!(parsed.get("sig"), Some(&"deadbeef".to_string()));
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
}

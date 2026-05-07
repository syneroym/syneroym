use anyhow::{Result, anyhow};
use bytes::Bytes;
use dashmap::DashMap;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler as IrohProtocolHandler};
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use syneroym_control_plane::ControlPlaneService;
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::{EndpointRegistry, SubstrateEndpoint};
use syneroym_rpc::framing;
use syneroym_rpc::{JsonRpcConverter, JsonRpcRequest, JsonRpcResponse, NativeService};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader, ReadBuf};
use tracing::{debug, error, warn};

use crate::net_iroh::IrohStream;
use crate::preamble::{RoutePreamble, RouteProtocol, RouteTransport};
use crate::routing::{DeliveryMode, ProtocolAdapter, ResolvedRoute, RouteExecution, RoutingPlan};

use syneroym_app_sandbox::AppSandboxEngine;

// ---------------------------------------------------------------------------
// ReaderWriter — reunites a tokio split pair into a single AsyncRead+AsyncWrite
// so that hyper (which needs one unified I/O type) can drive the Iroh stream.
// ---------------------------------------------------------------------------

struct ReaderWriter<R, W> {
    reader: R,
    writer: W,
}

impl<R: AsyncRead + Unpin, W: Unpin> AsyncRead for ReaderWriter<R, W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl<R: Unpin, W: AsyncWrite + Unpin> AsyncWrite for ReaderWriter<R, W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}

// ---------------------------------------------------------------------------
// RouteHandler
// ---------------------------------------------------------------------------

pub(crate) struct RouteHandler {
    registry: EndpointRegistry,
    native_dispatch: DashMap<String, Arc<dyn NativeService>>,
    app_sandbox_engine: Arc<AppSandboxEngine>,
}

impl fmt::Debug for RouteHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteHandler")
            .field("registry", &self.registry)
            .field("native_dispatch_len", &self.native_dispatch.len())
            .finish_non_exhaustive()
    }
}

impl RouteHandler {
    pub(crate) async fn init(
        service_id: String,
        config: &SubstrateConfig,
        registry: EndpointRegistry,
    ) -> Result<Self> {
        let app_sandbox_engine =
            Arc::new(AppSandboxEngine::init(config, registry.get_all_endpoints()).await?);

        let s = Self {
            registry: registry.clone(),
            native_dispatch: DashMap::new(),
            app_sandbox_engine: app_sandbox_engine.clone(),
        };

        let substrate_service =
            ControlPlaneService::init(service_id.clone(), app_sandbox_engine, registry).await?;
        s.register_native_service(service_id, Arc::new(substrate_service));
        Ok(s)
    }

    fn register_native_service(&self, service_id: String, service: Arc<dyn NativeService>) {
        self.native_dispatch.insert(service_id, service);
    }

    // -----------------------------------------------------------------------
    // Public stream entry-point — detects HTTP vs binary framing by peeking.
    // -----------------------------------------------------------------------

    pub async fn handle_stream<S>(&self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (read_half, write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);

        // All streams now start with a preamble identifying transport and protocol.
        let preamble = read_preamble(&mut reader).await?;

        match preamble.transport {
            RouteTransport::Http => {
                // Reunite reader + writer into a single I/O type for hyper.
                let io = TokioIo::new(ReaderWriter { reader, writer: write_half });
                return self.handle_http_stream(io, preamble).await;
            }
            RouteTransport::Binary => {
                let resolved_route = self.resolve_route(preamble)?;
                let routing_plan = self.plan_route(&resolved_route);
                log_route(&resolved_route, &routing_plan);

                match &routing_plan.execution {
                    RouteExecution::NativeJsonRpc { .. }
                    | RouteExecution::ExecuteWasm { .. }
                    | RouteExecution::Adapted { .. } => {
                        self.handle_json_rpc_loop(
                            reader,
                            &mut { write_half },
                            &resolved_route,
                            &routing_plan,
                        )
                        .await?;
                    }
                    RouteExecution::WasmWrpcPassthrough { channel_id } => {
                        debug!("Passthrough wRPC stream to Wasm channel: {}", channel_id);
                        self.handle_passthrough(reader, &mut { write_half }, channel_id).await?;
                    }
                    RouteExecution::Unsupported => {
                        warn!(
                            protocol = %resolved_route.request.protocol,
                            interface = resolved_route.request.interface.as_str(),
                            service_id = resolved_route.request.service_id.as_str(),
                            delivery_mode = ?routing_plan.delivery_mode,
                            endpoint = ?resolved_route.endpoint,
                            "unsupported routing combination"
                        );
                    }
                }
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // HTTP framing path — uses hyper_util::auto for transparent H1 / H2.
    // -----------------------------------------------------------------------

    async fn handle_http_stream<I>(&self, io: TokioIo<I>, preamble: RoutePreamble) -> Result<()>
    where
        I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        // SAFETY: The raw pointer in HttpHandler is only ever dereferenced inside
        // `handle_http_request`, which is driven to completion by `serve_connection`
        // below before this function returns.  `&self` (and thus the RouteHandler)
        // is guaranteed alive for that entire duration.
        let handler = Arc::new(HttpHandlerSafe(Arc::new(HttpHandler {
            route_handler: self as *const RouteHandler,
            preamble,
        })));

        AutoBuilder::new(hyper_util::rt::TokioExecutor::new())
            .serve_connection(
                io,
                hyper::service::service_fn(move |req| {
                    let h = handler.clone();
                    async move { h.0.handle_http_request(req).await }
                }),
            )
            .await
            .map_err(|e| anyhow!("HTTP connection error: {e}"))
    }

    // -----------------------------------------------------------------------
    // Shared single-request dispatch core — called by both framing paths.
    // -----------------------------------------------------------------------

    async fn dispatch_json_rpc_once(
        &self,
        resolved: &ResolvedRoute,
        plan: &RoutingPlan,
        body: &[u8],
    ) -> Result<Vec<u8>> {
        match &plan.execution {
            RouteExecution::NativeJsonRpc { channel_id } => {
                let service = self
                    .native_dispatch
                    .get(channel_id.as_str())
                    .map(|s| s.clone())
                    .ok_or_else(|| anyhow!("Native service not found for {}", channel_id))?;

                let (request, invocation) =
                    JsonRpcConverter::json_to_native(&resolved.request.interface, body)
                        .map_err(|e| anyhow!("JSON parse error: {e}"))?;

                match service.dispatch(invocation).await {
                    Ok(native_response) => {
                        JsonRpcConverter::native_to_json(&request, native_response)
                    }
                    Err(e) => {
                        error!("Native service error: {}", e);
                        JsonRpcConverter::json_error(request.id.clone(), -32603, e.to_string())
                    }
                }
            }
            RouteExecution::ExecuteWasm { channel_id } => {
                let request: JsonRpcRequest =
                    serde_json::from_slice(body).map_err(|e| anyhow!("JSON parse error: {e}"))?;

                match self
                    .app_sandbox_engine
                    .execute_wasm(channel_id, &resolved.request.interface, &request)
                    .await
                {
                    Ok(wasm_result) => {
                        let json_response = JsonRpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: serde_json::Value::String(wasm_result),
                            id: request.id.clone(),
                        };
                        serde_json::to_vec(&json_response).map_err(Into::into)
                    }
                    Err(e) => {
                        error!("WASM execution error: {}", e);
                        JsonRpcConverter::json_error(request.id.clone(), -32603, e.to_string())
                    }
                }
            }
            RouteExecution::Adapted { adapter } => {
                let message = adapter_not_implemented_message(*adapter);
                JsonRpcConverter::json_error(None, -32601, message)
            }
            RouteExecution::WasmWrpcPassthrough { .. } | RouteExecution::Unsupported => {
                Err(anyhow!(
                    "Execution plan {:?} not supported in request-response mode",
                    plan.execution
                ))
            }
        }
    }

    // -----------------------------------------------------------------------
    // Route resolution & planning (unchanged logic, shared by both paths).
    // -----------------------------------------------------------------------

    fn resolve_route(&self, preamble: RoutePreamble) -> Result<ResolvedRoute> {
        let endpoint =
            self.registry.lookup(&preamble.service_id, &preamble.interface).ok_or_else(|| {
                anyhow!("Service {} not found in local registry", preamble.service_id)
            })?;

        Ok(ResolvedRoute { request: preamble, endpoint })
    }

    fn plan_route(&self, route: &ResolvedRoute) -> RoutingPlan {
        // Keep this mapping intentionally direct and easy to revise.
        match (&route.request.protocol, &route.endpoint) {
            (
                RouteProtocol::JsonRpc,
                SubstrateEndpoint::NativeHostChannel { channel_details: channel_id },
            ) => RoutingPlan {
                delivery_mode: DeliveryMode::Broker,
                execution: RouteExecution::NativeJsonRpc { channel_id: channel_id.clone() },
            },
            (
                RouteProtocol::Wrpc,
                SubstrateEndpoint::WasmChannel { channel_details: channel_id },
            ) => RoutingPlan {
                delivery_mode: DeliveryMode::PassThrough,
                execution: RouteExecution::WasmWrpcPassthrough { channel_id: channel_id.clone() },
            },
            (
                RouteProtocol::JsonRpc,
                SubstrateEndpoint::WasmChannel { channel_details: channel_id },
            ) => RoutingPlan {
                delivery_mode: DeliveryMode::Broker,
                execution: RouteExecution::ExecuteWasm { channel_id: channel_id.clone() },
            },
            (RouteProtocol::JsonRpc, SubstrateEndpoint::PodmanSocket { .. }) => RoutingPlan {
                delivery_mode: DeliveryMode::Adapt,
                execution: RouteExecution::Adapted { adapter: ProtocolAdapter::JsonRpcToPodman },
            },
            _ => RoutingPlan::unsupported(),
        }
    }

    // -----------------------------------------------------------------------
    // Binary-framed loop handlers (Iroh / wRPC path).
    // -----------------------------------------------------------------------

    async fn handle_passthrough<R, W>(
        &self,
        _client_read: R,
        _client_write: &mut W,
        _channel_id: &str,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send,
    {
        Err(anyhow!("Passthrough target connection logic not implemented yet"))
    }

    async fn handle_json_rpc_loop<R, W>(
        &self,
        mut reader: BufReader<R>,
        writer: &mut W,
        resolved: &ResolvedRoute,
        plan: &RoutingPlan,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send,
    {
        loop {
            let frame = framing::read_frame(&mut reader).await?;
            if frame.is_empty() {
                break;
            }

            match self.dispatch_json_rpc_once(resolved, plan, &frame).await {
                Ok(payload) => {
                    framing::write_frame(writer, &payload).await?;
                }
                Err(e) => {
                    error!("JSON-RPC dispatch error: {}", e);
                    let error_payload = JsonRpcConverter::json_error(None, -32603, e.to_string())?;
                    framing::write_frame(writer, &error_payload).await?;
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// HttpHandler — wraps RouteHandler for use as a hyper service.
//
// We use a raw pointer + explicit lifetime contract rather than Arc<RouteHandler>
// to avoid requiring RouteHandler: Send + Sync on the outer type (it already
// satisfies this in practice via DashMap, but the pointer lets us stay in the
// existing ownership model without cloning the handler per-connection).
// ---------------------------------------------------------------------------

struct HttpHandler {
    /// Raw pointer to the owning `RouteHandler`.  Valid for the lifetime of
    /// `handle_http_stream`, which drives the `serve_connection` future to
    /// completion before returning.  Never stored beyond that scope.
    route_handler: *const RouteHandler,
    /// The preamble that initiated this stream.
    preamble: RoutePreamble,
}

// SAFETY: HttpHandler is only ever used within handle_http_stream, which holds
// &self (and thus the RouteHandler) alive for the full duration.
unsafe impl Send for HttpHandler {}
unsafe impl Sync for HttpHandler {}

/// Newtype that makes `Arc<HttpHandler>` usable in async move closures passed
/// to hyper's `service_fn`.
struct HttpHandlerSafe(Arc<HttpHandler>);

// SAFETY: Same guarantee as HttpHandler above.
unsafe impl Send for HttpHandlerSafe {}
unsafe impl Sync for HttpHandlerSafe {}

impl HttpHandler {
    async fn handle_http_request(
        &self,
        req: Request<Incoming>,
    ) -> std::result::Result<Response<Full<Bytes>>, std::convert::Infallible> {
        // All errors are mapped to HTTP error responses; hyper must not see Err.
        let response = self.try_handle_http_request(req).await.unwrap_or_else(|e| {
            error!("HTTP JSON-RPC handler error: {e}");
            http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        });
        Ok(response)
    }

    async fn try_handle_http_request(
        &self,
        req: Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>> {
        // Safety: pointer is valid for the lifetime of handle_http_stream.
        let route_handler = unsafe { &*self.route_handler };

        // Validate method.
        if req.method() != hyper::Method::POST {
            return Ok(http_error(StatusCode::METHOD_NOT_ALLOWED, "Only POST is supported".into()));
        }

        // Validate Content-Type.
        let content_type = req
            .headers()
            .get(hyper::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !content_type.starts_with("application/json") {
            return Ok(http_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Content-Type must be application/json".into(),
            ));
        }

        // The stream-level preamble is the definitive source of routing truth.
        // We no longer validate against the HTTP path, allowing this router to be placed behind
        // transparent proxies that don't rewrite URLs.

        // ALWAYS use the stream-level preamble for routing decisions.
        let resolved = match route_handler.resolve_route(self.preamble.clone()) {
            Ok(r) => r,
            Err(e) => {
                return Ok(http_error(StatusCode::NOT_FOUND, e.to_string()));
            }
        };
        let plan = route_handler.plan_route(&resolved);
        log_route(&resolved, &plan);

        // Collect body.
        let body_bytes =
            req.collect().await.map_err(|e| anyhow!("Failed to read HTTP body: {e}"))?.to_bytes();

        if body_bytes.is_empty() {
            return Ok(http_error(StatusCode::BAD_REQUEST, "Empty request body".into()));
        }

        // Dispatch.
        match route_handler.dispatch_json_rpc_once(&resolved, &plan, &body_bytes).await {
            Ok(payload) => Ok(Response::builder()
                .status(StatusCode::OK)
                .header(hyper::header::CONTENT_TYPE, "application/json")
                .body(Full::new(Bytes::from(payload)))
                .expect("valid response")),
            Err(e) => Ok(http_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        }
    }
}

fn http_error(status: StatusCode, message: String) -> Response<Full<Bytes>> {
    // Return a JSON-RPC-style error body so callers get a parseable error.
    let body =
        format!(r#"{{"jsonrpc":"2.0","error":{{"code":-32603,"message":{message:?}}},"id":null}}"#);
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(body)))
        .expect("valid error response")
}

// ---------------------------------------------------------------------------
// Iroh protocol handler
// ---------------------------------------------------------------------------

impl IrohProtocolHandler for RouteHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let endpoint_id = connection.remote_id();
        debug!("accepted connection from {endpoint_id}");

        // There could be multiple streams on the same connection
        loop {
            match connection.accept_bi().await {
                Ok((send, recv)) => {
                    let iroh_stream = IrohStream::new(send, recv);
                    if let Err(e) = self.handle_stream(iroh_stream).await {
                        error!("Error handling Iroh stream: {}", e);
                    }
                    debug!("handled stream");
                }
                Err(e) => {
                    debug!("Connection {endpoint_id} closed or error: {e}");
                    break;
                }
            }
        }

        connection.closed().await;
        debug!("connection closed");

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn read_preamble<R>(reader: &mut BufReader<R>) -> Result<RoutePreamble>
where
    R: AsyncRead + Unpin,
{
    let mut raw_preamble = String::new();
    let read = reader.read_line(&mut raw_preamble).await?;
    if read == 0 {
        return Err(anyhow!("Stream closed before reading preamble"));
    }

    RoutePreamble::parse(&raw_preamble)
}

fn log_route(route: &ResolvedRoute, plan: &RoutingPlan) {
    debug!(
        protocol = %route.request.protocol,
        interface = route.request.interface.as_str(),
        service_id = route.request.service_id.as_str(),
        delivery_mode = ?plan.delivery_mode,
        execution = ?plan.execution,
        endpoint = ?route.endpoint,
        "router planned stream handling"
    );
}

fn adapter_not_implemented_message(adapter: ProtocolAdapter) -> &'static str {
    match adapter {
        ProtocolAdapter::JsonRpcToWrpc => {
            "JSON-RPC to wRPC component bridging is not implemented yet"
        }
        ProtocolAdapter::JsonRpcToPodman => {
            "JSON-RPC to Podman backend bridging is not implemented yet"
        }
    }
}

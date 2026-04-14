use anyhow::{Result, anyhow};
use dashmap::DashMap;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler as IrohProtocolHandler};
use std::fmt;
use std::sync::Arc;
use syneroym_control_plane::ControlPlaneService;
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::{EndpointRegistry, SubstrateEndpoint};
use syneroym_rpc::{JsonRpcConverter, JsonRpcRequest, JsonRpcResponse, NativeService};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tracing::{debug, error, warn};

use crate::net_iroh::IrohStream;
use crate::preamble::{RoutePreamble, RouteProtocol};
use crate::routing::{DeliveryMode, ProtocolAdapter, ResolvedRoute, RouteExecution, RoutingPlan};

use syneroym_app_sandbox::AppSandboxEngine;

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

    pub async fn handle_stream<S>(&self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (read_half, mut write_half) = tokio::io::split(stream);
        let mut reader = BufReader::new(read_half);

        let resolved_route = self.resolve_route(read_preamble(&mut reader).await?)?;
        let routing_plan = self.plan_route(&resolved_route);
        log_route(&resolved_route, &routing_plan);

        match &routing_plan.execution {
            RouteExecution::NativeJsonRpc { channel_id } => {
                self.handle_json_to_native(
                    reader,
                    &mut write_half,
                    &resolved_route.request.interface,
                    channel_id,
                )
                .await?;
            }
            RouteExecution::ExecuteWasm { channel_id } => {
                self.handle_json_to_wasm(
                    reader,
                    &mut write_half,
                    resolved_route.request.interface,
                    channel_id,
                )
                .await?;
                debug!("ExecuteWasm done");
            }
            RouteExecution::WasmWrpcPassthrough { channel_id } => {
                debug!("Passthrough wRPC stream to Wasm channel: {}", channel_id);
                self.handle_passthrough(reader, &mut write_half, channel_id).await?;
            }
            RouteExecution::Adapted { adapter } => {
                let message = adapter_not_implemented_message(*adapter);
                let payload = JsonRpcConverter::json_error(None, -32601, message)?;
                write_half.write_all(&payload).await?;
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

        Ok(())
    }

    fn resolve_route(&self, preamble: RoutePreamble) -> Result<ResolvedRoute> {
        let endpoint =
            self.registry.lookup(&preamble.service_id, &preamble.interface).ok_or_else(|| {
                anyhow!("Service {} not found in local registry", preamble.service_id)
            })?;

        Ok(ResolvedRoute { request: preamble, endpoint })
    }

    fn plan_route(&self, route: &ResolvedRoute) -> RoutingPlan {
        // Keep this mapping intentionally direct and easy to revise.
        // The current plan categories are only a readable description of the
        // request handling paths we know about today; they are not intended as
        // a final statement on how routing must work forever.
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

    async fn handle_json_to_wasm<R, W>(
        &self,
        mut reader: BufReader<R>,
        writer: &mut W,
        interface: String,
        channel_id: &str,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send,
    {
        loop {
            let mut frame = Vec::new();
            let read = reader.read_until(b'\n', &mut frame).await?;
            if read == 0 {
                break;
            }

            while frame.last() == Some(&b'\n') || frame.last() == Some(&b'\r') {
                frame.pop();
            }
            if frame.is_empty() {
                continue;
            }

            let request: JsonRpcRequest = match serde_json::from_slice(&frame) {
                Ok(parsed) => parsed,
                Err(error) => {
                    let payload = JsonRpcConverter::json_error(None, -32700, error.to_string())?;
                    writer.write_all(&payload).await?;
                    continue;
                }
            };

            match self.app_sandbox_engine.execute_wasm(channel_id, &interface, &request).await {
                Ok(wasm_result) => {
                    let json_response = JsonRpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: serde_json::Value::String(wasm_result),
                        id: request.id.clone(),
                    };
                    let mut payload = serde_json::to_vec(&json_response)?;
                    payload.push(b'\n');
                    debug!("writing wasm response");
                    writer.write_all(&payload).await?;
                    debug!("writing wasm response");
                }
                Err(e) => {
                    error!("WASM execution error: {}", e);
                    let error_payload =
                        JsonRpcConverter::json_error(request.id.clone(), -32603, e.to_string())?;
                    writer.write_all(&error_payload).await?;
                }
            }
        }
        Ok(())
    }

    async fn handle_json_to_native<R, W>(
        &self,
        mut reader: BufReader<R>,
        writer: &mut W,
        interface: &str,
        channel_id: &str,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send,
    {
        debug!(">>> Entered handle_json_to_native for channel: {}", channel_id);
        let service = self
            .native_dispatch
            .get(channel_id)
            .map(|s| s.clone())
            .ok_or_else(|| anyhow!("Native service not found for {}", channel_id))?;

        loop {
            let mut frame = Vec::new();
            debug!(">>> Waiting to read frame...");
            let read = reader.read_until(b'\n', &mut frame).await?;
            debug!(">>> Read frame of {} bytes", read);
            if read == 0 {
                debug!(">>> Reached EOF");
                break;
            }

            while frame.last() == Some(&b'\n') || frame.last() == Some(&b'\r') {
                frame.pop();
            }
            if frame.is_empty() {
                continue;
            }

            debug!(">>> Parsing JSON frame...");
            let (request, invocation) = match JsonRpcConverter::json_to_native(interface, &frame) {
                Ok(parsed) => parsed,
                Err(error) => {
                    debug!(">>> JSON parse error: {}", error);
                    let payload = JsonRpcConverter::json_error(None, -32700, error.to_string())?;
                    writer.write_all(&payload).await?;
                    continue;
                }
            };
            debug!(">>> Dispatched to native service...");

            match service.dispatch(invocation).await {
                Ok(native_response) => {
                    debug!(">>> Native service succeeded");
                    let json_response =
                        JsonRpcConverter::native_to_json(&request, native_response)?;
                    writer.write_all(&json_response).await?;
                }
                Err(e) => {
                    error!("Native service error: {}", e);
                    let error_payload =
                        JsonRpcConverter::json_error(request.id.clone(), -32603, e.to_string())?;
                    writer.write_all(&error_payload).await?;
                }
            }
        }

        Ok(())
    }
}

impl IrohProtocolHandler for RouteHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let endpoint_id = connection.remote_id();
        debug!("accepted connection from {endpoint_id}");

        let (send, recv) = connection.accept_bi().await?;

        let iroh_stream = IrohStream::new(send, recv);
        if let Err(e) = self.handle_stream(iroh_stream).await {
            error!("Error handling Iroh stream: {}", e);
        }
        debug!("handled stream");

        connection.closed().await;
        debug!("connection closed");

        Ok(())
    }
}

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

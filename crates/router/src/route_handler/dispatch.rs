use super::RouteHandler;
use crate::preamble::RouteProtocol;
use crate::routing::{DeliveryMode, ProtocolAdapter, ResolvedRoute, RouteExecution, RoutingPlan};
use anyhow::{Result, anyhow};
use std::sync::Arc;
use syneroym_core::registry::SubstrateEndpoint;
use syneroym_rpc::framing;
use syneroym_rpc::{JsonRpcConverter, JsonRpcRequest, JsonRpcResponse, NativeService};
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tracing::{debug, error};

impl RouteHandler {
    /// Looks up a native service by its channel ID.
    fn native_service(&self, channel_id: &str) -> Option<Arc<dyn NativeService>> {
        self.inner.native_dispatch.get(channel_id).as_deref().cloned()
    }

    /// Dispatches a single JSON-RPC request based on the provided routing plan.
    ///
    /// This handles Native, Wasm, and Adapted execution modes.
    pub async fn dispatch_json_rpc_once(
        &self,
        resolved: &ResolvedRoute,
        plan: &RoutingPlan,
        body: &[u8],
    ) -> Result<Vec<u8>> {
        match &plan.execution {
            RouteExecution::NativeJsonRpc { channel_id } => {
                let service = self
                    .native_service(channel_id)
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
                        JsonRpcConverter::json_error(request.id.clone(), e.code(), e.to_string())
                    }
                }
            }
            RouteExecution::ExecuteWasm { channel_id } => {
                let request: JsonRpcRequest =
                    serde_json::from_slice(body).map_err(|e| anyhow!("JSON parse error: {e}"))?;

                match self
                    .inner
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

    /// Resolves a `RoutePreamble` to a `ResolvedRoute` by looking up the service in the registry.
    pub fn resolve_route(
        &self,
        mut preamble: crate::preamble::RoutePreamble,
    ) -> Result<ResolvedRoute> {
        let (endpoint, canonical_interface) = self
            .inner
            .registry
            .lookup(&preamble.service_id, &preamble.interface)
            .ok_or_else(|| anyhow!("Interface '{}' not found", preamble.interface))?;

        // Normalize the interface to the canonical name (full name, not short hash)
        preamble.interface = canonical_interface;

        Ok(ResolvedRoute { request: preamble, endpoint })
    }

    /// Creates a `RoutingPlan` for a `ResolvedRoute` based on the protocol and endpoint type.
    pub fn plan_route(&self, route: &ResolvedRoute) -> RoutingPlan {
        match (&route.request.protocol, &route.endpoint) {
            (
                RouteProtocol::JsonRpc,
                SubstrateEndpoint::NativeHostChannel { service_id: channel_id },
            ) => RoutingPlan {
                delivery_mode: DeliveryMode::Broker,
                execution: RouteExecution::NativeJsonRpc { channel_id: channel_id.clone() },
            },
            (RouteProtocol::Wrpc, SubstrateEndpoint::WasmChannel { service_id: channel_id }) => {
                RoutingPlan {
                    delivery_mode: DeliveryMode::PassThrough,
                    execution: RouteExecution::WasmWrpcPassthrough {
                        channel_id: channel_id.clone(),
                    },
                }
            }
            (RouteProtocol::JsonRpc, SubstrateEndpoint::WasmChannel { service_id: channel_id }) => {
                RoutingPlan {
                    delivery_mode: DeliveryMode::Broker,
                    execution: RouteExecution::ExecuteWasm { channel_id: channel_id.clone() },
                }
            }
            (RouteProtocol::JsonRpc, SubstrateEndpoint::PodmanSocket { .. }) => RoutingPlan {
                delivery_mode: DeliveryMode::Adapt,
                execution: RouteExecution::Adapted { adapter: ProtocolAdapter::JsonRpcToPodman },
            },
            _ => RoutingPlan::unsupported(),
        }
    }

    /// Handles a passthrough stream (e.g., wRPC) to a target channel.
    ///
    /// # Note
    /// Full wRPC stream passthrough is not yet implemented. Callers should treat
    /// `WasmWrpcPassthrough` as an unsupported path until this is filled in.
    #[allow(dead_code)]
    pub async fn handle_passthrough<R, W>(
        &self,
        _client_read: R,
        _client_write: &mut W,
        _channel_id: &str,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send,
    {
        // TODO: implement wRPC stream proxying once the transport layer matures.
        Err(anyhow!("Passthrough target connection logic not implemented yet"))
    }

    /// Runs a loop that reads JSON-RPC frames from the reader and dispatches them.
    ///
    /// This is used for binary streams where multiple requests can be sent sequentially.
    pub async fn handle_json_rpc_loop<R, W>(
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

/// Logs information about the resolved route and the chosen routing plan.
pub fn log_route(route: &ResolvedRoute, plan: &RoutingPlan) {
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

pub fn adapter_not_implemented_message(adapter: ProtocolAdapter) -> &'static str {
    match adapter {
        ProtocolAdapter::JsonRpcToWrpc => {
            "JSON-RPC to wRPC component bridging is not implemented yet"
        }
        ProtocolAdapter::JsonRpcToPodman => {
            "JSON-RPC to Podman backend bridging is not implemented yet"
        }
    }
}

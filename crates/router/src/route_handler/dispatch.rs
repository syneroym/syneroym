//! Local endpoint stream dispatcher
//!
//! Hooks active streams up to their target local services (e.g. WASM sandbox
//! input, native services, or TCP socket).

use std::{sync::Arc, time::Instant};

use anyhow::{Result, anyhow};
use syneroym_core::local_registry::SubstrateEndpoint;
use syneroym_rpc::{JsonRpcConverter, JsonRpcRequest, JsonRpcResponse, NativeService, framing};
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tracing::{debug, error};

use super::RouteHandler;
use crate::{
    preamble::{RoutePreamble, RouteProtocol, RouteTransport},
    routing::{AdaptationStage, RoutePipeline, ServiceStage},
};

impl RouteHandler {
    /// Looks up a native service by its channel ID.
    fn native_service(&self, channel_id: &str) -> Option<Arc<dyn NativeService>> {
        self.inner.native_dispatch.get(channel_id).as_deref().cloned()
    }

    /// Dispatches a single JSON-RPC request based on the provided routing
    /// pipeline.
    ///
    /// This handles Native, Wasm, and Adapted execution modes.
    pub async fn dispatch_json_rpc_once(
        &self,
        pipeline: &RoutePipeline,
        preamble: &RoutePreamble,
        body: &[u8],
    ) -> Result<Vec<u8>> {
        let start = Instant::now();
        metrics::counter!("substrate.request.total").increment(1);

        let result = match (&pipeline.adaptation, &pipeline.service) {
            (AdaptationStage::None, ServiceStage::NativeService { service_id }) => {
                let service = self
                    .native_service(service_id)
                    .ok_or_else(|| anyhow!("Native service not found for {service_id}"))?;

                let (request, invocation) =
                    JsonRpcConverter::json_to_native(&preamble.interface, body)
                        .map_err(|e| anyhow!("JSON parse error: {e}"))?;

                match service.dispatch(invocation).await {
                    Ok(native_response) => {
                        JsonRpcConverter::native_to_json(&request, native_response)
                    }
                    Err(e) => {
                        error!("Native service error: {}", e);
                        metrics::counter!("substrate.request.errors").increment(1);
                        JsonRpcConverter::json_error(request.id.clone(), e.code(), e.to_string())
                    }
                }
            }
            (AdaptationStage::JsonRpcToWasm, ServiceStage::WasmComponent { service_id }) => {
                let request: JsonRpcRequest =
                    serde_json::from_slice(body).map_err(|e| anyhow!("JSON parse error: {e}"))?;

                if let Some(app_sandbox_engine) = &self.inner.app_sandbox_engine {
                    match app_sandbox_engine
                        .execute_wasm(service_id, &preamble.interface, &request)
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
                            metrics::counter!("substrate.request.errors").increment(1);
                            JsonRpcConverter::json_error(request.id.clone(), -32603, e.to_string())
                        }
                    }
                } else {
                    let message = "App sandbox engine not available in coordinator mode";
                    metrics::counter!("substrate.request.errors").increment(1);
                    JsonRpcConverter::json_error(request.id.clone(), -32603, message.to_string())
                }
            }
            (AdaptationStage::JsonRpcToWrpc, ServiceStage::WasmComponent { .. }) => {
                let message = "JSON-RPC to wRPC component bridging is not implemented yet";
                metrics::counter!("substrate.request.errors").increment(1);
                JsonRpcConverter::json_error(None, -32601, message.to_string())
            }
            _ => {
                metrics::counter!("substrate.request.errors").increment(1);
                Err(anyhow!(
                    "Execution stage {:?} with adaptation {:?} not supported in request-response \
                     mode",
                    pipeline.service,
                    pipeline.adaptation
                ))
            }
        };

        metrics::histogram!("substrate.request.duration_ms")
            .record(start.elapsed().as_secs_f64() * 1000.0);
        result
    }

    /// Creates a `RoutePipeline` for a given preamble and endpoint.
    #[must_use]
    pub fn plan_pipeline(
        &self,
        preamble: &RoutePreamble,
        endpoint: &SubstrateEndpoint,
    ) -> RoutePipeline {
        use crate::routing::{EncryptionStage, TransportStage};

        let encryption = match preamble.enc.as_deref() {
            Some("ecdh-p256") => EncryptionStage::TerminateEcdhP256,
            _ => EncryptionStage::None,
        };

        let (adaptation, service) = match (&preamble.protocol, endpoint) {
            (RouteProtocol::JsonRpc, SubstrateEndpoint::NativeHostChannel { service_id }) => (
                AdaptationStage::None,
                ServiceStage::NativeService { service_id: service_id.clone() },
            ),
            (RouteProtocol::Wrpc, SubstrateEndpoint::WasmChannel { service_id }) => (
                AdaptationStage::None, // NOTE: wRPC not yet implemented, might need adaptation
                ServiceStage::WasmComponent { service_id: service_id.clone() },
            ),
            (RouteProtocol::JsonRpc, SubstrateEndpoint::WasmChannel { service_id }) => (
                AdaptationStage::JsonRpcToWasm,
                ServiceStage::WasmComponent { service_id: service_id.clone() },
            ),
            (_, SubstrateEndpoint::TcpHostPort { host, port }) => {
                (AdaptationStage::None, ServiceStage::TcpProxy { host: host.clone(), port: *port })
            }
            _ => (AdaptationStage::None, ServiceStage::Unsupported),
        };

        let mut transport = match preamble.transport {
            RouteTransport::Http => TransportStage::Http,
            RouteTransport::Binary => TransportStage::Binary,
            RouteTransport::Raw => TransportStage::Raw,
        };

        // Passthrough services (like TcpProxy or direct WasmComponent passthrough (wRPC
        // — TODO: not yet implemented)) do not perform substrate-level wire
        // framing; they bypass transport decoding and stream raw bytes
        // directly.
        if let ServiceStage::TcpProxy { .. } = &service {
            transport = TransportStage::Raw;
        } else if let (RouteProtocol::Wrpc, ServiceStage::WasmComponent { .. }) =
            (&preamble.protocol, &service)
        {
            // NOTE: wRPC is not yet implemented, so this block is more of a placeholder for
            // future logic.
            transport = TransportStage::Raw;
        }

        RoutePipeline { encryption, transport, adaptation, service }
    }

    /// Runs a loop that reads JSON-RPC frames from the reader and dispatches
    /// them.
    ///
    /// This is used for binary streams where multiple requests can be sent
    /// sequentially.
    pub async fn handle_json_rpc_loop<R, W>(
        &self,
        mut reader: BufReader<R>,
        writer: &mut W,
        preamble: &RoutePreamble,
        pipeline: &RoutePipeline,
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

            match self.dispatch_json_rpc_once(pipeline, preamble, &frame).await {
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

/// Logs information about the planned route pipeline.
pub fn log_pipeline(
    preamble: &RoutePreamble,
    pipeline: &RoutePipeline,
    endpoint: &SubstrateEndpoint,
) {
    debug!(
        protocol = %preamble.protocol,
        interface = preamble.interface.as_str(),
        service_id = preamble.service_id.as_str(),
        encryption = ?pipeline.encryption,
        transport = ?pipeline.transport,
        adaptation = ?pipeline.adaptation,
        service = ?pipeline.service,
        endpoint = ?endpoint,
        "router planned stream handling"
    );
}

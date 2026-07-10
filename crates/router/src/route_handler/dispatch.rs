//! Local endpoint stream dispatcher
//!
//! Hooks active streams up to their target local services (e.g. WASM sandbox
//! input, native services, or TCP socket).

use std::{sync::Arc, time::Instant};

use anyhow::{Result, anyhow};
use serde_json::Value;
use syneroym_core::local_registry::SubstrateEndpoint;
use syneroym_mqtt_broker::namespace_topic;
use syneroym_rpc::{JsonRpcConverter, JsonRpcRequest, JsonRpcResponse, NativeService, framing};
use tokio::{
    io::{AsyncRead, AsyncWrite, BufReader},
    sync::oneshot,
};
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
                                result: Value::String(wasm_result),
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

    /// Entry point for `TransportStage::Binary` streams. Reads the first
    /// frame itself (rather than re-implementing frame reading inside
    /// `handle_json_rpc_loop` too) so it can special-case a
    /// `messaging/subscribe` request -- the one native-capability method
    /// that needs to hold the writer open across multiple pushed
    /// notifications instead of returning after a single response
    /// (`NativeService::dispatch`'s one-request-one-response shape can't
    /// express this; see ADR-0010 Finding A2). Every other method falls
    /// through into the unchanged generic per-frame loop.
    pub async fn handle_binary_stream<R, W>(
        &self,
        mut reader: BufReader<R>,
        mut writer: W,
        preamble: &RoutePreamble,
        pipeline: &RoutePipeline,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let frame = framing::read_frame(&mut reader).await?;
        if frame.is_empty() {
            return Ok(());
        }

        if preamble.interface == "messaging"
            && let ServiceStage::NativeService { service_id } = &pipeline.service
            && let Ok(request) = serde_json::from_slice::<JsonRpcRequest>(&frame)
            && request.method == "subscribe"
        {
            #[derive(serde::Deserialize)]
            struct SubscribeParams {
                topic: String,
            }
            let params: SubscribeParams = serde_json::from_value(request.params)
                .map_err(|e| anyhow!("messaging/subscribe: invalid params: {e}"))?;
            return self
                .handle_messaging_subscribe(
                    reader,
                    writer,
                    service_id.clone(),
                    params.topic,
                    request.id,
                )
                .await;
        }

        match self.dispatch_json_rpc_once(pipeline, preamble, &frame).await {
            Ok(payload) => framing::write_frame(&mut writer, &payload).await?,
            Err(e) => {
                error!("JSON-RPC dispatch error: {}", e);
                let error_payload = JsonRpcConverter::json_error(None, -32603, e.to_string())?;
                framing::write_frame(&mut writer, &error_payload).await?;
            }
        }

        self.handle_json_rpc_loop(reader, &mut writer, preamble, pipeline).await
    }

    /// Holds `writer` open across the connection's lifetime, forwarding
    /// every message the broker delivers as a `messaging/message`
    /// notification frame (`id: null`). A native subscriber has no
    /// separate stream to send an explicit `unsubscribe` request on, so
    /// client-initiated stream closure is treated as the unsubscribe
    /// signal -- detected on its own task (rather than raced per-loop-
    /// iteration against `receiver.recv()`) so a partially-read frame can
    /// never be silently dropped mid-cancellation.
    async fn handle_messaging_subscribe<R, W>(
        &self,
        mut reader: BufReader<R>,
        mut writer: W,
        service_id: String,
        topic: String,
        request_id: Option<Value>,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let namespaced = namespace_topic(&service_id, &topic);
        let (handle, mut receiver) = self
            .inner
            .messaging_broker
            .subscribe(&namespaced)
            .await
            .map_err(|e| anyhow!("broker subscribe failed: {e}"))?;

        let ack = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Value::String("subscribed".to_string()),
            id: request_id,
        };
        framing::write_frame(&mut writer, &serde_json::to_vec(&ack)?).await?;

        let (closed_tx, mut closed_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                match framing::read_frame(&mut reader).await {
                    Ok(f) if f.is_empty() => break,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
            let _ = closed_tx.send(());
        });

        loop {
            tokio::select! {
                notification = receiver.recv() => {
                    let Some((notify_topic, payload)) = notification else { break };
                    let notify = JsonRpcRequest {
                        jsonrpc: "2.0".to_string(),
                        method: "messaging/message".to_string(),
                        params: serde_json::json!({"topic": notify_topic, "payload": payload}),
                        id: None,
                    };
                    let Ok(bytes) = serde_json::to_vec(&notify) else { continue };
                    if framing::write_frame(&mut writer, &bytes).await.is_err() {
                        break;
                    }
                }
                _ = &mut closed_rx => break,
            }
        }

        drop(handle);
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

//! Local endpoint stream dispatcher
//!
//! Hooks active streams up to their target local services (e.g. WASM sandbox
//! input, native services, or TCP socket).

use std::{future::Future, pin::Pin, sync::Arc, time::Instant};

use anyhow::{Result, anyhow};
use serde_json::Value;
use syneroym_core::local_registry::SubstrateEndpoint;
use syneroym_mqtt_broker::namespace_topic;
use syneroym_rpc::{
    CallerContext, JsonRpcConverter, JsonRpcRequest, JsonRpcResponse, MESSAGING_MESSAGE_METHOD,
    MessagingNotification, NativeService, framing,
};
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

/// A native-capability `(interface, method)` pair that needs to hold a
/// binary stream's writer open across multiple pushed frames instead of
/// returning after a single response (ADR-0010 Finding A2) -- declared
/// here as a single reusable lookup rather than an ad-hoc condition, so
/// Slice 6B's `stream-cursor`/`stream-sink` methods (`task.md` lines
/// 472-663) add a variant + [`Self::lookup`] entry + match arm in
/// `handle_binary_stream`, not another hand-rolled "read the first frame,
/// check interface/method, dispatch" special-case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LongLivedStreamMethod {
    MessagingSubscribe,
}

impl LongLivedStreamMethod {
    const REGISTRY: &'static [(&'static str, &'static str, Self)] =
        &[("messaging", "subscribe", Self::MessagingSubscribe)];

    fn lookup(interface: &str, method: &str) -> Option<Self> {
        Self::REGISTRY
            .iter()
            .find(|(i, m, _)| *i == interface && *m == method)
            .map(|(_, _, kind)| *kind)
    }
}

impl RouteHandler {
    /// Looks up a native service by its channel ID.
    fn native_service(&self, channel_id: &str) -> Option<Arc<dyn NativeService>> {
        self.inner.native_dispatch.get(channel_id).as_deref().cloned()
    }

    /// Dispatches a single JSON-RPC request based on the provided routing
    /// pipeline.
    ///
    /// This handles Native, Wasm, and Adapted execution modes.
    ///
    /// `caller` is `None` for a connection with no verifiable identity
    /// (ADR-0016 §3). The Native-service arm below rejects it before
    /// dispatch -- every other arm here never reaches a native capability,
    /// so it's unused there.
    pub async fn dispatch_json_rpc_once(
        &self,
        pipeline: &RoutePipeline,
        preamble: &RoutePreamble,
        caller: Option<&CallerContext>,
        body: &[u8],
    ) -> Result<Vec<u8>> {
        let start = Instant::now();
        metrics::counter!("substrate.request.total").increment(1);

        let result = match (&pipeline.adaptation, &pipeline.service) {
            (AdaptationStage::None, ServiceStage::NativeService { service_id }) => {
                let service = self
                    .native_service(service_id)
                    .ok_or_else(|| anyhow!("Native service not found for {service_id}"))?;

                // TODO(M04B/FDAE): B0 gate only proves *an* identity is
                // present. Which callers may actually reach this native
                // service (service-owner / substrate-owner) and with what
                // row/column scope is enforced by the FDAE policy engine
                // (M04B), evaluated against `caller.session`. Until then any
                // verified identity passes.
                let caller = caller.cloned().ok_or_else(|| {
                    anyhow!("unauthenticated caller for native interface '{}'", preamble.interface)
                })?;

                let (request, invocation) =
                    JsonRpcConverter::json_to_native(&preamble.interface, caller, body)
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
            // M3B Slice 6B stream-protocol requests (ADR-0014): without
            // this arm, `raw://<protocol>|<service_id>` against a
            // `WasmChannel` endpoint falls through to `Unsupported` --
            // `wrpc://` is the only other route that reaches
            // `WasmComponent` under `Raw` transport, and it force-overrides
            // transport separately below.
            (RouteProtocol::Raw, SubstrateEndpoint::WasmChannel { service_id }) => (
                AdaptationStage::None,
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

    /// Dispatches one JSON-RPC frame and writes the response -- or a
    /// JSON-RPC error frame, on dispatch failure -- back to `writer`.
    /// Shared by `handle_binary_stream`'s first frame and
    /// `handle_json_rpc_loop`'s per-frame body.
    async fn dispatch_and_write_frame<W>(
        &self,
        writer: &mut W,
        pipeline: &RoutePipeline,
        preamble: &RoutePreamble,
        caller: Option<&CallerContext>,
        frame: &[u8],
    ) -> Result<()>
    where
        W: AsyncWrite + Unpin + Send,
    {
        match self.dispatch_json_rpc_once(pipeline, preamble, caller, frame).await {
            Ok(payload) => framing::write_frame(writer, &payload).await?,
            Err(e) => {
                error!("JSON-RPC dispatch error: {}", e);
                let error_payload = JsonRpcConverter::json_error(None, -32603, e.to_string())?;
                framing::write_frame(writer, &error_payload).await?;
            }
        }
        Ok(())
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
        caller: Option<&CallerContext>,
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
            self.dispatch_and_write_frame(writer, pipeline, preamble, caller, &frame).await?;
        }
        Ok(())
    }

    /// Entry point for `TransportStage::Binary` streams. Reads the first
    /// frame itself (rather than re-implementing frame reading inside
    /// `handle_json_rpc_loop` too) so it can special-case the small set of
    /// native-capability methods in [`LongLivedStreamMethod`] that need
    /// to hold the writer open across multiple pushed frames instead of
    /// returning after a single response (`NativeService::dispatch`'s
    /// one-request-one-response shape can't express this; see ADR-0010
    /// Finding A2). Every other method falls through into the unchanged
    /// generic per-frame loop.
    pub async fn handle_binary_stream<R, W>(
        &self,
        mut reader: BufReader<R>,
        mut writer: W,
        preamble: &RoutePreamble,
        pipeline: &RoutePipeline,
        caller: Option<CallerContext>,
        stop_signal: Pin<Box<dyn Future<Output = ()> + Send>>,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let frame = framing::read_frame(&mut reader).await?;
        if frame.is_empty() {
            return Ok(());
        }

        if let ServiceStage::NativeService { service_id } = &pipeline.service
            && let Ok(request) = serde_json::from_slice::<JsonRpcRequest>(&frame)
            && let Some(method) =
                LongLivedStreamMethod::lookup(&preamble.interface, &request.method)
        {
            return match method {
                LongLivedStreamMethod::MessagingSubscribe => {
                    // `messaging/subscribe` is a native-capability call
                    // reached outside `dispatch_json_rpc_once` (it never
                    // returns a single response), so it needs its own
                    // `None`-caller gate to close the enforcement gap
                    // uniformly (ADR-0016 §3).
                    if caller.is_none() {
                        let error_payload = JsonRpcConverter::json_error(
                            request.id.clone(),
                            -32603,
                            "unauthenticated caller for native interface 'messaging'",
                        )?;
                        framing::write_frame(&mut writer, &error_payload).await?;
                        return Ok(());
                    }
                    #[derive(serde::Deserialize)]
                    struct SubscribeParams {
                        topic: String,
                    }
                    let params: SubscribeParams = serde_json::from_value(request.params)
                        .map_err(|e| anyhow!("messaging/subscribe: invalid params: {e}"))?;
                    self.handle_messaging_subscribe(
                        reader,
                        writer,
                        service_id.clone(),
                        params.topic,
                        request.id,
                        stop_signal,
                    )
                    .await
                }
            };
        }

        self.dispatch_and_write_frame(&mut writer, pipeline, preamble, caller.as_ref(), &frame)
            .await?;
        self.handle_json_rpc_loop(reader, &mut writer, preamble, pipeline, caller.as_ref()).await
    }

    /// Holds `writer` open across the connection's lifetime, forwarding
    /// every message the broker delivers as a `messaging/message`
    /// notification frame (`id: null`). A native subscriber has no
    /// separate stream to send an explicit `unsubscribe` request on, so
    /// either client-initiated stream closure (detected on its own task,
    /// rather than raced per-loop-iteration against `receiver.recv()`, so
    /// a partially-read frame can never be silently dropped mid-
    /// cancellation) or the transport signaling it no longer wants data
    /// pushed to it (`stop_signal`, e.g. QUIC `STOP_SENDING` -- see
    /// `crate::stop_signal`) is treated as the unsubscribe signal.
    async fn handle_messaging_subscribe<R, W>(
        &self,
        mut reader: BufReader<R>,
        mut writer: W,
        service_id: String,
        topic: String,
        request_id: Option<Value>,
        mut stop_signal: Pin<Box<dyn Future<Output = ()> + Send>>,
    ) -> Result<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let namespaced = namespace_topic(&service_id, &topic);
        let (handle, mut receiver) = self
            .inner
            .messaging_broker
            .subscribe(namespaced)
            .await
            .map_err(|e| anyhow!("broker subscribe failed: {e}"))?;

        let ack = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Value::String("subscribed".to_string()),
            id: request_id,
        };
        framing::write_frame(&mut writer, &serde_json::to_vec(&ack)?).await?;

        let (closed_tx, mut closed_rx) = oneshot::channel::<()>();
        let reader_task = tokio::spawn(async move {
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
                    let Some((topic, payload)) = notification else { break };
                    let Ok(params) = serde_json::to_value(MessagingNotification { topic, payload })
                    else {
                        continue;
                    };
                    let notify = JsonRpcRequest {
                        jsonrpc: "2.0".to_string(),
                        method: MESSAGING_MESSAGE_METHOD.to_string(),
                        params,
                        id: None,
                    };
                    let Ok(bytes) = serde_json::to_vec(&notify) else { continue };
                    if framing::write_frame(&mut writer, &bytes).await.is_err() {
                        break;
                    }
                }
                _ = &mut closed_rx => break,
                () = &mut stop_signal => break,
            }
        }

        reader_task.abort();
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

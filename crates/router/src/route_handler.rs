use anyhow::{Result, anyhow};
use dashmap::DashMap;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler as IrohProtocolHandler};
use std::fmt;
use std::sync::Arc;
use syneroym_control_plane::SubstrateService;
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::{EndpointRegistry, SubstrateEndpoint};
use syneroym_rpc::{JsonRpcConverter, NativeService};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tracing::{debug, error};

use crate::net_iroh::IrohStream;
use crate::preamble::RoutePreamble;

pub(crate) struct RouteHandler {
    registry: EndpointRegistry,
    native_dispatch: DashMap<String, Arc<dyn NativeService>>,
}

impl fmt::Debug for RouteHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionRouter")
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
        let s = Self { registry: registry.clone(), native_dispatch: DashMap::new() };

        let substrate_service =
            SubstrateService::init(service_id.clone(), config, registry).await?;
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

        let mut preamble = String::new();
        let read = reader.read_line(&mut preamble).await?;
        if read == 0 {
            return Err(anyhow!("Stream closed before reading preamble"));
        }
        let preamble = RoutePreamble::parse(&preamble)?;

        let endpoint = self.registry.lookup(&preamble.service_id).ok_or_else(|| {
            anyhow!("Service {} not found in local registry", preamble.service_id)
        })?;

        tracing::info!(
            "Router handling stream: protocol={} interface={} service_id={}",
            preamble.protocol,
            preamble.interface,
            preamble.service_id
        );

        match (preamble.protocol.as_str(), endpoint) {
            ("json-rpc", SubstrateEndpoint::NativeHostChannel { channel_id }) => {
                self.handle_json_to_native(
                    reader,
                    &mut write_half,
                    &preamble.interface,
                    &channel_id,
                )
                .await?;
            }
            ("wrpc", SubstrateEndpoint::WasmChannel { channel_id }) => {
                tracing::info!("Passthrough wRPC stream to Wasm channel: {}", channel_id);
                self.handle_passthrough(reader, &mut write_half, &channel_id).await?;
            }
            ("json-rpc", SubstrateEndpoint::WasmChannel { channel_id }) => {
                tracing::info!("Protocol conversion stream to Wasm channel: {}", channel_id);
                let payload = JsonRpcConverter::json_error(
                    None,
                    -32601,
                    "JSON-RPC to wRPC component bridging is not implemented yet",
                )?;
                write_half.write_all(&payload).await?;
            }
            ("json-rpc", SubstrateEndpoint::PodmanSocket { socket_path }) => {
                tracing::info!("Routing to Podman socket: {}", socket_path);
                let payload = JsonRpcConverter::json_error(
                    None,
                    -32601,
                    "JSON-RPC to Podman backend bridging is not implemented yet",
                )?;
                write_half.write_all(&payload).await?;
            }
            (proto, endpoint) => {
                tracing::warn!("Unsupported routing combination: {} to {:?}", proto, endpoint);
            }
        }

        Ok(())
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
        let service = self
            .native_dispatch
            .get(channel_id)
            .map(|s| s.clone())
            .ok_or_else(|| anyhow!("Native service not found for {}", channel_id))?;

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

            let (request, invocation) = match JsonRpcConverter::json_to_native(interface, &frame) {
                Ok(parsed) => parsed,
                Err(error) => {
                    let payload = JsonRpcConverter::json_error(None, -32700, error.to_string())?;
                    writer.write_all(&payload).await?;
                    continue;
                }
            };

            match service.dispatch(invocation).await {
                Ok(native_response) => {
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

        connection.closed().await;

        Ok(())
    }
}

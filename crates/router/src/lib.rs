pub mod net_iroh;
pub mod substrate_service;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use dashmap::DashMap;
use iroh::endpoint::{Connection, presets};
use iroh::protocol::{AcceptError, ProtocolHandler as IrohProtocolHandler, Router as IrohRouter};
use iroh::{RelayMap, RelayMode, RelayUrl, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;
use syneroym_core::config::IrohRelayConfig;
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::{EndpointRegistry, SubstrateEndpoint};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use tracing::debug;
use tracing::error;
use tracing::info;

use crate::net_iroh::IrohStream;

pub const SYNEROYM_ALPN: &[u8] = b"syneroym/0.1";

/// The Connection Router (The Data Plane)
/// Internal traffic cop that uses the Endpoint Registry to look up
/// the destination for an incoming wRPC stream.
#[derive(Debug, Clone)]
pub struct ConnectionRouter {
    iroh_router: Option<IrohRouter>,
}

impl ConnectionRouter {
    pub async fn init(
        registry: EndpointRegistry,
        config: SubstrateConfig,
        iroh_secret_key: [u8; 32],
        service_id: String,
    ) -> Result<Self> {
        let mut router = Self { iroh_router: None };

        for comm in &config.substrate.communication_interfaces {
            match comm.as_str() {
                "iroh" => {
                    if let Some(iroh_config) = config.uplink.iroh.as_ref() {
                        tracing::info!("Initializing Iroh interface for Router...");
                        let iroh_router = router
                            .init_iroh(
                                iroh_config,
                                iroh::SecretKey::from_bytes(&iroh_secret_key),
                                RouteHandler::init(service_id.clone(), &config, registry.clone()),
                            )
                            .await?;
                        router.iroh_router = Some(iroh_router);
                    }
                }
                "webrtc" => {
                    tracing::info!(
                        "WebRTC interface initialization not yet implemented in Router."
                    );
                    // net_webrtc::init(config, self.clone()).await?;
                }
                _ => {
                    tracing::info!("Unknown or unimplemented communication interface: {}", comm);
                }
            }
        }

        Ok(router)
    }

    async fn init_iroh(
        &self,
        config: &IrohRelayConfig,
        secret_key: SecretKey,
        route_handler: RouteHandler,
    ) -> Result<IrohRouter> {
        debug!("Initializing Iroh communication...");

        // Bind endpoint
        let mut ep_bldr = iroh::Endpoint::builder(presets::N0);
        // If a relay URL is provided in the config, use it. Otherwise, the default from presets::N0 will be used.
        if let Ok(relay_url) = config.relay_url.parse::<RelayUrl>() {
            ep_bldr = iroh::Endpoint::empty_builder()
                .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
        }

        let ep_bldr = ep_bldr.secret_key(secret_key);
        let ep = ep_bldr.bind().await?;

        let iroh_router: IrohRouter =
            IrohRouter::builder(ep).accept(SYNEROYM_ALPN, route_handler).spawn();

        info!("Iroh listening on ALPN: {:?}", std::str::from_utf8(SYNEROYM_ALPN).unwrap());

        Ok(iroh_router)
    }

    pub async fn run(&self) -> Result<()> {
        info!("running connection router");
        let endpoint = self.iroh_router.as_ref().map(|router| router.endpoint());
        if let Some(endpoint) = endpoint {
            endpoint.closed().await;
        } else {
            // If iroh is not configured, router has nothing to do and can pend forever.
            std::future::pending::<()>().await;
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        info!("shutting down connection router");
        if let Some(router) = self.iroh_router.as_ref() {
            router.shutdown().await?;
        }
        Ok(())
    }
}

//
pub struct RouteHandler {
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
    fn init(service_id: String, config: &SubstrateConfig, registry: EndpointRegistry) -> Self {
        let s = Self { registry: registry.clone(), native_dispatch: DashMap::new() };

        let substrate_service = Arc::new(substrate_service::SubstrateService::new(
            service_id.clone(),
            config,
            registry,
        ));
        s.register_native_service(service_id, substrate_service);
        s
    }

    /// Register a channel for a local native service
    fn register_native_service(&self, service_id: String, service: Arc<dyn NativeService>) {
        self.native_dispatch.insert(service_id, service);
    }

    /// Accept a new bidirectional stream, read the preamble, and establish
    /// the appropriate framing, protocol conversion, and routing loop.
    /// Preamble format example: `json-rpc://<interface>.<service_id>`
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
                let payload = ProtocolConverter::json_error(
                    None,
                    -32601,
                    "wRPC passthrough backend is not implemented yet",
                )?;
                write_half.write_all(&payload).await?;
            }
            ("json-rpc", SubstrateEndpoint::WasmChannel { channel_id }) => {
                tracing::info!("Protocol conversion stream to Wasm channel: {}", channel_id);
                let payload = ProtocolConverter::json_error(
                    None,
                    -32601,
                    "JSON-RPC to wRPC component bridging is not implemented yet",
                )?;
                write_half.write_all(&payload).await?;
            }
            ("json-rpc", SubstrateEndpoint::PodmanSocket { socket_path }) => {
                tracing::info!("Routing to Podman socket: {}", socket_path);
                let payload = ProtocolConverter::json_error(
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

    /// Continuously identifies frames and builds the incoming JSON-RPC struct from bytes.
    /// Converts that to wRPC, calls the native service method, takes the output wRPC WIT,
    /// converts it back to JSON-RPC, and writes it back to the router bidirectional stream.
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

            let (request, invocation) = match ProtocolConverter::json_to_native(interface, &frame) {
                Ok(parsed) => parsed,
                Err(error) => {
                    let payload = ProtocolConverter::json_error(None, -32700, error.to_string())?;
                    writer.write_all(&payload).await?;
                    continue;
                }
            };

            match service.dispatch(invocation).await {
                Ok(native_response) => {
                    let json_response =
                        ProtocolConverter::native_to_json(&request, native_response)?;
                    writer.write_all(&json_response).await?;
                }
                Err(e) => {
                    tracing::error!("Native service error: {}", e);
                    let error_payload =
                        ProtocolConverter::json_error(request.id.clone(), -32603, e.to_string())?;
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

        // We expect the connecting peer to open a single bi-directional stream.
        let (send, recv) = connection.accept_bi().await?;

        let iroh_stream = IrohStream::new(send, recv);
        if let Err(e) = self.handle_stream(iroh_stream).await {
            error!("Error handling Iroh stream: {}", e);
        }

        // Wait until the remote closes the connection, which it does once it
        // received the response.
        connection.closed().await;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePreamble {
    pub protocol: String,
    pub interface: String,
    pub service_id: String,
}

impl RoutePreamble {
    pub fn parse(raw: &str) -> Result<Self> {
        let (protocol, target) = raw
            .trim()
            .split_once("://")
            .ok_or_else(|| anyhow!("Invalid preamble format: {raw}"))?;
        let (interface, service_id) = target
            .split_once('.')
            .ok_or_else(|| anyhow!("Invalid preamble target format: {target}"))?;

        if protocol.is_empty() || interface.is_empty() || service_id.is_empty() {
            return Err(anyhow!("Incomplete preamble: {raw}"));
        }

        Ok(Self {
            protocol: protocol.to_string(),
            interface: interface.to_string(),
            service_id: service_id.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub id: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    #[serde(default)]
    pub id: Option<Value>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self { jsonrpc: "2.0", result: Some(result), error: None, id }
    }

    pub fn error(id: Option<Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            result: None,
            error: Some(JsonRpcError { code, message: message.into() }),
            id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NativeInvocation {
    pub interface: String,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct NativeResponse {
    pub result: Value,
}

/// The Protocol Converter
/// Acts as the protocol adapter from framed bytes to typed substrate calls.
#[derive(Debug, Clone)]
pub struct ProtocolConverter {}

impl ProtocolConverter {
    pub fn json_to_native(
        interface: &str,
        frame: &[u8],
    ) -> Result<(JsonRpcRequest, NativeInvocation)> {
        let request: JsonRpcRequest = serde_json::from_slice(frame)?;
        if request.jsonrpc != "2.0" {
            return Err(anyhow!("Unsupported JSON-RPC version: {}", request.jsonrpc));
        }

        let method = request
            .method
            .rsplit_once('.')
            .map(|(_, method)| method)
            .unwrap_or(request.method.as_str())
            .to_string();

        Ok((
            request.clone(),
            NativeInvocation {
                interface: interface.to_string(),
                method,
                params: request.params.clone(),
            },
        ))
    }

    pub fn native_to_json(request: &JsonRpcRequest, response: NativeResponse) -> Result<Vec<u8>> {
        let payload = JsonRpcResponse::success(request.id.clone(), response.result);
        let mut encoded = serde_json::to_vec(&payload)?;
        encoded.push(b'\n');
        Ok(encoded)
    }

    pub fn json_error(id: Option<Value>, code: i64, message: impl Into<String>) -> Result<Vec<u8>> {
        let payload = JsonRpcResponse::error(id, code, message);
        let mut encoded = serde_json::to_vec(&payload)?;
        encoded.push(b'\n');
        Ok(encoded)
    }
}

#[async_trait]
pub trait NativeService: Send + Sync {
    async fn dispatch(&self, invocation: NativeInvocation) -> Result<NativeResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_route_preamble() {
        let parsed = RoutePreamble::parse("json-rpc://health.substrate-123\n").unwrap();
        assert_eq!(parsed.protocol, "json-rpc");
        assert_eq!(parsed.interface, "health");
        assert_eq!(parsed.service_id, "substrate-123");
    }

    #[test]
    fn converts_json_rpc_into_native_invocation() {
        let frame = br#"{"jsonrpc":"2.0","id":1,"method":"health.ping"}"#;
        let (_, invocation) = ProtocolConverter::json_to_native("health", frame).unwrap();
        assert_eq!(invocation.interface, "health");
        assert_eq!(invocation.method, "ping");
        assert!(invocation.params.is_none());
    }
}

pub mod net_iroh;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use dashmap::DashMap;
use iroh::protocol::Router;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::{EndpointRegistry, SubstrateEndpoint};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::info;

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

impl Default for ProtocolConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolConverter {
    pub fn new() -> Self {
        Self {}
    }

    pub fn json_to_native(
        &self,
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

    pub fn native_to_json(
        &self,
        request: &JsonRpcRequest,
        response: NativeResponse,
    ) -> Result<Vec<u8>> {
        let payload = JsonRpcResponse::success(request.id.clone(), response.result);
        let mut encoded = serde_json::to_vec(&payload)?;
        encoded.push(b'\n');
        Ok(encoded)
    }

    pub fn json_error(
        &self,
        id: Option<Value>,
        code: i64,
        message: impl Into<String>,
    ) -> Result<Vec<u8>> {
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
/// The Connection Router (The Data Plane)
/// Internal traffic cop that uses the Endpoint Registry to look up
/// the destination for an incoming wRPC stream.
pub struct ConnectionRouter {
    config: SubstrateConfig,
    registry: Arc<EndpointRegistry>,
    protocol_converter: Arc<ProtocolConverter>,
    native_dispatch: DashMap<String, Arc<dyn NativeService>>,
    // Hold reference to the running Iroh router to keep it alive.
    iroh_router: Mutex<Option<Router>>,
    // The secret key for Iroh communication.
    iroh_secret_key: [u8; 32],
}

impl fmt::Debug for ConnectionRouter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionRouter")
            .field("config", &self.config)
            .field("registry", &self.registry)
            .field("protocol_converter", &self.protocol_converter)
            .field("native_dispatch_len", &self.native_dispatch.len())
            .finish_non_exhaustive()
    }
}

impl ConnectionRouter {
    pub async fn init(
        registry: Arc<EndpointRegistry>,
        config: SubstrateConfig,
        iroh_secret_key: [u8; 32],
    ) -> Result<Arc<Self>> {
        let router = Arc::new(Self {
            config: config.clone(),
            registry,
            protocol_converter: Arc::new(ProtocolConverter::new()),
            native_dispatch: DashMap::new(),
            iroh_router: Mutex::new(None),
            iroh_secret_key,
        });

        for comm in &config.substrate.communication_interfaces {
            match comm.as_str() {
                "iroh" => {
                    if let Some(iroh_config) = config.uplink.iroh.as_ref() {
                        tracing::info!("Initializing Iroh interface for Router...");
                        let iroh_secret_key = iroh::SecretKey::from_bytes(&router.iroh_secret_key);
                        if let Some(iroh_router) =
                            net_iroh::init(iroh_config, iroh_secret_key, router.clone()).await?
                        {
                            let mut lock = router.iroh_router.lock().await;
                            *lock = Some(iroh_router);
                        }
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

    pub async fn run(self: Arc<Self>) -> Result<()> {
        info!("running connection router");
        let endpoint = {
            let router = self.iroh_router.lock().await;
            router.as_ref().map(|router| router.endpoint().clone())
        };
        if let Some(endpoint) = endpoint {
            endpoint.closed().await;
        } else {
            // If iroh is not configured, router has nothing to do and can pend forever.
            std::future::pending::<()>().await;
        }
        Ok(())
    }

    /// Register a channel for a local native service
    pub fn register_native_service(&self, service_id: String, service: Arc<dyn NativeService>) {
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
                let payload = self.protocol_converter.json_error(
                    None,
                    -32601,
                    "wRPC passthrough backend is not implemented yet",
                )?;
                write_half.write_all(&payload).await?;
            }
            ("json-rpc", SubstrateEndpoint::WasmChannel { channel_id }) => {
                tracing::info!("Protocol conversion stream to Wasm channel: {}", channel_id);
                let payload = self.protocol_converter.json_error(
                    None,
                    -32601,
                    "JSON-RPC to wRPC component bridging is not implemented yet",
                )?;
                write_half.write_all(&payload).await?;
            }
            ("json-rpc", SubstrateEndpoint::PodmanSocket { socket_path }) => {
                tracing::info!("Routing to Podman socket: {}", socket_path);
                let payload = self.protocol_converter.json_error(
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

        let converter = self.protocol_converter.clone();
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

            let (request, invocation) = match converter.json_to_native(interface, &frame) {
                Ok(parsed) => parsed,
                Err(error) => {
                    let payload = converter.json_error(None, -32700, error.to_string())?;
                    writer.write_all(&payload).await?;
                    continue;
                }
            };

            match service.dispatch(invocation).await {
                Ok(native_response) => {
                    let json_response = converter.native_to_json(&request, native_response)?;
                    writer.write_all(&json_response).await?;
                }
                Err(e) => {
                    tracing::error!("Native service error: {}", e);
                    let error_payload =
                        converter.json_error(request.id.clone(), -32603, e.to_string())?;
                    writer.write_all(&error_payload).await?;
                }
            }
        }

        Ok(())
    }
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
        let converter = ProtocolConverter::new();
        let frame = br#"{"jsonrpc":"2.0","id":1,"method":"health.ping"}"#;
        let (_, invocation) = converter.json_to_native("health", frame).unwrap();
        assert_eq!(invocation.interface, "health");
        assert_eq!(invocation.method, "ping");
        assert!(invocation.params.is_none());
    }
}

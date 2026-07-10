#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Syneroym App SDK
//!
//! High-level APIs and traits to help third-party developers build apps
//! that integrate seamlessly with the Syneroym runtime and services.

use std::{
    fmt::{self, Debug, Formatter},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use iroh::{
    Endpoint, EndpointAddr, RelayMap, RelayMode, RelayUrl,
    endpoint::{Connection, SendStream},
};
pub mod mapper;
use serde_json::Value;
use syneroym_core::dht_registry::{EndpointMechanism, RegistryClient, SignedEndpointInfo};
use syneroym_router::{RoutePreamble, RouteProtocol, RouteTransport, SYNEROYM_ALPN};
use syneroym_rpc::{JsonRpcRequest, JsonRpcResponse, framing};
pub use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, ContainerManifest, ContainerPortMapping, ContainerVolumeMapping,
    DeployManifest, DeploymentPlan, NetworkEndpoint, PlannedService, ServiceConfig, ServiceType,
    TcpManifest, WasmManifest,
};
use tokio::{io, net::TcpStream, sync::mpsc, time};
use tracing::debug;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeployedService {
    pub service_id: String,
    pub interfaces: Vec<String>,
    pub endpoint_type: String,
}

pub struct SyneroymClient {
    service_id: String,
    registry_url: String,
    provided_mechanisms: Option<Vec<EndpointMechanism>>,
    connection: Option<TransportConnection>,
}

impl Debug for SyneroymClient {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyneroymClient")
            .field("service_id", &self.service_id)
            .field("registry_url", &self.registry_url)
            .field("provided_mechanisms", &self.provided_mechanisms)
            .field("connection", &self.connection)
            .finish()
    }
}

#[derive(Clone)]
pub enum TransportConnection {
    Iroh {
        /// The endpoint must be kept alive for the duration of the connection.
        /// Dropping it closes the underlying QUIC socket, terminating all
        /// streams.
        endpoint: Endpoint,
        conn: Connection,
    },
}

impl Debug for TransportConnection {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Iroh { conn, .. } => f
                .debug_struct("TransportConnection::Iroh")
                .field("endpoint", &"iroh::Endpoint")
                .field("conn", &format!("{:?}", conn.remote_id()))
                .finish(),
        }
    }
}

/// A live `messaging/subscribe` stream: `.recv()` yields `(topic, payload)`
/// pairs as the broker delivers them. Dropping it drops the send half of
/// the underlying bidirectional stream, which the router-side handler
/// observes as the client having gone away (close-as-unsubscribe).
pub struct MessageStream {
    receiver: mpsc::Receiver<(String, Vec<u8>)>,
    send: SendStream,
}

impl Debug for MessageStream {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageStream").finish_non_exhaustive()
    }
}

impl MessageStream {
    pub async fn recv(&mut self) -> Option<(String, Vec<u8>)> {
        self.receiver.recv().await
    }

    /// Closes the send half only (without dropping `self`), signalling
    /// the router-side handler to unsubscribe (close-as-unsubscribe).
    /// `.recv()` remains usable afterward and resolves to `None` once the
    /// router's own writer task exits in response and closes its side.
    pub fn stop(&mut self) -> Result<()> {
        self.send.finish().map_err(Into::into)
    }
}

impl SyneroymClient {
    #[must_use]
    pub const fn new(service_id: String, registry_url: String) -> Self {
        Self { service_id, registry_url, provided_mechanisms: None, connection: None }
    }

    #[must_use]
    pub const fn new_with_mechanisms(
        service_id: String,
        mechanisms: Vec<EndpointMechanism>,
    ) -> Self {
        Self {
            service_id,
            registry_url: String::new(),
            provided_mechanisms: Some(mechanisms),
            connection: None,
        }
    }

    pub async fn connect(&mut self) -> Result<()> {
        if self.connection.is_some() {
            return Ok(());
        }

        debug!("Connecting to {} via registry or provided mechanisms", self.service_id);

        let mechanisms = if let Some(m) = &self.provided_mechanisms {
            m.clone()
        } else if !self.registry_url.is_empty() {
            let registry_client = RegistryClient::new(true, Some(self.registry_url.clone()));
            let info = registry_client.lookup(&self.service_id, true).await?.info;
            // The lookup might have been done by an alias. Update service_id to the
            // canonical DID.
            self.service_id = info.service_id;
            info.mechanisms
        } else {
            return Err(anyhow::anyhow!("No registry URL or mechanisms provided"));
        };

        self.connect_with_mechanisms(mechanisms).await
    }

    pub async fn connect_with_mechanisms(
        &mut self,
        mechanisms: Vec<EndpointMechanism>,
    ) -> Result<()> {
        // Try mechanisms. Currently only Iroh is implemented.
        for mechanism in mechanisms {
            match mechanism {
                EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url } => {
                    let mut endpoint_addr: EndpointAddr =
                        serde_json::from_slice(&endpoint_addr_bytes)?;

                    let mut ep_bldr = Endpoint::empty_builder();
                    if let Some(relay) = relay_url
                        && let Ok(url) = relay.parse::<RelayUrl>()
                    {
                        ep_bldr =
                            ep_bldr.relay_mode(RelayMode::Custom(RelayMap::from(url.clone())));
                        endpoint_addr = endpoint_addr.with_relay_url(url);
                    }

                    let endpoint = ep_bldr.bind().await?;
                    match endpoint.connect(endpoint_addr, SYNEROYM_ALPN).await {
                        Ok(conn) => {
                            self.connection = Some(TransportConnection::Iroh { endpoint, conn });
                            return Ok(());
                        }
                        Err(e) => {
                            endpoint.close().await;
                            return Err(e.into());
                        }
                    }
                }
                EndpointMechanism::WebRtc { .. } => {
                    // Not implemented
                }
            }
        }

        Err(anyhow::anyhow!("No supported communication mechanism found for {}", self.service_id))
    }

    pub async fn lookup(&self) -> Result<SignedEndpointInfo> {
        self.lookup_registry().await
    }

    pub async fn wait_for_discovery(&mut self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.lookup_registry().await.is_ok() {
                return Ok(());
            }
            time::sleep(Duration::from_millis(500)).await;
        }
        Err(anyhow::anyhow!("Timed out waiting for {} to be discovered", self.service_id))
    }

    pub async fn wait_for_ready(&mut self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            match self.connect().await {
                Ok(()) => {
                    // Check if readyz is ok
                    match self.request("orchestrator", "readyz", serde_json::json!({})).await {
                        Ok(res) if res.result == serde_json::json!({"status": "ok"}) => {
                            return Ok(());
                        }
                        Ok(_) => debug!("Substrate not ready yet (readyz != ok)"),
                        Err(e) => debug!("readyz request failed: {}", e),
                    }
                }
                Err(e) => {
                    debug!("Connect attempt failed, retrying: {}", e);
                }
            }
            time::sleep(Duration::from_millis(500)).await;
        }
        Err(anyhow::anyhow!("Timed out waiting for {} to become ready", self.service_id))
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(TransportConnection::Iroh { endpoint, .. }) = self.connection.take() {
            endpoint.close().await;
        }
        Ok(())
    }

    #[must_use]
    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    #[must_use]
    pub fn connection(&self) -> Option<TransportConnection> {
        self.connection.clone()
    }

    pub async fn request(
        &self,
        interface: &str,
        method: &str,
        params: Value,
    ) -> Result<JsonRpcResponse> {
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(Value::Number(1.into())),
        };
        self.request_raw(interface, request).await
    }

    pub async fn request_raw(
        &self,
        interface_name: &str,
        request: JsonRpcRequest,
    ) -> Result<JsonRpcResponse> {
        let conn_wrapper = self.connection.as_ref().context("Not connected")?;
        match conn_wrapper {
            TransportConnection::Iroh { conn, .. } => {
                let (mut send, mut recv) = conn.open_bi().await?;

                // Every stream must start with a RoutePreamble identifying the target service.
                let preamble = RoutePreamble::binary_json_rpc(&self.service_id, interface_name);
                send.write_all(preamble.to_preamble_line().as_bytes()).await?;

                let req_bytes = serde_json::to_vec(&request)?;
                framing::write_frame(&mut send, &req_bytes).await?;
                debug!(">>> Wrote request for method: {} to {}", request.method, self.service_id);
                send.finish()?;

                let frame = framing::read_frame(&mut recv).await?;
                if frame.is_empty() {
                    return Err(anyhow::anyhow!(
                        "Empty response from stream for method {}",
                        request.method
                    ));
                }
                let res: JsonRpcResponse = serde_json::from_slice(&frame)?;
                debug!("got json response for method: {}: {:?}", request.method, res);
                Ok(res)
            }
        }
    }

    /// Subscribes to `topic` on `interface`'s messaging capability, over a
    /// live push channel. Unlike `request`/`request_raw`, this does **not**
    /// finish the send half of the stream after writing the request:
    /// finishing it would make the router-side reader hit EOF and tear the
    /// whole handler down before any notification arrives. Dropping the
    /// returned `MessageStream` closes the send half, which the router
    /// treats as the unsubscribe signal.
    pub async fn subscribe(&self, interface: &str, topic: &str) -> Result<MessageStream> {
        let conn_wrapper = self.connection.as_ref().context("Not connected")?;
        match conn_wrapper {
            TransportConnection::Iroh { conn, .. } => {
                let (mut send, mut recv) = conn.open_bi().await?;

                let preamble = RoutePreamble::binary_json_rpc(&self.service_id, interface);
                send.write_all(preamble.to_preamble_line().as_bytes()).await?;

                let request = JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    method: "subscribe".to_string(),
                    params: serde_json::json!({"topic": topic}),
                    id: Some(Value::Number(1.into())),
                };
                let req_bytes = serde_json::to_vec(&request)?;
                framing::write_frame(&mut send, &req_bytes).await?;

                let ack_frame = framing::read_frame(&mut recv).await?;
                if ack_frame.is_empty() {
                    return Err(anyhow::anyhow!(
                        "Empty ack for subscribe on topic {topic} (interface {interface})"
                    ));
                }
                let ack: JsonRpcResponse = serde_json::from_slice(&ack_frame)?;
                debug!("subscribe ack for topic {}: {:?}", topic, ack);

                let (tx, rx) = mpsc::channel(1024);
                tokio::spawn(async move {
                    loop {
                        match framing::read_frame(&mut recv).await {
                            Ok(frame) if frame.is_empty() => break,
                            Ok(frame) => {
                                let Ok(notify) = serde_json::from_slice::<JsonRpcRequest>(&frame)
                                else {
                                    continue;
                                };
                                if notify.method != "messaging/message" {
                                    continue;
                                }
                                let Some(topic) =
                                    notify.params.get("topic").and_then(|v| v.as_str())
                                else {
                                    continue;
                                };
                                let Some(payload) = notify.params.get("payload").and_then(|v| {
                                    serde_json::from_value::<Vec<u8>>(v.clone()).ok()
                                }) else {
                                    continue;
                                };
                                if tx.send((topic.to_string(), payload)).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                });

                Ok(MessageStream { receiver: rx, send })
            }
        }
    }

    pub async fn deploy_svc_wasm(
        &self,
        service_id: String,
        interfaces: Vec<String>,
        wasm_bytes: Vec<u8>,
        registry_certificate: Option<SignedEndpointInfo>,
    ) -> Result<()> {
        let registry_certificate = registry_certificate
            .map(|c| serde_json::to_string(&c))
            .transpose()
            .map_err(|e| anyhow::anyhow!("Failed to serialize registry certificate: {e}"))?;
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema_path: None,
                rotation_policy: None,
            },
            service_type: ServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wasm_bytes),
                hash: None,
                interfaces,
            }),
            registry_certificate,
        };
        let params = serde_json::to_value((service_id, manifest))?;
        let res = self.request("orchestrator", "deploy", params).await?;
        if res.result == serde_json::json!({"status": "deployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment failed: {:?}", res.result))
        }
    }

    pub async fn deploy_svc_tcp(
        &self,
        service_id: String,
        endpoints: Vec<NetworkEndpoint>,
        registry_certificate: Option<SignedEndpointInfo>,
    ) -> Result<()> {
        let registry_certificate = registry_certificate
            .map(|c| serde_json::to_string(&c))
            .transpose()
            .map_err(|e| anyhow::anyhow!("Failed to serialize registry certificate: {e}"))?;
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema_path: None,
                rotation_policy: None,
            },
            service_type: ServiceType::Tcp(TcpManifest { endpoints }),
            registry_certificate,
        };
        let params = serde_json::to_value((service_id, manifest))?;
        let res = self.request("orchestrator", "deploy", params).await?;
        if res.result == serde_json::json!({"status": "deployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment failed: {:?}", res.result))
        }
    }

    pub async fn deploy_container(
        &self,
        service_id: String,
        image: String,
        ports: Vec<ContainerPortMapping>,
        volumes: Vec<ContainerVolumeMapping>,
        registry_certificate: Option<SignedEndpointInfo>,
    ) -> Result<()> {
        let registry_certificate = registry_certificate
            .map(|c| serde_json::to_string(&c))
            .transpose()
            .map_err(|e| anyhow::anyhow!("Failed to serialize registry certificate: {e}"))?;
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema_path: None,
                rotation_policy: None,
            },
            service_type: ServiceType::Container(ContainerManifest {
                source: ArtifactSource::Binary(vec![]),
                hash: None,
                image,
                ports,
                volumes,
            }),
            registry_certificate,
        };
        let params = serde_json::to_value((service_id, manifest))?;
        let res = self.request("orchestrator", "deploy", params).await?;
        if res.result == serde_json::json!({"status": "deployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment failed: {:?}", res.result))
        }
    }

    pub async fn deploy_plan(&self, plan: DeploymentPlan) -> Result<()> {
        let params = serde_json::to_value((plan,))?;
        let res = self.request("orchestrator", "deploy-plan", params).await?;
        if res.result == serde_json::json!({"status": "deployed_plan"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Deployment of plan failed: {:?}", res.result))
        }
    }

    pub async fn undeploy(&self, service_id: String) -> Result<()> {
        let params = serde_json::to_value((service_id,))?;
        let res = self.request("orchestrator", "undeploy", params).await?;
        if res.result == serde_json::json!({"status": "undeployed"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Undeployment failed: {:?}", res.result))
        }
    }

    pub async fn list_svcs(&self) -> Result<Vec<DeployedService>> {
        let res = self.request("orchestrator", "list", serde_json::json!({})).await?;
        let services: Vec<DeployedService> = serde_json::from_value(res.result)?;
        Ok(services)
    }

    pub async fn passthrough(
        &self,
        interface_name: &str,
        initial_bytes: &[u8],
        tcp_stream: &mut TcpStream,
    ) -> Result<()> {
        let conn_wrapper = self.connection.as_ref().context("Not connected")?.clone();
        Self::passthrough_with_conn(
            conn_wrapper,
            &self.service_id,
            interface_name,
            initial_bytes,
            tcp_stream,
        )
        .await
    }

    pub async fn passthrough_with_conn(
        conn_wrapper: TransportConnection,
        service_id: &str,
        interface_name: &str,
        initial_bytes: &[u8],
        tcp_stream: &mut TcpStream,
    ) -> Result<()> {
        match conn_wrapper {
            TransportConnection::Iroh { conn, .. } => {
                let (mut send, recv) = conn.open_bi().await?;

                // Use HTTP transport for passthrough of raw requests.
                let preamble = RoutePreamble {
                    transport: RouteTransport::Http,
                    protocol: RouteProtocol::JsonRpc,
                    interface: interface_name.to_string(),
                    service_id: service_id.to_string(),
                    enc: None,
                    pubkey: None,
                    delegation: None,
                }
                .to_preamble_line();
                send.write_all(preamble.as_bytes()).await?;

                send.write_all(initial_bytes).await?;

                let mut joined_iroh = io::join(recv, send);
                if let Err(e) = io::copy_bidirectional(tcp_stream, &mut joined_iroh).await {
                    debug!("Bidirectional copy error between TCP and Iroh: {}", e);
                }

                Ok(())
            }
        }
    }

    pub async fn lookup_registry(&self) -> Result<SignedEndpointInfo> {
        let registry_client = RegistryClient::new(true, Some(self.registry_url.clone()));
        registry_client.lookup(&self.service_id, true).await
    }

    pub async fn inject_kek(&self, kek_hex: String) -> Result<()> {
        let params = serde_json::to_value((kek_hex,))?;
        let res = self.request("security", "inject-kek", params).await?;
        if res.result == serde_json::json!({"status": "injected"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("KEK injection failed: {:?}", res.result))
        }
    }

    pub async fn rotate_kek(&self, new_kek_hex: String) -> Result<()> {
        let params = serde_json::to_value((new_kek_hex,))?;
        let res = self.request("security", "rotate-kek", params).await?;
        if res.result == serde_json::json!({"status": "rotated"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("KEK rotation failed: {:?}", res.result))
        }
    }

    pub async fn set_secret(&self, service_id: String, key: String, value: Vec<u8>) -> Result<()> {
        let params = serde_json::to_value((service_id, key, value))?;
        let res = self.request("security", "set-secret", params).await?;
        if res.result == serde_json::json!({"status": "secret_set"}) {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Secret setting failed: {:?}", res.result))
        }
    }
}

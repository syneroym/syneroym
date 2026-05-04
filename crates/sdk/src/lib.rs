use anyhow::{Context, Result};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use std::time::Duration;
use syneroym_core::community_registry::{EndpointMechanism, SignedEndpointInfo};
use syneroym_rpc::JsonRpcResponse;
use tracing::debug;

pub struct SyneroymClient {
    service_id: String,
    registry_url: String,
    provided_mechanisms: Option<Vec<EndpointMechanism>>,
    connection: Option<TransportConnection>,
}

enum TransportConnection {
    Iroh { _endpoint: iroh::Endpoint, conn: Connection },
}

impl SyneroymClient {
    pub fn new(service_id: String, registry_url: String) -> Self {
        Self { service_id, registry_url, provided_mechanisms: None, connection: None }
    }

    pub fn new_with_mechanisms(service_id: String, mechanisms: Vec<EndpointMechanism>) -> Self {
        Self {
            service_id,
            registry_url: String::new(),
            provided_mechanisms: Some(mechanisms),
            connection: None,
        }
    }

    pub async fn connect(&mut self) -> Result<()> {
        debug!("Connecting to {} via registry or provided mechanisms", self.service_id);

        let mechanisms = if let Some(m) = &self.provided_mechanisms {
            m.clone()
        } else if !self.registry_url.is_empty() {
            self.lookup_registry().await?.info.mechanisms
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
                    let endpoint_addr: iroh::EndpointAddr =
                        serde_json::from_slice(&endpoint_addr_bytes)?;

                    let mut ep_bldr = iroh::Endpoint::empty_builder();
                    if let Some(relay) = relay_url
                        && let Ok(url) = relay.parse::<iroh::RelayUrl>()
                    {
                        ep_bldr =
                            ep_bldr.relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::from(url)));
                    }

                    let endpoint = ep_bldr.bind().await?;
                    let conn =
                        endpoint.connect(endpoint_addr, syneroym_router::SYNEROYM_ALPN).await?;

                    self.connection = Some(TransportConnection::Iroh { _endpoint: endpoint, conn });
                    return Ok(());
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
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if self.lookup_registry().await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err(anyhow::anyhow!("Timed out waiting for {} to be discovered", self.service_id))
    }

    pub async fn wait_for_ready(&mut self, timeout: Duration) -> Result<()> {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            match self.connect().await {
                Ok(_) => {
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
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Err(anyhow::anyhow!("Timed out waiting for {} to become ready", self.service_id))
    }

    pub async fn request(
        &mut self,
        interface: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<JsonRpcResponse> {
        let conn_wrapper = self.connection.as_mut().context("Not connected")?;
        match conn_wrapper {
            TransportConnection::Iroh { conn, .. } => {
                let (mut send, mut recv) = conn.open_bi().await?;
                self.send_json_rpc_request_over_stream(
                    interface, &mut send, &mut recv, method, params,
                )
                .await
            }
        }
    }

    async fn send_json_rpc_request_over_stream(
        &self,
        interface_name: &str,
        send: &mut SendStream,
        recv: &mut RecvStream,
        method: &str,
        params: serde_json::Value,
    ) -> Result<JsonRpcResponse> {
        // The preamble uses the interface name and the service_id this client was initialized with.
        let preamble = format!("json-rpc://{}.{}\n", interface_name, self.service_id);
        send.write_all(preamble.as_bytes()).await?;

        let request = syneroym_rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(serde_json::Value::Number(1.into())),
        };
        let req_bytes = serde_json::to_vec(&request)?;
        syneroym_rpc::framing::write_frame(send, &req_bytes).await?;
        debug!(">>> Wrote request for method: {} to {}", method, self.service_id);
        send.finish()?;

        let frame = syneroym_rpc::framing::read_frame(recv).await?;
        if frame.is_empty() {
            return Err(anyhow::anyhow!("Empty response from stream for method {}", method));
        }
        let res: JsonRpcResponse = serde_json::from_slice(&frame)?;
        debug!("got json response for method: {}: {:?}", method, res);
        Ok(res)
    }

    async fn lookup_registry(&self) -> Result<SignedEndpointInfo> {
        let client = reqwest::Client::new();
        // Use resolve=true to handle services hosted on substrates
        let url = format!("{}/lookup/{}?resolve=true", self.registry_url, self.service_id);
        let response = client.get(&url).send().await?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Registry lookup failed with status: {} for URL: {}",
                response.status(),
                url
            ));
        }
        let info = response.json::<SignedEndpointInfo>().await?;
        Ok(info)
    }
}

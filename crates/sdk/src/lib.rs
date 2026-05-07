use anyhow::{Context, Result};
use iroh::endpoint::Connection;
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
        if self.connection.is_some() {
            return Ok(());
        }

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
        let request = syneroym_rpc::JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: Some(serde_json::Value::Number(1.into())),
        };
        self.request_raw(interface, request).await
    }

    pub async fn request_raw(
        &mut self,
        interface_name: &str,
        request: syneroym_rpc::JsonRpcRequest,
    ) -> Result<JsonRpcResponse> {
        let conn_wrapper = self.connection.as_mut().context("Not connected")?;
        match conn_wrapper {
            TransportConnection::Iroh { conn, .. } => {
                let (mut send, mut recv) = conn.open_bi().await?;

                let preamble = if interface_name.is_empty() {
                    format!("json-rpc://{}\n", self.service_id)
                } else {
                    format!(
                        "json-rpc://{}{}{}\n",
                        interface_name,
                        syneroym_core::constants::PREAMBLE_SEPARATOR,
                        self.service_id
                    )
                };
                send.write_all(preamble.as_bytes()).await?;

                let req_bytes = serde_json::to_vec(&request)?;
                syneroym_rpc::framing::write_frame(&mut send, &req_bytes).await?;
                debug!(">>> Wrote request for method: {} to {}", request.method, self.service_id);
                send.finish()?;

                let frame = syneroym_rpc::framing::read_frame(&mut recv).await?;
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

    pub async fn passthrough(
        &mut self,
        interface_name: &str,
        initial_bytes: &[u8],
        tcp_stream: &mut tokio::net::TcpStream,
    ) -> Result<()> {
        let conn_wrapper = self.connection.as_mut().context("Not connected")?;
        match conn_wrapper {
            TransportConnection::Iroh { conn, .. } => {
                let (mut send, mut recv) = conn.open_bi().await?;

                let preamble = if interface_name.is_empty() {
                    format!("http://{}\n", self.service_id)
                } else {
                    format!(
                        "http://{}{}{}\n",
                        interface_name,
                        syneroym_core::constants::PREAMBLE_SEPARATOR,
                        self.service_id
                    )
                };
                send.write_all(preamble.as_bytes()).await?;

                send.write_all(initial_bytes).await?;

                let (mut tcp_read, mut tcp_write) = tcp_stream.split();

                let send_task = tokio::io::copy(&mut tcp_read, &mut send);
                let recv_task = tokio::io::copy(&mut recv, &mut tcp_write);

                tokio::select! {
                    res = send_task => {
                        if let Err(e) = res {
                            debug!("Error copying from TCP to Iroh: {}", e);
                        }
                    }
                    res = recv_task => {
                        if let Err(e) = res {
                            debug!("Error copying from Iroh to TCP: {}", e);
                        }
                    }
                }

                Ok(())
            }
        }
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

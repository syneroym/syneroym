use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;
use syneroym_core::config::SubstrateConfig;
use syneroym_sdk::SyneroymClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

struct GatewayState {
    registry_url: String,
    clients: DashMap<String, Arc<Mutex<SyneroymClient>>>,
}

/// ClientGateway: Acts as an entry point for local HTTP/WebSocket clients to reach the wider Syneroym network.
/// It accepts TCP traffic, reads the HTTP headers to extract the routing target from the `Host` header,
/// and streams the raw bytes over the Syneroym network.
pub struct ClientGateway {
    port: u16,
    state: Arc<GatewayState>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl ClientGateway {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("initializing client gateway");

        let port = config.roles.client_gateway.as_ref().map(|g| g.http_port).unwrap_or(7000);
        let registry_url = config.substrate.registry_url.clone().unwrap_or_default();

        let state = Arc::new(GatewayState { registry_url, clients: DashMap::new() });

        Ok(Self { port, state, shutdown_tx: None })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("running client gateway on port {}", self.port);
        let state = self.state.clone();

        let addr = format!("0.0.0.0:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;

        let (tx, mut rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(tx);

        loop {
            tokio::select! {
                Ok((stream, _)) = listener.accept() => {
                    let state_clone = state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, state_clone).await {
                            error!("Connection handling error: {}", e);
                        }
                    });
                }
                _ = &mut rx => {
                    break;
                }
            }
        }

        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("shutting down client gateway");
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    state: Arc<GatewayState>,
) -> Result<()> {
    let mut buf = [0u8; 4096];
    let mut bytes_read = 0;

    // Read enough to find the end of the HTTP headers
    loop {
        let n = stream.read(&mut buf[bytes_read..]).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("Connection closed before headers finished"));
        }
        bytes_read += n;
        tracing::debug!("gateway read {} bytes, total {}", n, bytes_read);

        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut headers);

        match req.parse(&buf[..bytes_read]) {
            Ok(httparse::Status::Complete(_header_len)) => {
                let host_header = req
                    .headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case("host"))
                    .map(|h| std::str::from_utf8(h.value).unwrap_or(""))
                    .unwrap_or("");

                let mut host = host_header;
                if let Some((h, p)) = host_header.rsplit_once(':')
                    && !p.is_empty()
                    && p.chars().all(|c| c.is_ascii_digit())
                {
                    host = h;
                }
                let host_base = host.strip_suffix(".localhost").unwrap_or(host);

                // We split by the last '--' because interface might have '-'
                let (interface, service_id) = match host_base.rsplit_once("--") {
                    Some((i, s)) => (i, s),
                    None => {
                        return write_bad_request(&mut stream, "Invalid Host header format").await;
                    }
                };

                let service_id = service_id.to_string();
                let interface = interface.to_string();

                debug!("Proxying to interface: {}, service_id: {}", interface, service_id);

                let client_arc = state
                    .clients
                    .entry(service_id.clone())
                    .or_insert_with(|| {
                        Arc::new(Mutex::new(SyneroymClient::new(
                            service_id.clone(),
                            state.registry_url.clone(),
                        )))
                    })
                    .clone();

                let mut client = client_arc.lock().await;

                if let Err(e) = client.connect().await {
                    error!("Gateway failed to connect to service {}: {}", service_id, e);
                    return write_bad_gateway(&mut stream).await;
                }

                // Call SDK passthrough logic
                client.passthrough(&interface, &buf[..bytes_read], &mut stream).await?;
                return Ok(());
            }
            Ok(httparse::Status::Partial) => {
                if bytes_read >= buf.len() {
                    return write_bad_request(&mut stream, "Headers too large").await;
                }
                continue;
            }
            Err(e) => {
                return write_bad_request(&mut stream, &format!("Invalid HTTP request: {}", e))
                    .await;
            }
        }
    }
}

async fn write_bad_request(stream: &mut tokio::net::TcpStream, msg: &str) -> Result<()> {
    let response =
        format!("HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\n\r\n{}", msg.len(), msg);
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

async fn write_bad_gateway(stream: &mut tokio::net::TcpStream) -> Result<()> {
    let msg = "Bad Gateway";
    let response =
        format!("HTTP/1.1 502 Bad Gateway\r\nContent-Length: {}\r\n\r\n{}", msg.len(), msg);
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

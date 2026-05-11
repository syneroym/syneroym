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

        // Shutdown all cached clients to close their Iroh endpoints gracefully
        for entry in self.state.clients.iter() {
            let mut client = entry.value().lock().await;
            let _ = client.shutdown().await;
        }

        Ok(())
    }
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    state: Arc<GatewayState>,
) -> Result<()> {
    // Limit header reads to 8 KB — the conventional maximum for HTTP/1.1 headers.
    // Requests with larger headers (e.g. very large JWTs) will receive a 400 response.
    const MAX_HEADER_BYTES: usize = 8 * 1024;
    let mut buf = [0u8; MAX_HEADER_BYTES];
    let mut bytes_read = 0;

    // Read enough to find the end of the HTTP headers
    loop {
        let n = stream.read(&mut buf[bytes_read..]).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("Connection closed before headers finished"));
        }
        bytes_read += n;
        debug!("gateway read {} bytes, total {}", n, bytes_read);

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

                // Parse host_base according to `<nickname>-p<pubkeyhash>-i<interfacehash>` or `<nickname>-p<pubkeyhash>`
                // Split by '-' and parse from the right to support nicknames with dashes
                let mut parts: Vec<&str> = host_base.split('-').collect();

                let mut interfacehash = None;
                if let Some(last) = parts.last()
                    && last.starts_with('i')
                    && last.len() > 1
                {
                    interfacehash = Some(&last[1..]);
                    parts.pop();
                }

                let mut pubkeyhash = None;
                if let Some(last) = parts.last()
                    && last.starts_with('p')
                    && last.len() > 1
                {
                    pubkeyhash = Some(&last[1..]);
                    parts.pop();
                }

                let nickname = if !parts.is_empty() { Some(parts.join("-")) } else { None };

                let lookup_alias = if let Some(n) = nickname {
                    format!("{n}-p{}", pubkeyhash.unwrap_or_default())
                } else {
                    format!("p{}", pubkeyhash.unwrap_or_default())
                };

                // The interface is now the short hash, fallback to "default" if omitted
                let interface = interfacehash.unwrap_or("default").to_string();
                let service_id = lookup_alias;

                debug!(
                    "Proxying to interface (hash): {}, service_id (alias): {}",
                    interface, service_id
                );

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

                let (conn, service_id) = {
                    let mut client = client_arc.lock().await;
                    if let Err(e) = client.connect().await {
                        error!("Gateway failed to connect to service {}: {}", service_id, e);
                        return write_json_rpc_error(&mut stream, 502, "Bad Gateway").await;
                    }
                    (
                        client.connection().ok_or_else(|| anyhow::anyhow!("Connection lost"))?,
                        client.service_id().to_string(),
                    )
                };

                SyneroymClient::passthrough_with_conn(
                    conn,
                    &service_id,
                    &interface,
                    &buf[..bytes_read],
                    &mut stream,
                )
                .await?;
                return Ok(());
            }
            Ok(httparse::Status::Partial) => {
                if bytes_read >= buf.len() {
                    return write_json_rpc_error(&mut stream, 400, "Headers too large").await;
                }
                continue;
            }
            Err(e) => {
                return write_json_rpc_error(
                    &mut stream,
                    400,
                    &format!("Invalid HTTP request: {}", e),
                )
                .await;
            }
        }
    }
}

/// Writes a JSON-RPC error response as an HTTP response.
async fn write_json_rpc_error(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    message: &str,
) -> Result<()> {
    let body =
        format!(r#"{{"jsonrpc":"2.0","error":{{"code":-32603,"message":{message:?}}},"id":null}}"#);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

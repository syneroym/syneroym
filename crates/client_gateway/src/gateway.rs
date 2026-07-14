//! HTTP Client Gateway
//!
//! Proxies external client requests into the internal Syneroym network,
//! managing routing, protocol translation, and error boundaries.

use std::{
    fmt::{self, Debug, Formatter},
    fs, str,
    sync::Arc,
};

use anyhow::Result;
use dashmap::DashMap;
use httparse::{EMPTY_HEADER, Request, Status};
use syneroym_core::config::{DEFAULT_SUBSTRATE_KEY_FILE, SubstrateConfig};
use syneroym_identity::Identity;
use syneroym_sdk::SyneroymClient;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, oneshot, oneshot::Sender},
};
use tracing::{debug, error, info};

/// Loads the node's own substrate identity, the same key file
/// `syneroym_substrate::identity::setup_substrate_identity` loads (by the
/// same path-resolution rule) -- generating and persisting it if this is
/// the first component to run (init order between the client gateway and
/// the connection router's own identity setup is not guaranteed, see
/// `RuntimeServices::init` vs. `setup_connection_router` in
/// `crates/substrate/src/runtime.rs`), so whichever runs first creates the
/// on-disk key and the other loads the same one back.
///
/// TODO(post-B0): present the substrate-owner (controller) DID as caller by
/// carrying an owner->node DelegationCertificate here (verify_preamble
/// already resolves master_did from it). Requires provisioning that
/// owner-signed delegation (none exists yet -- only ControllerAgreement).
/// B0 uses node DID.
fn load_or_generate_node_identity(config: &SubstrateConfig) -> Result<Identity> {
    let key_path = config
        .identity
        .key
        .clone()
        .unwrap_or_else(|| config.app_data_dir.join(DEFAULT_SUBSTRATE_KEY_FILE));

    if let Some(parent) = key_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if key_path.exists() {
        Identity::load_from_path(&key_path)
    } else {
        let id = Identity::generate()?;
        id.save_to_path(&key_path)?;
        Ok(id)
    }
}

#[derive(Debug)]
struct GatewayState {
    registry_url: String,
    clients: DashMap<String, Arc<Mutex<SyneroymClient>>>,
    /// The node's own identity (M04A Slice B0, ADR-0016 §0.5) -- presented
    /// as the caller DID for every proxied request. Reconstructed per
    /// downstream `SyneroymClient` from the same key bytes (`Identity` is
    /// deliberately not `Clone`), rather than shared as a single instance.
    identity: Identity,
}

/// `ClientGateway`: Acts as an entry point for local HTTP/WebSocket clients to
/// reach the wider Syneroym network.
///
/// It accepts TCP traffic, reads the HTTP headers to extract the routing target
/// from the `Host` header, and streams the raw bytes over the Syneroym network.
pub struct ClientGateway {
    port: u16,
    state: Arc<GatewayState>,
    shutdown_tx: Option<Sender<()>>,
}

impl Debug for ClientGateway {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientGateway")
            .field("port", &self.port)
            .field("state", &self.state)
            .field("shutdown_tx", &self.shutdown_tx.as_ref().map(|_| "oneshot::Sender"))
            .finish()
    }
}

impl ClientGateway {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("initializing client gateway");

        let port = config.roles.client_gateway.as_ref().map_or(7000, |g| g.http_port);
        let registry_url = config.substrate.registry_url.clone().unwrap_or_default();
        let identity = load_or_generate_node_identity(config)?;

        let state = Arc::new(GatewayState { registry_url, clients: DashMap::new(), identity });

        Ok(Self { port, state, shutdown_tx: None })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("running client gateway on port {}", self.port);
        let state = self.state.clone();

        // TODO: For now, basic security via access from local machine only instead of
        // 0.0.0.0 interface
        let addr = format!("127.0.0.1:{}", self.port);
        let listener = TcpListener::bind(&addr).await?;

        let (tx, mut rx) = oneshot::channel();
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
        for entry in &self.state.clients {
            let mut client = entry.value().lock().await;
            let _ = client.shutdown().await;
        }

        Ok(())
    }
}

async fn handle_connection(mut stream: TcpStream, state: Arc<GatewayState>) -> Result<()> {
    // Limit header reads to 8 KB — the conventional maximum for HTTP/1.1 headers.
    // Requests with larger headers (e.g. very large JWTs) will receive a 400
    // response.
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

        let mut headers = [EMPTY_HEADER; 64];
        let mut req = Request::new(&mut headers);

        match req.parse(&buf[..bytes_read]) {
            Ok(Status::Complete(_header_len)) => {
                let (service_id, interface) = match parse_target_service_and_interface(&req) {
                    Some(res) => res,
                    None => {
                        return write_json_rpc_error(
                            &mut stream,
                            400,
                            "Missing or invalid Host header",
                        )
                        .await;
                    }
                };

                debug!(
                    "Proxying to interface (hash): {}, service_id (alias): {}",
                    interface, service_id
                );

                let client_arc = state
                    .clients
                    .entry(service_id.clone())
                    .or_insert_with(|| {
                        // Reconstructed from the same key bytes rather than
                        // shared, since `Identity` is deliberately not
                        // `Clone` -- every downstream client presents the
                        // same node DID.
                        let identity = Identity::from_bytes(&state.identity.to_bytes());
                        Arc::new(Mutex::new(SyneroymClient::new_with_identity(
                            service_id.clone(),
                            state.registry_url.clone(),
                            identity,
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

                let passthrough_identity = Identity::from_bytes(&state.identity.to_bytes());
                SyneroymClient::passthrough_with_conn(
                    conn,
                    &service_id,
                    &interface,
                    &buf[..bytes_read],
                    &mut stream,
                    &passthrough_identity,
                )
                .await?;
                return Ok(());
            }
            Ok(Status::Partial) => {
                if bytes_read >= buf.len() {
                    return write_json_rpc_error(&mut stream, 400, "Headers too large").await;
                }
                continue;
            }
            Err(e) => {
                return write_json_rpc_error(
                    &mut stream,
                    400,
                    &format!("Invalid HTTP request: {e}"),
                )
                .await;
            }
        }
    }
}

fn parse_target_service_and_interface(req: &Request) -> Option<(String, String)> {
    let host_header = req
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("host"))
        .map_or("", |h| str::from_utf8(h.value).unwrap_or(""));

    if host_header.is_empty() {
        return None;
    }

    let mut host = host_header;
    if let Some((h, p)) = host_header.rsplit_once(':')
        && !p.is_empty()
        && p.chars().all(|c| c.is_ascii_digit())
    {
        host = h;
    }
    let host_base = host.strip_suffix(".localhost").unwrap_or(host);

    // Parse host_base according to `<nickname>-p<pubkeyhash>-i<interfacehash>` or
    // `<nickname>-p<pubkeyhash>` Split by '-' and parse from the right to
    // support nicknames with dashes
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

    let nickname = if parts.is_empty() { None } else { Some(parts.join("-")) };

    let lookup_alias = if let Some(n) = nickname {
        format!("{n}-p{}", pubkeyhash.unwrap_or_default())
    } else {
        format!("p{}", pubkeyhash.unwrap_or_default())
    };

    // The interface is now the short hash, fallback to empty if omitted
    let interface = interfacehash.unwrap_or("").to_string();
    let service_id = lookup_alias;

    Some((service_id, interface))
}

/// Writes a JSON-RPC error response as an HTTP response.
async fn write_json_rpc_error(stream: &mut TcpStream, status: u16, message: &str) -> Result<()> {
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

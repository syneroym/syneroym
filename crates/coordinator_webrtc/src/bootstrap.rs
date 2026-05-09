use anyhow::{Result, anyhow};
use askama::Template;
use iroh::Endpoint;
use std::net::SocketAddr;
use std::sync::Arc;
use syneroym_core::protocol_utils::{
    extract_host_from_http, extract_service_from_host, extract_sni, is_tls_client_hello,
};
use syneroym_core::registry::EndpointRegistry;
use syneroym_router::SYNEROYM_ALPN;
use syneroym_router::net_iroh::IrohStream;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info};

pub struct BootstrapState {
    pub iroh: Endpoint,
    pub signaling_server_url: String,
    pub registry: EndpointRegistry,
}

#[derive(Template)]
#[template(path = "peer-proxy.html")]
struct PeerProxyTemplate<'a> {
    target_peer_id: &'a str,
    signaling_server_url: &'a str,
    http_version: &'a str,
}

pub async fn start(port: u16, state: Arc<BootstrapState>) -> Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await?;
    info!("Bootstrap server listening on http://{}", addr);

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, state).await {
                error!("Error handling bootstrap connection from {}: {}", peer_addr, e);
            }
        });
    }
}

async fn handle_connection(mut stream: TcpStream, state: Arc<BootstrapState>) -> Result<()> {
    let mut buf = vec![0u8; 4096];
    let n = stream.peek(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let svc_name = if is_tls_client_hello(&buf[..n]) {
        let sni = match extract_sni(&buf[..n]) {
            Ok(s) => s,
            Err(e) => return Err(anyhow!("Failed to extract SNI: {}", e)),
        };
        extract_service_from_host(&sni)?
    } else {
        let host = extract_host_from_http(&buf[..n])?;
        extract_service_from_host(&host)?
    };

    debug!("Bootstrap request for service: {}", svc_name);

    // If it's a WebSocket upgrade, we tunnel to Iroh
    if is_websocket_upgrade(&buf[..n]) {
        debug!("WebSocket upgrade detected, tunneling to Iroh for {}", svc_name);
        return tunnel_to_iroh(stream, state, svc_name).await;
    }

    // Serve the bootstrap page
    let tpl = PeerProxyTemplate {
        target_peer_id: &svc_name,
        signaling_server_url: &state.signaling_server_url,
        http_version: "1.1",
    };
    let html = tpl.render()?;

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );

    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

fn is_websocket_upgrade(buf: &[u8]) -> bool {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);
    if let Ok(httparse::Status::Complete(_)) = req.parse(buf)
        && let Some(upgrade) = req.headers.iter().find(|h| h.name.eq_ignore_ascii_case("Upgrade"))
    {
        return upgrade.value.eq_ignore_ascii_case(b"websocket");
    }
    false
}

async fn tunnel_to_iroh(
    mut stream: TcpStream,
    state: Arc<BootstrapState>,
    svc_name: String,
) -> Result<()> {
    let endpoints = state.registry.lookup_by_service(&svc_name);
    let (iface, _endpoint) =
        endpoints.first().ok_or_else(|| anyhow!("Service not found in registry: {}", svc_name))?;

    // In 0.97, NodeId is PublicKey.
    let node_id =
        svc_name.parse::<iroh::PublicKey>().map_err(|_| anyhow!("Invalid NodeId: {}", svc_name))?;

    // In 0.97, connect takes an EndpointAddr and alpn.
    let endpoint_addr = iroh::EndpointAddr::from(node_id);
    let connection = state.iroh.connect(endpoint_addr, SYNEROYM_ALPN).await?;
    let (send, recv) = connection.open_bi().await?;

    let mut iroh_stream = IrohStream::new(send, recv);

    // Send preamble to Iroh
    let preamble = format!("http://{}|{}\n", iface, svc_name);
    iroh_stream.write_all(preamble.as_bytes()).await?;

    tokio::io::copy_bidirectional(&mut stream, &mut iroh_stream).await?;
    Ok(())
}

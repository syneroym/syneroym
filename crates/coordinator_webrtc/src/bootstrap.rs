use anyhow::{Result, anyhow};
use askama::Template;
use iroh::Endpoint;
use std::sync::Arc;
use syneroym_core::protocol_utils::{extract_service_from_host, extract_sni, is_tls_client_hello};
use syneroym_core::registry::EndpointRegistry;
use syneroym_router::SYNEROYM_ALPN;
use syneroym_router::net_iroh::IrohStream;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

pub struct BootstrapState {
    pub iroh: Endpoint,
    pub external_host: Option<String>,
    pub signaling_port: u16,
    pub registry: EndpointRegistry,
    pub registry_url: Option<String>,
}

#[derive(Template)]
#[template(path = "peer-proxy.html")]
struct PeerProxyTemplate<'a> {
    target_peer_id: &'a str,
    target_service_id: &'a str,
    signaling_server_url: String,
    http_version: &'a str,
}

pub async fn start(listener: TcpListener, state: Arc<BootstrapState>) -> Result<()> {
    info!("Bootstrap server listening on http://{}", listener.local_addr()?);

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

    let (is_tls, host, path) = if is_tls_client_hello(&buf[..n]) {
        let sni = match extract_sni(&buf[..n]) {
            Ok(s) => s,
            Err(e) => return Err(anyhow!("Failed to extract SNI: {}", e)),
        };
        (true, sni.clone(), None)
    } else {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut headers);
        req.parse(&buf[..n])?;
        let host = req
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("Host"))
            .map(|h| String::from_utf8_lossy(h.value).to_string())
            .ok_or_else(|| anyhow!("Host header not found"))?;

        let is_https = req
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("X-Forwarded-Proto"))
            .map(|h| String::from_utf8_lossy(h.value).eq_ignore_ascii_case("https"))
            .unwrap_or(false);

        (is_https, host, req.path.map(|p| p.to_string()))
    };

    let mut svc_name = extract_service_from_host(&host)?;

    // Fallback for localhost testing: if host is localhost, use first path segment as service name
    if (svc_name == "localhost" || svc_name == "127.0.0.1")
        && let Some(ref p) = path
    {
        let segments: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
        if !segments.is_empty()
            && segments[0] != "__syneroym"
            && segments[0] != "sw.js"
            && segments[0] != "favicon.ico"
        {
            svc_name = segments[0].to_string();
        }
    }

    let mut requested_interface = None;
    let mut parts: Vec<&str> = svc_name.split('-').collect();
    if parts.len() > 1
        && let Some(last) = parts.last()
        && last.starts_with('i')
        && last.len() > 1
    {
        requested_interface = Some(last[1..].to_string());
        parts.pop();
        svc_name = parts.join("-");
    }

    debug!(
        "Bootstrap request for service: {} (iface: {:?}, path: {:?})",
        svc_name, requested_interface, path
    );

    // Resolve alias to canonical DID if possible
    let mut target_peer_id = svc_name.clone();
    let mut target_service_id = svc_name.clone();

    if let Some(registry_url) = &state.registry_url {
        debug!("Attempting to resolve alias: {}", svc_name);
        match syneroym_core::community_registry::RegistryClient::lookup(
            registry_url,
            &svc_name,
            true,
        )
        .await
        {
            Ok(info) => {
                info!(
                    "Resolved service alias '{}' to substrate DID '{}' and service DID '{}'",
                    svc_name, info.info.substrate_id, info.info.service_id
                );
                target_peer_id = info.info.substrate_id;
                target_service_id = info.info.service_id;
            }
            Err(e) => {
                warn!(
                    "Failed to resolve alias '{}' (might be a DID or not registered): {}",
                    svc_name, e
                );
            }
        }
    }

    // Serve the Service Worker
    if let Some(ref p) = path
        && (p == "/__syneroym/sw.js" || p == "/sw.js")
    {
        info!("Serving sw.js to client");
        let sw_js = include_str!("../templates/sw.js");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nService-Worker-Allowed: /\r\nConnection: close\r\n\r\n{}",
            sw_js.len(),
            sw_js
        );
        stream.write_all(response.as_bytes()).await?;
        stream.flush().await?;
        return Ok(());
    }

    // If it's a WebSocket upgrade, we tunnel to Iroh
    if is_websocket_upgrade(&buf[..n]) {
        debug!("WebSocket upgrade detected, tunneling to Iroh for {}", svc_name);
        return tunnel_to_iroh(
            stream,
            state,
            target_peer_id,
            target_service_id,
            requested_interface,
        )
        .await;
    }

    // Construct signaling server URL
    let scheme = if is_tls { "wss" } else { "ws" };
    let signaling_server_url =
        construct_signaling_url(scheme, &host, &state.external_host, state.signaling_port);

    // Serve the bootstrap page
    let tpl = PeerProxyTemplate {
        target_peer_id: &target_peer_id,
        target_service_id: &target_service_id,
        signaling_server_url,
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

fn construct_signaling_url(
    scheme: &str,
    host: &str,
    external_host: &Option<String>,
    signaling_port: u16,
) -> String {
    let signaling_host = if let Some(h) = external_host {
        h.clone()
    } else {
        // Strip port from Host header if present
        host.split(':').next().unwrap_or("localhost").to_string()
    };

    format!("{}://{}:{}/ws", scheme, signaling_host, signaling_port)
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
    peer_id: String,
    service_id: String,
    requested_interface: Option<String>,
) -> Result<()> {
    let iface = if let Some(req_iface) = requested_interface {
        req_iface
    } else {
        let endpoints = state.registry.lookup_by_service(&service_id);
        endpoints
            .first()
            .ok_or_else(|| anyhow!("Service not found in registry: {}", service_id))?
            .0
            .clone()
    };

    // In 0.97, NodeId is PublicKey.
    let node_id =
        peer_id.parse::<iroh::PublicKey>().map_err(|_| anyhow!("Invalid NodeId: {}", peer_id))?;

    // In 0.97, connect takes an EndpointAddr and alpn.
    let endpoint_addr = iroh::EndpointAddr::from(node_id);
    let connection = state.iroh.connect(endpoint_addr, SYNEROYM_ALPN).await?;
    let (send, recv) = connection.open_bi().await?;

    let mut iroh_stream = IrohStream::new(send, recv);

    // Send preamble to Iroh
    let preamble = format!("http://{}|{}\n", iface, service_id);
    iroh_stream.write_all(preamble.as_bytes()).await?;

    tokio::io::copy_bidirectional(&mut stream, &mut iroh_stream).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_construct_signaling_url() {
        // Case 1: No external host, simple hostname
        assert_eq!(
            construct_signaling_url("ws", "localhost", &None, 7963),
            "ws://localhost:7963/ws"
        );

        // Case 2: No external host, hostname with port
        assert_eq!(
            construct_signaling_url("ws", "192.168.1.10:7962", &None, 7963),
            "ws://192.168.1.10:7963/ws"
        );

        // Case 3: External host override
        assert_eq!(
            construct_signaling_url("wss", "localhost", &Some("syneroym.io".to_string()), 443),
            "wss://syneroym.io:443/ws"
        );

        // Case 4: No external host, complex domain
        assert_eq!(
            construct_signaling_url("ws", "coordinator.local:7962", &None, 7963),
            "ws://coordinator.local:7963/ws"
        );
    }

    #[test]
    fn test_svc_name_interface_splitting() {
        let cases = vec![
            ("myapp-p1234-i80", "myapp-p1234", Some("80")),
            ("myapp-i80", "myapp", Some("80")),
            ("my-app-i80", "my-app", Some("80")),
            ("myapp-p1234", "myapp-p1234", None),
            ("my-i-80", "my-i-80", None), // last part 80 doesn't start with i
            ("i80", "i80", None),         // parts.len() == 1
        ];

        for (input, expected_svc, expected_iface) in cases {
            let mut svc_name = input.to_string();
            let mut requested_interface = None;
            let mut parts: Vec<&str> = svc_name.split('-').collect();
            if parts.len() > 1
                && let Some(last) = parts.last()
                && last.starts_with('i')
                && last.len() > 1
            {
                requested_interface = Some(last[1..].to_string());
                parts.pop();
                svc_name = parts.join("-");
            }
            assert_eq!(svc_name, expected_svc, "Failed for input: {}", input);
            assert_eq!(
                requested_interface.as_deref(),
                expected_iface,
                "Failed for input: {}",
                input
            );
        }
    }
}

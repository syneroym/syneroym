use anyhow::{Result, anyhow};
use askama::Template;
use iroh::Endpoint;
use std::sync::Arc;
use syneroym_core::community_registry::EndpointMechanism;
use syneroym_core::protocol_utils::{extract_service_from_host, extract_sni, is_tls_client_hello};
use syneroym_core::registry::EndpointRegistry;
use syneroym_identity::substrate::resolve_did_key;
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
    let mut buf = vec![0u8; 8192];
    let n = stream.peek(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let (is_tls, host, path) = if is_tls_client_hello(&buf[..n]) {
        let host = extract_sni(&buf[..n])?;
        (true, host, None)
    } else {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut headers);
        req.parse(&buf[..n])?;

        let host_header = req
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("host"))
            .map(|h| std::str::from_utf8(h.value).unwrap_or(""))
            .unwrap_or("");

        (false, host_header.to_string(), req.path.map(|p| p.to_string()))
    };

    // Serve the Service Worker or handle other special paths immediately
    if let Some(ref p) = path {
        if p == "/__syneroym/sw.js" || p == "/sw.js" {
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
        if p == "/favicon.ico" {
            return Ok(());
        }
    }

    let mut svc_name = extract_service_from_host(&host)?;
    let mut requested_interface = None;

    if (svc_name == "localhost" || svc_name == "127.0.0.1")
        && let Some(ref p) = path
    {
        let segments: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
        if !segments.is_empty() {
            svc_name = segments[0].to_string();
        }
    }

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

    // Resolve alias to canonical DID if possible
    let mut target_peer_id = svc_name.clone();
    let mut target_service_id = svc_name.clone();
    let mut target_iroh_addr = None;

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

                // Capture Iroh mechanism for direct connection
                for mechanism in info.info.mechanisms {
                    if let EndpointMechanism::Iroh { endpoint_addr_bytes, .. } = mechanism
                        && let Ok(addr) =
                            serde_json::from_slice::<iroh::EndpointAddr>(&endpoint_addr_bytes)
                    {
                        target_iroh_addr = Some(addr);
                        break;
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Failed to resolve alias '{}' (might be a DID or not registered): {}",
                    svc_name, e
                );
            }
        }
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
            target_iroh_addr,
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
        http_version: "HTTP/1.1",
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
    // Use Partial here because we might not have a full body, but we should have full headers
    // since we call this after find_double_crlf.
    if let Ok(status) = req.parse(buf)
        && (status.is_partial() || status.is_complete())
    {
        let has_upgrade = req.headers.iter().any(|h| {
            h.name.eq_ignore_ascii_case("Upgrade") && h.value.eq_ignore_ascii_case(b"websocket")
        });
        let has_connection = req.headers.iter().any(|h| {
            h.name.eq_ignore_ascii_case("Connection")
                && String::from_utf8_lossy(h.value).to_lowercase().contains("upgrade")
        });
        return has_upgrade && has_connection;
    }
    false
}

async fn tunnel_to_iroh(
    mut stream: TcpStream,
    state: Arc<BootstrapState>,
    peer_id: String,
    service_id: String,
    requested_interface: Option<String>,
    iroh_addr: Option<iroh::EndpointAddr>,
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

    let endpoint_addr = if let Some(addr) = iroh_addr {
        addr
    } else {
        // Fallback: Resolve NodeId from peer_id (which might be a DID)
        let node_id = if peer_id.starts_with("did:key:h") {
            let pubkey = resolve_did_key(&peer_id)?;
            iroh::PublicKey::from_bytes(pubkey.as_bytes())?
        } else {
            peer_id
                .parse::<iroh::PublicKey>()
                .map_err(|_| anyhow!("Invalid NodeId: {}", peer_id))?
        };
        iroh::EndpointAddr::from(node_id)
    };

    debug!("Connecting to Iroh node: {:?}", endpoint_addr);
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

    #[test]
    fn test_did_to_public_key_resolution() {
        use ed25519_dalek::VerifyingKey;
        use syneroym_identity::substrate::derive_did_key;

        let mut pubkey_bytes = [0u8; 32];
        pubkey_bytes[0] = 1; // Just some non-zero byte
        let pubkey = VerifyingKey::from_bytes(&pubkey_bytes).unwrap();
        let did = derive_did_key(&pubkey);

        // Resolve back
        let resolved_pubkey = resolve_did_key(&did).expect("Failed to resolve DID");
        let node_id = iroh::PublicKey::from_bytes(resolved_pubkey.as_bytes())
            .expect("Failed to create NodeId");

        // Manual verification of the Iroh part
        let raw_z32 = &did["did:key:h".len()..];
        let bytes = z32::decode(raw_z32.as_bytes()).expect("Failed to decode z32");
        // Skip multicodec prefix (0xed, 0x01)
        let iroh_pubkey_bytes: [u8; 32] = bytes[2..].try_into().unwrap();
        let manual_node_id = iroh::PublicKey::from_bytes(&iroh_pubkey_bytes).unwrap();

        assert_eq!(node_id, manual_node_id);
        assert_eq!(resolved_pubkey.as_bytes(), pubkey.as_bytes());
    }
}

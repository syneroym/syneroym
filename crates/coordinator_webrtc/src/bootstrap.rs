use anyhow::anyhow;
use askama::Template;
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode, header},
    response::{Html, IntoResponse},
    routing::get,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
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
use tracing::{debug, error, info};

pub struct BootstrapState {
    pub iroh: Endpoint,
    pub external_host: Option<String>,
    pub signaling_port: u16,
    pub registry: EndpointRegistry,
    pub registry_url: Option<String>,
}

#[derive(Template)]
#[template(path = "peer-proxy.html")]
struct PeerProxyTemplate {
    target_peer_id: String,
    target_service_id: String,
    signaling_server_url: String,
    http_version: String,
}

pub async fn start(listener: TcpListener, state: Arc<BootstrapState>) -> anyhow::Result<()> {
    info!("Bootstrap server listening on http://{}", listener.local_addr()?);

    let app = app(state.clone());

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let state = state.clone();
        let app = app.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, state, app).await {
                error!("Error handling bootstrap connection from {}: {}", peer_addr, e);
            }
        });
    }
}

fn app(state: Arc<BootstrapState>) -> Router {
    Router::new()
        .route("/sw.js", get(handle_sw))
        .route("/__syneroym/sw.js", get(handle_sw))
        .fallback(handle_bootstrap)
        .with_state(state)
}

async fn handle_sw() -> impl IntoResponse {
    info!("Serving sw.js to client");
    let sw_js = include_str!("../templates/sw.js");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::HeaderName::from_static("service-worker-allowed"), "/"),
        ],
        sw_js,
    )
}

async fn handle_bootstrap(
    State(state): State<Arc<BootstrapState>>,
    req: Request<Body>,
) -> impl IntoResponse {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost")
        .to_string();
    let path = req.uri().path();
    if path == "/favicon.ico" {
        return StatusCode::NOT_FOUND.into_response();
    }

    let mut svc_name = match extract_service_from_host(&host) {
        Ok(name) => name,
        Err(_) => host.clone(),
    };

    let mut parts: Vec<&str> = svc_name.split('-').collect();
    if parts.len() > 1
        && let Some(last) = parts.last()
        && last.starts_with('i')
        && last.len() > 1
    {
        parts.pop();
        svc_name = parts.join("-");
    }

    // Resolve alias to canonical DID if possible
    let mut target_peer_id = svc_name.clone();
    let mut target_service_id = svc_name.clone();

    if let Some(registry_url) = &state.registry_url {
        debug!("Attempting to resolve alias: {}", svc_name);
        if let Ok(info) =
            syneroym_core::community_registry::RegistryClient::lookup(registry_url, &svc_name, true)
                .await
        {
            info!(
                "Resolved service alias '{}' to substrate DID '{}' and service DID '{}'",
                svc_name, info.info.substrate_id, info.info.service_id
            );
            target_peer_id = info.info.substrate_id;
            target_service_id = info.info.service_id;
        }
    }

    let signaling_server_url =
        construct_signaling_url("ws", &host, &state.external_host, state.signaling_port);

    let tpl = PeerProxyTemplate {
        target_peer_id,
        target_service_id,
        signaling_server_url,
        http_version: "HTTP/1.1".to_string(),
    };

    match tpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            error!("Failed to render template: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    state: Arc<BootstrapState>,
    app: Router,
) -> anyhow::Result<()> {
    let mut buf = [0u8; 8192];
    let n = stream.peek(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    // Hybrid Dispatch:
    // 1. Check for TLS (SNI sniffing)
    if is_tls_client_hello(&buf[..n]) {
        let host = extract_sni(&buf[..n])?;
        return handle_raw_tls_tunnel(stream, state, host).await;
    }

    // 2. Check for WebSocket upgrade (Raw Tunneling)
    if is_websocket_upgrade(&buf[..n]) {
        let host = syneroym_core::protocol_utils::extract_host_from_http(&buf[..n])?;
        let path = extract_path_from_http(&buf[..n]).unwrap_or_else(|| "/".to_string());

        debug!("WebSocket upgrade detected, tunneling to Iroh for {}", host);
        return handle_ws_tunnel(stream, state, host, path).await;
    }

    // 3. Otherwise, pass to Axum
    let service = TowerToHyperService::new(app);
    Builder::new(TokioExecutor::new())
        .serve_connection(TokioIo::new(stream), service)
        .await
        .map_err(|e| anyhow::anyhow!("Axum serve error: {}", e))
}

async fn handle_raw_tls_tunnel(
    _stream: TcpStream,
    _state: Arc<BootstrapState>,
    host: String,
) -> anyhow::Result<()> {
    // Current implementation doesn't actually have a TLS tunnel target,
    // but we keep the logic structure for future use or to avoid breaking SNI sniffing tests.
    debug!("TLS SNI detected: {}, but raw TLS tunneling is not yet implemented", host);
    Ok(())
}

async fn handle_ws_tunnel(
    stream: TcpStream,
    state: Arc<BootstrapState>,
    host: String,
    _path: String,
) -> anyhow::Result<()> {
    let mut svc_name = extract_service_from_host(&host)?;
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

    let mut target_peer_id = svc_name.clone();
    let mut target_service_id = svc_name.clone();
    let mut target_iroh_addr = None;

    if let Some(registry_url) = &state.registry_url
        && let Ok(info) =
            syneroym_core::community_registry::RegistryClient::lookup(registry_url, &svc_name, true)
                .await
    {
        target_peer_id = info.info.substrate_id;
        target_service_id = info.info.service_id;
        for mechanism in info.info.mechanisms {
            if let EndpointMechanism::Iroh { endpoint_addr_bytes, .. } = mechanism
                && let Ok(addr) = serde_json::from_slice::<iroh::EndpointAddr>(&endpoint_addr_bytes)
            {
                target_iroh_addr = Some(addr);
                break;
            }
        }
    }

    tunnel_to_iroh(
        stream,
        state,
        target_peer_id,
        target_service_id,
        requested_interface,
        target_iroh_addr,
    )
    .await
}

fn extract_path_from_http(buf: &[u8]) -> Option<String> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);
    if let Ok(httparse::Status::Complete(_)) = req.parse(buf) {
        return req.path.map(|p| p.to_string());
    }
    None
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
) -> anyhow::Result<()> {
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

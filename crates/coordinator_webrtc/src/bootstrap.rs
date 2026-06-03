//! WebRTC Bootstrap page server
//!
//! Hosts static/dynamic HTML pages and assets to assist peer discovery
//! and WebRTC initialization inside web clients.

use askama::Template;
use axum::{
    Router,
    body::Body,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{Request, StatusCode, header},
    response::{Html, IntoResponse},
    routing::{any, get},
};
use futures::{SinkExt, StreamExt};
use iroh::Endpoint;
use std::sync::Arc;
use syneroym_core::community_registry::EndpointMechanism;
use syneroym_core::protocol_utils::extract_service_from_host;
use syneroym_core::registry::EndpointRegistry;
use syneroym_identity::substrate::resolve_did_key;
use syneroym_router::SYNEROYM_ALPN;
use syneroym_router::net_iroh::IrohStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, error, info};

pub struct BootstrapState {
    pub iroh: Endpoint,
    pub external_host: Option<String>,
    pub signaling_port: u16,
    pub registry: EndpointRegistry,
    pub registry_url: Option<String>,
}

impl std::fmt::Debug for BootstrapState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapState")
            .field("iroh", &"iroh::Endpoint")
            .field("external_host", &self.external_host)
            .field("signaling_port", &self.signaling_port)
            .field("registry", &self.registry)
            .field("registry_url", &self.registry_url)
            .finish()
    }
}

#[derive(Template)]
#[template(path = "peer-proxy.html")]
struct PeerProxyTemplate {
    target_peer_id: String,
    target_service_id: String,
    signaling_server_url: String,
    http_version: String,
    target_pubkey_hex: String,
}

pub async fn start(listener: TcpListener, state: Arc<BootstrapState>) -> anyhow::Result<()> {
    info!("Bootstrap server listening on http://{}", listener.local_addr()?);

    let app = app(state.clone());
    axum::serve(listener, app).await?;
    Ok(())
}

fn app(state: Arc<BootstrapState>) -> Router {
    Router::new()
        .route("/sw.js", get(handle_sw))
        .route("/__syneroym/sw.js", get(handle_sw))
        .route("/__syneroym/tunnel", any(handle_tunnel_upgrade))
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
        let registry_client = syneroym_core::community_registry::RegistryClient::new(
            true,
            Some(registry_url.to_string()),
        );
        if let Ok(info) = registry_client.lookup(&svc_name, true).await {
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

    let target_pubkey_hex = match resolve_did_key(&target_peer_id) {
        Ok(pubkey) => hex::encode(pubkey.as_bytes()),
        Err(e) => {
            error!("Failed to resolve target_peer_id '{}' DID: {}", target_peer_id, e);
            String::new()
        }
    };

    let tpl = PeerProxyTemplate {
        target_peer_id,
        target_service_id,
        signaling_server_url,
        http_version: "HTTP/1.1".to_string(),
        target_pubkey_hex,
    };

    match tpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            error!("Failed to render template: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn handle_tunnel_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<BootstrapState>>,
) -> impl IntoResponse {
    debug!("[BlindTunnel] WebSocket upgrade request; upgrading connection");
    ws.on_upgrade(move |socket| handle_blind_tunnel(socket, state))
}

async fn handle_blind_tunnel(socket: WebSocket, state: Arc<BootstrapState>) {
    debug!("[BlindTunnel] Connection upgraded; waiting for preamble message");
    let (ws_sender, mut ws_receiver) = socket.split();

    // 1. Read preamble
    let (preamble, preamble_str) = match read_preamble_from_ws(&mut ws_receiver).await {
        Some(res) => res,
        None => return,
    };

    // 2. Resolve registry URL
    let registry_url = if let Some(url) = &state.registry_url {
        url
    } else {
        error!("[BlindTunnel] No community registry configured; cannot resolve substrate");
        return;
    };

    // 3. Resolve Iroh Endpoint
    let endpoint_addr =
        match resolve_iroh_endpoint_from_registry(&preamble.service_id, registry_url).await {
            Some(addr) => addr,
            None => return,
        };

    // 4. Connect to Iroh Node and forward preamble
    let iroh_stream = match connect_iroh_stream(&state.iroh, endpoint_addr, &preamble_str).await {
        Some(stream) => stream,
        None => return,
    };

    // 5. Pipe bidirectionally WS <-> Iroh
    pipe_ws_and_iroh(ws_sender, ws_receiver, iroh_stream).await;
    debug!("[BlindTunnel] Tunnel closed for service '{}'", preamble.service_id);
}

async fn read_preamble_from_ws(
    ws_receiver: &mut futures::stream::SplitStream<WebSocket>,
) -> Option<(syneroym_router::RoutePreamble, String)> {
    let msg = match ws_receiver.next().await {
        Some(Ok(Message::Binary(bin))) => {
            debug!("[BlindTunnel] Received binary preamble ({} bytes)", bin.len());
            bin.to_vec()
        }
        Some(Ok(Message::Text(txt))) => {
            debug!("[BlindTunnel] Received text preamble ({} bytes)", txt.len());
            txt.as_bytes().to_vec()
        }
        _ => {
            error!("[BlindTunnel] Failed to read preamble; closing tunnel");
            return None;
        }
    };

    let preamble_str = match String::from_utf8(msg) {
        Ok(s) => s,
        Err(e) => {
            error!("[BlindTunnel] Invalid UTF-8 preamble: {e}");
            return None;
        }
    };

    debug!("[BlindTunnel] Raw preamble: {:?}", preamble_str.trim());

    let preamble = match syneroym_router::RoutePreamble::parse(&preamble_str) {
        Ok(p) => p,
        Err(e) => {
            error!("[BlindTunnel] Failed to parse preamble '{}': {e}", preamble_str.trim());
            return None;
        }
    };

    debug!(
        "[BlindTunnel] Preamble parsed: transport={:?} protocol={:?} interface='{}' service_id='{}' enc={:?}",
        preamble.transport,
        preamble.protocol,
        preamble.interface,
        preamble.service_id,
        preamble.enc
    );

    Some((preamble, preamble_str))
}

async fn resolve_iroh_endpoint_from_registry(
    service_id: &str,
    registry_url: &str,
) -> Option<iroh::EndpointAddr> {
    debug!("[BlindTunnel] Looking up service '{}' in registry at '{}'", service_id, registry_url);
    let registry_client = syneroym_core::community_registry::RegistryClient::new(
        true,
        Some(registry_url.to_string()),
    );
    let info = match registry_client.lookup(service_id, true).await {
        Ok(i) => {
            debug!(
                "[BlindTunnel] Registry OK: substrate_id='{}' service_id='{}' mechanisms={}",
                i.info.substrate_id,
                i.info.service_id,
                i.info.mechanisms.len()
            );
            i
        }
        Err(e) => {
            error!("[BlindTunnel] Registry lookup failed for '{}': {e}", service_id);
            return None;
        }
    };

    // Prefer an explicit Iroh mechanism; fall back to deriving from the substrate DID.
    let mut iroh_addr_from_mechanism = None;
    for mechanism in &info.info.mechanisms {
        if let EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url } = mechanism
            && let Ok(addr) = serde_json::from_slice::<iroh::EndpointAddr>(endpoint_addr_bytes)
        {
            let mut addr = addr;
            if let Some(url_str) = relay_url
                && let Ok(url) = url_str.parse::<iroh::RelayUrl>()
            {
                addr = addr.with_relay_url(url);
            }
            iroh_addr_from_mechanism = Some(addr);
            break;
        }
    }

    if let Some(addr) = iroh_addr_from_mechanism {
        debug!("[BlindTunnel] Using explicit Iroh mechanism from registry: {:?}", addr);
        Some(addr)
    } else {
        debug!(
            "[BlindTunnel] No explicit Iroh mechanism; deriving from substrate DID '{}'",
            info.info.substrate_id
        );
        match resolve_did_key(&info.info.substrate_id) {
            Ok(pubkey) => match iroh::PublicKey::from_bytes(pubkey.as_bytes()) {
                Ok(pk) => {
                    let addr = iroh::EndpointAddr::from(pk);
                    debug!("[BlindTunnel] Derived Iroh endpoint addr: {:?}", addr);
                    Some(addr)
                }
                Err(e) => {
                    error!("[BlindTunnel] Invalid substrate public key bytes: {e}");
                    None
                }
            },
            Err(e) => {
                error!(
                    "[BlindTunnel] Failed to resolve substrate DID '{}': {e}",
                    info.info.substrate_id
                );
                None
            }
        }
    }
}

async fn connect_iroh_stream(
    endpoint: &iroh::Endpoint,
    endpoint_addr: iroh::EndpointAddr,
    preamble_str: &str,
) -> Option<IrohStream> {
    debug!("[BlindTunnel] Connecting to Iroh node: {:?}", endpoint_addr);
    let connection = match endpoint.connect(endpoint_addr, SYNEROYM_ALPN).await {
        Ok(c) => {
            debug!(
                "[BlindTunnel] Iroh connection established (ALPN={})",
                std::str::from_utf8(SYNEROYM_ALPN).unwrap_or("<invalid>")
            );
            c
        }
        Err(e) => {
            error!("[BlindTunnel] Failed to connect to Iroh node: {e}");
            return None;
        }
    };

    let (send, recv) = match connection.open_bi().await {
        Ok(streams) => {
            debug!("[BlindTunnel] Bi-directional Iroh stream opened");
            streams
        }
        Err(e) => {
            error!("[BlindTunnel] Failed to open bi-directional stream: {e}");
            return None;
        }
    };

    let mut iroh_stream = IrohStream::new(send, recv);

    // Forward the preamble to the Iroh stream
    debug!("[BlindTunnel] Forwarding preamble to Iroh ({} bytes)", preamble_str.len());
    if let Err(e) = iroh_stream.write_all(preamble_str.as_bytes()).await {
        error!("[BlindTunnel] Failed to write preamble to Iroh stream: {e}");
        return None;
    }

    Some(iroh_stream)
}

async fn pipe_ws_and_iroh(
    mut ws_sender: futures::stream::SplitSink<WebSocket, Message>,
    mut ws_receiver: futures::stream::SplitStream<WebSocket>,
    iroh_stream: IrohStream,
) {
    debug!("[BlindTunnel] Preamble sent; starting bidirectional pipe WS<->Iroh");
    let (mut iroh_read, mut iroh_write) = tokio::io::split(iroh_stream);

    let ws_to_iroh = async move {
        while let Some(msg_res) = ws_receiver.next().await {
            match msg_res {
                Ok(Message::Binary(bin)) => {
                    if let Err(e) = iroh_write.write_all(&bin).await {
                        error!("[BlindTunnel][WS->Iroh] Failed to write binary data to Iroh: {e}");
                        break;
                    }
                }
                Ok(Message::Text(txt)) => {
                    if let Err(e) = iroh_write.write_all(txt.as_bytes()).await {
                        error!("[BlindTunnel][WS->Iroh] Failed to write text data to Iroh: {e}");
                        break;
                    }
                }
                Ok(Message::Close(_)) => {
                    break;
                }
                Err(e) => {
                    error!("[BlindTunnel][WS->Iroh] WS reader error: {e}");
                    break;
                }
                _ => {}
            }
        }
        let _ = iroh_write.shutdown().await;
    };

    let iroh_to_ws = async move {
        let mut buf = vec![0u8; 16384];
        loop {
            match iroh_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
                    if let Err(e) = ws_sender.send(Message::Binary(chunk.into())).await {
                        error!("[BlindTunnel][Iroh->WS] Failed to send WebSocket message: {e}");
                        break;
                    }
                }
                Err(e) => {
                    error!("[BlindTunnel][Iroh->WS] Iroh stream read error: {e}");
                    break;
                }
            }
        }
    };

    tokio::select! {
        () = ws_to_iroh => {}
        () = iroh_to_ws => {}
    }
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

    format!("{scheme}://{signaling_host}:{signaling_port}/ws")
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

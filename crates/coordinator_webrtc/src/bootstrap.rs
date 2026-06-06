//! WebRTC Bootstrap page server
//!
//! Hosts static/dynamic HTML pages and assets to assist peer discovery
//! and WebRTC initialization inside web clients.

use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    io::Write,
    sync::{Arc, OnceLock},
};

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
use flate2::{Compression, write::GzEncoder};
use futures::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use header::HOST;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayUrl};
use syneroym_core::{
    dht_registry::{EndpointMechanism, RegistryClient},
    local_registry::EndpointRegistry,
    protocol_utils::parse_target_host,
};
use syneroym_identity::substrate::resolve_did_key;
use syneroym_router::{RoutePreamble, SYNEROYM_ALPN, net_iroh::IrohStream};
use tokio::{
    io,
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};
use tracing::{debug, error, info};

pub struct BootstrapState {
    pub iroh: Endpoint,
    pub external_host: Option<String>,
    pub signaling_port: u16,
    pub registry: EndpointRegistry,
    pub registry_url: Option<String>,
    pub registry_client: RegistryClient,
    /// Cache of active peer connections to prevent concurrent, redundant QUIC
    /// handshake requests. When multiple web resources are requested
    /// simultaneously through a service worker tunnel, they can trigger
    /// concurrent connection attempts at the exact same millisecond. Without
    /// serialization, these overlapping `endpoint.connect()` calls can
    /// initiate competing handshakes to the same target peer,
    /// causing protocol conflicts and timeouts in the underlying QUIC stack.
    pub connection_cache: Mutex<HashMap<PublicKey, iroh::endpoint::Connection>>,
}

impl Debug for BootstrapState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
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
        .route("/__syneroym/peer-proxy.js", get(handle_peer_proxy_js))
        .route("/__syneroym/tunnel", any(handle_tunnel_upgrade))
        .fallback(handle_bootstrap)
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(state)
}

static SW_JS_GZ: OnceLock<Option<Vec<u8>>> = OnceLock::new();
static PEER_PROXY_JS_GZ: OnceLock<Option<Vec<u8>>> = OnceLock::new();

fn compress_gzip(data: &str) -> Result<Vec<u8>, std::io::Error> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data.as_bytes())?;
    encoder.finish()
}
fn serve_cached_js(
    js_content: &'static str,
    name: &str,
    cache: &OnceLock<Option<Vec<u8>>>,
) -> axum::response::Response {
    let gzipped_opt = cache.get_or_init(|| {
        compress_gzip(js_content).inspect_err(|e| error!("Failed to compress {}: {}", name, e)).ok()
    });

    let mut response = match gzipped_opt {
        Some(gzipped) => {
            let mut res = gzipped.clone().into_response();
            res.headers_mut()
                .insert(header::CONTENT_ENCODING, header::HeaderValue::from_static("gzip"));
            res
        }
        None => {
            let mut res = js_content.as_bytes().to_vec().into_response();
            res.headers_mut().insert(
                header::HeaderName::from_static("x-compression-failed"),
                header::HeaderValue::from_static("true"),
            );
            res
        }
    };

    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, header::HeaderValue::from_static("application/javascript"));
    response
}

async fn handle_sw() -> impl IntoResponse {
    info!("Serving sw.js to client");
    let sw_js = include_str!(concat!(env!("OUT_DIR"), "/sw.js"));
    let mut response = serve_cached_js(sw_js, "sw.js", &SW_JS_GZ);
    response.headers_mut().insert(
        header::HeaderName::from_static("service-worker-allowed"),
        header::HeaderValue::from_static("/"),
    );
    response
}

async fn handle_peer_proxy_js() -> impl IntoResponse {
    let js = include_str!(concat!(env!("OUT_DIR"), "/peer-proxy.js"));
    serve_cached_js(js, "peer-proxy.js", &PEER_PROXY_JS_GZ)
}

async fn handle_bootstrap(
    State(state): State<Arc<BootstrapState>>,
    req: Request<Body>,
) -> impl IntoResponse {
    let host =
        req.headers().get(HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost").to_string();
    let path = req.uri().path();
    if path == "/favicon.ico" {
        return StatusCode::NOT_FOUND.into_response();
    }

    let (mut target_peer_id, _interface) = match parse_target_host(&host) {
        Some(res) => res,
        None => (host.clone(), "".to_string()),
    };

    let mut target_service_id = target_peer_id.clone();

    if state.registry_url.is_some() {
        debug!("Attempting to resolve alias: {}", target_peer_id);
        if let Ok(info) = state.registry_client.lookup(&target_peer_id, true).await {
            info!(
                "Resolved service alias '{}' to substrate DID '{}' and service DID '{}'",
                target_peer_id, info.info.substrate_id, info.info.service_id
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

    if state.registry_url.is_none() {
        error!("[BlindTunnel] No community registry configured; cannot resolve substrate");
        return;
    }

    // 3. Resolve Iroh Endpoint
    let target_addr =
        match resolve_iroh_endpoint_from_registry(&preamble.service_id, &state.registry_client)
            .await
        {
            Some(addr) => addr,
            None => return,
        };

    // 4. Connect to Iroh Node and forward preamble
    let (iroh_stream, connection) =
        match connect_iroh_stream(state.clone(), target_addr, &preamble_str).await {
            Some(res) => res,
            None => return,
        };

    // 5. Pipe bidirectionally WS <-> Iroh
    pipe_ws_and_iroh(ws_sender, ws_receiver, iroh_stream, connection).await;
    debug!("[BlindTunnel] Tunnel closed for service '{}'", preamble.service_id);
}

async fn read_preamble_from_ws(
    ws_receiver: &mut SplitStream<WebSocket>,
) -> Option<(RoutePreamble, String)> {
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

    let preamble = match RoutePreamble::parse(&preamble_str) {
        Ok(p) => p,
        Err(e) => {
            error!("[BlindTunnel] Failed to parse preamble '{}': {e}", preamble_str.trim());
            return None;
        }
    };

    debug!(
        "[BlindTunnel] Preamble parsed: transport={:?} protocol={:?} interface='{}' \
         service_id='{}' enc={:?}",
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
    registry_client: &RegistryClient,
) -> Option<EndpointAddr> {
    debug!("[BlindTunnel] Looking up service '{}' in registry", service_id);
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

    // Prefer an explicit Iroh mechanism; fall back to deriving from the substrate
    // DID.
    let mut iroh_addr_from_mechanism = None;
    for mechanism in &info.info.mechanisms {
        if let EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url } = mechanism
            && let Ok(addr) = serde_json::from_slice::<EndpointAddr>(endpoint_addr_bytes)
        {
            let mut addr = addr;
            if let Some(url_str) = relay_url
                && let Ok(url) = url_str.parse::<RelayUrl>()
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
            Ok(pubkey) => match PublicKey::from_bytes(pubkey.as_bytes()) {
                Ok(pk) => {
                    let addr = EndpointAddr::from(pk);
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
    state: Arc<BootstrapState>,
    endpoint_addr: EndpointAddr,
    preamble_str: &str,
) -> Option<(IrohStream, iroh::endpoint::Connection)> {
    let peer_id = endpoint_addr.id;
    debug!("[BlindTunnel] Connecting to Iroh node: {:?}", endpoint_addr);

    // Acquire lock on the connection cache.
    // Serializing here prevents multiple concurrent HTTP requests from attempting
    // to initiate overlapping QUIC handshakes to the same peer node
    // simultaneously, which causes Iroh/QUIC protocol conflicts and handshake
    // failures.
    let mut cache = state.connection_cache.lock().await;

    // Check if we have a cached connection
    let mut connection = cache.get(&peer_id).cloned();

    // If we have a cached connection, check if it's still alive/usable.
    if let Some(ref conn) = connection {
        if let Some(err) = conn.close_reason() {
            debug!("[BlindTunnel] Cached connection is closed ({err:?}), discarding");
            cache.remove(&peer_id);
            connection = None;
        }
    }

    let conn = match connection {
        Some(conn) => {
            debug!("[BlindTunnel] Reusing cached connection for peer {:?}", peer_id);
            conn
        }
        None => {
            let conn = match state.iroh.connect(endpoint_addr, SYNEROYM_ALPN).await {
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
            cache.insert(peer_id, conn.clone());
            conn
        }
    };

    // Drop the cache lock before performing potentially long stream operations!
    drop(cache);

    let (send, recv) = match conn.open_bi().await {
        Ok(streams) => {
            debug!("[BlindTunnel] Bi-directional Iroh stream opened");
            streams
        }
        Err(e) => {
            error!("[BlindTunnel] Failed to open bi-directional stream: {e}");
            // Remove the failed connection from cache
            let mut cache = state.connection_cache.lock().await;
            if let Some(existing) = cache.get(&peer_id)
                && existing.stable_id() == conn.stable_id()
            {
                cache.remove(&peer_id);
            }
            return None;
        }
    };

    let mut iroh_stream = IrohStream::new(send, recv).with_conn(conn.clone());

    // Forward the preamble to the Iroh stream
    debug!("[BlindTunnel] Forwarding preamble to Iroh ({} bytes)", preamble_str.len());
    if let Err(e) = iroh_stream.write_all(preamble_str.as_bytes()).await {
        error!("[BlindTunnel] Failed to write preamble to Iroh stream: {e}");
        return None;
    }
    if let Err(e) = iroh_stream.flush().await {
        error!("[BlindTunnel] Failed to flush preamble to Iroh stream: {e}");
        return None;
    }

    Some((iroh_stream, conn))
}

async fn pipe_ws_and_iroh(
    mut ws_sender: SplitSink<WebSocket, Message>,
    mut ws_receiver: SplitStream<WebSocket>,
    iroh_stream: IrohStream,
    connection: iroh::endpoint::Connection,
) {
    debug!("[BlindTunnel] Preamble sent; starting bidirectional pipe WS<->Iroh");
    let _conn_ref = &connection;
    let (mut iroh_read, mut iroh_write) = io::split(iroh_stream);

    let ws_to_iroh = async {
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
                t => {
                    error!("[BlindTunnel][WS->Iroh] Unknown WS message type: {t:?}");
                    break;
                }
            }
        }
        let _ = iroh_write.shutdown().await;
    };

    let iroh_to_ws = async {
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
        let _ = ws_sender.close().await;
    };

    tokio::select! {
        _ = ws_to_iroh => {
            debug!("[BlindTunnel] ws_to_iroh finished, tearing down tunnel");
        }
        _ = iroh_to_ws => {
            debug!("[BlindTunnel] iroh_to_ws finished, tearing down tunnel");
        }
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
        let node_id =
            PublicKey::from_bytes(resolved_pubkey.as_bytes()).expect("Failed to create NodeId");

        // Manual verification of the Iroh part
        let raw_z32 = &did["did:key:h".len()..];
        let bytes = z32::decode(raw_z32.as_bytes()).expect("Failed to decode z32");
        // Skip multicodec prefix (0xed, 0x01)
        let iroh_pubkey_bytes: [u8; 32] = bytes[2..].try_into().unwrap();
        let manual_node_id = PublicKey::from_bytes(&iroh_pubkey_bytes).unwrap();

        assert_eq!(node_id, manual_node_id);
        assert_eq!(resolved_pubkey.as_bytes(), pubkey.as_bytes());
    }
}

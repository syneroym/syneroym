//! WebRTC Bootstrap page server
//!
//! Hosts static/dynamic HTML pages and assets to assist peer discovery
//! and WebRTC initialization inside web clients.

use std::{
    collections::HashMap,
    fmt::{self, Debug, Formatter},
    io::{Error as IoError, Write},
    str,
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
    response::{Html, IntoResponse, Response},
    routing::{any, get},
};
use flate2::{Compression, write::GzEncoder};
use futures::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use header::HOST;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayUrl, endpoint::Connection};
use syneroym_core::{
    dht_registry::{EndpointMechanism, RegistryClient},
    local_registry::EndpointRegistry,
    protocol_utils::{TargetHost, parse_target_host},
};
use syneroym_identity::substrate::resolve_did_key;
use syneroym_router::{RoutePreamble, SYNEROYM_ALPN, net_iroh::IrohStream};
use syneroym_sdk::topology::AppHostResolver;
use tokio::{
    io,
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};
use tower_http::compression::CompressionLayer;
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
    pub connection_cache: Mutex<HashMap<PublicKey, Connection>>,
    /// S3, D-S3-7/D-S3-11: resolves an app-scoped (`-a…-s…`) bootstrap
    /// host through Tier 1 and Tier 2, shared with the client gateway's
    /// implementation rather than reimplemented.
    pub app_host_resolver: AppHostResolver,
}

impl Debug for BootstrapState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
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
    /// The interface the route preamble names, already resolved from the
    /// hostname by `parse_target_host` (D-S3-16). Empty when the host
    /// carried no `-i`, which the destination resolves (D-S3-15).
    /// Interpolated rather than re-derived in the page: the page used to
    /// parse `location.hostname` itself, in two places, which made the
    /// browser a third implementation of a grammar that now lives in one
    /// function.
    target_interface: String,
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
        .layer(CompressionLayer::new())
        .with_state(state)
}

static SW_JS_GZ: OnceLock<Option<Vec<u8>>> = OnceLock::new();
static PEER_PROXY_JS_GZ: OnceLock<Option<Vec<u8>>> = OnceLock::new();

fn compress_gzip(data: &str) -> Result<Vec<u8>, IoError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data.as_bytes())?;
    encoder.finish()
}
fn serve_cached_js(
    js_content: &'static str,
    name: &str,
    cache: &OnceLock<Option<Vec<u8>>>,
) -> Response {
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

    let (mut target_peer_id, mut target_service_id, target_interface, already_resolved) =
        match parse_target_host(&host) {
            None => (host.clone(), host.clone(), String::new(), false),
            Some(TargetHost::Service { lookup_alias, interface }) => {
                (lookup_alias.clone(), lookup_alias, interface, false)
            }
            Some(TargetHost::App {
                app_lookup_alias,
                app_did_hash,
                service_name_hash,
                interface,
            }) => {
                // D-S3-11: resolve through Tier 1 -> Tier 2 -> member
                // selection, exactly as the client gateway's own
                // app-scoped path does (both D-S3-5 binding checks are
                // applied inside `resolve_app_host`, shared rather than
                // reimplemented). A failed resolve returns an error,
                // never the raw-host fallback below -- that fallback
                // would dial whatever the hostname happened to spell.
                let member_did = match state
                    .app_host_resolver
                    .resolve_app_host(&app_lookup_alias, &app_did_hash, &service_name_hash, None)
                    .await
                {
                    Ok(m) => m,
                    Err(e) => {
                        error!("coordinator failed to resolve app-scoped host '{host}': {e:#}");
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                };
                // Tier 3, exactly as the physical path already does it:
                // the member DID's own endpoint record names the
                // substrate hosting it. Already resolved -- the alias
                // lookup below is for the physical path's own alias,
                // which `member_did` already is not.
                match state.registry_client.lookup(&member_did, true).await {
                    Ok(rec) => (rec.info.substrate_id, member_did, interface, true),
                    Err(e) => {
                        error!(
                            "coordinator failed to resolve Tier 3 for member '{member_did}': {e:#}"
                        );
                        return StatusCode::BAD_GATEWAY.into_response();
                    }
                }
            }
        };

    if !already_resolved && state.registry_url.is_some() {
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
        target_interface,
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
) -> Option<(IrohStream, Connection)> {
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
    if let Some(ref conn) = connection
        && let Some(err) = conn.close_reason()
    {
        debug!("[BlindTunnel] Cached connection is closed ({err:?}), discarding");
        cache.remove(&peer_id);
        connection = None;
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
                        str::from_utf8(SYNEROYM_ALPN).unwrap_or("<invalid>")
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
    connection: Connection,
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

    // ── S3: app-scoped bootstrap resolution (tests 93-96) ───────────────

    use std::{
        collections::VecDeque,
        sync::{Mutex as StdMutex, atomic::AtomicUsize},
    };

    use http_body_util::BodyExt;
    use syneroym_app_orchestration::{
        AppDid, AppInstanceId, LogicalResolver, LogicalServiceName, ServiceId,
        SignedTopologyDocument, StaticInventory, TopologyDocument, TopologyEpoch, TopologyMode,
    };
    use syneroym_community_registry::EcosystemRegistry;
    use syneroym_core::{
        config::{ServiceRegistryRole, SubstrateConfig},
        dht_registry::{EndpointInfo, EndpointType, SignedEndpointInfo},
        storage::MockStorage,
        util,
    };
    use syneroym_identity::{Identity, substrate::derive_did_key};
    use syneroym_router::net_iroh;
    use syneroym_sdk::topology::{AppHostResolver, Tier1Lookup, Tier2Fetch};
    use tower::ServiceExt;

    #[derive(Debug)]
    struct FakeTier1 {
        response: SignedEndpointInfo,
    }

    #[async_trait::async_trait]
    impl Tier1Lookup for FakeTier1 {
        async fn lookup(&self, _alias: &str) -> anyhow::Result<SignedEndpointInfo> {
            Ok(self.response.clone())
        }
    }

    #[derive(Debug, Default)]
    struct FakeTier2 {
        calls: AtomicUsize,
        responses: StdMutex<VecDeque<SignedTopologyDocument>>,
    }

    #[async_trait::async_trait]
    impl Tier2Fetch for FakeTier2 {
        async fn fetch_via(
            &self,
            _supervisor_did: &str,
            _app_did: &AppDid,
            _service_name: &LogicalServiceName,
        ) -> anyhow::Result<SignedTopologyDocument> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("FakeTier2 has no more queued responses"))
        }
    }

    /// Spins up a real `syneroym-community-registry` HTTP server on
    /// `127.0.0.1:0` -- Tier 3 (`state.registry_client.lookup`) is a real
    /// `RegistryClient`, not a seam, so testing the coordinator's own
    /// resolution end to end needs a real registry behind it, the same
    /// shape the milestone's e2e tests already use. Returns the server's
    /// URL; the caller must keep the returned `EcosystemRegistry` alive
    /// for the test's duration.
    async fn spawn_test_registry(base: &std::path::Path) -> (String, EcosystemRegistry) {
        let mut config = SubstrateConfig {
            app_local_data_dir: base.join("data"),
            app_data_dir: base.join("user_data"),
            ..Default::default()
        };
        config.substrate.enable_bep0044_dht = false;
        config.roles.community_registry = Some(ServiceRegistryRole {
            http_bind_address: "127.0.0.1:0".to_string(),
            ..Default::default()
        });
        let mut registry = EcosystemRegistry::init(&config).await.unwrap();
        let url = registry.bind().await.unwrap();
        registry.spawn().await.unwrap();
        (url, registry)
    }

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    async fn test_state(
        registry_url: Option<String>,
        tier1: FakeTier1,
        fetcher: Option<FakeTier2>,
    ) -> Arc<BootstrapState> {
        let iroh = net_iroh::build_iroh_endpoint(None, None, None).await.unwrap();
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
        let registry_client = RegistryClient::new(false, registry_url.clone());
        let app_host_resolver = AppHostResolver::new(
            Box::new(tier1),
            fetcher.map(|f| Box::new(f) as Box<dyn Tier2Fetch>),
            LogicalResolver::new(Arc::new(StaticInventory::new())),
        );
        Arc::new(BootstrapState {
            iroh,
            external_host: None,
            signaling_port: 0,
            registry,
            registry_url,
            registry_client,
            connection_cache: Mutex::new(HashMap::new()),
            app_host_resolver,
        })
    }

    async fn body_text(response: axum::response::Response) -> String {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).to_string()
    }

    /// Registers a member's own endpoint record, plus a real substrate
    /// record for the node hosting it, in the real test registry -- Tier
    /// 3's `resolve: true` lookup recurses into `substrate_id` to fetch
    /// its mechanisms, so that value must itself be a resolvable record,
    /// not a placeholder string. Returns `(member_did, substrate_did)`.
    async fn register_member(registry_url: &str) -> (String, String) {
        let client = RegistryClient::new(false, Some(registry_url.to_string()));

        let substrate_identity = Identity::generate().unwrap();
        let substrate_did = derive_did_key(&substrate_identity.public_key());
        let substrate_record = EndpointInfo {
            service_id: substrate_did.clone(),
            substrate_id: substrate_did.clone(),
            endpoint_type: EndpointType::Substrate,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            not_after: now_secs() + 3600,
            generation: 0,
        }
        .sign(&substrate_identity)
        .unwrap();
        client.register(&substrate_record, false).await.unwrap();

        let member_identity = Identity::generate().unwrap();
        let member_did = derive_did_key(&member_identity.public_key());
        let member_record = EndpointInfo {
            service_id: member_did.clone(),
            substrate_id: substrate_did.clone(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: None,
            is_private: false,
            ttl: None,
            not_after: now_secs() + 3600,
            generation: 0,
        }
        .sign(&member_identity)
        .unwrap();
        client.register(&member_record, false).await.unwrap();

        (member_did, substrate_did)
    }

    fn app_signed_topology_doc(
        app_did: &AppDid,
        master: &Identity,
        member_did: &str,
    ) -> SignedTopologyDocument {
        let now = now_secs();
        let doc = TopologyDocument {
            app_instance_id: AppInstanceId::new("my-chat-app"),
            app_did: app_did.clone(),
            service_name: LogicalServiceName::new("backend"),
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new(member_did.to_string())],
            sharding_strategy: None,
            epoch: TopologyEpoch(1),
            generation: 0,
            issued_at: now,
            not_after: now + 3600,
            cache_ttl_ms: 60_000,
        };
        doc.sign(master).unwrap()
    }

    /// Test 93: an axum-level test over `app(state)`, asserting
    /// `TARGET_SERVICE_ID` in the rendered HTML is a member DID from the
    /// document and `TARGET_PEER_ID` is the substrate hosting it.
    #[tokio::test]
    async fn an_app_scoped_bootstrap_request_renders_the_resolved_member_did() {
        let dir = tempfile::tempdir().unwrap();
        let (registry_url, _registry) = spawn_test_registry(dir.path()).await;

        let app_master = Identity::generate().unwrap();
        let app_did = AppDid::new(derive_did_key(&app_master.public_key()));
        let a_hash = util::short_hash(app_did.as_str());
        let s_hash = util::short_hash("backend");

        let (member_did, substrate_did) = register_member(&registry_url).await;

        let tier1_record = SignedEndpointInfo {
            info: EndpointInfo {
                service_id: app_did.as_str().to_string(),
                substrate_id: "did:key:zSupervisor".to_string(),
                endpoint_type: EndpointType::Service,
                mechanisms: vec![],
                nickname: Some("my-chat-app".to_string()),
                is_private: false,
                ttl: None,
                not_after: now_secs() + 3600,
                generation: 0,
            },
            pkarr_packet_hex: String::new(),
        };
        let doc = app_signed_topology_doc(&app_did, &app_master, &member_did);
        let fetcher = FakeTier2::default();
        fetcher.responses.lock().unwrap().push_back(doc);
        let state =
            test_state(Some(registry_url), FakeTier1 { response: tier1_record }, Some(fetcher))
                .await;

        let host = format!("my-chat-app-a{a_hash}-s{s_hash}.localhost");
        let req = Request::builder().uri("/").header(HOST, host).body(Body::empty()).unwrap();
        let response = app(state).oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_text(response).await;
        assert!(body.contains(&format!("TARGET_SERVICE_ID = \"{member_did}\"")), "{body}");
        assert!(body.contains(&format!("TARGET_PEER_ID = \"{substrate_did}\"")), "{body}");
    }

    /// Test 94: D-S3-16 -- a host with an explicit `-i` renders that hash
    /// into `TARGET_INTERFACE`, and one without renders the empty string.
    /// Without this, the JS deletion is unverified and the regression
    /// §0.1 describes (every host silently losing its interface) has no
    /// failing test anywhere.
    #[tokio::test]
    async fn the_rendered_page_carries_the_hosts_interface() {
        let dir = tempfile::tempdir().unwrap();
        let (registry_url, _registry) = spawn_test_registry(dir.path()).await;

        let app_master = Identity::generate().unwrap();
        let app_did = AppDid::new(derive_did_key(&app_master.public_key()));
        let a_hash = util::short_hash(app_did.as_str());
        let s_hash = util::short_hash("backend");
        let i_hash = util::short_hash("default");

        let (member_did, _substrate_did) = register_member(&registry_url).await;

        let tier1_record = SignedEndpointInfo {
            info: EndpointInfo {
                service_id: app_did.as_str().to_string(),
                substrate_id: "did:key:zSupervisor".to_string(),
                endpoint_type: EndpointType::Service,
                mechanisms: vec![],
                nickname: Some("my-chat-app".to_string()),
                is_private: false,
                ttl: None,
                not_after: now_secs() + 3600,
                generation: 0,
            },
            pkarr_packet_hex: String::new(),
        };

        // With an explicit `-i`.
        let doc = app_signed_topology_doc(&app_did, &app_master, &member_did);
        let fetcher = FakeTier2::default();
        fetcher.responses.lock().unwrap().push_back(doc);
        let state = test_state(
            Some(registry_url.clone()),
            FakeTier1 { response: tier1_record.clone() },
            Some(fetcher),
        )
        .await;
        let host = format!("my-chat-app-a{a_hash}-s{s_hash}-i{i_hash}.localhost");
        let req = Request::builder().uri("/").header(HOST, host).body(Body::empty()).unwrap();
        let response = app(state).oneshot(req).await.unwrap();
        let body = body_text(response).await;
        assert!(body.contains(&format!("TARGET_INTERFACE = \"{i_hash}\"")), "{body}");

        // Without one.
        let doc = app_signed_topology_doc(&app_did, &app_master, &member_did);
        let fetcher = FakeTier2::default();
        fetcher.responses.lock().unwrap().push_back(doc);
        let state =
            test_state(Some(registry_url), FakeTier1 { response: tier1_record }, Some(fetcher))
                .await;
        let host = format!("my-chat-app-a{a_hash}-s{s_hash}.localhost");
        let req = Request::builder().uri("/").header(HOST, host).body(Body::empty()).unwrap();
        let response = app(state).oneshot(req).await.unwrap();
        let body = body_text(response).await;
        assert!(body.contains("TARGET_INTERFACE = \"\""), "{body}");
    }

    /// Finding B4: a value ending in a backslash (reachable through the
    /// deliberately permissive `-i` segment, D-S3-12) must not be able to
    /// escape the JS string literal it is rendered into. Constructed
    /// directly against `PeerProxyTemplate` rather than through a real
    /// hostname, since the point is the template's own escaping, not the
    /// parser -- `parse_target_host` never rejects this value, so the
    /// template is the only remaining backstop. Before the fix (hand-
    /// written quotes around askama's default HTML escaper), the rendered
    /// script read `const TARGET_INTERFACE = "a\";alert(1);//";`, letting
    /// the closing quote escape and the rest run as script.
    #[test]
    fn a_value_ending_in_a_backslash_cannot_escape_its_js_string_literal() {
        let hostile = r#"a\";alert(1);//"#;
        let tpl = PeerProxyTemplate {
            target_peer_id: "did:key:zPeer".to_string(),
            target_service_id: "did:key:zService".to_string(),
            signaling_server_url: "ws://localhost/ws".to_string(),
            http_version: "HTTP/1.1".to_string(),
            target_pubkey_hex: "deadbeef".to_string(),
            target_interface: hostile.to_string(),
        };
        let html = tpl.render().unwrap();

        // The naive, unescaped concatenation a regression would produce.
        assert!(
            !html.contains(&format!("TARGET_INTERFACE = \"{hostile}\"")),
            "the hostile value must not appear unescaped: {html}"
        );
        // What must appear instead is a complete, self-quoting JSON
        // string literal for the constant -- round-tripping it back
        // through a JSON parser is the actual proof the escaping is
        // correct, not just different.
        let line = html
            .lines()
            .find(|l| l.trim_start().starts_with("const TARGET_INTERFACE"))
            .expect("TARGET_INTERFACE line");
        let literal = line
            .trim_start()
            .strip_prefix("const TARGET_INTERFACE = ")
            .unwrap()
            .trim_end_matches(';');
        let decoded: String = serde_json::from_str(literal).unwrap();
        assert_eq!(decoded, hostile);
    }

    /// Test 95: the no-regression half, on the same handler -- an
    /// unscoped bootstrap request is unchanged.
    #[tokio::test]
    async fn an_unscoped_bootstrap_request_is_unchanged() {
        let dummy_tier1 = SignedEndpointInfo {
            info: EndpointInfo {
                service_id: "did:key:zUnused".to_string(),
                substrate_id: "did:key:zUnused".to_string(),
                endpoint_type: EndpointType::Service,
                mechanisms: vec![],
                nickname: None,
                is_private: false,
                ttl: None,
                not_after: now_secs() + 3600,
                generation: 0,
            },
            pkarr_packet_hex: String::new(),
        };
        let state = test_state(None, FakeTier1 { response: dummy_tier1 }, None).await;

        let sh = util::short_hash("did:key:zSvc");
        let ih = util::short_hash("default");
        let host = format!("my-svc-s{sh}-i{ih}.localhost");
        let req = Request::builder().uri("/").header(HOST, &host).body(Body::empty()).unwrap();
        let response = app(state).oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_text(response).await;
        assert!(body.contains(&format!("TARGET_SERVICE_ID = \"my-svc-{sh}\"")), "{body}");
        assert!(body.contains(&format!("TARGET_INTERFACE = \"{ih}\"")), "{body}");
    }

    /// Test 96: the phase-4c refusal -- 502, and the raw hostname never
    /// reaches `resolve_did_key`.
    #[tokio::test]
    async fn an_app_scoped_host_that_fails_to_resolve_does_not_fall_back_to_the_raw_host() {
        let dummy_tier1 = SignedEndpointInfo {
            info: EndpointInfo {
                service_id: "did:key:zUnused".to_string(),
                substrate_id: "did:key:zUnused".to_string(),
                endpoint_type: EndpointType::Service,
                mechanisms: vec![],
                nickname: None,
                is_private: false,
                ttl: None,
                not_after: now_secs() + 3600,
                generation: 0,
            },
            pkarr_packet_hex: String::new(),
        };
        // No fetcher configured -- `resolve_app_host` fails immediately
        // ("no community registry configured"), exactly D-S3-6's
        // uncredentialed case.
        let state = test_state(None, FakeTier1 { response: dummy_tier1 }, None).await;

        let a_hash = util::short_hash("did:key:zApp");
        let s_hash = util::short_hash("backend");
        let host = format!("my-chat-app-a{a_hash}-s{s_hash}.localhost");
        let req = Request::builder().uri("/").header(HOST, &host).body(Body::empty()).unwrap();
        let response = app(state).oneshot(req).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = body_text(response).await;
        assert!(!body.contains(&host), "must not fall back to the raw host: {body}");
    }
}

use super::*;

pub(super) async fn handle_tunnel_upgrade(
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

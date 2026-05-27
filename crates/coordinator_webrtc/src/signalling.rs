//! WebRTC Signaling Server
//!
//! Implements the WebRTC signaling logic over WebSocket, helping peers
//! exchange SDP offers/answers and ICE candidates to establish direct connections.

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use futures::{sink::SinkExt, stream::StreamExt};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

struct SignallingState {
    // Map of connected peers: PeerID -> Tx channel
    peers: Mutex<HashMap<String, broadcast::Sender<String>>>,
}

pub async fn start(listener: tokio::net::TcpListener) -> anyhow::Result<()> {
    let state = Arc::new(SignallingState { peers: Mutex::new(HashMap::new()) });

    let app = Router::new().route("/ws", get(ws_handler)).with_state(state);

    info!("Signaling server listening on {}", listener.local_addr()?);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<SignallingState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: Arc<SignallingState>) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Perform a simple handshake: wait for {"type": "register", "id": "my-id"}
    let peer_id = if let Some(Ok(Message::Text(text))) = ws_stream.next().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if v["type"] == "register" {
                v["id"].as_str().map(std::string::ToString::to_string)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let peer_id = if let Some(id) = peer_id {
        id
    } else {
        warn!("Signaling client did not register correctly. Closing.");
        return;
    };

    info!("Peer registered in signaling: {}", peer_id);

    let (send_ch, mut rcv_ch) = broadcast::channel(100);

    {
        let mut peers = match state.peers.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        peers.insert(peer_id.clone(), send_ch.clone());
    }

    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rcv_ch.recv().await {
            if ws_sink.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_stream.next().await {
        if let Message::Text(text) = msg {
            debug!("Signaling message from {}: {}", peer_id, text);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
                && let Some(target) = v.get("target").and_then(|t| t.as_str())
            {
                let peers = match state.peers.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                if let Some(target_tx) = peers.get(target) {
                    let _ = target_tx.send(text.to_string());
                } else {
                    warn!("Target peer {} not found for signaling from {}", target, peer_id);
                }
            }
        }
    }

    // Cleanup
    send_task.abort();
    {
        let mut peers = match state.peers.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        peers.remove(&peer_id);
    }
    info!("Peer disconnected from signaling: {}", peer_id);
}

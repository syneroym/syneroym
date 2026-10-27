//! WebRTC transport for the connection router.
//!
//! The node registers with the signaling server and answers each offer it
//! receives with a new peer connection. Every data channel the browser opens
//! on that connection becomes one byte stream for the route handler.

use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use anyhow::Result;
use async_trait::async_trait;
use bytes::BytesMut;
use futures::{SinkExt, StreamExt, stream::SplitSink};
use rtc::ice::mdns::MulticastDnsMode;
use syneroym_core::config::WebRtcParentConfig;
use tokio::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf},
    net::TcpStream,
    sync::Notify,
    time,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};
use tracing::{debug, error, info, warn};
use webrtc::{
    data_channel::{DataChannel, DataChannelEvent},
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
        RTCIceGatheringState, RTCIceServer, RTCPeerConnectionState, RTCSessionDescription,
        SettingEngineBuilder,
    },
};

use crate::{route_handler::RouteHandler, stop_signal::StopSignal};

/// How long an answer waits for local ICE candidates. After this it is sent
/// with the candidates gathered so far.
const ICE_GATHER_LIMIT: Duration = Duration::from_secs(5);

/// Size of one data channel message the bridge sends. The SCTP layer accepts
/// up to 256 KiB per message; browsers send at most 16 KiB.
const SEND_CHUNK: usize = 64 * 1024;

/// How long a closing data channel waits for its queued messages to be
/// acknowledged, and how often it checks.
const DRAIN_LIMIT: Duration = Duration::from_secs(10);
const DRAIN_POLL: Duration = Duration::from_millis(5);

/// The signaling WebSocket's write half, as returned by
/// `tokio_tungstenite::connect_async` for a plain client request.
type SignalingSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// Starts the signaling client in the background. A signaling failure is
/// logged and ends WebRTC service for this node; other transports continue.
pub fn spawn_signaling(
    service_id: String,
    config: &WebRtcParentConfig,
    route_handler: RouteHandler,
) {
    let signaling_url = config.signaling_url.clone();
    let ice_servers: Vec<RTCIceServer> = config
        .stun_servers
        .iter()
        .map(|url| RTCIceServer { urls: vec![url.clone()], ..Default::default() })
        .collect();
    tokio::spawn(async move {
        if let Err(e) =
            connect_signaling(&service_id, &signaling_url, &ice_servers, &route_handler).await
        {
            error!("WebRTC Signaling client error connecting to {}: {:?}", signaling_url, e);
        }
    });
}

async fn connect_signaling(
    peer_id: &str,
    url: &str,
    ice_servers: &[RTCIceServer],
    route_handler: &RouteHandler,
) -> Result<()> {
    info!("Connecting to signaling server at {}", url);
    let (ws_stream, _) = tokio_tungstenite::connect_async(url).await?;
    let (mut write, mut read) = ws_stream.split();

    let register_msg = serde_json::json!({ "type": "register", "id": peer_id });
    write.send(Message::Text(register_msg.to_string().into())).await?;
    info!("Registered with signaling server as {}", peer_id);

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                error!("WebSocket error: {:?}", e);
                break;
            }
        };
        let Message::Text(text) = msg else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        match v["type"].as_str() {
            Some("offer") => {
                handle_offer_message(&v, peer_id, ice_servers, route_handler, &mut write).await?;
            }
            Some(other) => debug!("Unhandled signaling message: {}", other),
            None => {}
        }
    }

    Ok(())
}

/// Answers one `"offer"` signaling message with a new peer connection. A
/// missing `sdp` field or a full connection cap skips the message. Any other
/// error ends the signaling connection.
async fn handle_offer_message(
    v: &serde_json::Value,
    peer_id: &str,
    ice_servers: &[RTCIceServer],
    route_handler: &RouteHandler,
    write: &mut SignalingSink,
) -> Result<()> {
    debug!("Received Offer from {:?}", v["sender"]);
    let Some(sdp) = v["sdp"].as_str() else {
        return Ok(());
    };
    let sender_id = v["sender"].as_str().unwrap_or("unknown").to_string();

    let Some(slot) = route_handler.acquire_connection_slot() else {
        warn!("[WebRTC] Rejecting connection from {} due to connection cap", sender_id);
        return Ok(());
    };

    let handler = Arc::new(PeerHandler {
        route_handler: route_handler.clone(),
        gathered: Notify::new(),
        ended: Notify::new(),
    });
    // Remote mDNS candidates are dropped and local host candidates use IPs,
    // so the node never binds the mDNS port.
    let setting_engine =
        SettingEngineBuilder::new().with_multicast_dns_mode(MulticastDnsMode::Disabled).build();
    let pc = PeerConnectionBuilder::new()
        .with_configuration(
            RTCConfigurationBuilder::new().with_ice_servers(ice_servers.to_vec()).build(),
        )
        .with_setting_engine(setting_engine)
        .with_handler(handler.clone())
        .with_udp_addrs(vec!["0.0.0.0:0"])
        .build()
        .await?;

    let answer = match answer_offer(&pc, sdp, &handler).await {
        Ok(answer) => answer,
        Err(e) => {
            let _ = pc.close().await;
            return Err(e);
        }
    };

    let answer_msg = serde_json::json!({
        "type": "answer",
        "target": sender_id,
        "sender": peer_id,
        "sdp": answer.sdp
    });
    write.send(Message::Text(answer_msg.to_string().into())).await?;
    info!("Sent Answer to {}", sender_id);

    // The peer connection and its connection slot live until the connection
    // fails or closes.
    tokio::spawn(async move {
        handler.ended.notified().await;
        if let Err(e) = pc.close().await {
            error!("Failed to close PeerConnection: {}", e);
        }
        drop(slot);
    });
    Ok(())
}

/// Applies the offer and returns the answer with the local ICE candidates
/// gathered within `ICE_GATHER_LIMIT`. Candidates are not sent separately, so
/// they must be in the answer.
async fn answer_offer(
    pc: &impl PeerConnection,
    sdp: &str,
    handler: &PeerHandler,
) -> Result<RTCSessionDescription> {
    pc.set_remote_description(RTCSessionDescription::offer(sdp.to_string())?).await?;
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer.clone()).await?;
    if time::timeout(ICE_GATHER_LIMIT, handler.gathered.notified()).await.is_err() {
        debug!("ICE gathering not complete after {:?}; answering anyway", ICE_GATHER_LIMIT);
    }
    Ok(pc.local_description().await.unwrap_or(answer))
}

/// Receives the events of one peer connection.
struct PeerHandler {
    route_handler: RouteHandler,
    /// Notified when local ICE gathering is complete.
    gathered: Notify,
    /// Notified when the connection fails, disconnects or closes.
    ended: Notify,
}

#[async_trait]
impl PeerConnectionEventHandler for PeerHandler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            self.gathered.notify_one();
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        info!("WebRTC Peer Connection State has changed: {}", state);
        if matches!(
            state,
            RTCPeerConnectionState::Failed
                | RTCPeerConnectionState::Disconnected
                | RTCPeerConnectionState::Closed
        ) {
            self.ended.notify_one();
        }
    }

    async fn on_data_channel(&self, channel: Arc<dyn DataChannel>) {
        tokio::spawn(serve_data_channel(channel, self.route_handler.clone()));
    }
}

/// Hands an open data channel to the route handler as a byte stream. The
/// browser opens a channel named `_init` only to start ICE; it carries no
/// stream.
async fn serve_data_channel(channel: Arc<dyn DataChannel>, route_handler: RouteHandler) {
    let label = match channel.label().await {
        Ok(label) => label,
        Err(e) => {
            error!("Failed to read DataChannel label: {}", e);
            return;
        }
    };
    if label == "_init" {
        debug!("dummy WebRTC DataChannel '_init'");
        return;
    }
    info!("DataChannel '{}' open", label);
    if let Err(e) = route_handler.handle_stream(WebRTCStream::new(channel)).await {
        error!("Error handling WebRTC stream on '{}': {}", label, e);
    }
}

/// A WebRTC data channel as `tokio::io::AsyncRead` and `AsyncWrite`.
///
/// A data channel is message-oriented and has an async (not poll-based)
/// API, so a background task bridges it to a `tokio::io::duplex` stream.
#[derive(Debug)]
pub struct WebRTCStream {
    inner: DuplexStream,
}

impl WebRTCStream {
    #[must_use]
    pub fn new(channel: Arc<dyn DataChannel>) -> Self {
        let (local, remote) = io::duplex(SEND_CHUNK);
        let (remote_read, remote_write) = io::split(remote);
        tokio::spawn(async move {
            let inbound = tokio::spawn(channel_to_duplex(channel.clone(), remote_write));
            let outbound = tokio::spawn(duplex_to_channel(remote_read, channel));
            let _ = tokio::join!(inbound, outbound);
            debug!("WebRTCStream bridge tasks finished");
        });
        Self { inner: local }
    }
}

/// Copies each received message into the duplex, then signals EOF when the
/// channel closes.
async fn channel_to_duplex(
    channel: Arc<dyn DataChannel>,
    mut remote_write: io::WriteHalf<DuplexStream>,
) {
    while let Some(event) = channel.poll().await {
        match event {
            DataChannelEvent::OnMessage(msg) => {
                if let Err(e) = remote_write.write_all(&msg.data).await {
                    debug!("WebRTCStream bridge: failed to write to duplex: {}", e);
                    break;
                }
            }
            DataChannelEvent::OnClose => break,
            _ => {}
        }
    }
    let _ = remote_write.shutdown().await;
}

/// Sends what the route handler writes as data channel messages, then closes
/// the channel so the browser's `onclose` fires. Without the close the
/// browser's response body never ends.
async fn duplex_to_channel(
    mut remote_read: io::ReadHalf<DuplexStream>,
    channel: Arc<dyn DataChannel>,
) {
    let mut buf = vec![0u8; SEND_CHUNK];
    loop {
        match remote_read.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = channel.send(BytesMut::from(&buf[..n])).await {
                    error!("WebRTCStream bridge: WebRTC write error: {}", e);
                    break;
                }
            }
            Err(e) => {
                debug!("WebRTCStream bridge: duplex read error: {}", e);
                break;
            }
        }
    }
    drain(channel.as_ref()).await;
    if let Err(e) = channel.close().await {
        debug!("WebRTCStream bridge: DataChannel close error: {}", e);
    }
}

/// Waits until SCTP has released every byte handed to `send`, for at most
/// `DRAIN_LIMIT`. `send` only queues a message, and `close` resets the
/// stream at once, so a message still queued then never reaches the browser.
/// An error means the channel or connection is already gone.
async fn drain(channel: &dyn DataChannel) {
    let deadline = Instant::now() + DRAIN_LIMIT;
    while matches!(channel.outstanding_bytes().await, Ok(n) if n > 0) && Instant::now() < deadline {
        time::sleep(DRAIN_POLL).await;
    }
}

impl AsyncRead for WebRTCStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for WebRTCStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// WebRTC data channels have no equivalent to QUIC's `STOP_SENDING`, so
/// this accepts the trait's default never-resolving signal.
impl StopSignal for WebRTCStream {}

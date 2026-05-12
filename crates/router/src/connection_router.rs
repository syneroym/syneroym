use anyhow::Result;
use futures::{SinkExt, StreamExt};
use iroh::endpoint::presets;
use iroh::protocol::Router as IrohRouter;
use iroh::{EndpointAddr, RelayMap, RelayMode, RelayUrl, SecretKey};
use std::sync::Arc;
use syneroym_core::config::{IrohRelayConfig, SubstrateConfig, WebRtcRelayConfig};
use syneroym_core::registry::EndpointRegistry;
use tracing::{debug, error, info};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice::mdns::MulticastDnsMode;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use crate::net_webrtc::WebRTCStream;
use crate::route_handler::RouteHandler;

pub const SYNEROYM_ALPN: &[u8] = b"syneroym/0.1";

/// The Connection Router (The Data Plane)
/// Internal traffic cop that uses the Endpoint Registry to look up
/// the destination for an incoming wRPC stream.
#[derive(Debug, Clone)]
pub struct ConnectionRouter {
    iroh_router: Option<IrohRouter>,
}

impl ConnectionRouter {
    pub async fn init(
        registry: EndpointRegistry,
        config: SubstrateConfig,
        iroh_secret_key: [u8; 32],
        service_id: String,
    ) -> Result<Self> {
        let mut router = Self { iroh_router: None };
        let route_handler =
            RouteHandler::init(service_id.clone(), &config, registry.clone()).await?;

        for comm in &config.substrate.communication_interfaces {
            match comm.as_str() {
                "iroh" => {
                    if let Some(iroh_config) = config.uplink.iroh.as_ref() {
                        info!("Initializing Iroh interface for Router...");
                        let iroh_router = router
                            .init_iroh(
                                iroh_config,
                                iroh::SecretKey::from_bytes(&iroh_secret_key),
                                route_handler.clone(),
                            )
                            .await?;
                        router.iroh_router = Some(iroh_router);
                    }
                }
                "webrtc" => {
                    if let Some(webrtc_config) = config.uplink.webrtc.as_ref() {
                        info!("Initializing WebRTC interface for Router...");
                        router
                            .init_webrtc(webrtc_config, service_id.clone(), route_handler.clone())
                            .await?;
                    }
                }
                _ => {
                    info!("Unknown or unimplemented communication interface: {}", comm);
                }
            }
        }

        Ok(router)
    }

    async fn init_iroh(
        &self,
        config: &IrohRelayConfig,
        secret_key: SecretKey,
        route_handler: RouteHandler,
    ) -> Result<IrohRouter> {
        debug!("Initializing Iroh communication...");

        let mut ep_bldr = iroh::Endpoint::builder(presets::N0);
        if let Ok(relay_url) = config.relay_url.parse::<RelayUrl>() {
            ep_bldr = iroh::Endpoint::empty_builder()
                .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
        }

        let ep_bldr = ep_bldr.secret_key(secret_key);
        let ep = ep_bldr.bind().await?;

        let iroh_router: IrohRouter =
            IrohRouter::builder(ep).accept(SYNEROYM_ALPN, route_handler).spawn();
        iroh_router.endpoint().online().await;

        info!(
            "Iroh listening on ALPN: {:?}",
            std::str::from_utf8(SYNEROYM_ALPN).unwrap_or("<invalid utf8>")
        );

        Ok(iroh_router)
    }

    async fn init_webrtc(
        &self,
        config: &WebRtcRelayConfig,
        service_id: String,
        route_handler: RouteHandler,
    ) -> Result<()> {
        let signaling_url = config.signaling_server_url.clone();

        // 2. Initialize WebRTC API
        let mut m = MediaEngine::default();
        m.register_default_codecs()?;

        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut m)?;

        let mut s = SettingEngine::default();
        s.detach_data_channels();
        s.set_ice_multicast_dns_mode(MulticastDnsMode::Disabled);

        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
            .with_setting_engine(s)
            .build();

        let rtc_config = RTCConfiguration {
            ice_servers: config
                .stun_servers
                .iter()
                .map(|url| webrtc::ice_transport::ice_server::RTCIceServer {
                    urls: vec![url.clone()],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };

        // 3. Connect to Signaling Server and handle incoming connections
        let api = Arc::new(api);
        let rtc_config = rtc_config.clone();

        tokio::spawn(async move {
            if let Err(e) =
                connect_signaling(service_id, &signaling_url, api, rtc_config, route_handler).await
            {
                error!("WebRTC Signaling client error connectiong to {}: {:?}", signaling_url, e);
            }
        });

        Ok(())
    }

    pub async fn run(&self) -> Result<()> {
        info!("running connection router");
        let endpoint = self.iroh_router.as_ref().map(|router| router.endpoint());
        if let Some(endpoint) = endpoint {
            endpoint.closed().await;
        } else {
            std::future::pending::<()>().await;
        }
        Ok(())
    }

    pub fn endpoint_addr(&self) -> Option<EndpointAddr> {
        self.iroh_router.as_ref().map(|router| router.endpoint().addr())
    }

    pub async fn shutdown(&self) -> Result<()> {
        info!("shutting down connection router");
        if let Some(router) = self.iroh_router.as_ref() {
            let ep = router.endpoint().clone();
            router.shutdown().await?;
            ep.close().await;
        }
        Ok(())
    }
}

async fn connect_signaling(
    peer_id: String,
    url: &String,
    api: Arc<webrtc::api::API>,
    config: RTCConfiguration,
    route_handler: RouteHandler,
) -> Result<()> {
    info!("Connecting to signaling server at {}", url);
    let (ws_stream, _) = tokio_tungstenite::connect_async(url).await?;
    let (mut write, mut read) = ws_stream.split();

    // Register
    let register_msg = serde_json::json!({
        "type": "register",
        "id": peer_id
    });
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(register_msg.to_string().into()))
        .await?;
    info!("Registered with signaling server as {}", peer_id);

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                error!("WebSocket error: {:?}", e);
                break;
            }
        };

        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
            let v: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let type_str = match v["type"].as_str() {
                Some(s) => s,
                None => continue,
            };

            match type_str {
                "offer" => {
                    debug!("Received Offer from {:?}", v["sender"]);
                    let sdp = match v["sdp"].as_str() {
                        Some(s) => s,
                        None => continue,
                    };

                    let sender_id = v["sender"].as_str().unwrap_or("unknown").to_string();
                    let peer_id = peer_id.clone();
                    let api = api.clone();
                    let config = config.clone();
                    let route_handler = route_handler.clone();

                    // Create new PeerConnection
                    let pc = Arc::new(api.new_peer_connection(config.clone()).await?);

                    // Set Data Channel handler
                    let rh = route_handler.clone();
                    pc.on_data_channel(Box::new(move |d: Arc<RTCDataChannel>| {
                        let rh = rh.clone();
                        Box::pin(async move {
                            handle_data_channel(d, rh).await;
                        })
                    }));

                    let pc_clone = pc.clone();
                    pc.on_peer_connection_state_change(Box::new(
                        move |s: RTCPeerConnectionState| {
                            info!("WebRTC Peer Connection State has changed: {}", s);
                            if s == RTCPeerConnectionState::Failed
                                || s == RTCPeerConnectionState::Disconnected
                            {
                                let pc = pc_clone.clone();
                                Box::pin(async move {
                                    if let Err(e) = pc.close().await {
                                        error!("Failed to close PeerConnection: {}", e);
                                    }
                                })
                            } else {
                                Box::pin(async {})
                            }
                        },
                    ));

                    // Set Remote Description
                    let desc = RTCSessionDescription::offer(sdp.to_string())?;
                    pc.set_remote_description(desc).await?;

                    // Create Answer
                    let answer = pc.create_answer(None).await?;
                    pc.set_local_description(answer.clone()).await?;

                    // Send Answer back
                    let answer_msg = serde_json::json!({
                        "type": "answer",
                        "target": sender_id,
                        "sender": peer_id,
                        "sdp": answer.sdp
                    });
                    write
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            answer_msg.to_string().into(),
                        ))
                        .await?;
                    info!("Sent Answer to {}", sender_id);
                }
                _ => {
                    debug!("Unhandled signaling message: {}", type_str);
                }
            }
        }
    }

    Ok(())
}

async fn handle_data_channel(d: Arc<RTCDataChannel>, route_handler: RouteHandler) {
    let d_label = d.label().to_owned();
    info!("New DataChannel {}", d_label);

    let d2 = d.clone();
    d.on_open(Box::new(move || {
        let d = d2.clone();
        let d_label = d_label.clone();
        let rh = route_handler.clone();
        Box::pin(async move {
            info!("DataChannel '{}' open", d_label);

            match d.detach().await {
                Ok(rtc_detached) => {
                    debug!("DataChannel '{}' detached successfully", d_label);
                    let rtc_stream = WebRTCStream::new(rtc_detached);

                    if let Err(e) = rh.handle_stream(rtc_stream).await {
                        error!("Error handling WebRTC stream on '{}': {}", d_label, e);
                    }
                }
                Err(e) => {
                    error!("Failed to detach DataChannel '{}': {}", d_label, e);
                }
            }
        })
    }));
}

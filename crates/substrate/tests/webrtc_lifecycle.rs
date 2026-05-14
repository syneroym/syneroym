use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::handler::viewport::Viewport;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use syneroym_coordinator_webrtc::CoordinatorWebRtc;
use syneroym_core::config::{CoordinatorRole, CoordinatorWebRtcConfig, SubstrateConfig};
use syneroym_core::registry::EndpointRegistry;
use syneroym_router::ConnectionRouter;
use syneroym_router::net_webrtc::WebRTCStream;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{debug, info};
use webrtc::api::APIBuilder;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

const SIGNALLING_PORT: u16 = 7557;
const BOOTSTRAP_PORT: u16 = 7113;

#[tokio::test]
async fn test_webrtc_lifecycle() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let temp_dir = tempdir()?;
    let mut config = SubstrateConfig {
        app_local_data_dir: temp_dir.path().join("data"),
        app_data_dir: temp_dir.path().join("user_data"),
        ..SubstrateConfig::default()
    };
    config.resolve_paths();

    // 1. Start Coordinator
    config.roles.coordinator = Some(CoordinatorRole {
        webrtc: Some(CoordinatorWebRtcConfig {
            enable_signalling: true,
            signalling_bind_address: format!("0.0.0.0:{}", SIGNALLING_PORT),
            bootstrap_page_bind_address: format!("0.0.0.0:{}", BOOTSTRAP_PORT),
            ..Default::default()
        }),
        ..Default::default()
    });

    let mut coordinator = CoordinatorWebRtc::init(&config).await?;
    let coordinator_endpoint = coordinator.endpoint();
    let coord_handle = tokio::spawn(async move {
        coordinator.run().await.expect("Coordinator failed");
    });

    // 2. Start Substrate Router with WebRTC enabled
    let data_store = syneroym_core::storage::init_store(&config).await?;
    let registry = EndpointRegistry::new(data_store).await?;
    let service_id = "test-substrate".to_string();

    config.substrate.communication_interfaces = vec!["webrtc".to_string()];
    config.uplink.webrtc = Some(syneroym_core::config::WebRtcRelayConfig {
        signaling_server_url: format!("ws://localhost:{}/ws", SIGNALLING_PORT),
        ..Default::default()
    });

    // Register a mock service
    registry
        .register(
            "orchestrator".to_string(),
            "health".to_string(),
            syneroym_core::registry::SubstrateEndpoint::NativeHostChannel {
                service_id: "orchestrator".to_string(),
            },
        )
        .await?;

    let router =
        ConnectionRouter::init(registry.clone(), config.clone(), [0; 32], service_id.clone())
            .await?;
    let router_clone = router.clone();
    let router_handle = tokio::spawn(async move {
        router_clone.run().await.expect("Router failed");
    });

    // Wait for registration
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // 3. Emulate Browser Client
    let signalling_url = format!("ws://localhost:{}/ws", SIGNALLING_PORT);
    let (ws_stream, _) = tokio_tungstenite::connect_async(&signalling_url).await?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let client_id = "browser-client".to_string();
    ws_write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({"type": "register", "id": client_id}).to_string().into(),
        ))
        .await?;

    // Create WebRTC Connection with detach enabled
    let mut s = SettingEngine::default();
    s.detach_data_channels();
    let api = APIBuilder::new().with_setting_engine(s).build();
    let pc = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

    let dc = pc.create_data_channel("test-data", None).await?;
    let (offer_done_tx, mut offer_done_rx) = tokio::sync::mpsc::channel(1);

    let pc_clone = pc.clone();
    tokio::spawn(async move {
        let offer = pc_clone.create_offer(None).await.unwrap();
        pc_clone.set_local_description(offer.clone()).await.unwrap();

        // Wait for ICE gathering
        let mut gather_finished = pc_clone.gathering_complete_promise().await;
        let _ = gather_finished.recv().await;

        let offer = pc_clone.local_description().await.unwrap();
        offer_done_tx.send(offer.sdp).await.unwrap();
    });

    let offer_sdp = offer_done_rx.recv().await.unwrap();
    ws_write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({
                "type": "offer",
                "target": service_id,
                "sender": client_id,
                "sdp": offer_sdp
            })
            .to_string()
            .into(),
        ))
        .await?;

    // Wait for Answer
    let answer_sdp = loop {
        if let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = ws_read.next().await
        {
            let v: serde_json::Value = serde_json::from_str(&text)?;
            if v["type"] == "answer" {
                break v["sdp"].as_str().unwrap().to_string();
            }
        }
    };

    pc.set_remote_description(RTCSessionDescription::answer(answer_sdp)?).await?;

    // Wait for DataChannel to open
    let (dc_open_tx, mut dc_open_rx) = tokio::sync::mpsc::channel(1);
    dc.on_open(Box::new(move || {
        let dc_open_tx = dc_open_tx.clone();
        Box::pin(async move {
            let _ = dc_open_tx.send(()).await;
        })
    }));

    tokio::time::timeout(Duration::from_secs(5), dc_open_rx.recv()).await?.unwrap();
    debug!("DataChannel opened in test");

    // 4. Send request over DataChannel
    let detached_dc = dc.detach().await?;
    let mut dc_stream = WebRTCStream::new(detached_dc);

    // Send Preamble: http://interface|service_id\n
    let preamble = "http://health|orchestrator\n";
    dc_stream.write_all(preamble.as_bytes()).await?;

    // Send HTTP Request
    let req = "POST /v1/orchestrator/health HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
    dc_stream.write_all(req.as_bytes()).await?;

    // Read Response
    let mut resp_buf = vec![0u8; 1024];
    let n = dc_stream.read(&mut resp_buf).await?;
    let resp_text = String::from_utf8_lossy(&resp_buf[..n]);
    debug!("Received response over WebRTC: {}", resp_text);

    assert!(resp_text.contains("HTTP/1.1"));

    // Cleanup
    let _ = router.shutdown().await;
    coordinator_endpoint.close().await;
    coord_handle.abort();
    router_handle.abort();

    Ok(())
}

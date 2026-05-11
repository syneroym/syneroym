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

    // Send Preamble: [1 byte len][service name]
    let target_service = "orchestrator";
    let svc_bytes = target_service.as_bytes();
    dc_stream.write_u8(svc_bytes.len() as u8).await?;
    dc_stream.write_all(svc_bytes).await?;

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

#[tokio::test]
#[ignore = "some issue with this test need to troubleshoot"]
async fn test_webrtc_browser_automation() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let temp_dir = tempdir()?;
    let mut config = SubstrateConfig {
        app_local_data_dir: temp_dir.path().join("data"),
        app_data_dir: temp_dir.path().join("user_data"),
        ..SubstrateConfig::default()
    };
    config.resolve_paths();

    // 1. Start a Mock HTTP Service
    let app = axum::Router::new()
        .fallback(axum::routing::get(|| async { "Hello from Syneroym Service" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let mock_port = listener.local_addr()?.port();
    let mock_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("Mock service failed");
    });

    // 2. Start Coordinator
    config.roles.coordinator = Some(CoordinatorRole {
        webrtc: Some(CoordinatorWebRtcConfig {
            enable_signalling: true,
            signalling_bind_address: "127.0.0.1:0".to_string(),
            bootstrap_page_bind_address: "127.0.0.1:0".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });

    let mut coordinator = CoordinatorWebRtc::init(&config).await?;
    let coordinator_endpoint = coordinator.endpoint();
    // We need to know the actual ports assigned
    let actual_signalling_port = coordinator.signalling_port();
    let actual_bootstrap_port = coordinator.bootstrap_port();

    let coord_handle = tokio::spawn(async move {
        coordinator.run().await.expect("Coordinator failed");
    });

    // 3. Start Substrate Router
    let data_store = syneroym_core::storage::init_store(&config).await?;
    let registry = EndpointRegistry::new(data_store).await?;
    let service_id = "test-service".to_string();

    config.substrate.communication_interfaces = vec!["webrtc".to_string()];
    config.uplink.webrtc = Some(syneroym_core::config::WebRtcRelayConfig {
        signaling_server_url: format!("ws://localhost:{}/ws", actual_signalling_port),
        ..Default::default()
    });

    // Register the mock service as a TCP endpoint
    registry
        .register(
            service_id.clone(),
            "http".to_string(),
            syneroym_core::registry::SubstrateEndpoint::TcpHostPort {
                host: "127.0.0.1".to_string(),
                port: mock_port,
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

    // 4. Launch Browser and Navigate
    let browser_temp_dir = tempdir()?;
    let chrome_executable = std::env::var("CHROME_PATH").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string()
        } else {
            // Default for Linux in CI
            "google-chrome".to_string()
        }
    });

    let (mut browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .chrome_executable(chrome_executable)
            .user_data_dir(browser_temp_dir.path())
            .request_timeout(Duration::from_secs(30))
            .window_size(1280, 720)
            .viewport(Viewport {
                width: 1280,
                height: 720,
                device_scale_factor: None,
                emulating_mobile: false,
                is_landscape: false,
                has_touch: false,
            })
            // --- TOGGLE HEADLESS MODE ---
            // .with_head() // Uncomment this line to show the browser window during manual troubleshooting
            .build()
            .map_err(|e| anyhow::anyhow!(e))?,
    )
    .await?;

    let handle = tokio::spawn(async move {
        while let Some(h) = handler.next().await {
            if h.is_err() {
                break;
            }
        }
    });

    let page = browser.new_page("about:blank").await?;
    page.enable_log().await?;
    page.enable_runtime().await?;
    page.execute(chromiumoxide::cdp::browser_protocol::service_worker::EnableParams::default())
        .await?;

    let mut events =
        page.event_listener::<chromiumoxide::cdp::browser_protocol::log::EventEntryAdded>().await?;
    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            info!("[Browser Log] {}", event.entry.text);
        }
    });

    let mut console_events = page
        .event_listener::<chromiumoxide::cdp::js_protocol::runtime::EventConsoleApiCalled>()
        .await?;
    tokio::spawn(async move {
        while let Some(event) = console_events.next().await {
            info!("[Browser Console] {:?}", event.args);
        }
    });

    // Navigate to the bootstrap page for our service
    let url = format!("http://localhost:{}/{}", actual_bootstrap_port, service_id);
    info!("Navigating to {}", url);
    page.goto(url).await?;

    // Wait for the WebRTC connection to establish and content to load.
    // The page reloads content when connected. We check for the mock service response text.
    let mut success = false;
    for i in 0..40 {
        let content = page.content().await?;
        if content.contains("Hello from Syneroym Service") {
            success = true;
            info!("Successfully verified WebRTC content in browser!");
            break;
        }

        if i % 5 == 0 {
            let sw_status: String = page
                .evaluate("navigator.serviceWorker.controller ? 'Active' : 'None'")
                .await?
                .into_value()?;
            let pc_status: String = page
                .evaluate(
                    "window.peerConnection ? window.peerConnection.connectionState : 'Undefined'",
                )
                .await?
                .into_value()?;
            let logs: Vec<String> = page.evaluate("window.logs || []").await?.into_value()?;
            debug!(
                "Status check {} - SW: {}, PC: {}, Content length: {}",
                i,
                sw_status,
                pc_status,
                content.len()
            );
            for log in logs {
                info!("[Browser Internal] {}", log);
            }
            // Clear logs to avoid duplicates
            page.evaluate("window.logs = []").await?;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    assert!(success, "Failed to load proxied content in browser");

    // Cleanup
    browser.close().await?;
    handle.await?;
    coordinator_endpoint.close().await;
    coord_handle.abort();
    router_handle.abort();
    mock_handle.abort();

    Ok(())
}

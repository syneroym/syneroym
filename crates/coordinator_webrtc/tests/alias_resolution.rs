#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests for WebRTC alias resolution and discovery
//!
//! Verifies that shorthash aliases and nicknames resolve correctly under
//! simulated WebRTC connections.

use axum::{Json, Router, routing::get};
use std::sync::Arc;
use syneroym_coordinator_webrtc::bootstrap::{BootstrapState, start};
use syneroym_core::community_registry::{EndpointInfo, EndpointType, SignedEndpointInfo};
use syneroym_core::registry::EndpointRegistry;
use tokio::net::TcpListener;
// use std::time::Duration;

#[tokio::test]
async fn test_bootstrap_alias_resolution() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt::try_init();

    // 1. Start Mock Registry
    let substrate_id = "did:key:hsubstrate123".to_string();
    let service_id = "did:key:hservice123".to_string();
    let nickname = "demo1".to_string();
    let alias = format!("{}-p{}", nickname, "shorthash");

    let mock_info = SignedEndpointInfo {
        info: EndpointInfo {
            service_id: service_id.clone(),
            substrate_id: substrate_id.clone(),
            endpoint_type: EndpointType::Service,
            mechanisms: vec![],
            nickname: Some(nickname.clone()),
            is_private: false,
            ttl: None,
        },
        signature: "sig".to_string(),
    };

    let app =
        Router::new().route("/lookup/{id}", get(move || async move { Json(mock_info.clone()) }));
    let registry_listener = TcpListener::bind("127.0.0.1:0").await?;
    let registry_port = registry_listener.local_addr()?.port();
    let registry_url = format!("http://127.0.0.1:{registry_port}");

    tokio::spawn(async move {
        axum::serve(registry_listener, app).await.unwrap();
    });

    // 2. Start Bootstrap Server
    let temp_dir = tempfile::tempdir()?;
    let mut config = syneroym_core::config::SubstrateConfig {
        app_local_data_dir: temp_dir.path().to_path_buf(),
        ..Default::default()
    };
    config.resolve_paths(); // Ensure db_dir is relative to temp_dir

    let data_store = syneroym_core::storage::init_store(&config).await?;
    let registry = EndpointRegistry::new(data_store).await?;

    // In a real scenario, we need an iroh endpoint, but for this test we'll just mock the state
    // and call handle_connection if it were public, or just test the start/serve logic.
    // Since handle_connection is private, we'll test by hitting the server.

    let bootstrap_listener = TcpListener::bind("127.0.0.1:0").await?;
    let bootstrap_port = bootstrap_listener.local_addr()?.port();

    // We need a dummy iroh endpoint
    let iroh_config = iroh::Endpoint::builder(iroh::endpoint::presets::N0);
    let iroh_endpoint = iroh_config.bind().await?;

    let state = Arc::new(BootstrapState {
        iroh: iroh_endpoint.clone(),
        external_host: None,
        signaling_port: 7963,
        registry,
        registry_url: Some(registry_url),
    });

    let state_clone = state.clone();
    tokio::spawn(async move {
        start(bootstrap_listener, state_clone).await.unwrap();
    });

    // 3. Hit the bootstrap server with the alias
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{bootstrap_port}");
    let resp = client.get(&url).header("Host", format!("{alias}.localhost")).send().await?;

    assert!(resp.status().is_success());
    let body = resp.text().await?;

    // 4. Verify that the rendered HTML contains the substrate DID as TARGET_PEER_ID
    assert!(body.contains(&format!("const TARGET_PEER_ID = \"{substrate_id}\"")));

    iroh_endpoint.close().await;
    Ok(())
}

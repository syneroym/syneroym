#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::time::Duration;

use anyhow::Result;
use iroh::{
    EndpointAddr, RelayUrl, SecretKey,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler, Router},
};
use reqwest::Client;
use syneroym_community_registry::EcosystemRegistry;
use syneroym_coordinator_iroh::{CoordinatorInfo, CoordinatorIroh};
use syneroym_core::config::{
    AccessControl, CoordinatorIrohConfig, CoordinatorRole, ServiceRegistryRole, SubstrateConfig,
};
use syneroym_router::net_iroh;
use tokio::io;
use tokio::time;

const ALPN: &[u8] = b"syneroym-test-idle-eviction/1";

#[derive(Debug, Clone)]
struct EchoHandler;

impl ProtocolHandler for EchoHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let _ = io::copy(&mut recv, &mut send).await;
        let _ = send.finish();
        connection.closed().await;
        Ok(())
    }
}

#[tokio::test]
async fn test_quic_idle_eviction() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();

    // 1. Spawn global registry R
    let mut config_r = SubstrateConfig {
        app_local_data_dir: base_path.join("data_r"),
        app_data_dir: base_path.join("user_data_r"),
        ..Default::default()
    };
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        access: AccessControl::String("everyone".to_string()),
        http_bind_address: "127.0.0.1:0".to_string(),
        parent_registry_url: None,
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;

    // 2. Spawn Coordinator C with a relay server
    let mut config_c = SubstrateConfig {
        app_local_data_dir: base_path.join("data_c"),
        app_data_dir: base_path.join("user_data_c"),
        ..Default::default()
    };
    config_c.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: true,
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            community_registry_url: Some(r_url.clone()),
            idle_timeout_secs: None,
            share_in_registry: true,
        }),
        ..Default::default()
    });
    let mut c = CoordinatorIroh::init(&config_c).await?;
    let c_info_addr = c.info_addr().unwrap();

    let info_client = Client::new();
    let c_info: CoordinatorInfo =
        info_client.get(format!("http://{c_info_addr}/v1/info")).send().await?.json().await?;
    let c_relay_url = c_info.relay_url.clone().unwrap();
    let relay_url = c_relay_url.parse::<RelayUrl>()?;

    // 3. Build two endpoints with idle timeout = 2 seconds
    let secret_1 = SecretKey::generate(&mut rand::rng());
    let secret_2 = SecretKey::generate(&mut rand::rng());

    let ep1 = net_iroh::build_iroh_endpoint(Some(c_relay_url.clone()), Some(secret_1), Some(2)).await?;
    let ep2 = net_iroh::build_iroh_endpoint(Some(c_relay_url.clone()), Some(secret_2), Some(2)).await?;

    // Wait for endpoints to connect to relay
    ep1.online().await;
    ep2.online().await;

    // Spawn router on ep1
    let ep1_addr = ep1.addr();
    let _router1 = Router::builder(ep1.clone()).accept(ALPN, EchoHandler).spawn();

    // 4. Connect ep2 to ep1
    let peer_addr = EndpointAddr::new(ep1_addr.id).with_relay_url(relay_url);
    let conn = ep2.connect(peer_addr, ALPN).await?;

    // Open a stream to make sure connection actually works
    {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(b"hello").await?;
        send.finish()?;
        let response = recv.read_to_end(100).await?;
        assert_eq!(response, b"hello");
    }

    // Now wait for 4 seconds without any activity (longer than the 2s timeout)
    time::sleep(Duration::from_secs(4)).await;

    // The connection should be evicted / closed by now.
    // Let's verify by waiting for closed with a short timeout, or trying to open a stream and expecting error.
    let close_result = time::timeout(Duration::from_secs(1), conn.closed()).await;
    assert!(close_result.is_ok(), "Connection should have closed due to idle timeout");

    // Cleanup
    c.shutdown().await?;

    Ok(())
}

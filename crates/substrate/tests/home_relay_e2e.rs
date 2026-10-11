#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests verifying dynamic home relay switching, confirmation,
//! rollback on failure, and deferred registry publication.

use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use iroh::RelayUrl;
use iroh_relay::server::{AccessConfig, RelayConfig as ServerRelayConfig, Server, ServerConfig};
use syneroym_core::dht_registry::{EndpointMechanism, RegistryClient, SignedEndpointInfo};
use tokio::time;

use crate::common::SubstrateNode;

mod common;

/// Spawns a plain-HTTP relay server on a dynamic local port.
async fn spawn_relay() -> Result<(Server, RelayUrl)> {
    let http_bind_addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let relay_config = ServerRelayConfig {
        http_bind_addr,
        tls: None,
        limits: Default::default(),
        key_cache_capacity: Some(1024),
        access: AccessConfig::Everyone,
    };
    let server_config =
        ServerConfig::<(), ()> { relay: Some(relay_config), quic: None, ..Default::default() };
    let server = Server::spawn(server_config).await.context("failed to spawn relay server")?;
    let http_addr = server.http_addr().context("relay server has no HTTP address")?;
    let url: RelayUrl =
        format!("http://{http_addr}").parse().context("failed to parse relay url")?;
    Ok((server, url))
}

fn get_iroh_relay_url(record: &SignedEndpointInfo) -> Option<String> {
    for mechanism in &record.info.mechanisms {
        if let EndpointMechanism::Iroh { relay_url, .. } = mechanism {
            return relay_url.clone();
        }
    }
    None
}

async fn wait_for_registry_relay(
    client: &RegistryClient,
    did: &str,
    expected_relay: &str,
    timeout: Duration,
) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Ok(rec) = client.lookup(did, false).await
            && get_iroh_relay_url(&rec).as_deref() == Some(expected_relay)
        {
            return Ok(());
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("timed out waiting for registry record to list relay {expected_relay}")
}

#[tokio::test]
async fn substrate_switches_home_relay_when_watch_channel_updates() -> Result<()> {
    let _serial_guard = common::serial_guard().await;

    let (_server_a, relay_a_url) = spawn_relay().await?;
    let node = SubstrateNode::builder().shared_relay(relay_a_url.to_string()).boot().await;

    let registry_client = RegistryClient::new(false, Some(node.registry_url().to_string()));
    let record = registry_client.lookup(node.did(), false).await?;
    assert_eq!(get_iroh_relay_url(&record), Some(relay_a_url.to_string()));

    let (_server_b, relay_b_url) = spawn_relay().await?;
    node.home_relay().set(relay_b_url.clone()).await?;

    assert_eq!(node.home_relay().current_url(), Some(relay_b_url.clone()));
    wait_for_registry_relay(
        &registry_client,
        node.did(),
        relay_b_url.as_str(),
        Duration::from_secs(10),
    )
    .await?;

    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn substrate_rolls_back_when_candidate_relay_unreachable() -> Result<()> {
    let _serial_guard = common::serial_guard().await;

    let (_server_a, relay_a_url) = spawn_relay().await?;
    let node = SubstrateNode::builder().shared_relay(relay_a_url.to_string()).boot().await;

    let registry_client = RegistryClient::new(false, Some(node.registry_url().to_string()));
    let record = registry_client.lookup(node.did(), false).await?;
    assert_eq!(get_iroh_relay_url(&record), Some(relay_a_url.to_string()));

    let [bad_port] = common::alloc_ports::<1>();
    common::release_held_ports(&[bad_port]);
    let unreachable_url: RelayUrl = format!("http://127.0.0.1:{bad_port}").parse()?;

    let switch_result = node.home_relay().set(unreachable_url).await;
    assert!(switch_result.is_err(), "expected unreachable relay switch to fail");

    assert_eq!(node.home_relay().current_url(), Some(relay_a_url.clone()));

    let current_record = registry_client.lookup(node.did(), false).await?;
    assert_eq!(get_iroh_relay_url(&current_record), Some(relay_a_url.to_string()));

    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn substrate_without_home_relay_skips_registration_until_relay_configured() -> Result<()> {
    let _serial_guard = common::serial_guard().await;

    let node = SubstrateNode::builder()
        .configure(|cfg| {
            cfg.parent_coordinator.iroh.url = None;
        })
        .skip_ready_wait()
        .boot()
        .await;

    let registry_client = RegistryClient::new(false, Some(node.registry_url().to_string()));
    time::sleep(Duration::from_secs(2)).await;

    let initial_lookup = registry_client.lookup(node.did(), false).await;
    assert!(initial_lookup.is_err(), "expected lookup to fail before home relay is set");

    let (_server_a, relay_a_url) = spawn_relay().await?;
    node.home_relay().set(relay_a_url.clone()).await?;

    wait_for_registry_relay(
        &registry_client,
        node.did(),
        relay_a_url.as_str(),
        Duration::from_secs(10),
    )
    .await?;

    node.teardown().await;
    Ok(())
}

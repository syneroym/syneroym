#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests verifying dynamic home relay switching, confirmation,
//! rollback on failure, and deferred registry publication.

use std::time::Duration;

use anyhow::Result;
use syneroym_core::dht_registry::RegistryClient;
use tokio::time;

use crate::common::{
    SubstrateNode, dead_relay, get_iroh_relay_url, spawn_relay, wait_for_registry_relay,
};

mod common;

#[tokio::test]
async fn substrate_switches_home_relay_when_watch_channel_updates() -> Result<()> {
    let _serial_guard = common::serial_guard().await;

    let (_server_a, relay_a_url) = spawn_relay().await?;
    let mut node = SubstrateNode::builder().shared_relay(relay_a_url.to_string()).boot().await;

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

    node.substrate_client.shutdown().await?;
    node.substrate_client.connect().await?;
    node.substrate_client.request("orchestrator", "readyz", serde_json::json!({})).await?;

    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn substrate_rolls_back_when_candidate_relay_unreachable() -> Result<()> {
    let _serial_guard = common::serial_guard().await;

    let (_server_a, relay_a_url) = spawn_relay().await?;
    let mut node = SubstrateNode::builder().shared_relay(relay_a_url.to_string()).boot().await;

    let registry_client = RegistryClient::new(false, Some(node.registry_url().to_string()));
    let record = registry_client.lookup(node.did(), false).await?;
    assert_eq!(get_iroh_relay_url(&record), Some(relay_a_url.to_string()));

    let (_dead_listener, unreachable_url) = dead_relay().await?;

    let switch_result = node.home_relay().set(unreachable_url).await;
    assert!(switch_result.is_err(), "expected unreachable relay switch to fail");

    assert_eq!(node.home_relay().current_url(), Some(relay_a_url.clone()));

    let current_record = registry_client.lookup(node.did(), false).await?;
    assert_eq!(get_iroh_relay_url(&current_record), Some(relay_a_url.to_string()));

    node.substrate_client.shutdown().await?;
    node.substrate_client.connect().await?;
    node.substrate_client.request("orchestrator", "readyz", serde_json::json!({})).await?;

    node.teardown().await;
    Ok(())
}

#[tokio::test]
async fn a_relay_that_is_down_at_boot_is_used_once_it_comes_up() -> Result<()> {
    let _serial_guard = common::serial_guard().await;

    let builder_b = SubstrateNode::builder();
    let b_relay = format!("http://127.0.0.1:{}", builder_b.ports().iroh);

    let mut node_a =
        SubstrateNode::builder().shared_relay(b_relay.clone()).skip_ready_wait().boot().await;

    let registry_client = RegistryClient::new(false, Some(node_a.registry_url().to_string()));
    time::sleep(Duration::from_secs(3)).await;

    let initial_lookup = registry_client.lookup(node_a.did(), false).await;
    assert!(initial_lookup.is_err(), "node a should have no published record while relay is down");

    let node_b = builder_b.boot().await;

    wait_for_registry_relay(
        &registry_client,
        node_a.did(),
        b_relay.as_str(),
        Duration::from_secs(60),
    )
    .await?;

    node_a.substrate_client.wait_for_ready(Duration::from_secs(30)).await?;

    node_a.teardown().await;
    node_b.teardown().await;
    Ok(())
}

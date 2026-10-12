//! Shared test helpers for `syneroym-coordinator-iroh` integration tests.

#![allow(dead_code)]

use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use syneroym_community_registry::EcosystemRegistry;
use syneroym_coordinator_iroh::CoordinatorIroh;
use syneroym_core::config::{ServiceRegistryRole, SubstrateConfig};
use tokio::time;

/// Waits until the coordinator's endpoint lists its relay.
pub async fn wait_for_relay(coord: &CoordinatorIroh, timeout: Duration) -> Result<()> {
    let start = Instant::now();

    while start.elapsed() < timeout {
        if let Some(addr) = coord.endpoint_addr()
            && addr.relay_urls().next().is_some()
        {
            return Ok(());
        }
        time::sleep(Duration::from_millis(50)).await;
    }

    bail!("coordinator endpoint did not list a relay within {timeout:?}")
}

/// Spawns a lightweight HTTP community registry (no DHT), reachable at the
/// returned URL.
pub async fn spawn_registry(base_path: &Path) -> Result<(EcosystemRegistry, String)> {
    let mut config_r = SubstrateConfig {
        app_local_data_dir: base_path.join("data_r"),
        app_data_dir: base_path.join("user_data_r"),
        ..Default::default()
    };
    config_r.substrate.enable_bep0044_dht = false;
    config_r.roles.community_registry = Some(ServiceRegistryRole {
        http_bind_address: "127.0.0.1:0".to_string(),
        ..Default::default()
    });
    let mut registry_r = EcosystemRegistry::init(&config_r).await?;
    let r_url = registry_r.bind().await?;
    registry_r.spawn().await?;
    Ok((registry_r, r_url))
}

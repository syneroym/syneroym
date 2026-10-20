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
pub mod nodes;
pub mod passport;

/// A coordinator on a host with a fresh node key and no controller.
pub async fn init_coordinator(config: &SubstrateConfig) -> Result<CoordinatorIroh> {
    use syneroym_core::host_identity::HostIdentity;
    CoordinatorIroh::init(config, &HostIdentity::random_unowned()).await
}

/// The relay URL of the coordinator's endpoint, if online.
pub fn relay_url(coord: &CoordinatorIroh) -> Option<iroh::RelayUrl> {
    coord.endpoint_addr().and_then(|a| a.relay_urls().next().cloned())
}

/// Generates a self-signed certificate for testing TLS.
pub fn generate_self_signed_cert(cert_path: &Path, key_path: &Path, common_name: &str) {
    let status = std::process::Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            key_path.to_str().unwrap(),
            "-out",
            cert_path.to_str().unwrap(),
            "-days",
            "1",
            "-subj",
            &format!("/CN=127.0.0.1/OU={common_name}"),
            "-addext",
            "basicConstraints = critical,CA:FALSE",
            "-addext",
            "extendedKeyUsage = serverAuth",
            "-addext",
            "subjectAltName = IP:127.0.0.1",
        ])
        .status()
        .expect("Failed to execute openssl");
    assert!(status.success(), "openssl command failed");
}

#[derive(Debug, Clone)]
pub struct Echo;

impl iroh::protocol::ProtocolHandler for Echo {
    async fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        let (mut send, mut recv) = connection.accept_bi().await?;
        let _ = tokio::io::copy(&mut recv, &mut send).await?;
        send.finish()?;
        connection.closed().await;
        Ok(())
    }
}

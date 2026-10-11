//! Shared test helpers for `syneroym-coordinator-iroh` integration tests.

#![allow(dead_code)]

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use syneroym_coordinator_iroh::CoordinatorIroh;
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

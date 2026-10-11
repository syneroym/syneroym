//! Shared test helpers for `syneroym-coordinator-iroh` integration tests.

#![allow(dead_code)]

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use reqwest::Client;
use syneroym_coordinator_iroh::{CoordinatorInfo, CoordinatorIroh};
use tokio::time;

/// Polls the coordinator info endpoint until its relay URL is advertised.
pub async fn wait_for_relay(coord: &CoordinatorIroh, timeout: Duration) -> Result<()> {
    let info_addr =
        coord.info_addr().ok_or_else(|| anyhow::anyhow!("coordinator has no info_addr"))?;
    let url = format!("http://{info_addr}/v1/info");
    let client = Client::new();
    let start = Instant::now();

    while start.elapsed() < timeout {
        if let Ok(res) = client.get(&url).send().await
            && let Ok(info) = res.json::<CoordinatorInfo>().await
            && info.relay_url.is_some()
        {
            return Ok(());
        }
        time::sleep(Duration::from_millis(50)).await;
    }

    bail!("coordinator at {info_addr} did not advertise a relay_url within {timeout:?}")
}

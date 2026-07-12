//! Integration test for the coordinator's connection cap.
//!
//! The test fires more simultaneous connection attempts than the configured
//! cap allows, holds the accepted connections open so the cap stays saturated
//! while the excess attempts are made, then asserts that exactly `CAP`
//! connections were accepted and the remainder rejected.
//!
//! Every network step is deadline-bounded (see `ATTEMPT_DEADLINE` /
//! `SETTLE_DEADLINE`): a single stalled attempt fails the test promptly with a
//! clear message instead of hanging CI, which is what a missing timeout here
//! used to do under load.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use reqwest::Client;
use syneroym_coordinator_iroh::{CoordinatorInfo, CoordinatorIroh};
use syneroym_core::{
    config::{CoordinatorIrohConfig, CoordinatorRole, SubstrateConfig},
    dht_registry::EndpointMechanism,
};
use syneroym_sdk::SyneroymClient;
use tokio::{
    sync::{Notify, mpsc},
    task::JoinSet,
    time,
};
use tracing_subscriber::EnvFilter;

/// Connection cap the coordinator is configured with.
const CAP: usize = 10;
/// Simultaneous connection attempts. Anything over `CAP` must be rejected.
const ATTEMPTS: usize = 15;

/// Ceiling on a single connect+probe round-trip. Generous for a loaded CI
/// runner, but finite so one stalled attempt cannot wedge the whole test.
const ATTEMPT_DEADLINE: Duration = Duration::from_secs(20);
/// Backstop around tallying every verdict and draining the held connections.
const SETTLE_DEADLINE: Duration = Duration::from_secs(40);

/// The outcome of one connection attempt, from the client's point of view.
#[derive(Clone, Copy)]
enum Verdict {
    /// The coordinator established the connection and serviced a probe.
    Accepted,
    /// The coordinator refused the connection (cap reached), or the attempt
    /// failed to complete within `ATTEMPT_DEADLINE`.
    Rejected,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accepts_up_to_cap_and_rejects_the_rest() -> Result<()> {
    init_tracing();

    let temp_dir = tempfile::tempdir()?;
    let mut coordinator = start_capped_coordinator(temp_dir.path()).await?;
    let dial = fetch_dial_info(&coordinator).await?;

    // Accepted attempts park on `release`, holding their connection (and thus
    // their server-side slot) open until every attempt has been tallied. That
    // keeps the cap saturated, so the excess attempts are genuinely rejected.
    let release = Arc::new(Notify::new());
    let (verdict_tx, mut verdict_rx) = mpsc::channel(ATTEMPTS);

    let mut attempts = JoinSet::new();
    for i in 0..ATTEMPTS {
        let dial = dial.clone();
        let release = release.clone();
        let verdict_tx = verdict_tx.clone();
        attempts.spawn(async move {
            // Stagger slightly to avoid local port exhaustion and to give the
            // first `CAP` attempts time to claim their slots.
            time::sleep(Duration::from_millis(i as u64 * 10)).await;

            let mut client = dial.new_client();
            let verdict = probe_connection(&mut client).await;
            let _ = verdict_tx.send(verdict).await;

            if matches!(verdict, Verdict::Accepted) {
                release.notified().await;
            }
            let _ = client.shutdown().await;
        });
    }
    drop(verdict_tx);

    let (accepted, rejected) = tally_verdicts(&mut verdict_rx).await?;
    println!("accepted: {accepted}, rejected: {rejected}");

    // Release the held connections and let every task finish cleanly.
    release.notify_waiters();
    let drain = async { while attempts.join_next().await.is_some() {} };
    time::timeout(SETTLE_DEADLINE, drain).await.context("timed out draining connection tasks")?;

    assert_eq!(accepted, CAP, "exactly {CAP} connections should be accepted");
    assert_eq!(
        rejected,
        ATTEMPTS - CAP,
        "exactly {} connections should be rejected",
        ATTEMPTS - CAP
    );

    coordinator.shutdown().await
}

/// Drive one connection to a verdict, bounded by `ATTEMPT_DEADLINE`.
///
/// The probe deliberately targets an under-registered service, so a *serviced*
/// connection surfaces as either a success or an "Empty response from stream"
/// drop — both prove the stream was accepted and multiplexed by the server. A
/// capped connection is refused outright, and a stalled one trips the deadline;
/// both count as rejected.
async fn probe_connection(client: &mut SyneroymClient) -> Verdict {
    let probe = async move {
        client.connect().await?;
        client.request("orchestrator", "readyz", serde_json::json!({})).await
    };
    match time::timeout(ATTEMPT_DEADLINE, probe).await {
        Ok(Ok(_)) => Verdict::Accepted,
        Ok(Err(e)) if e.to_string().contains("Empty response from stream") => Verdict::Accepted,
        Ok(Err(_)) | Err(_) => Verdict::Rejected,
    }
}

/// Collect exactly `ATTEMPTS` verdicts, bounded by `SETTLE_DEADLINE`, and
/// return `(accepted, rejected)` counts.
async fn tally_verdicts(rx: &mut mpsc::Receiver<Verdict>) -> Result<(usize, usize)> {
    let collect = async {
        let (mut accepted, mut rejected) = (0, 0);
        for _ in 0..ATTEMPTS {
            match rx.recv().await {
                Some(Verdict::Accepted) => accepted += 1,
                Some(Verdict::Rejected) => rejected += 1,
                None => break,
            }
        }
        (accepted, rejected)
    };
    time::timeout(SETTLE_DEADLINE, collect)
        .await
        .context("timed out collecting connection verdicts (a connect attempt likely hung)")
}

/// Everything a client needs to dial the coordinator under test.
#[derive(Clone)]
struct DialInfo {
    substrate_id: String,
    endpoint_addr_bytes: Vec<u8>,
    relay_url: String,
}

impl DialInfo {
    fn new_client(&self) -> SyneroymClient {
        SyneroymClient::new_with_mechanisms(
            self.substrate_id.clone(),
            vec![EndpointMechanism::Iroh {
                endpoint_addr_bytes: self.endpoint_addr_bytes.clone(),
                relay_url: Some(self.relay_url.clone()),
            }],
        )
    }
}

/// Start a coordinator that hosts a local relay and enforces a `CAP`-connection
/// limit, with discovery/DHT/registry disabled for a self-contained test.
async fn start_capped_coordinator(base_path: &Path) -> Result<CoordinatorIroh> {
    let mut config = SubstrateConfig {
        app_local_data_dir: base_path.join("data_c"),
        app_data_dir: base_path.join("user_data_c"),
        ..Default::default()
    };
    config.substrate.enable_bep0044_dht = false;
    config.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: false,
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            community_registry_url: None,
            idle_timeout_secs: None,
            share_in_registry: false,
            max_connections: Some(CAP),
        }),
        ..Default::default()
    });
    CoordinatorIroh::init(&config).await
}

/// Ask the running coordinator (over its local `/v1/info` endpoint) for the
/// address bytes and relay URL a client needs to reach it.
async fn fetch_dial_info(coordinator: &CoordinatorIroh) -> Result<DialInfo> {
    let info_addr = coordinator.info_addr().context("coordinator has no info address")?;
    let info: CoordinatorInfo =
        Client::new().get(format!("http://{info_addr}/v1/info")).send().await?.json().await?;
    Ok(DialInfo {
        substrate_id: info.substrate_id,
        endpoint_addr_bytes: info.endpoint_addr_bytes,
        relay_url: info.relay_url.context("coordinator did not advertise a relay URL")?,
    })
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug,iroh=info")),
        )
        .try_init();
}

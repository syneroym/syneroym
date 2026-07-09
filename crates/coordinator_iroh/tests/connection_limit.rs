//! Integration tests for the connection limit cap
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::{sync::Arc, time::Duration};

use anyhow::Result;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_connection_limit() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug,iroh=info")),
        )
        .try_init();

    let temp_dir = tempfile::tempdir()?;
    let base_path = temp_dir.path();
    let cap = 10;
    let attempts = 15;

    // 1. Spawn a coordinator C with a connection cap
    let mut config_c = SubstrateConfig {
        app_local_data_dir: base_path.join("data_c"),
        app_data_dir: base_path.join("user_data_c"),
        ..Default::default()
    };
    config_c.substrate.enable_bep0044_dht = false;
    config_c.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: false,
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            community_registry_url: None,
            idle_timeout_secs: None,
            share_in_registry: false,
            max_connections: Some(cap),
        }),
        ..Default::default()
    });

    let mut c = CoordinatorIroh::init(&config_c).await?;
    let c_info_addr = c.info_addr().unwrap();

    let info_client = Client::new();
    let c_info: CoordinatorInfo =
        info_client.get(format!("http://{c_info_addr}/v1/info")).send().await?.json().await?;
    let c_relay_url = c_info.relay_url.clone().unwrap();

    let mut join_set = JoinSet::new();

    let (tx, mut rx) = mpsc::channel(attempts);
    let notify = Arc::new(Notify::new());

    for i in 0..attempts {
        let endpoint_addr_bytes = c_info.endpoint_addr_bytes.clone();
        let relay_url = c_relay_url.clone();
        let tx = tx.clone();
        let notify = notify.clone();

        let substrate_id = c_info.substrate_id.clone();
        join_set.spawn(async move {
            // Stagger attempts slightly to prevent local socket/port exhaustion
            time::sleep(Duration::from_millis(i as u64 * 10)).await;

            let mut client = SyneroymClient::new_with_mechanisms(
                substrate_id,
                vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url: Some(relay_url) }],
            );

            let res = client.connect().await;
            match res {
                Ok(_) => {
                    let res = client.request("orchestrator", "readyz", serde_json::json!({})).await;

                    let success = match res {
                        Ok(_) => true,
                        Err(e) => {
                            let err_msg = e.to_string();
                            // NOTE: The router will log an ERROR here: "Endpoint not found in
                            // registry or DHT..." This is EXPECTED. The
                            // test uses "orchestrator" which is not fully registered
                            // in the mock registry, triggering a fallback to the DHT which fails
                            // and drops the stream. If the stream drops
                            // with "Empty response from stream", it proves the connection was
                            // successfully accepted and multiplexed by the server before the
                            // graceful drop.
                            err_msg.contains("Empty response from stream")
                        }
                    };

                    let _ = tx.send(success).await;

                    // Keep the connection open until all attempts are processed
                    notify.notified().await;

                    let _ = client.shutdown().await;
                    success
                }
                Err(e) => {
                    tracing::error!("Connection attempt {} failed: {}", i, e);
                    let _ = tx.send(false).await;
                    let _ = client.shutdown().await;
                    false
                }
            }
        });
    }

    drop(tx);

    let mut success_count = 0;
    let mut fail_count = 0;

    for _ in 0..attempts {
        if let Some(success) = rx.recv().await {
            if success {
                success_count += 1;
            } else {
                fail_count += 1;
            }
        }
    }

    println!("Successful connections: {success_count}, Failed: {fail_count}");

    // Release all held connections
    notify.notify_waiters();

    // Wait for all tasks to cleanly exit
    while let Some(res) = join_set.join_next().await {
        // We already tallied the successes, this just ensures clean shutdown
        if let Err(e) = res {
            println!("Task panicked: {e}");
        }
    }
    assert_eq!(success_count, cap, "Exactly {cap} connections should succeed");
    assert_eq!(
        fail_count,
        attempts - cap,
        "Exactly {} connections should be rejected",
        attempts - cap
    );

    c.shutdown().await?;

    Ok(())
}

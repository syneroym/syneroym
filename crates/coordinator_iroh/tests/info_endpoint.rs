//! Integration tests for the coordinator HTTP /v1/info endpoint.

#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use axum::{Router, http::StatusCode, routing};
use iroh::{Endpoint, RelayMap, RelayMode, RelayUrl, SecretKey, endpoint::presets};
use reqwest::Client;
use syneroym_coordinator_iroh::CoordinatorIroh;
use syneroym_core::{
    config::{CoordinatorAccess, CoordinatorIrohConfig, CoordinatorRole, SubstrateConfig},
    coordinator_info::{CoordinatorInfo, CoordinatorStatus, INFO_PATH},
    dht_registry::RegistryClient,
};
use tokio::{net::TcpListener, time};

async fn fetch_info(coord: &CoordinatorIroh) -> Result<CoordinatorInfo> {
    let addr = coord.info_addr().context("no info address")?;
    Ok(Client::new().get(format!("http://{addr}{INFO_PATH}")).send().await?.json().await?)
}

/// Polls `/v1/info` until `pred` holds, or fails after `deadline` with the
/// last answer in the error.
async fn wait_for_info(
    coord: &CoordinatorIroh,
    deadline: Duration,
    pred: impl Fn(&CoordinatorInfo) -> bool,
) -> Result<CoordinatorInfo> {
    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < deadline {
        if let Ok(info) = fetch_info(coord).await {
            if pred(&info) {
                return Ok(info);
            }
            last = Some(info);
        }
        time::sleep(Duration::from_millis(50)).await;
    }
    bail!("timed out waiting for info condition; last response: {last:?}")
}

#[tokio::test]
async fn relay_count_follows_connections_and_capacity() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let mut config = SubstrateConfig {
        app_local_data_dir: temp_dir.path().join("local"),
        app_data_dir: temp_dir.path().join("data"),
        ..Default::default()
    };
    config.substrate.enable_bep0044_dht = false;
    config.roles.coordinator = Some(CoordinatorRole {
        access: CoordinatorAccess::Everyone,
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: true,
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            info_http_bind_address: Some("127.0.0.1:0".to_string()),
            max_relay_clients: Some(2),
            ..Default::default()
        }),
        ..Default::default()
    });

    let mut coord = common::init_coordinator(&config).await?;
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;

    // Step 2: The coordinator endpoint connects to its own relay and counts as 1.
    let info = wait_for_info(&coord, Duration::from_secs(20), |info| {
        info.status == CoordinatorStatus::Healthy
            && info.relay.as_ref().and_then(|r| r.clients) == Some(1)
    })
    .await?;

    // Step 3: Connect a client endpoint to the coordinator's relay.
    let relay_url: RelayUrl = info.relay_url.context("missing relay_url")?.parse()?;
    let client = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)))
        .bind()
        .await?;
    time::timeout(Duration::from_secs(20), client.online()).await?;

    // Step 4: With client connected, relay has 2 connections reaching capacity.
    wait_for_info(&coord, Duration::from_secs(20), |info| {
        info.status == CoordinatorStatus::AtCapacity
            && info.relay.as_ref().and_then(|r| r.clients) == Some(2)
            && info.relay.as_ref().and_then(|r| r.max_clients) == Some(2)
    })
    .await?;

    // Step 5: Close the client connection and confirm count decreases to 1.
    let _ = client.close().await;
    wait_for_info(&coord, Duration::from_secs(20), |info| {
        info.status == CoordinatorStatus::Healthy
            && info.relay.as_ref().and_then(|r| r.clients) == Some(1)
    })
    .await?;

    coord.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn coordinator_record_is_registered_under_its_did() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let (mut registry_r, r_url) = common::spawn_registry(temp_dir.path()).await?;

    let mut config = SubstrateConfig {
        app_local_data_dir: temp_dir.path().join("local"),
        app_data_dir: temp_dir.path().join("data"),
        ..Default::default()
    };
    config.substrate.enable_bep0044_dht = false;
    config.roles.coordinator = Some(CoordinatorRole {
        access: CoordinatorAccess::Everyone,
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: true,
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            info_http_bind_address: Some("127.0.0.1:0".to_string()),
            community_registry_url: Some(r_url.clone()),
            share_in_registry: true,
            ..Default::default()
        }),
        ..Default::default()
    });

    let mut coord = common::init_coordinator(&config).await?;
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;

    let info = wait_for_info(&coord, Duration::from_secs(20), |info| {
        info.registry.as_ref().map(|r| r.registered).unwrap_or(false)
    })
    .await?;

    let reg = info.registry.context("missing registry info")?;
    let reg_client = RegistryClient::new(false, Some(r_url));
    let lookup_res = reg_client.lookup(&reg.did, false).await?;

    assert_eq!(lookup_res.info.service_id, reg.did);
    assert!(reg.did.starts_with("did:key:"));
    let id_info = info.identity.as_ref().context("missing identity")?;
    assert!(id_info.did.starts_with("did:key:"));
    assert_eq!(id_info.controller_status, "none");

    coord.shutdown().await?;
    registry_r.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn probes_do_not_call_the_registry() -> Result<()> {
    let register_count = Arc::new(AtomicUsize::new(0));
    let lookup_count = Arc::new(AtomicUsize::new(0));

    let reg_c = Arc::clone(&register_count);
    let look_c = Arc::clone(&lookup_count);

    let app = Router::new()
        .route(
            "/register",
            routing::post(move || {
                let reg_c = Arc::clone(&reg_c);
                async move {
                    reg_c.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        )
        .route(
            "/lookup/{service_id}",
            routing::get(move || {
                let look_c = Arc::clone(&look_c);
                async move {
                    look_c.fetch_add(1, Ordering::SeqCst);
                    StatusCode::NOT_FOUND
                }
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let mock_addr = listener.local_addr()?;
    let mock_url = format!("http://{mock_addr}");
    let mock_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let temp_dir = tempfile::tempdir()?;
    let mut config = SubstrateConfig {
        app_local_data_dir: temp_dir.path().join("local"),
        app_data_dir: temp_dir.path().join("data"),
        ..Default::default()
    };
    config.substrate.enable_bep0044_dht = false;
    config.roles.coordinator = Some(CoordinatorRole {
        access: CoordinatorAccess::Everyone,
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: false,
            enable_relay: false,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            info_http_bind_address: Some("127.0.0.1:0".to_string()),
            community_registry_url: Some(mock_url),
            share_in_registry: true,
            ..Default::default()
        }),
        ..Default::default()
    });

    let mut coord = common::init_coordinator(&config).await?;

    wait_for_info(&coord, Duration::from_secs(10), |info| {
        info.registry.as_ref().map(|r| r.registered).unwrap_or(false)
    })
    .await?;

    let mut last_info = None;
    for _ in 0..20 {
        let info = time::timeout(Duration::from_secs(2), fetch_info(&coord)).await??;
        last_info = Some(info);
    }

    assert_eq!(lookup_count.load(Ordering::SeqCst), 0);
    assert_eq!(register_count.load(Ordering::SeqCst), 1);

    let last = last_info.context("missing last info")?;
    let relay = last.relay.context("missing relay info")?;
    assert!(!relay.online);
    assert_eq!(relay.clients, None);
    assert_eq!(last.status, CoordinatorStatus::Healthy);

    coord.shutdown().await?;
    mock_handle.abort();
    Ok(())
}

#[tokio::test]
async fn restricted_access_does_not_count_excluded_endpoints() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let allowed_secret = SecretKey::generate();
    let allowed_id = allowed_secret.public().to_string();

    let mut config = SubstrateConfig {
        app_local_data_dir: temp_dir.path().join("local"),
        app_data_dir: temp_dir.path().join("data"),
        ..Default::default()
    };
    config.substrate.enable_bep0044_dht = false;
    config.roles.coordinator = Some(CoordinatorRole {
        access: CoordinatorAccess::List(vec![allowed_id]),
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: false,
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            info_http_bind_address: Some("127.0.0.1:0".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    });

    let mut coord = common::init_coordinator(&config).await?;
    common::wait_for_relay(&coord, Duration::from_secs(20)).await?;

    // The coordinator's own endpoint connects to its own relay and counts as 1.
    let info = wait_for_info(&coord, Duration::from_secs(5), |info| {
        info.relay.as_ref().and_then(|r| r.clients) == Some(1)
    })
    .await?;

    let relay_url: RelayUrl = info.relay_url.context("missing relay_url")?.parse()?;

    // Connect the allowed control client and wait until the relay counts it.
    let allowed_client = Endpoint::builder(presets::Minimal)
        .secret_key(allowed_secret)
        .relay_mode(RelayMode::Custom(RelayMap::from(relay_url.clone())))
        .bind()
        .await?;
    time::timeout(Duration::from_secs(20), allowed_client.online()).await?;

    wait_for_info(&coord, Duration::from_secs(10), |info| {
        info.relay.as_ref().and_then(|r| r.clients) == Some(2)
    })
    .await?;

    // Connect an excluded client and verify the relay client count stays at 2.
    let excluded_client = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)))
        .bind()
        .await?;

    time::sleep(Duration::from_millis(500)).await;
    let info_after = fetch_info(&coord).await?;
    assert_eq!(info_after.relay.as_ref().and_then(|r| r.clients), Some(2));

    let _ = excluded_client.close().await;
    let _ = allowed_client.close().await;
    coord.shutdown().await?;
    Ok(())
}

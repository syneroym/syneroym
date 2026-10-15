#![allow(clippy::cognitive_complexity, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests verifying coordinator startup behavior and configuration
//! validation.

mod common;

use iroh::SecretKey;
use syneroym_core::config::{
    CoordinatorAccess, CoordinatorIrohConfig, CoordinatorRole, SubstrateConfig, SubstrateTlsConfig,
};

fn test_config() -> SubstrateConfig {
    let mut config = SubstrateConfig::default();
    config.substrate.enable_bep0044_dht = false;
    config.roles.coordinator = Some(CoordinatorRole {
        access: CoordinatorAccess::Everyone,
        iroh: Some(CoordinatorIrohConfig {
            enable_signalling: false,
            enable_relay: false,
            http_bind_address: "127.0.0.1:0".to_string(),
            quic_bind_address: "127.0.0.1:0".to_string(),
            info_http_bind_address: Some("127.0.0.1:0".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    });
    config
}

#[tokio::test]
async fn passport_access_needs_tls() {
    let valid_id = SecretKey::generate(&mut rand::rng()).public().to_string();
    let variants =
        vec![CoordinatorAccess::Passport, CoordinatorAccess::ListOrPassport(vec![valid_id])];

    for access in &variants {
        let mut config = test_config();
        config.roles.coordinator.as_mut().unwrap().access = access.clone();

        let result = common::init_coordinator(&config).await;
        assert!(result.is_err(), "expected error for passport access without TLS");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("[tls]"), "unexpected error message: {err}");
    }

    let temp_dir = tempfile::tempdir().unwrap();
    let cert_path = temp_dir.path().join("cert.pem");
    let key_path = temp_dir.path().join("key.pem");
    common::generate_self_signed_cert(&cert_path, &key_path, "localhost");

    for access in variants {
        let mut config = test_config();
        config.app_local_data_dir = temp_dir.path().join("local");
        config.app_data_dir = temp_dir.path().join("data");
        config.tls = Some(SubstrateTlsConfig {
            cert_path: cert_path.clone(),
            key_path: key_path.clone(),
            reload_on_sigusr1: false,
        });
        let role = config.roles.coordinator.as_mut().unwrap();
        role.access = access;
        role.iroh.as_mut().unwrap().enable_relay = true;

        let result = common::init_coordinator(&config).await;
        assert!(result.is_ok(), "expected coordinator to boot with TLS: {:?}", result.err());
        let mut coord = result.unwrap();
        coord.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn check_access_rejects_bad_ids() {
    let mut config = test_config();
    config.roles.coordinator.as_mut().unwrap().access =
        CoordinatorAccess::List(vec!["not-a-valid-endpoint-id".to_string()]);
    let result = common::init_coordinator(&config).await;
    assert!(result.is_err(), "expected error for invalid endpoint id");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("is not an Iroh endpoint id"), "unexpected error message: {err}");
}

#[tokio::test]
async fn accepts_everyone_access() {
    let mut config = test_config();
    config.roles.coordinator.as_mut().unwrap().access = CoordinatorAccess::Everyone;

    let coord = common::init_coordinator(&config).await;
    assert!(coord.is_ok(), "expected coordinator to initialize successfully with 'everyone'");
}

#[tokio::test]
async fn rejects_invalid_parent_relay_url() {
    let mut config = test_config();
    config.parent_coordinator.iroh.url = Some("not-a-valid-url".to_string());

    let result = common::init_coordinator(&config).await;
    assert!(result.is_err(), "expected error for invalid parent relay url");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("parent_coordinator.iroh.url") || err.contains("invalid"),
        "unexpected error message: {err}"
    );
}

#[tokio::test]
async fn boots_without_parent_relay_url() {
    let mut config = test_config();
    config.parent_coordinator.iroh.url = None;

    let coord = common::init_coordinator(&config).await;
    assert!(coord.is_ok(), "expected coordinator to boot without parent relay url");
}

#[tokio::test]
async fn rejects_zero_max_relay_clients() {
    let mut config = test_config();
    let iroh_cfg = config.roles.coordinator.as_mut().unwrap().iroh.as_mut().unwrap();
    iroh_cfg.enable_relay = true;
    iroh_cfg.max_relay_clients = Some(0);

    let result = common::init_coordinator(&config).await;
    assert!(result.is_err(), "expected error for max_relay_clients = 0");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("max_relay_clients"), "unexpected error message: {err}");
}

#[tokio::test]
async fn accepts_one_max_relay_clients() {
    let mut config = test_config();
    let iroh_cfg = config.roles.coordinator.as_mut().unwrap().iroh.as_mut().unwrap();
    iroh_cfg.enable_relay = true;
    iroh_cfg.max_relay_clients = Some(1);

    let result = common::init_coordinator(&config).await;
    assert!(result.is_ok(), "expected coordinator to boot with max_relay_clients = 1");
}

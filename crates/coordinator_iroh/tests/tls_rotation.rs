#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::{
    fs,
    path::Path,
    process::{self, Command},
    time::Duration,
};

use anyhow::Result;
use reqwest::{Certificate, Client};
use rustls::crypto::ring;
use syneroym_coordinator_iroh::CoordinatorIroh;
use syneroym_core::config::{
    CoordinatorIrohConfig, CoordinatorRole, LogTarget, SubstrateConfig, SubstrateTlsConfig,
};
use tokio::time;

fn generate_self_signed_cert(cert_path: &Path, key_path: &Path, common_name: &str) {
    let status = Command::new("openssl")
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

#[tokio::test]
async fn test_tls_rotation_sigusr1() -> Result<()> {
    // Install default crypto provider for rustls
    let _ = ring::default_provider().install_default();

    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let base_path = temp_dir.path();

    let cert_path = base_path.join("cert.pem");
    let key_path = base_path.join("key.pem");

    // 1. Generate Cert A
    generate_self_signed_cert(&cert_path, &key_path, "cert-a");
    let cert_a_bytes = fs::read(&cert_path)?;

    // Set up coordinator config
    let mut config = SubstrateConfig {
        app_local_data_dir: base_path.join("data"),
        app_data_dir: base_path.join("user_data"),
        app_cache_dir: base_path.join("cache"),
        app_log_dir: base_path.join("logs"),
        profile: "full".to_string(),
        tls: Some(SubstrateTlsConfig {
            cert_path: cert_path.clone(),
            key_path: key_path.clone(),
            reload_on_sigusr1: true,
        }),
        ..SubstrateConfig::default()
    };
    config.substrate.enable_bep0044_dht = false;
    config.resolve_paths();
    config.logging.target = LogTarget::File;

    config.roles.coordinator = Some(CoordinatorRole {
        iroh: Some(CoordinatorIrohConfig {
            // Use a purely local relay instead of falling back to iroh's public N0
            // relay/discovery infrastructure, so this test doesn't depend on real
            // internet connectivity to bring the endpoint online. Relay TLS is
            // intentionally left unconfigured (see `role.tls` in
            // `CoordinatorRole`): it's independent of the `SubstrateTlsConfig`
            // above, which only covers the /v1/info HTTPS endpoint under test.
            enable_relay: true,
            http_bind_address: "127.0.0.1:0".to_string(), // Dynamic port
            quic_bind_address: "127.0.0.1:0".to_string(), // Dynamic port
            ..Default::default()
        }),
        ..Default::default()
    });

    let mut coord = CoordinatorIroh::init(&config).await?;
    let info_addr = coord.info_addr().expect("HTTP info server address not set");

    // Spawn coordinator run loop
    let coord_handle = tokio::spawn(async move {
        if let Err(e) = coord.run().await {
            eprintln!("Coordinator error: {:?}", e);
        }
    });

    // Create client A that only trusts Cert A
    let reqwest_cert_a = Certificate::from_pem(&cert_a_bytes)?;
    let client_a = Client::builder().add_root_certificate(reqwest_cert_a.clone()).build()?;

    let url = format!("https://{}/v1/info", info_addr);

    // Initial connection should succeed with Client A
    let resp = client_a.get(&url).send().await?;
    assert!(resp.status().is_success());

    // 2. Generate Cert B and write to the same path
    generate_self_signed_cert(&cert_path, &key_path, "cert-b");
    let cert_b_bytes = fs::read(&cert_path)?;

    // Create client B that only trusts Cert B
    let reqwest_cert_b = Certificate::from_pem(&cert_b_bytes)?;
    let client_b = Client::builder().add_root_certificate(reqwest_cert_b).build()?;

    // Prior to rotation, client B should fail to connect
    let resp_b_fail = client_b.get(&url).send().await;
    assert!(resp_b_fail.is_err());

    // Send SIGUSR1 to reload certs using safe external command 'kill'
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-USR1", &process::id().to_string()])
            .status()
            .expect("Failed to run kill command");
        assert!(status.success(), "kill command failed");
    }

    // Wait for the watcher to detect the signal and reload
    time::sleep(Duration::from_millis(500)).await;

    // A brand new client only trusting Cert A should now fail
    let client_a_new = Client::builder().add_root_certificate(reqwest_cert_a.clone()).build()?;
    let resp_a_new_fail = client_a_new.get(&url).send().await;
    assert!(resp_a_new_fail.is_err(), "New connection with old cert should fail");

    // Client B should now succeed (proves new cert is used for new handshakes)
    let resp_b_success = client_b.get(&url).send().await?;
    assert!(resp_b_success.status().is_success());

    // Existing client A connection (which has keep-alive pool) should still succeed
    // (proves connection was not dropped and TLS session remains active)
    let resp_a_keepalive = client_a.get(&url).send().await?;
    assert!(resp_a_keepalive.status().is_success());

    // Clean up
    coord_handle.abort();
    Ok(())
}

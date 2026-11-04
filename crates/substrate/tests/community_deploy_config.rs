#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The files under `deploy/` that run the community node in Docker.
//!
//! Most config structs accept unknown keys, so a misspelled key in the example
//! config would still parse and then be ignored. These tests check the parsed
//! value of every key the example sets.

use std::{
    fs,
    net::Ipv4Addr,
    path::{Path, PathBuf},
};

use syneroym_core::config::{IdentityMode, SubstrateConfig};

fn repo_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative)
}

fn example_config() -> SubstrateConfig {
    let text = fs::read_to_string(repo_file("deploy/config.community.example.toml")).unwrap();
    let mut config: SubstrateConfig = toml::from_str(&text).unwrap();
    config.resolve_paths();
    config
}

#[test]
fn the_community_example_config_parses_and_opens_the_gateway() {
    let config = example_config();

    let gateway = config.roles.client_gateway.expect("the example runs the client gateway");
    assert_eq!(gateway.http_port, 7960);
    assert_eq!(gateway.identity_mode, IdentityMode::Open);
    assert_eq!(gateway.bind_ip(), Ipv4Addr::UNSPECIFIED);
    assert!(gateway.remote_access.is_some(), "a container gateway must have remote access");
}

#[test]
fn the_community_example_config_keeps_state_in_the_mounted_volumes() {
    let config = example_config();
    let mounted = ["/var/lib/syneroym", "/etc/syneroym", "/etc/letsencrypt"].map(Path::new);
    let mut paths = vec![
        config.app_config_dir.clone(),
        config.app_data_dir.clone(),
        config.app_local_data_dir.clone(),
        config.app_cache_dir.clone(),
        config.app_log_dir.clone(),
        config.storage.db_dir.clone(),
        config.storage.blobs_dir.clone(),
        config.storage.blob_store.local_root.clone(),
    ];
    paths.extend(config.identity.key.clone());
    paths.extend(config.identity.agreement.clone());
    if let Some(tls) = config.roles.coordinator.as_ref().and_then(|c| c.tls.as_ref()) {
        paths.extend([tls.cert_path.clone(), tls.key_path.clone()]);
    }
    for path in &paths {
        assert!(
            mounted.iter().any(|root| path.starts_with(root)),
            "{} is outside the mounted volumes",
            path.display()
        );
    }

    assert_eq!(config.substrate.registry_url.as_deref(), Some("http://127.0.0.1:7961"));
    assert_eq!(config.parent_coordinator.iroh.url.as_deref(), Some("http://127.0.0.1:7964"));
    let registry = config.roles.community_registry.expect("the example runs the registry");
    assert_eq!(registry.http_bind_address, "0.0.0.0:7961");
    let iroh = config.roles.coordinator.and_then(|c| c.iroh).expect("the example runs the relay");
    assert!(iroh.enable_relay);
    assert_eq!(iroh.http_bind_address, "0.0.0.0:7964");
    assert_eq!(iroh.quic_bind_address, "0.0.0.0:7965");
}

#[test]
fn the_deploy_files_expose_the_quic_port_as_udp() {
    let compose = fs::read_to_string(repo_file("deploy/docker-compose.community.yml")).unwrap();
    assert!(compose.contains("7965:7965/udp"), "the compose file maps QUIC as TCP");

    let dockerfile = fs::read_to_string(repo_file("Dockerfile")).unwrap();
    assert!(
        dockerfile.contains("EXPOSE 7964 7965/udp 7961 7960"),
        "the Dockerfile exposes QUIC as TCP"
    );
}

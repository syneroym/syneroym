#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! CLI entry point for running the Syneroym substrate.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use syneroym_core::config::SubstrateConfig;
use syneroym_substrate::run;
use tokio::runtime::Builder;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the syneroym substrate
    Run {
        /// Path to the configuration file
        #[arg(short, long, value_name = "FILE")]
        config: Option<PathBuf>,

        /// Profile to use, overriding config file
        #[arg(long)]
        profile: Option<String>,

        /// Enable Coordinator Iroh
        #[arg(long)]
        enable_coordinator_iroh: Option<bool>,

        /// Iroh relay URL
        #[arg(long)]
        iroh_relay_url: Option<String>,

        /// Enable Coordinator WebRTC
        #[arg(long)]
        enable_coordinator_webrtc: Option<bool>,

        /// Path to the node's private key
        #[arg(long)]
        key: Option<PathBuf>,

        /// Controller DID
        #[arg(long)]
        controller_did: Option<String>,

        /// Path to the `ControllerAgreement` JSON
        #[arg(long)]
        agreement: Option<PathBuf>,

        /// Require a valid `ControllerAgreement` to start
        #[arg(long, default_missing_value = "true", num_args = 0..=1)]
        require_agreement: Option<bool>,

        /// Optional nickname for the substrate
        #[arg(long)]
        nickname: Option<String>,

        /// Optional external host for the WebRTC coordinator
        #[arg(long)]
        external_host: Option<String>,
    },
}

pub(crate) fn resolve_config(command: Commands) -> Result<SubstrateConfig> {
    match command {
        Commands::Run {
            config: config_path,
            profile,
            enable_coordinator_iroh,
            iroh_relay_url,
            enable_coordinator_webrtc,
            key,
            controller_did,
            agreement,
            require_agreement,
            nickname,
            external_host,
        } => {
            // Load from file if provided, otherwise use defaults
            let mut config = if let Some(path) = config_path {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read config file at {path:?}"))?;
                toml::from_str(&content)
                    .with_context(|| format!("Failed to parse config file at {path:?}"))?
            } else {
                dev_mode_config()
            };

            // Override with CLI arguments
            if let Some(p) = profile {
                config.profile = p;
            }

            if let Some(k) = key {
                config.identity.key = Some(k);
            }
            if let Some(c) = controller_did {
                config.identity.controller_did = Some(c);
            }
            if let Some(a) = agreement {
                config.identity.agreement = Some(a);
            }
            if let Some(r) = require_agreement {
                config.identity.require_agreement = r;
            }
            if let Some(n) = nickname {
                config.identity.nickname = Some(n);
            }

            // Consistency checks for identity configuration
            if config.identity.require_agreement && config.identity.agreement.is_none() {
                anyhow::bail!(
                    "Inconsistent configuration: `require_agreement` is true, but no `agreement` path is provided."
                );
            }

            if config.identity.agreement.is_some() && config.identity.controller_did.is_some() {
                anyhow::bail!(
                    "Inconsistent configuration: Both `agreement` and `controller_did` are provided. Please provide only one."
                );
            }

            if let Some(enable) = enable_coordinator_iroh {
                let coordinator = config.roles.coordinator.get_or_insert_with(Default::default);
                let iroh = coordinator.iroh.get_or_insert_with(Default::default);
                iroh.enable_signalling = enable;
                iroh.enable_relay = enable;
            }

            if let Some(url) = iroh_relay_url {
                let iroh = config.parent_coordinator.iroh.get_or_insert_with(Default::default);
                iroh.url = url;
            }

            if let Some(enable) = enable_coordinator_webrtc {
                let coordinator = config.roles.coordinator.get_or_insert_with(Default::default);
                let webrtc = coordinator.webrtc.get_or_insert_with(Default::default);
                webrtc.enable_signalling = enable;
                webrtc.enable_relay = enable;
            }

            if let Some(host) = external_host {
                let coordinator = config.roles.coordinator.get_or_insert_with(Default::default);
                let webrtc = coordinator.webrtc.get_or_insert_with(Default::default);
                webrtc.external_host = Some(host);
            }

            // Resolve relative paths against base directories
            config.resolve_paths();

            Ok(config)
        }
    }
}

fn main() -> Result<()> {
    if rustls::crypto::ring::default_provider().install_default().is_err() {
        eprintln!("Failed to install rustls default crypto provider");
        std::process::exit(1);
    }

    let cli = Cli::parse();
    let config = resolve_config(cli.command)?;

    // Since all tokio tuning options are not available in the #[tokio::main] macro, configure it explicitly
    Builder::new_multi_thread()
        // More tokio tuning needed later, tune via config or environment variables.
        //.worker_threads(4)
        //.max_blocking_threads(16)
        .enable_all()
        .build()
        .context("Failed to build tokio runtime")?
        .block_on(run(config))
}

/// Returns a default configuration suitable for local development.
/// This enables all core roles and points to a local registry.
fn dev_mode_config() -> SubstrateConfig {
    let mut c = SubstrateConfig::default();
    // Enable all roles for local development by default
    c.roles.app_sandbox = Some(Default::default());
    c.roles.community_registry = Some(Default::default());

    c.roles.coordinator = Some(syneroym_core::config::CoordinatorRole {
        iroh: Some(syneroym_core::config::CoordinatorIrohConfig {
            enable_signalling: true,
            enable_relay: true,
            ..Default::default()
        }),
        webrtc: Some(syneroym_core::config::CoordinatorWebRtcConfig {
            enable_signalling: true,
            enable_relay: true,
            ..Default::default()
        }),
        ..Default::default()
    });

    c.roles.client_gateway = Some(Default::default());

    // Enable observability by default in dev mode
    c.roles.observability = Some(syneroym_core::config::ObservabilityRole {
        health: Some(syneroym_core::config::EndpointConfig {
            enabled: true,
            bind_address: "0.0.0.0:7966".to_string(),
            endpoint: "/health".to_string(),
        }),
        metrics: Some(syneroym_core::config::EndpointConfig {
            enabled: true,
            bind_address: "0.0.0.0:7967".to_string(),
            endpoint: "/metrics".to_string(),
        }),
        tracing: Some(Default::default()),
    });

    c.parent_coordinator.iroh = Some(Default::default());
    c.parent_coordinator.webrtc = Some(Default::default());

    // Default to local registry
    c.substrate.registry_url = Some("http://localhost:7961".to_string());
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_config_precedence() {
        // 1. Setup a dummy config file overriding some defaults
        let mut config_file = NamedTempFile::new().expect("Failed to create temp config file");
        let config_toml = r#"
        profile = "config_profile"
        
        [parent_coordinator.iroh]
        relay_url = "http://config.relay:3340"
        
        [roles.coordinator.iroh]
        enable_signalling = true
        enable_relay = true
        http_bind_address = "0.0.0.0:8000"
        "#;
        write!(config_file, "{config_toml}").expect("Failed to write config file");

        // 2. Setup CLI arguments overriding some config file values and some defaults
        // - Override config file: iroh_relay_url ("http://cli.relay:3340" vs "http://config.relay:3340")
        // - Override default (not in config): enable_coordinator_webrtc (true vs default false)
        let config_path_str =
            config_file.path().to_str().expect("Failed to convert config path to string");
        let args = vec![
            "syneroym",
            "run",
            "--config",
            config_path_str,
            "--iroh-relay-url",
            "http://cli.relay:3340",
            "--enable-coordinator-webrtc",
            "true",
        ];

        let cli = Cli::parse_from(args);

        // 3. Resolve config
        let config = resolve_config(cli.command).expect("Failed to resolve config");

        // 4. Verify precedence

        // Assert: Value specified in config file, but NOT overridden in CLI
        assert_eq!(config.profile, "config_profile", "Profile should be from config file");
        assert!(
            config.roles.coordinator.as_ref().unwrap().iroh.as_ref().unwrap().enable_signalling,
            "Coordinator iroh should be enabled from config"
        );
        assert_eq!(
            config.roles.coordinator.as_ref().unwrap().iroh.as_ref().unwrap().http_bind_address,
            "0.0.0.0:8000",
            "Coordinator iroh http bind address should be from config"
        );

        // Assert: Value specified in CLI overriding default (not in config)
        assert!(
            config.roles.coordinator.as_ref().unwrap().webrtc.as_ref().unwrap().enable_signalling,
            "Coordinator webrtc should be enabled from CLI"
        );

        // Assert: Value specified in CLI overriding config file
        assert_eq!(
            config.parent_coordinator.iroh.as_ref().unwrap().url,
            "http://cli.relay:3340",
            "Iroh relay URL should be overridden by CLI"
        );

        // 5. Verify defaults are retained for fields not specified in config or CLI
        let default_config = SubstrateConfig::default();

        // Assert defaults that were NOT overridden
        assert_eq!(
            config.config_version, default_config.config_version,
            "Config version should remain default"
        );
        assert_eq!(
            config.app_config_dir, default_config.app_config_dir,
            "App config dir should remain default"
        );
        assert_eq!(
            config.app_local_data_dir, default_config.app_local_data_dir,
            "App local data dir should remain default"
        );
    }

    #[test]
    fn test_config_consistency_checks() {
        // Test require_agreement without agreement
        let args = vec!["syneroym", "run", "--require-agreement"];
        let cli = Cli::parse_from(args);
        let result = resolve_config(cli.command);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Inconsistent configuration: `require_agreement` is true, but no `agreement` path is provided."
        );

        // Test agreement and controller_did provided together
        let args = vec![
            "syneroym",
            "run",
            "--agreement",
            "some/path.json",
            "--controller-did",
            "did:key:something",
        ];
        let cli = Cli::parse_from(args);
        let result = resolve_config(cli.command);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Inconsistent configuration: Both `agreement` and `controller_did` are provided. Please provide only one."
        );
    }
}

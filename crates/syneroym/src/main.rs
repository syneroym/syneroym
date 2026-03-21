use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use syneroym_substrate::{config::SubstrateConfig, run};

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
        } => {
            // Load from file if provided, otherwise use defaults
            let mut config = if let Some(path) = config_path {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read config file at {:?}", path))?;
                toml::from_str(&content)
                    .with_context(|| format!("Failed to parse config file at {:?}", path))?
            } else {
                SubstrateConfig::default()
            };

            // Override with CLI arguments
            if let Some(p) = profile {
                config.profile = p;
            }

            if let Some(enable) = enable_coordinator_iroh {
                if let Some(ref mut coordinator) = config.roles.coordinator {
                    if let Some(ref mut iroh) = coordinator.iroh {
                        iroh.enabled = enable;
                    } else {
                        coordinator.iroh =
                            Some(syneroym_substrate::config::CoordinatorIrohConfig {
                                enabled: enable,
                                ..Default::default()
                            });
                    }
                } else {
                    let coordinator = syneroym_substrate::config::CoordinatorRole {
                        iroh: Some(syneroym_substrate::config::CoordinatorIrohConfig {
                            enabled: enable,
                            ..Default::default()
                        }),
                        ..Default::default()
                    };
                    config.roles.coordinator = Some(coordinator);
                }
            }

            if let Some(url) = iroh_relay_url {
                if let Some(ref mut iroh) = config.relay.iroh {
                    iroh.relay_url = url;
                } else {
                    config.relay.iroh =
                        Some(syneroym_substrate::config::IrohRelayConfig { relay_url: url });
                }
            }

            if let Some(enable) = enable_coordinator_webrtc {
                if let Some(ref mut coordinator) = config.roles.coordinator {
                    if let Some(ref mut webrtc) = coordinator.webrtc {
                        webrtc.enabled = enable;
                    } else {
                        coordinator.webrtc =
                            Some(syneroym_substrate::config::CoordinatorWebRtcConfig {
                                enabled: enable,
                                ..Default::default()
                            });
                    }
                } else {
                    let coordinator = syneroym_substrate::config::CoordinatorRole {
                        webrtc: Some(syneroym_substrate::config::CoordinatorWebRtcConfig {
                            enabled: enable,
                            ..Default::default()
                        }),
                        ..Default::default()
                    };
                    config.roles.coordinator = Some(coordinator);
                }
            }

            Ok(config)
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = resolve_config(cli.command)?;
    // Run substrate
    run(config).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_config_precedence() {
        // 1. Setup a dummy config file overriding some defaults
        let mut config_file = NamedTempFile::new().unwrap();
        let config_toml = r#"
        profile = "config_profile"
        
        [relay.iroh]
        relay_url = "http://config.relay:3340"
        
        [roles.coordinator.iroh]
        enabled = true
        http_bind_address = "0.0.0.0:8000"
        "#;
        write!(config_file, "{}", config_toml).unwrap();

        // 2. Setup CLI arguments overriding some config file values and some defaults
        // - Override config file: iroh_relay_url ("http://cli.relay:3340" vs "http://config.relay:3340")
        // - Override default (not in config): enable_coordinator_webrtc (true vs default false)
        let config_path_str = config_file.path().to_str().unwrap();
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
            config.roles.coordinator.as_ref().unwrap().iroh.as_ref().unwrap().enabled,
            "Coordinator iroh should be enabled from config"
        );
        assert_eq!(
            config.roles.coordinator.as_ref().unwrap().iroh.as_ref().unwrap().http_bind_address,
            "0.0.0.0:8000",
            "Coordinator iroh http bind address should be from config"
        );

        // Assert: Value specified in CLI overriding default (not in config)
        assert!(
            config.roles.coordinator.as_ref().unwrap().webrtc.as_ref().unwrap().enabled,
            "Coordinator webrtc should be enabled from CLI"
        );

        // Assert: Value specified in CLI overriding config file
        assert_eq!(
            config.relay.iroh.as_ref().unwrap().relay_url,
            "http://cli.relay:3340",
            "Iroh relay URL should be overridden by CLI"
        );
    }

    #[test]
    fn test_config_defaults_not_wrongly_overridden() {
        // 1. Setup a dummy config file overriding ONLY one default
        let mut config_file = NamedTempFile::new().unwrap();
        let config_toml = r#"
        profile = "only_profile_override"
        "#;
        write!(config_file, "{}", config_toml).unwrap();

        // 2. Setup CLI arguments overriding ONLY one default
        let config_path_str = config_file.path().to_str().unwrap();
        let args = vec![
            "syneroym",
            "run",
            "--config",
            config_path_str,
            "--enable-coordinator-webrtc",
            "true",
        ];

        let cli = Cli::parse_from(args);

        // 3. Resolve config
        let config = resolve_config(cli.command).expect("Failed to resolve config");

        // 4. Verify defaults are retained
        let default_config = SubstrateConfig::default();

        assert_eq!(config.profile, "only_profile_override", "Profile should be from config file");
        assert!(
            config.roles.coordinator.as_ref().unwrap().webrtc.as_ref().unwrap().enabled,
            "Coordinator webrtc should be enabled from CLI"
        );

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

        // Make sure iroh relay url is the default one, since we didn't specify it in config or CLI
        // SubstrateConfig::default().relay.iroh is None by default, but if someone changes the default logic, this checks it safely.
        if let Some(iroh_relay) = config.relay.iroh {
            let default_iroh = default_config.relay.iroh.unwrap_or_default();
            assert_eq!(
                iroh_relay.relay_url, default_iroh.relay_url,
                "Iroh relay URL should remain default"
            );
        }
    }
}

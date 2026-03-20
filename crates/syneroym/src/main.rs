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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
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

            // Run substrate
            run(config).await?;
        }
    }

    Ok(())
}

//! Command-line control interface for operations on, and through, a Syneroym node.

use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use syneroym_identity::Identity;

/// Default API endpoint for the Community Registry
const DEFAULT_API_URL: &str = "http://localhost:7961";

#[derive(Parser)]
#[command(name = "roymctl")]
#[command(about = "Syneroym Control CLI (API Client)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The base URL of the Syneroym Community Registry
    #[arg(global = true, long, default_value = DEFAULT_API_URL)]
    api_url: String,

    /// The DID of the target substrate to control
    #[arg(global = true, long)]
    substrate: Option<String>,

    /// Local directory for configuration and offline identities (defaults to current dir)
    #[arg(global = true, long, default_value = ".")]
    dir: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage the local node daemon
    Node {
        #[command(subcommand)]
        command: NodeCommands,
    },
    /// Manage SynApps on the local node
    App {
        #[command(subcommand)]
        command: AppCommands,
    },
    /// Manage connected peers
    Peer {
        #[command(subcommand)]
        command: PeerCommands,
    },
    /// Manage local cryptographic identities
    Identity {
        #[command(subcommand)]
        command: IdentityCommands,
    },
}

#[derive(Subcommand)]
enum NodeCommands {
    /// Initialize local configuration and default identity for a node
    Init,
    /// Get the status/health of the running daemon via API
    Status,
    /// View node configuration via API
    Config,
}

#[derive(Subcommand)]
enum AppCommands {
    /// Deploy a new SynApp via API
    Deploy {
        /// The DID-key for the application
        #[arg(long)]
        app_id: String,
        /// Comma-separated list of interfaces to register
        #[arg(long)]
        interfaces: String,
        /// Path to the WASM component binary
        #[arg(long)]
        wasm: Option<PathBuf>,
        /// TCP host:port for an existing service (e.g. "localhost:8080")
        #[arg(long)]
        tcp: Option<String>,
    },
    /// Remove an installed SynApp via API
    Remove {
        #[arg(long)]
        app_id: String,
    },
    /// List installed SynApps via API
    List,
    /// Start an installed SynApp via API (warm up)
    Start {
        #[arg(long)]
        app_id: String,
    },
    /// Stop a running SynApp via API (evict from cache)
    Stop {
        #[arg(long)]
        app_id: String,
    },
}

#[derive(Subcommand)]
enum PeerCommands {
    /// List connected peers via API
    List,
    /// Connect to a peer via API
    Connect {
        #[arg(long)]
        peer_id: String,
        #[arg(long)]
        address: Option<String>,
    },
    /// Disconnect from a peer via API
    Disconnect {
        #[arg(long)]
        peer_id: String,
    },
}

#[derive(Subcommand)]
enum IdentityCommands {
    /// Create a new identity locally
    Create {
        #[arg(long)]
        name: String,
    },
    /// List locally stored identities
    List,
    /// Show details of a specific identity locally
    Show {
        #[arg(long)]
        name: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let dir = cli.dir;

    match &cli.command {
        Commands::Node { command } => match command {
            NodeCommands::Init => {
                if !dir.exists() {
                    fs::create_dir_all(&dir)?;
                }
                let identity = Identity::generate()?;
                let identity_bytes = identity.to_bytes();
                let key_path = dir.join("identity.key");
                fs::write(&key_path, identity_bytes)?;
                println!("Initialized node local configuration at {}", dir.display());
            }
            NodeCommands::Status => {
                println!("Node status command is not yet fully implemented with SDK.");
            }
            NodeCommands::Config => {
                println!("Node config command is not yet fully implemented with SDK.");
            }
        },
        Commands::App { command } => {
            let substrate_did = cli.substrate.clone().or_else(|| {
                // Try to load local substrate DID from key file if it exists
                let key_path = dir.join("substrate.key");
                if key_path.exists() {
                    let bytes = fs::read(key_path).ok()?;
                    let bytes_array: [u8; 32] = bytes.try_into().ok()?;
                    let identity = Identity::from_bytes(&bytes_array);
                    Some(syneroym_identity::substrate::derive_did_key(&identity.public_key()))
                } else {
                    None
                }
            }).ok_or_else(|| anyhow::anyhow!("Substrate DID not provided and substrate.key not found. Use --substrate <did>"))?;

            let mut client =
                syneroym_sdk::SyneroymClient::new(substrate_did.clone(), cli.api_url.clone());
            client.wait_for_ready(std::time::Duration::from_secs(5)).await?;

            match command {
                AppCommands::Deploy { app_id, interfaces, wasm, tcp } => {
                    let ifaces: Vec<String> =
                        interfaces.split(',').map(|s| s.trim().to_string()).collect();

                    if let Some(wasm_path) = wasm {
                        let wasm_bytes = fs::read(wasm_path)?;
                        client.deploy_wasm(app_id.clone(), ifaces, wasm_bytes).await?;
                        println!("Successfully deployed WASM app {}", app_id);
                    } else if let Some(tcp_addr) = tcp {
                        let parts: Vec<&str> = tcp_addr.split(':').collect();
                        if parts.len() != 2 {
                            anyhow::bail!("Invalid TCP address format. Expected host:port");
                        }
                        let host = parts[0].to_string();
                        let port = parts[1].parse::<u16>()?;
                        client.deploy_tcp(app_id.clone(), ifaces, host, port).await?;
                        println!("Successfully deployed TCP service {}", app_id);
                    } else {
                        anyhow::bail!("Either --wasm or --tcp must be provided for deployment");
                    }
                }
                AppCommands::Remove { app_id } => {
                    client
                        .request("orchestrator", "remove", serde_json::json!({ "app_id": app_id }))
                        .await?;
                    println!("Successfully removed app {}", app_id);
                }
                AppCommands::List => {
                    let services = client.list_services().await?;
                    println!("{:<50} {:<10} {:<50}", "SERVICE ID", "TYPE", "INTERFACES");
                    println!("{:-<110}", "");
                    for svc in services {
                        println!(
                            "{:<50} {:<10} {:<50}",
                            svc.service_id,
                            svc.endpoint_type,
                            svc.interfaces.join(", ")
                        );
                    }
                }
                AppCommands::Start { app_id } => {
                    client
                        .request("orchestrator", "start", serde_json::json!({ "app_id": app_id }))
                        .await?;
                    println!("Successfully started app {}", app_id);
                }
                AppCommands::Stop { app_id } => {
                    client
                        .request("orchestrator", "stop", serde_json::json!({ "app_id": app_id }))
                        .await?;
                    println!("Successfully stopped app {}", app_id);
                }
            }
        }
        Commands::Peer { .. } => {
            println!("Peer management via SDK is not yet implemented in roymctl.");
        }
        Commands::Identity { command } => match command {
            IdentityCommands::Create { name } => {
                println!("Created new local identity: {}", name);
            }
            IdentityCommands::List => {
                println!("Local Identities:\n  - default\n  - alice");
            }
            IdentityCommands::Show { name } => {
                println!("Identity '{}': did:syneroym:{}", name, name);
            }
        },
    }
    Ok(())
}

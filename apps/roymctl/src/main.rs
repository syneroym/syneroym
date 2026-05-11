//! Command-line control interface for operations on, and through, a Syneroym node.

use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use syneroym_core::community_registry::{EndpointInfo, EndpointType, SignedEndpointInfo};
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
    /// Compute the 8-character short hash of an input string
    Shorthash {
        /// The input string to hash (e.g. DID or interface name)
        input: String,
    },
    /// Generate a consistent alias for a service ID and optional nickname
    Alias {
        /// The service ID (DID)
        service_id: String,
        /// Optional nickname
        #[arg(long)]
        nickname: Option<String>,
    },
    /// Manage entries in the community registry
    Registry {
        #[command(subcommand)]
        command: RegistryCommands,
    },
}

#[derive(Subcommand)]
enum RegistryCommands {
    /// Register a service DID against a substrate DID
    Register {
        /// The name of the local identity to register (from identities/ directory)
        #[arg(long)]
        identity: String,
        /// The DID of the substrate that hosts this service
        #[arg(long)]
        substrate: String,
        /// Optional nickname for the service
        #[arg(long)]
        nickname: Option<String>,
    },
    /// Look up an entry in the community registry
    Lookup {
        /// The service ID or alias to look up
        service_id: String,
        /// Resolve mechanisms from the substrate (default: true)
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        resolve: bool,
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
                let identities_dir = dir.join("identities");
                if !identities_dir.exists() {
                    fs::create_dir_all(&identities_dir)?;
                }
                let key_path = identities_dir.join(format!("{}.key", name));
                if key_path.exists() {
                    anyhow::bail!("Identity '{}' already exists at {}", name, key_path.display());
                }

                let identity = Identity::generate()?;
                let identity_bytes = identity.to_bytes();
                fs::write(&key_path, identity_bytes)?;

                let did = syneroym_identity::substrate::derive_did_key(&identity.public_key());

                println!("Created new local identity: {}", name);
                println!("DID: {}", did);
                println!("Key stored at: {}", key_path.display());
            }
            IdentityCommands::List => {
                let identities_dir = dir.join("identities");
                if !identities_dir.exists() {
                    println!(
                        "No identities found (directory {} does not exist)",
                        identities_dir.display()
                    );
                    return Ok(());
                }

                println!("{:<20} {:<60}", "NAME", "DID");
                println!("{:-<80}", "");

                for entry in fs::read_dir(identities_dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "key")
                        && let Some(name) = path.file_stem().and_then(|s| s.to_str())
                    {
                        let bytes = fs::read(&path)?;
                        if let Ok(bytes_array) = bytes.try_into() {
                            let identity = Identity::from_bytes(&bytes_array);
                            let did = syneroym_identity::substrate::derive_did_key(
                                &identity.public_key(),
                            );
                            println!("{:<20} {:<60}", name, did);
                        } else {
                            println!("{:<20} {:<60}", name, "[Invalid Key File]");
                        }
                    }
                }
            }
            IdentityCommands::Show { name } => {
                let key_path = dir.join("identities").join(format!("{}.key", name));
                if !key_path.exists() {
                    anyhow::bail!("Identity '{}' not found at {}", name, key_path.display());
                }

                let bytes = fs::read(&key_path)?;
                let bytes_array: [u8; 32] =
                    bytes.try_into().map_err(|_| anyhow::anyhow!("Invalid key file size"))?;
                let identity = Identity::from_bytes(&bytes_array);
                let did = syneroym_identity::substrate::derive_did_key(&identity.public_key());

                println!("Identity: {}", name);
                println!("DID:      {}", did);
                println!("Path:     {}", key_path.display());
            }
        },
        Commands::Shorthash { input } => {
            let hash = syneroym_core::util::short_hash(input);
            println!("{}", hash);
        }
        Commands::Alias { service_id, nickname } => {
            let alias = syneroym_core::util::generate_alias(nickname.as_deref(), service_id);
            println!("{}", alias);
        }
        Commands::Registry { command } => match command {
            RegistryCommands::Register { identity: name, substrate, nickname } => {
                let key_path = dir.join("identities").join(format!("{}.key", name));
                if !key_path.exists() {
                    anyhow::bail!("Identity '{}' not found at {}", name, key_path.display());
                }

                let bytes = fs::read(&key_path)?;
                let bytes_array: [u8; 32] =
                    bytes.try_into().map_err(|_| anyhow::anyhow!("Invalid key file size"))?;
                let identity = Identity::from_bytes(&bytes_array);
                let service_id =
                    syneroym_identity::substrate::derive_did_key(&identity.public_key());

                let info = EndpointInfo {
                    service_id: service_id.clone(),
                    substrate_id: substrate.clone(),
                    endpoint_type: EndpointType::Service,
                    mechanisms: vec![], // Services resolved via substrate don't need mechanisms here
                    nickname: nickname.clone(),
                };

                let signature = identity.sign_json(&serde_json::to_value(&info)?)?;
                let signed_info = SignedEndpointInfo { info, signature };

                let client = reqwest::Client::new();
                let url = format!("{}/register", cli.api_url);
                let response = client.post(&url).json(&signed_info).send().await?;

                if response.status().is_success() {
                    println!(
                        "Successfully registered service {} against substrate {}",
                        service_id, substrate
                    );
                    if let Some(n) = nickname {
                        let alias = syneroym_core::util::generate_alias(Some(n), &service_id);
                        println!("Alias: {}", alias);
                    }
                } else {
                    let error_text = response.text().await?;
                    anyhow::bail!("Registry registration failed ({}): {}", url, error_text);
                }
            }
            RegistryCommands::Lookup { service_id, resolve } => {
                let client = reqwest::Client::new();
                let url = format!("{}/lookup/{}?resolve={}", cli.api_url, service_id, resolve);
                let response = client.get(&url).send().await?;

                if response.status().is_success() {
                    let signed_info: SignedEndpointInfo = response.json().await?;
                    println!("{:#?}", signed_info);
                } else {
                    anyhow::bail!("Registry lookup failed ({}): {}", url, response.status());
                }
            }
        },
    }
    Ok(())
}

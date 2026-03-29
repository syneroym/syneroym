//! Command-line control interface for operations on, and through, a Syneroym node.

use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use syneroym_identity::Identity;

/// Default API endpoint for the local Substrate Daemon
const DEFAULT_API_URL: &str = "http://localhost:3000";

#[derive(Parser)]
#[command(name = "roymctl")]
#[command(about = "Syneroym Control CLI (API Client)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The base URL of the Syneroym Substrate API
    #[arg(global = true, long, default_value = DEFAULT_API_URL)]
    api_url: String,

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
    /// Deploy a new SynApp (WASM component) via API
    Deploy {
        #[arg(long)]
        app_id: String,
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Remove an installed SynApp via API
    Remove {
        #[arg(long)]
        app_id: String,
    },
    /// List installed SynApps via API
    List,
    /// Start an installed SynApp via API
    Start {
        #[arg(long)]
        app_id: String,
    },
    /// Stop a running SynApp via API
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

/// A mock API client for the CLI to interact with the Substrate Daemon.
/// In the future, this will use `reqwest::Client` to make actual HTTP calls.
struct ApiClient {
    base_url: String,
    // client: reqwest::Client,
}

impl ApiClient {
    fn new(base_url: String) -> Self {
        Self { base_url }
    }

    async fn get(&self, path: &str) -> anyhow::Result<()> {
        // let url = format!("{}{}", self.base_url, path);
        // let res = self.client.get(&url).send().await?;
        // println!("{}", res.text().await?);
        println!("--> [API Mock] GET {}{}", self.base_url, path);
        Ok(())
    }

    async fn post(&self, path: &str, body_stub: &str) -> anyhow::Result<()> {
        println!("--> [API Mock] POST {}{} with body: {}", self.base_url, path, body_stub);
        Ok(())
    }

    async fn delete(&self, path: &str) -> anyhow::Result<()> {
        println!("--> [API Mock] DELETE {}{}", self.base_url, path);
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let dir = cli.dir;
    let api = ApiClient::new(cli.api_url);

    match &cli.command {
        Commands::Node { command } => match command {
            NodeCommands::Init => {
                // Initialize local files without API calls.
                if !dir.exists() {
                    fs::create_dir_all(&dir)?;
                }

                let identity = Identity::generate();
                let identity_bytes = identity.to_bytes();

                let key_path = dir.join("identity.key");
                fs::write(&key_path, identity_bytes)?;

                println!("Initialized node local configuration at {}", dir.display());
            }
            NodeCommands::Status => {
                api.get("/api/v1/node/status").await?;
            }
            NodeCommands::Config => {
                api.get("/api/v1/node/config").await?;
            }
        },
        Commands::App { command } => match command {
            AppCommands::Deploy { app_id, manifest } => {
                let body = format!(
                    "{{ \"app_id\": \"{}\", \"manifest\": \"{}\" }}",
                    app_id,
                    manifest.display()
                );
                api.post("/api/v1/apps", &body).await?;
            }
            AppCommands::Remove { app_id } => {
                api.delete(&format!("/api/v1/apps/{}", app_id)).await?;
            }
            AppCommands::List => {
                api.get("/api/v1/apps").await?;
            }
            AppCommands::Start { app_id } => {
                api.post(&format!("/api/v1/apps/{}/start", app_id), "{}").await?;
            }
            AppCommands::Stop { app_id } => {
                api.post(&format!("/api/v1/apps/{}/stop", app_id), "{}").await?;
            }
        },
        Commands::Peer { command } => match command {
            PeerCommands::List => {
                api.get("/api/v1/peers").await?;
            }
            PeerCommands::Connect { peer_id, address } => {
                let body =
                    format!("{{ \"peer_id\": \"{}\", \"address\": {:?} }}", peer_id, address);
                api.post("/api/v1/peers/connect", &body).await?;
            }
            PeerCommands::Disconnect { peer_id } => {
                api.post(&format!("/api/v1/peers/{}/disconnect", peer_id), "{}").await?;
            }
        },
        Commands::Identity { command } => match command {
            IdentityCommands::Create { name } => {
                // Keep identity management local to the CLI keystore
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

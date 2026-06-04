//! App sandbox deployment and lifecycle subcommands
//!
//! Commands to package, deploy, start, list, and terminate sandboxed guest
//! apps.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Subcommand;
use syneroym_core::dht_registry::{EndpointInfo, EndpointType};
use syneroym_identity::Identity;
use syneroym_sdk::SyneroymClient;

#[derive(Subcommand, Debug, Clone)]
pub enum AppCommands {
    /// Deploy a new `SynApp` via API
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
        /// Optional identity name for signing a registry certificate
        #[arg(long)]
        identity: Option<String>,
        /// Optional nickname for the registry
        #[arg(long)]
        nickname: Option<String>,
    },
    /// Remove an installed `SynApp` via API
    Remove {
        #[arg(long)]
        app_id: String,
    },
    /// List installed `SynApps` via API
    List,
    /// Start an installed `SynApp` via API (warm up)
    Start {
        #[arg(long)]
        app_id: String,
    },

    /// Stop a running `SynApp` via API (evict from cache)
    Stop {
        #[arg(long)]
        app_id: String,
    },
}

/// Handle `SynApp` management subcommands
pub async fn handle(
    command: &AppCommands,
    api_url: &str,
    substrate_did: String,
    dir: &Path,
) -> anyhow::Result<()> {
    let mut client = SyneroymClient::new(substrate_did.clone(), api_url.to_string());
    client.wait_for_ready(Duration::from_secs(5)).await?;

    match command {
        AppCommands::Deploy { app_id, interfaces, wasm, tcp, identity, nickname } => {
            let ifaces: Vec<String> = interfaces.split(',').map(|s| s.trim().to_string()).collect();

            let mut cert = None;
            if let Some(name) = identity {
                let id = load_identity(dir, name)?;

                let info = EndpointInfo {
                    service_id: app_id.clone(),
                    substrate_id: substrate_did.clone(),
                    endpoint_type: EndpointType::Service,
                    mechanisms: vec![],
                    nickname: nickname.clone(),
                    is_private: false,
                    ttl: None,
                };
                cert = Some(info.sign(&id)?);
            }

            if let Some(wasm_path) = wasm {
                let wasm_bytes = fs::read(wasm_path)?;
                client.deploy_wasm(app_id.clone(), ifaces, wasm_bytes, cert).await?;
                println!("Successfully deployed WASM app {app_id}");
            } else if let Some(tcp_addr) = tcp {
                let (host, port) = get_host_port_from_tcp_addr(tcp_addr)?;
                client.deploy_tcp(app_id.clone(), ifaces, host, port, cert).await?;
                println!("Successfully deployed TCP service {app_id}");
            } else {
                anyhow::bail!("Either --wasm or --tcp must be provided for deployment");
            }
        }
        AppCommands::Remove { app_id } => {
            client
                .request("orchestrator", "remove", serde_json::json!({ "app_id": app_id }))
                .await?;
            println!("Successfully removed app {app_id}");
        }
        AppCommands::List => {
            // Lists all installed SynApps registered in the local substrate registry.
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
            println!("Successfully started app {app_id}");
        }
        AppCommands::Stop { app_id } => {
            client.request("orchestrator", "stop", serde_json::json!({ "app_id": app_id })).await?;
            println!("Successfully stopped app {app_id}");
        }
    }
    Ok(())
}

fn get_host_port_from_tcp_addr(tcp_addr: &str) -> anyhow::Result<(String, u16)> {
    let parts: Vec<&str> = tcp_addr.split(':').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid TCP address format. Expected host:port");
    }
    let host = parts[0].to_string();
    let port = parts[1].parse::<u16>()?;
    Ok((host, port))
}

fn load_identity(dir: &Path, name: &str) -> anyhow::Result<Identity> {
    let key_path = dir.join("identities").join(format!("{name}.key"));
    if !key_path.exists() {
        anyhow::bail!("Identity '{}' not found at {}", name, key_path.display());
    }
    Identity::load_from_path(&key_path)
}

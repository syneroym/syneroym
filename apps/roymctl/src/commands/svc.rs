//! Svc sandbox deployment and lifecycle subcommands
//!
//! Commands to package, deploy, start, list, and terminate sandboxed guest
//! svcs.

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
pub enum SvcCommands {
    /// Deploy a new `SynSvc` via API
    Deploy {
        /// The DID-key for the service
        #[arg(long)]
        svc_id: String,
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
    /// Remove an installed `SynSvc` via API
    Remove {
        #[arg(long)]
        svc_id: String,
    },
    /// List installed `SynSvcs` via API
    List,
    /// Start an installed `SynSvc` via API (warm up)
    Start {
        #[arg(long)]
        svc_id: String,
    },

    /// Stop a running `SynSvc` via API (evict from cache)
    Stop {
        #[arg(long)]
        svc_id: String,
    },
}

/// Handle `SynSvc` management subcommands
pub async fn handle(
    command: &SvcCommands,
    api_url: &str,
    substrate_did: String,
    dir: &Path,
) -> anyhow::Result<()> {
    let mut client = SyneroymClient::new(substrate_did.clone(), api_url.to_string());
    client.wait_for_ready(Duration::from_secs(5)).await?;

    match command {
        SvcCommands::Deploy { svc_id, interfaces, wasm, tcp, identity, nickname } => {
            let ifaces: Vec<String> = interfaces.split(',').map(|s| s.trim().to_string()).collect();

            let mut cert = None;
            if let Some(name) = identity {
                let id = load_identity(dir, name)?;

                let info = EndpointInfo {
                    service_id: svc_id.clone(),
                    substrate_id: substrate_did.clone(),
                    endpoint_type: EndpointType::Service,
                    mechanisms: vec![],
                    nickname: nickname.clone(),
                    is_private: false,
                    ttl: None,
                    delegation: None,
                };
                cert = Some(info.sign(&id)?);
            }

            if let Some(wasm_path) = wasm {
                let wasm_bytes = fs::read(wasm_path)?;
                let interfaces_list =
                    if ifaces.is_empty() { vec!["default".to_string()] } else { ifaces };
                client.deploy_svc_wasm(svc_id.clone(), interfaces_list, wasm_bytes, cert).await?;
                println!("Successfully deployed WASM svc {svc_id}");
            } else if let Some(tcp_addr) = tcp {
                if ifaces.len() > 1 {
                    anyhow::bail!("TCP deployments only support a single interface for now");
                }
                let (host, port) = get_host_port_from_tcp_addr(tcp_addr)?;
                let iface = ifaces.first().cloned().unwrap_or_else(|| "default".to_string());
                let endpoints =
                    vec![syneroym_sdk::NetworkEndpoint { interface_name: iface, host, port }];
                client.deploy_svc_tcp(svc_id.clone(), endpoints, cert).await?;
                println!("Successfully deployed TCP service {svc_id}");
            } else {
                anyhow::bail!("Either --wasm or --tcp must be provided for deployment");
            }
        }
        SvcCommands::Remove { svc_id } => {
            client.undeploy(svc_id.clone()).await?;
            println!("Successfully removed svc {svc_id}");
        }
        SvcCommands::List => {
            // Lists all installed SynSvcs registered in the local substrate registry.
            let services = client.list_svcs().await?;
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
        SvcCommands::Start { svc_id } => {
            client
                .request("orchestrator", "start", serde_json::json!({ "service_id": svc_id }))
                .await?;
            println!("Successfully started svc {svc_id}");
        }
        SvcCommands::Stop { svc_id } => {
            client
                .request("orchestrator", "stop", serde_json::json!({ "service_id": svc_id }))
                .await?;
            println!("Successfully stopped svc {svc_id}");
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

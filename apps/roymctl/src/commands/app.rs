use clap::Subcommand;
use std::fs;
use std::path::PathBuf;

#[derive(Subcommand, Debug, Clone)]
pub enum AppCommands {
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

/// Handle SynApp management subcommands
pub async fn handle(
    command: &AppCommands,
    api_url: &str,
    substrate_did: String,
) -> anyhow::Result<()> {
    let mut client = syneroym_sdk::SyneroymClient::new(substrate_did, api_url.to_string());
    client.wait_for_ready(std::time::Duration::from_secs(5)).await?;

    match command {
        AppCommands::Deploy { app_id, interfaces, wasm, tcp } => {
            let ifaces: Vec<String> = interfaces.split(',').map(|s| s.trim().to_string()).collect();

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
            client.request("orchestrator", "stop", serde_json::json!({ "app_id": app_id })).await?;
            println!("Successfully stopped app {}", app_id);
        }
    }
    Ok(())
}

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
use syneroym_identity::{Identity, substrate};

use super::member_identity;

/// The attended posture's default certificate lifetime for a deploy-time
/// certification via `svc deploy --master` -- `identity certify-instance`
/// is the dedicated renewal command for a longer- or shorter-lived one.
const DEFAULT_INSTANCE_CERT_EXPIRES_HOURS: u64 = 24;

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
        /// Name of a local member master identity (ADR-0020 §1). When
        /// present, `--svc-id` must equal that identity's DID; the substrate
        /// is queried for the instance key it would derive, a
        /// `service-instance` certificate is issued and installed at
        /// deploy. Absent leaves the service its own master, exactly as
        /// before this flag existed.
        #[arg(long)]
        master: Option<String>,
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
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    let mut client = super::client_for(substrate_did.clone(), api_url, dir, run_as, ucan_path)?;
    client.wait_for_ready(Duration::from_secs(5)).await?;

    match command {
        SvcCommands::Deploy { svc_id, interfaces, wasm, tcp, identity, nickname, master } => {
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

            let instance_cert = match master {
                Some(name) => {
                    let master_identity = member_identity::resolve_member_master(dir, name)?;
                    let master_did = substrate::derive_did_key(&master_identity.public_key());
                    if master_did != *svc_id {
                        anyhow::bail!(
                            "--master '{name}' resolves to {master_did}, which does not match \
                             --svc-id {svc_id} -- an install-time certificate for this pair would \
                             be rejected"
                        );
                    }
                    Some(
                        member_identity::certify_instance(
                            &client,
                            &master_identity,
                            svc_id,
                            DEFAULT_INSTANCE_CERT_EXPIRES_HOURS,
                        )
                        .await?,
                    )
                }
                None => None,
            };

            if let Some(wasm_path) = wasm {
                let wasm_bytes = fs::read(wasm_path)?;
                let interfaces_list =
                    if ifaces.is_empty() { vec!["default".to_string()] } else { ifaces };
                client
                    .deploy_svc_wasm(
                        svc_id.clone(),
                        interfaces_list,
                        wasm_bytes,
                        cert,
                        instance_cert,
                    )
                    .await?;
                println!("Successfully deployed WASM svc {svc_id}");
            } else if let Some(tcp_addr) = tcp {
                if ifaces.len() > 1 {
                    anyhow::bail!("TCP deployments only support a single interface for now");
                }
                let (host, port) = get_host_port_from_tcp_addr(tcp_addr)?;
                let iface = ifaces.first().cloned().unwrap_or_else(|| "default".to_string());
                let endpoints =
                    vec![syneroym_sdk::NetworkEndpoint { interface_name: iface, host, port }];
                client.deploy_svc_tcp(svc_id.clone(), endpoints, cert, instance_cert).await?;
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
            println!(
                "{:<50} {:<10} {:<30} {:<50}",
                "SERVICE ID", "TYPE", "INSTANCE CERT EXPIRES", "INTERFACES"
            );
            println!("{:-<145}", "");
            for svc in services {
                println!(
                    "{:<50} {:<10} {:<30} {:<50}",
                    svc.service_id,
                    svc.endpoint_type,
                    format_expiry(svc.instance_certificate_expires_at),
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

/// `-` for a service with no installed instance certificate; otherwise an
/// RFC 3339 timestamp, so "when does this fall over" is answerable without
/// reading logs (ADR-0020 §3).
fn format_expiry(expires_at_secs: Option<u64>) -> String {
    match expires_at_secs {
        Some(secs) => chrono::DateTime::from_timestamp(secs as i64, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "-".to_string()),
        None => "-".to_string(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_expiry_shows_a_dash_for_no_certificate() {
        assert_eq!(format_expiry(None), "-");
    }

    #[test]
    fn format_expiry_shows_an_rfc3339_timestamp() {
        // 2024-01-01T00:00:00Z
        assert_eq!(format_expiry(Some(1_704_067_200)), "2024-01-01T00:00:00+00:00");
    }
}

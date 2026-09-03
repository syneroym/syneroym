//! Commands specific to the Roym product app.

use std::{path::Path, time::Duration};

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::json;
use syneroym_identity::{DelegationCertificate, Identity, substrate};
use syneroym_sdk::DeployedService;
use syneroym_signed_record::SCOPE_RECORD_SIGNING;

use crate::DEFAULT_GATEWAY_URL;

#[derive(Subcommand, Debug, Clone)]
pub enum RoymCommands {
    /// Enrol the person's record-signing certificate for Roym services.
    EnrolSigning {
        #[arg(long)]
        master: Option<String>,
        #[arg(long, default_value_t = 720)]
        expires_hours: u64,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        registry_url: Option<String>,
    },
    /// Query record-signing status across Roym services.
    SigningStatus {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Print this installation's own Roym Conversation service id and the
    /// gateway host for the Hub. Paste the service id into `profile.set` as
    /// `conversation_address` so others can message you, without reading a
    /// deploy log. Reads only what `svc list` already reports.
    Address {
        /// The domain the Hub gateway host is served under.
        #[arg(long, default_value = "localhost")]
        domain: String,
    },
}

/// Every Roym service that signs a record and so needs a record-signing
/// certificate of its own. `directory` and `transaction` sign nothing yet,
/// so a certificate there would be a verb no flow exercises.
const SIGNING_SERVICES: &[&str] = &["profile", "catalog", "conversation"];

pub async fn handle(
    command: &RoymCommands,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    match command {
        RoymCommands::EnrolSigning { master, expires_hours, gateway_url, host, registry_url } => {
            let master_name = master.as_deref().or(run_as).unwrap_or("owner");
            let key_path = dir.join("identities").join(format!("{master_name}.key"));
            if !key_path.exists() {
                anyhow::bail!(
                    "Master identity '{master_name}' not found at {}",
                    key_path.display()
                );
            }
            let master_identity = Identity::load_from_path(&key_path)?;
            let master_did = substrate::derive_did_key(&master_identity.public_key());

            super::member_identity::refresh_anchor_or_warn(
                registry_url.as_deref(),
                &master_identity,
            )
            .await?;

            let mut failures = 0u32;
            for prefix in SIGNING_SERVICES {
                match enrol_one(
                    prefix,
                    &master_identity,
                    &master_did,
                    master_name,
                    *expires_hours,
                    gateway_url,
                    host.as_deref(),
                    run_as,
                    ucan_path,
                    dir,
                )
                .await
                {
                    Ok(expires_at) => println!("{prefix}: enrolled until timestamp {expires_at}"),
                    Err(e) => {
                        failures += 1;
                        eprintln!("{prefix}: FAILED: {e:#}");
                    }
                }
            }
            if failures > 0 {
                anyhow::bail!("{failures} service(s) failed to enrol");
            }
        }
        RoymCommands::SigningStatus { gateway_url, host } => {
            let mut failures = 0u32;
            for prefix in SIGNING_SERVICES {
                match super::session::rpc_call(
                    gateway_url,
                    host.as_deref(),
                    run_as,
                    ucan_path,
                    dir,
                    &format!("{prefix}.signing-status"),
                    json!({}),
                )
                .await
                {
                    Ok(status_val) => println!(
                        "{prefix}: {}",
                        serde_json::to_string(&status_val).unwrap_or_default()
                    ),
                    Err(e) => {
                        failures += 1;
                        eprintln!("{prefix}: FAILED: {e:#}");
                    }
                }
            }
            if failures > 0 {
                anyhow::bail!("{failures} service(s) failed to report status");
            }
        }
        RoymCommands::Address { domain } => {
            let substrate_did = super::get_substrate_did(substrate_opt, dir)?;
            let mut client = super::client_for(substrate_did, api_url, dir, run_as, ucan_path)?;
            client.wait_for_ready(Duration::from_secs(5)).await?;

            let svcs = client.list_svcs().await?;
            let conversation_id = find_roym_service(&svcs, "conversation")?;
            let web_id = find_roym_service(&svcs, "web")?;
            let hub_host = syneroym_core::util::generate_service_host(None, &web_id, None, domain)?;

            println!("conversation service id: {conversation_id}");
            println!("  paste this into profile.set as `conversation_address`");
            println!("Hub gateway host:        {hub_host}");
        }
    }
    Ok(())
}

/// The physical service id of a Roym logical service, found by the app
/// interface it registers (`syneroym-roym:<name>/...`). Reads only what
/// `svc list` already returns, so it invents no resolution path: no host
/// surface reports a service its own routing address, so a person would
/// otherwise have to read it out of a deploy log.
fn find_roym_service(svcs: &[DeployedService], name: &str) -> Result<String> {
    let prefix = format!("syneroym-roym:{name}/");
    let matches: Vec<&str> = svcs
        .iter()
        .filter(|s| s.interfaces.iter().any(|i| i.starts_with(&prefix)))
        .map(|s| s.service_id.as_str())
        .collect();
    match matches.as_slice() {
        [] => anyhow::bail!(
            "no Roym '{name}' service is deployed on this installation -- deploy the Roym app \
             first"
        ),
        [only] => Ok((*only).to_string()),
        many => anyhow::bail!(
            "{} Roym '{name}' services are deployed; cannot choose one address",
            many.len()
        ),
    }
}

#[allow(clippy::too_many_arguments)]
async fn enrol_one(
    prefix: &str,
    master_identity: &Identity,
    master_did: &str,
    master_name: &str,
    expires_hours: u64,
    gateway_url: &str,
    host: Option<&str>,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
    dir: &Path,
) -> Result<u64> {
    let status_val = super::session::rpc_call(
        gateway_url,
        host,
        run_as,
        ucan_path,
        dir,
        &format!("{prefix}.signing-status"),
        json!({}),
    )
    .await
    .with_context(|| format!("failed to fetch {prefix}.signing-status"))?;

    if let Some(recorded_owner) = status_val.get("owner_did").and_then(|v| v.as_str())
        && recorded_owner != master_did
    {
        anyhow::bail!(
            "this installation's recorded owner is '{recorded_owner}', but master identity \
             '{master_name}' has DID '{master_did}'"
        );
    }

    let signing_did_str = status_val
        .get("signing_did")
        .and_then(|v| v.as_str())
        .context("signing-status output missing 'signing_did'")?;
    let signing_pubkey =
        substrate::resolve_did_key(signing_did_str).context("failed to resolve signing_did")?;

    let cert = DelegationCertificate::issue(
        master_identity,
        signing_pubkey,
        expires_hours * 3600,
        SCOPE_RECORD_SIGNING.to_string(),
    )?;

    super::session::rpc_call(
        gateway_url,
        host,
        run_as,
        ucan_path,
        dir,
        &format!("{prefix}.install-signing-certificate"),
        json!({ "certificate": cert.to_json()? }),
    )
    .await
    .with_context(|| format!("failed to install {prefix} signing certificate"))?;

    Ok(cert.expires_at_secs)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn svc(service_id: &str, interfaces: &[&str]) -> DeployedService {
        serde_json::from_value(json!({
            "service_id": service_id,
            "interfaces": interfaces,
            "endpoint_type": "wasm",
        }))
        .unwrap()
    }

    fn roym_svcs() -> Vec<DeployedService> {
        vec![
            svc("did:key:zWeb", &["syneroym-roym:web/api@0.1.0"]),
            svc("did:key:zProfile", &["syneroym-roym:profile/api@0.1.0"]),
            svc("did:key:zConv", &["syneroym-roym:conversation/api@0.1.0"]),
            svc("did:key:zOther", &["syneroym:http/incoming-handler@0.2.0"]),
        ]
    }

    #[test]
    fn find_roym_service_matches_by_app_interface() {
        let svcs = roym_svcs();
        assert_eq!(find_roym_service(&svcs, "conversation").unwrap(), "did:key:zConv");
        assert_eq!(find_roym_service(&svcs, "web").unwrap(), "did:key:zWeb");
    }

    #[test]
    fn find_roym_service_errors_when_absent() {
        let err = find_roym_service(&roym_svcs(), "directory").unwrap_err().to_string();
        assert!(err.contains("no Roym 'directory' service is deployed"), "{err}");
    }

    #[test]
    fn find_roym_service_errors_when_ambiguous() {
        let svcs = vec![
            svc("did:key:zConvA", &["syneroym-roym:conversation/api@0.1.0"]),
            svc("did:key:zConvB", &["syneroym-roym:conversation/api@0.1.0"]),
        ];
        let err = find_roym_service(&svcs, "conversation").unwrap_err().to_string();
        assert!(err.contains("2 Roym 'conversation' services"), "{err}");
    }
}

//! Commands specific to the Roym product app.

use std::path::Path;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::json;
use syneroym_identity::{DelegationCertificate, Identity, substrate};
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
}

pub async fn handle(
    command: &RoymCommands,
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

            super::member_identity::refresh_anchor_or_warn(
                registry_url.as_deref(),
                &master_identity,
            )
            .await?;

            let prefix = "profile";
            let status_val = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                &format!("{prefix}.signing-status"),
                json!({}),
            )
            .await
            .context("failed to fetch signing-status")?;

            if let Some(recorded_owner) = status_val.get("owner_did").and_then(|v| v.as_str()) {
                let master_did = substrate::derive_did_key(&master_identity.public_key());
                if recorded_owner != master_did {
                    anyhow::bail!(
                        "this installation's recorded owner is '{recorded_owner}', but master \
                         identity '{master_name}' has DID '{master_did}'"
                    );
                }
            }

            let signing_did_str = status_val
                .get("signing_did")
                .and_then(|v| v.as_str())
                .context("signing-status output missing 'signing_did'")?;

            let signing_pubkey = substrate::resolve_did_key(signing_did_str)
                .context("Failed to resolve signing_did")?;

            let cert = DelegationCertificate::issue(
                &master_identity,
                signing_pubkey,
                expires_hours * 3600,
                SCOPE_RECORD_SIGNING.to_string(),
            )?;

            let install_res = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                &format!("{prefix}.install-signing-certificate"),
                json!({ "certificate": cert.to_json()? }),
            )
            .await
            .context("failed to install signing certificate")?;

            println!("Enrolled signing certificate until timestamp {}", cert.expires_at_secs);
            println!("{}", serde_json::to_string_pretty(&install_res)?);
        }
        RoymCommands::SigningStatus { gateway_url, host } => {
            let prefix = "profile";
            let status_val = super::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                &format!("{prefix}.signing-status"),
                json!({}),
            )
            .await
            .context("failed to fetch signing-status")?;

            println!("{}", serde_json::to_string_pretty(&status_val)?);
        }
    }
    Ok(())
}

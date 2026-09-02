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

/// Every Roym service that signs a record and so needs a record-signing
/// certificate of its own. `directory` and `transaction` sign nothing yet,
/// so a certificate there would be a verb no flow exercises.
const SIGNING_SERVICES: &[&str] = &["profile", "catalog", "conversation"];

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
    }
    Ok(())
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

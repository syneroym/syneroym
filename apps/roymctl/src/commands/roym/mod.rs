//! Commands specific to the Roym product app.

use std::path::Path;

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::json;

use crate::DEFAULT_GATEWAY_URL;

pub mod address;
pub mod directory;
pub mod signing;
pub mod transaction;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use address::find_roym_service;
pub use directory::{DirectoryCommands, MemberCommands};
#[cfg(test)]
pub(crate) use syneroym_sdk::DeployedService;
pub use transaction::TransactionCommands;
#[cfg(test)]
pub(crate) use transaction::{parse_minor_units, parse_window};

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
    /// The SynOrg / Directory service: publishing a listing, adding
    /// sources, and searching -- the same JSON-RPC API the Hub drives,
    /// through the gateway, with no browser involved.
    Directory {
        #[command(subcommand)]
        command: DirectoryCommands,
    },
    /// The Transaction vertical: requests, quotes, agreements, sync and
    /// threads.
    Transaction {
        #[command(subcommand)]
        command: Box<TransactionCommands>,
    },
}

/// Parses `lat,lon,radius_m` at this boundary and converts to integer
/// micro-degrees: nothing decimal reaches a signed payload, and this is
/// the one place a person's decimal input becomes that integer.
///
/// Shared by `directory` and `transaction`: both accept a `--near` filter in
/// the same format.
pub(super) fn parse_near(input: &str) -> Result<serde_json::Value> {
    let parts: Vec<&str> = input.split(',').collect();
    let [lat, lon, radius] = parts.as_slice() else {
        anyhow::bail!("--near expects lat,lon,radius_m");
    };
    let lat: f64 = lat.trim().parse().context("invalid latitude")?;
    let lon: f64 = lon.trim().parse().context("invalid longitude")?;
    let radius: f64 = radius.trim().parse().context("invalid radius_m")?;
    Ok(json!({
        "kind": "circle",
        "lat_e6": (lat * 1e6).round() as i64,
        "lon_e6": (lon * 1e6).round() as i64,
        "radius_m": radius.round() as u64,
    }))
}

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
            signing::handle_enrol_signing(
                master.clone(),
                *expires_hours,
                gateway_url,
                host.as_deref(),
                registry_url.clone(),
                dir,
                run_as,
                ucan_path,
            )
            .await
        }
        RoymCommands::SigningStatus { gateway_url, host } => {
            signing::handle_signing_status(gateway_url, host.as_deref(), dir, run_as, ucan_path)
                .await
        }
        RoymCommands::Directory { command } => {
            directory::handle_directory(command, dir, run_as, ucan_path).await
        }
        RoymCommands::Transaction { command } => {
            transaction::handle_transaction(command, dir, run_as, ucan_path).await
        }
        RoymCommands::Address { domain } => {
            address::handle_address(domain, api_url, substrate_opt, dir, run_as, ucan_path).await
        }
    }
}

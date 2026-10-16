//! Substrate home-relay subcommands: configure or query the home relay
//! passport.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
use clap::{Args, Subcommand};
use syneroym_identity::Identity;
use syneroym_passport::{
    CapabilityToken, RELAY_HOME,
    wire::{HomeRelaySetRequest, HomeRelayStatus},
};

use crate::commands;

#[derive(Subcommand, Debug, Clone)]
pub enum HomeRelayCommands {
    /// Hand a relay passport to the running substrate. The command signs a
    /// child token of the passport for the substrate's node key with the
    /// `--as` identity, which must be the passport's owner and own the
    /// substrate. The substrate registers at once and renews by itself.
    /// A substrate whose home relay is a passport relay can only be reached
    /// by a direct path: run this before you point
    /// `[parent_coordinator.iroh] url` at the passport relay, then restart.
    Set(SetArgs),
    /// Show the stored passport, the lease and the last error.
    Status,
}

#[derive(Args, Debug, Clone)]
pub struct SetArgs {
    /// Base URL of the coordinator's info server, `https://host:port` (the
    /// address of `/v1/info`). It is not the relay URL.
    #[arg(long)]
    pub info_url: String,
    /// The passport file from `roymctl coordinator passport issue`.
    #[arg(long)]
    pub passport: PathBuf,
    /// The relay the passport is for. Defaults to the substrate's
    /// `parent_coordinator.iroh.url`.
    #[arg(long)]
    pub relay_url: Option<String>,
    /// A PEM root certificate to trust for the info server.
    #[arg(long)]
    pub ca_cert: Option<PathBuf>,
}

/// Offline part of `set`: read the files, sign the child token, check it.
/// Public so tests can send the request over another connection.
pub fn build_set_request(
    args: &SetArgs,
    node_did: &str,
    dir: &Path,
    run_as: Option<&str>,
) -> anyhow::Result<HomeRelaySetRequest> {
    let name =
        run_as.context("home-relay set signs with the passport owner's key: pass --as <name>")?;
    let key_path = commands::identity::require_key_present(dir, name, "Identity")?;
    let owner = Identity::load_from_path(key_path)?;
    let passport_raw = fs::read_to_string(&args.passport)
        .with_context(|| format!("cannot read {}", args.passport.display()))?;
    let passport: CapabilityToken =
        serde_json::from_str(&passport_raw).context("the passport file is not a passport")?;
    let now = syneroym_passport::now_secs();
    let life = passport.expires_at_secs.saturating_sub(now);
    let child = syneroym_passport::child_token(&owner, &passport, node_did, life, now)?;
    syneroym_passport::inspect_child_token(&child, &RELAY_HOME, node_did, now)
        .map_err(|e| anyhow!("{}: {e}", e.code()))?;
    let ca_cert_pem = args.ca_cert.as_ref().map(fs::read_to_string).transpose()?;
    Ok(HomeRelaySetRequest {
        token: child,
        info_url: args.info_url.clone(),
        relay_url: args.relay_url.clone(),
        ca_cert_pem,
    })
}

pub async fn set(
    args: &SetArgs,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<HomeRelayStatus> {
    let node_did = commands::get_substrate_did(substrate_opt.clone(), dir)?;
    let request = build_set_request(args, &node_did, dir, run_as)?;
    let client = commands::online_client(api_url, substrate_opt, dir, run_as, ucan_path).await?;
    let status = client.home_relay_set(&request).await?;
    Ok(status)
}

pub async fn status(
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<Option<HomeRelayStatus>> {
    let client = commands::online_client(api_url, substrate_opt, dir, run_as, ucan_path).await?;
    let status = client.home_relay_status().await?;
    Ok(status)
}

/// The one-line summary under the status JSON. `now` decides whether the
/// last lease still runs.
pub fn summary_line(status: Option<&HomeRelayStatus>, now: u64) -> String {
    let Some(status) = status else {
        return "no home-relay passport is stored".to_string();
    };
    if !status.active {
        return format!(
            "stored, not used: parent_coordinator.iroh.url is not {}",
            status.relay_url
        );
    }
    if let Some(lease_end) = status.lease_ends_at.filter(|end| *end > now) {
        if let Some(err) = &status.last_error {
            return format!(
                "the lease runs until {lease_end}, but the last registration failed: {}: {}",
                err.code, err.message
            );
        }
        return format!("registered; the lease ends at {lease_end}");
    }
    if let Some(err) = &status.last_error {
        return format!("not registered: {}: {}", err.code, err.message);
    }
    match status.next_attempt_at {
        Some(t) => format!("not registered yet; next attempt at {t}"),
        None => "no attempt planned".to_string(),
    }
}

pub async fn handle(
    command: &HomeRelayCommands,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    match command {
        HomeRelayCommands::Set(args) => {
            let status = set(args, api_url, substrate_opt, dir, run_as, ucan_path).await?;
            commands::print_json_result(&serde_json::to_value(&status)?)?;
            let now = syneroym_passport::now_secs();
            println!("{}", summary_line(Some(&status), now));
        }
        HomeRelayCommands::Status => {
            let status = status(api_url, substrate_opt, dir, run_as, ucan_path).await?;
            match &status {
                Some(s) => commands::print_json_result(&serde_json::to_value(s)?)?,
                None => println!("null"),
            }
            let now = syneroym_passport::now_secs();
            println!("{}", summary_line(status.as_ref(), now));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

//! Coordinator operator subcommands: passports for a private relay.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, ensure};
use clap::{ArgGroup, Args, Subcommand};
use serde_json::{Map, json};
use syneroym_core::config::DEFAULT_SUBSTRATE_KEY_FILE;
use syneroym_identity::{Identity, substrate};
use syneroym_passport::{PassportId, RELAY_HOME, facts};
use syneroym_sdk::SyneroymClient;

use super::identity;

const SECS_PER_DAY: u64 = 24 * 60 * 60;

#[derive(Subcommand, Debug, Clone)]
pub enum CoordinatorCommands {
    /// Operator passports for a relay with `access = "passport"`
    Passport {
        #[command(subcommand)]
        command: PassportCommands,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum PassportCommands {
    /// Issue a passport that lets one owner's substrates use this relay as
    /// their home relay. Offline: it only signs. Give the file to the owner.
    ///
    /// The passport id comes from the signed content. Two runs with the same
    /// options in the same second make the same passport, with one id. To
    /// give out two passports, make them differ (for example by expiry).
    ///
    /// A host with a controller accepts only passports signed by the
    /// controller.
    Issue(IssueArgs),
    /// Deny a passport on the running relay. The relay ends its leases at
    /// once and refuses the passport from now on. A connection that is
    /// already open stays until it closes. Run as the substrate's owner
    /// (`--as <name>`). There is no undo: issue a new passport instead.
    Deny {
        /// The passport id `issue` printed (64 hex characters).
        passport_id: String,
    },
    /// List the running leases of the relay. Run as the substrate's owner
    /// (`--as <name>`).
    Leases,
}

/// The flags of `coordinator passport issue`. The host node DID comes from
/// the global `--substrate` (or `<dir>/substrate.key`).
#[derive(Args, Debug, Clone)]
#[command(group(ArgGroup::new("signer").required(true).args(["from", "node_key"])))]
pub struct IssueArgs {
    /// Local identity that signs: the controller of the relay's host
    /// substrate.
    #[arg(long)]
    pub from: Option<String>,
    /// Sign with the node key `<dir>/substrate.key`. Only for a host
    /// substrate that has no controller.
    #[arg(long)]
    pub node_key: bool,
    /// The owner DID the passport is for.
    #[arg(long)]
    pub to: String,
    /// Days until the passport expires.
    #[arg(long)]
    pub expires_days: u64,
    /// Most distinct substrates that may hold a lease at one time.
    /// Omitted: no limit.
    #[arg(long)]
    pub max_nodes: Option<u64>,
    /// Where to write the passport. Omitted: standard output.
    #[arg(long)]
    pub out: Option<PathBuf>,
}

pub async fn handle(
    command: &CoordinatorCommands,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    let CoordinatorCommands::Passport { command } = command;
    match command {
        PassportCommands::Issue(args) => handle_issue(args, substrate_opt, dir).map(|_| ()),
        PassportCommands::Deny { passport_id } => {
            let ended = deny(passport_id, api_url, substrate_opt, dir, run_as, ucan_path).await?;
            println!("passport {passport_id} denied; {ended} running lease(s) ended");
            Ok(())
        }
        PassportCommands::Leases => {
            let list = leases(api_url, substrate_opt, dir, run_as, ucan_path).await?;
            super::print_json_result(&serde_json::to_value(list)?)
        }
    }
}

pub async fn deny(
    passport_id: &str,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<u64> {
    let id: PassportId = passport_id.parse()?;
    let client = online_client(api_url, substrate_opt, dir, run_as, ucan_path).await?;
    client.relay_passport_deny(&id).await
}

pub async fn leases(
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<Vec<syneroym_passport::wire::LeaseInfo>> {
    let client = online_client(api_url, substrate_opt, dir, run_as, ucan_path).await?;
    client.relay_passport_leases().await
}

async fn online_client(
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<SyneroymClient> {
    let substrate_did = super::get_substrate_did(substrate_opt, dir)?;
    let mut client = super::client_for(substrate_did, api_url, dir, run_as, ucan_path)?;
    client.wait_for_ready(Duration::from_secs(5)).await?;
    Ok(client)
}

/// Sign a relay passport, write it, and print its id. The operator needs
/// the id to deny the passport later.
fn handle_issue(
    args: &IssueArgs,
    substrate_opt: Option<String>,
    dir: &Path,
) -> anyhow::Result<PassportId> {
    let host_node_did = super::get_substrate_did(substrate_opt, dir)?;
    let signer = load_signer(dir, args.from.as_deref(), args.node_key, &host_node_did)?;
    ensure!(args.expires_days > 0, "--expires-days must be at least 1");
    let expires_in_secs =
        args.expires_days.checked_mul(SECS_PER_DAY).context("--expires-days is too large")?;
    let mut limits = Map::new();
    if let Some(n) = args.max_nodes {
        limits.insert(facts::MAX_NODES.to_string(), json!(n));
    }
    let passport = syneroym_passport::issue_passport(
        &signer,
        &host_node_did,
        &RELAY_HOME,
        &args.to,
        limits,
        expires_in_secs,
        syneroym_passport::now_secs(),
    )?;
    let json = serde_json::to_string_pretty(&passport)?;
    match &args.out {
        Some(path) => super::write_secret_file(path, json.as_bytes(), "passport")?,
        None => println!("{json}"),
    }
    let id = PassportId::of(&passport)?;
    eprintln!("passport id: {id}");
    Ok(id)
}

/// The signing key: a named local identity, or the node key of the host
/// substrate. The command never picks one by itself.
fn load_signer(
    dir: &Path,
    from: Option<&str>,
    node_key: bool,
    host_node_did: &str,
) -> anyhow::Result<Identity> {
    match (from, node_key) {
        (Some(name), false) => {
            Identity::load_from_path(identity::require_key_present(dir, name, "Identity")?)
        }
        (None, true) => {
            let key = Identity::load_from_path(dir.join(DEFAULT_SUBSTRATE_KEY_FILE))
                .context("--node-key given, but there is no substrate.key in --dir")?;
            let key_did = substrate::derive_did_key(&key.public_key());
            ensure!(
                key_did == host_node_did,
                "substrate.key is the key of {key_did}, not of --substrate {host_node_did}"
            );
            Ok(key)
        }
        _ => anyhow::bail!("pass exactly one of --from <name> or --node-key"),
    }
}

#[cfg(test)]
mod tests;

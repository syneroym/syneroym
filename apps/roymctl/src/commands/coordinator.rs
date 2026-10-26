//! Coordinator operator subcommands: passports for a private relay.

use std::path::Path;

use clap::{Args, Subcommand};
use serde_json::{Map, json};
use syneroym_passport::{PassportId, RELAY_HOME, facts};

use super::passport_issue::{self, IssueSigner};

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
pub struct IssueArgs {
    #[command(flatten)]
    pub signer: IssueSigner,
    /// Most distinct substrates that may hold a lease at one time.
    /// Omitted: no limit.
    #[arg(long)]
    pub max_nodes: Option<u64>,
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
    let client = super::online_client(api_url, substrate_opt, dir, run_as, ucan_path).await?;
    client.relay_passport_deny(&id).await
}

pub async fn leases(
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<Vec<syneroym_passport::wire::LeaseInfo>> {
    let client = super::online_client(api_url, substrate_opt, dir, run_as, ucan_path).await?;
    client.relay_passport_leases().await
}

/// Sign a relay passport, write it, and print its id. The operator needs
/// the id to deny the passport later.
fn handle_issue(
    args: &IssueArgs,
    substrate_opt: Option<String>,
    dir: &Path,
) -> anyhow::Result<PassportId> {
    let mut limits = Map::new();
    if let Some(n) = args.max_nodes {
        limits.insert(facts::MAX_NODES.to_string(), json!(n));
    }
    passport_issue::issue(&args.signer, &RELAY_HOME, limits, substrate_opt, dir)
}

#[cfg(test)]
mod tests;

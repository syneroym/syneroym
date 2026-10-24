//! Mailbox operator subcommands: passports that raise queue limits.

use std::path::Path;

use anyhow::ensure;
use clap::{Args, Subcommand};
use serde_json::{Map, json};
use syneroym_mailbox_format::DEDICATED_MAX_TTL_SECS;
use syneroym_passport::{MAILBOX_USE, PassportId, facts, wire::LeaseInfo};

use super::passport_issue::{self, IssueSigner, SECS_PER_DAY};

#[derive(Subcommand, Debug, Clone)]
pub enum MailboxCommands {
    /// Operator passports for a mailbox you run
    Passport {
        #[command(subcommand)]
        command: MailboxPassportCommands,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum MailboxPassportCommands {
    /// Issue a passport that raises the queue limits of one owner's
    /// services on this mailbox. Offline: it only signs. Give the file to
    /// the owner.
    ///
    /// Each limit is a floor-raiser: a number below the mailbox's own limit
    /// is ignored, and an omitted limit means the mailbox's own limit. The
    /// owner can have as many services as `--max-services` allows; omitted
    /// means no limit.
    ///
    /// The passport id comes from the signed content. Two runs with the same
    /// options in the same second make the same passport, with one id. To
    /// give out two passports, make them differ (for example by expiry).
    ///
    /// A host with a controller accepts only passports signed by the
    /// controller. A host with no controller accepts only its node key.
    Issue(MailboxIssueArgs),
    /// Deny a passport on the running mailbox. The mailbox ends its leases
    /// at once and refuses the passport from now on. Queues stay, with the
    /// mailbox's own limits. Run as the substrate's owner (`--as <name>`).
    /// A host with no controller and no `[iam].admin_ucan_root` has no
    /// owner, so this is refused there. There is no undo: issue a new
    /// passport instead.
    Deny {
        /// The passport id `issue` printed (64 hex characters).
        passport_id: String,
    },
    /// List the running leases of the mailbox. Run as the substrate's owner
    /// (`--as <name>`). Refused on a host with no controller and no
    /// `[iam].admin_ucan_root`.
    Leases,
}

/// The flags of `mailbox passport issue`. The host node DID comes from the
/// global `--substrate` (or `<dir>/substrate.key`).
#[derive(Args, Debug, Clone)]
pub struct MailboxIssueArgs {
    #[command(flatten)]
    pub signer: IssueSigner,
    /// Most distinct member masters that may hold a queue at one time.
    /// Omitted: no limit.
    #[arg(long)]
    pub max_services: Option<u64>,
    /// Bytes per queue. Omitted, or below the mailbox's own limit: the
    /// mailbox's own limit.
    #[arg(long)]
    pub max_bytes: Option<u64>,
    /// Messages per queue. Same rule.
    #[arg(long)]
    pub max_messages: Option<u64>,
    /// Longest message time-to-live, in days, 1 to 30. Same rule. Senders
    /// currently ask for at most 7 days, so a value above 7 has no effect
    /// until senders ask for more.
    #[arg(long)]
    pub max_ttl_days: Option<u64>,
}

pub async fn handle(
    command: &MailboxCommands,
    api_url: &str,
    substrate_opt: Option<String>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> anyhow::Result<()> {
    let MailboxCommands::Passport { command } = command;
    match command {
        MailboxPassportCommands::Issue(args) => handle_issue(args, substrate_opt, dir).map(|_| ()),
        MailboxPassportCommands::Deny { passport_id } => {
            let id: PassportId = passport_id.parse()?;
            let client =
                super::online_client(api_url, substrate_opt, dir, run_as, ucan_path).await?;
            let ended = client.mailbox_passport_deny(&id).await?;
            println!("passport {passport_id} denied; {ended} running lease(s) ended");
            Ok(())
        }
        MailboxPassportCommands::Leases => {
            let client =
                super::online_client(api_url, substrate_opt, dir, run_as, ucan_path).await?;
            let list: Vec<LeaseInfo> = client.mailbox_passport_leases().await?;
            super::print_json_result(&serde_json::to_value(list)?)
        }
    }
}

/// Signs a mailbox passport with the facts the flags name.
fn handle_issue(
    args: &MailboxIssueArgs,
    substrate_opt: Option<String>,
    dir: &Path,
) -> anyhow::Result<PassportId> {
    passport_issue::issue(&args.signer, &MAILBOX_USE, limits_of(args)?, substrate_opt, dir)
}

fn limits_of(args: &MailboxIssueArgs) -> anyhow::Result<Map<String, serde_json::Value>> {
    let mut limits = Map::new();
    for (key, value) in [
        (facts::MAX_SERVICES, args.max_services),
        (facts::MAX_BYTES, args.max_bytes),
        (facts::MAX_MESSAGES, args.max_messages),
    ] {
        if let Some(n) = value {
            limits.insert(key.to_string(), json!(n));
        }
    }
    if let Some(days) = args.max_ttl_days {
        let max_days = DEDICATED_MAX_TTL_SECS / SECS_PER_DAY;
        ensure!((1..=max_days).contains(&days), "--max-ttl-days must be from 1 to {max_days}");
        limits.insert(facts::MAX_TTL_SECS.to_string(), json!(days * SECS_PER_DAY));
    }
    Ok(limits)
}

#[cfg(test)]
mod tests;

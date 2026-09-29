//! Directory subcommands for Roym.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{Value, json};

use super::trust::CredentialCommands;
use crate::{
    DEFAULT_GATEWAY_URL,
    commands::{self, roym::trust, session},
};

#[derive(Subcommand, Debug, Clone)]
pub enum DirectoryCommands {
    /// List the directories this installation has been given.
    Sources {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Add a directory by its Roym Directory service DID.
    Add {
        did: String,
        #[arg(long)]
        label: Option<String>,
        /// Pin the SynOrg's issuer DID now, rather than trusting whatever
        /// this directory's `info` claims.
        #[arg(long)]
        issuer: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Remove a directory.
    Remove {
        did: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Search every added directory, in parallel, and merge the answers.
    /// Prints the verified hits, the refused evidence, and any source
    /// errors as their own blocks -- a CLI that prints only the good news
    /// hides exactly what the Hub is required to show.
    Find {
        #[arg(long)]
        text: Option<String>,
        #[arg(long = "category")]
        categories: Vec<String>,
        /// `lat,lon,radius_m` in decimal degrees and metres; converted to
        /// integer micro-degrees at this boundary, never signed as a
        /// decimal.
        #[arg(long)]
        near: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Publish one of this installation's own listings to a chosen
    /// directory.
    Publish {
        listing_id: String,
        #[arg(long)]
        to: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Read a directory's own public statement about itself.
    Info {
        did: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Create or update this installation's own SynOrg settings.
    Serve {
        #[arg(long)]
        name: String,
        #[arg(long)]
        rules_file: PathBuf,
        #[arg(long = "category")]
        categories: Vec<String>,
        #[arg(long)]
        support: String,
        #[arg(long)]
        dispute: String,
        #[arg(long, default_value_t = 30)]
        retention_days: u64,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// The SynOrg's own roster: add, remove, and list members.
    Member {
        #[command(subcommand)]
        command: MemberCommands,
    },
    /// Issue, list, or revoke membership credentials (SynOrg owner).
    Credential {
        #[command(subcommand)]
        command: CredentialCommands,
    },
    /// Fetch a member's standing from a source and check it on this node.
    Standing {
        #[arg(long)]
        source: String,
        #[arg(long)]
        member: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// The membership checks this node holds, re-evaluated now.
    Memberships {
        #[arg(long)]
        member: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum MemberCommands {
    Add {
        did: String,
        #[arg(long, default_value = "")]
        note: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    Remove {
        did: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    List {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Suspend a member -- membership-wide, or scoped to one listing.
    Suspend {
        #[arg(long)]
        member: String,
        #[arg(long)]
        rule: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        listing: Option<String>,
        #[arg(long)]
        until_secs: Option<u64>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Lift an earlier suspension.
    Lift {
        #[arg(long)]
        decision: String,
        #[arg(long)]
        reason: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// This SynOrg's moderation decision history.
    Decisions {
        #[arg(long)]
        member: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

/// The identity/gateway parameters shared by every subcommand call within
/// one `handle_directory`/`handle_member`/`handle_transaction` invocation
/// -- bundled so a call site names only what actually varies (the URL,
/// method, and params) instead of repeating all three on every line. Plain
/// `Copy` references, so passing it by value never fights the borrow
/// checker across an `.await`.
#[derive(Clone, Copy)]
pub(super) struct RpcCtx<'a> {
    pub(super) run_as: Option<&'a str>,
    pub(super) ucan_path: Option<&'a Path>,
    pub(super) dir: &'a Path,
}

/// Call `method` over the client gateway with `params`, and pretty-print
/// the JSON result -- the shape most `directory`/`member`/`transaction`
/// subcommands follow, `find` (which streams its own summary) and
/// `thread`/`quote` (which build a non-trivial params object or report
/// beyond one pretty-printed blob) aside.
pub(super) async fn call_and_print(
    ctx: RpcCtx<'_>,
    gateway_url: &str,
    host: Option<&str>,
    method: &str,
    params: Value,
) -> Result<()> {
    let v =
        session::rpc_call(gateway_url, host, ctx.run_as, ctx.ucan_path, ctx.dir, method, params)
            .await?;
    commands::print_json_result(&v)
}

pub(super) async fn handle_directory(
    command: &DirectoryCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match command {
        DirectoryCommands::Sources { gateway_url, host } => {
            call_and_print(ctx, gateway_url, host.as_deref(), "directory.sources", json!({}))
                .await?;
        }
        DirectoryCommands::Add { did, label, issuer, gateway_url, host } => {
            let mut params = json!({ "did": did });
            if let Some(l) = label {
                params["label"] = json!(l);
            }
            if let Some(i) = issuer {
                params["issuer_did"] = json!(i);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "directory.add-source", params)
                .await?;
        }
        DirectoryCommands::Remove { did, gateway_url, host } => {
            call_and_print(
                ctx,
                gateway_url,
                host.as_deref(),
                "directory.remove-source",
                json!({ "did": did }),
            )
            .await?;
        }
        DirectoryCommands::Publish { listing_id, to, gateway_url, host } => {
            call_and_print(
                ctx,
                gateway_url,
                host.as_deref(),
                "directory.publish-to-source",
                json!({ "listing_id": listing_id, "source": to }),
            )
            .await?;
        }
        DirectoryCommands::Info { did, gateway_url, host } => {
            call_and_print(
                ctx,
                gateway_url,
                host.as_deref(),
                "directory.probe-info",
                json!({ "did": did }),
            )
            .await?;
        }
        DirectoryCommands::Serve {
            name,
            rules_file,
            categories,
            support,
            dispute,
            retention_days,
            gateway_url,
            host,
        } => {
            let rules = fs::read_to_string(rules_file)
                .with_context(|| format!("reading {}", rules_file.display()))?;
            let params = json!({
                "name": name,
                "rules": rules,
                "area": [],
                "categories": categories,
                "support_contact": support,
                "dispute_path": dispute,
                "retention_secs": retention_days * 24 * 3600,
                "publication_limits": { "window_secs": 24 * 3600, "max_per_window": 20 },
            });
            call_and_print(ctx, gateway_url, host.as_deref(), "directory.set-settings", params)
                .await?;
        }
        DirectoryCommands::Member { .. }
        | DirectoryCommands::Credential { .. }
        | DirectoryCommands::Standing { .. }
        | DirectoryCommands::Memberships { .. } => {
            handle_directory_trust(command, dir, run_as, ucan_path).await?
        }
        DirectoryCommands::Find { text, categories, near, limit, gateway_url, host } => {
            super::find::find(
                text.as_deref(),
                categories,
                near.as_deref(),
                *limit,
                gateway_url,
                host.as_deref(),
                dir,
                run_as,
                ucan_path,
            )
            .await?;
        }
    }
    Ok(())
}

/// The four `DirectoryCommands` variants that are cross-installation
/// trust surfaces, kept apart from the general directory commands in
/// `handle_directory`.
async fn handle_directory_trust(
    command: &DirectoryCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    match command {
        DirectoryCommands::Member { command } => {
            handle_member(command, dir, run_as, ucan_path).await?
        }
        DirectoryCommands::Credential { command } => {
            trust::handle_credential(command, dir, run_as, ucan_path).await?
        }
        DirectoryCommands::Standing { source, member, gateway_url, host } => {
            trust::handle_standing(
                source,
                member,
                gateway_url,
                host.as_deref(),
                dir,
                run_as,
                ucan_path,
            )
            .await?
        }
        DirectoryCommands::Memberships { member, gateway_url, host } => {
            trust::handle_memberships(
                member.as_deref(),
                gateway_url,
                host.as_deref(),
                dir,
                run_as,
                ucan_path,
            )
            .await?
        }
        _ => unreachable!("handle_directory_trust is only called for its own four variants"),
    }
    Ok(())
}

async fn handle_member(
    command: &MemberCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match command {
        MemberCommands::Add { did, note, gateway_url, host } => {
            call_and_print(
                ctx,
                gateway_url,
                host.as_deref(),
                "member.add",
                json!({ "did": did, "note": note }),
            )
            .await?;
        }
        MemberCommands::Remove { did, gateway_url, host } => {
            call_and_print(
                ctx,
                gateway_url,
                host.as_deref(),
                "member.remove",
                json!({ "did": did }),
            )
            .await?;
        }
        MemberCommands::List { gateway_url, host } => {
            call_and_print(ctx, gateway_url, host.as_deref(), "member.list", json!({})).await?;
        }
        MemberCommands::Suspend {
            member,
            rule,
            reason,
            listing,
            until_secs,
            gateway_url,
            host,
        } => {
            let scope = match listing {
                Some(l) => json!({ "kind": "listing", "listing_id": l }),
                None => json!({ "kind": "membership" }),
            };
            let mut params = json!({
                "member_did": member,
                "rule": rule,
                "reason": reason,
                "scope": scope,
            });
            if let Some(u) = until_secs {
                params["until_secs"] = json!(u);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "member.suspend", params).await?;
        }
        MemberCommands::Lift { decision, reason, gateway_url, host } => {
            call_and_print(
                ctx,
                gateway_url,
                host.as_deref(),
                "member.lift",
                json!({ "decision_record_id": decision, "reason": reason }),
            )
            .await?;
        }
        MemberCommands::Decisions { member, gateway_url, host } => {
            let mut params = json!({});
            if let Some(m) = member {
                params["member_did"] = json!(m);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "member.decisions", params).await?;
        }
    }
    Ok(())
}

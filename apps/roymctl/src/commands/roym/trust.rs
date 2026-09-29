//! Cross-installation trust: credentials, revocations, moderation
//! decisions, and the consumer's own standing checks. Handlers only --
//! the CLI surface (`CredentialCommands`, and the `DirectoryCommands`/
//! `MemberCommands` variants that call into these) lives in `directory.rs`.

use std::path::Path;

use anyhow::Result;
use clap::Subcommand;
use serde_json::{Value, json};

use super::directory::{RpcCtx, call_and_print};
use crate::DEFAULT_GATEWAY_URL;

#[derive(Subcommand, Debug, Clone)]
pub enum CredentialCommands {
    /// Issue a signed membership credential to a member.
    Issue {
        #[arg(long)]
        member: String,
        #[arg(long = "category")]
        category: Vec<String>,
        /// A JSON array of `Area` values, e.g.
        /// `[{"kind":"named","label":"..."}]`.
        #[arg(long)]
        area_json: Option<String>,
        #[arg(long, default_value_t = 365)]
        expires_days: u64,
        #[arg(long)]
        note: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// List credentials this SynOrg has issued.
    List {
        #[arg(long)]
        member: Option<String>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Revoke a credential this SynOrg issued.
    Revoke {
        #[arg(long)]
        credential: String,
        #[arg(long)]
        reason: String,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

pub(super) async fn handle_credential(
    command: &CredentialCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match command {
        CredentialCommands::Issue {
            member,
            category,
            area_json,
            expires_days,
            note,
            gateway_url,
            host,
        } => {
            let area: Value = match area_json {
                Some(s) => serde_json::from_str(s)?,
                None => json!([]),
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let mut params = json!({
                "member_did": member,
                "categories": category,
                "area": area,
                "expires_at_secs": now + expires_days * 24 * 3600,
            });
            if let Some(n) = note {
                params["note"] = json!(n);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "credential.issue", params).await?;
        }
        CredentialCommands::List { member, gateway_url, host } => {
            let mut params = json!({});
            if let Some(m) = member {
                params["member_did"] = json!(m);
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "credential.list", params).await?;
        }
        CredentialCommands::Revoke { credential, reason, gateway_url, host } => {
            call_and_print(
                ctx,
                gateway_url,
                host.as_deref(),
                "revocation.issue",
                json!({ "credential_record_id": credential, "reason": reason }),
            )
            .await?;
        }
    }
    Ok(())
}

pub(super) async fn handle_standing(
    source: &str,
    member: &str,
    gateway_url: &str,
    host: Option<&str>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    call_and_print(
        ctx,
        gateway_url,
        host,
        "directory.check-standing",
        json!({ "source": source, "member_did": member }),
    )
    .await
}

pub(super) async fn handle_memberships(
    member: Option<&str>,
    gateway_url: &str,
    host: Option<&str>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    let mut params = json!({});
    if let Some(m) = member {
        params["member_did"] = json!(m);
    }
    call_and_print(ctx, gateway_url, host, "directory.memberships", params).await
}

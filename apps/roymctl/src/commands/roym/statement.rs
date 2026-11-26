//! Event log, statement, and deal defaults subcommands for Roym.

use std::path::Path;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use serde_json::json;
use syneroym_roym_core::lang::LangTag;

use super::directory::{RpcCtx, call_and_print};
use crate::DEFAULT_GATEWAY_URL;

#[derive(Subcommand, Debug, Clone)]
pub enum EventlogCommands {
    /// List entries in the agreement event log.
    List {
        #[arg(long)]
        after_secs: Option<u64>,
        #[arg(long)]
        after_id: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Rebuild the agreement event log from held snapshots and receipts.
    Rebuild {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

#[derive(Args, Debug, Clone)]
pub struct StatementArgs {
    /// Start date in UTC (YYYY-MM-DD).
    #[arg(long)]
    pub from: String,
    /// End date in UTC (YYYY-MM-DD).
    #[arg(long)]
    pub to: String,
    #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
    pub gateway_url: String,
    #[arg(long)]
    pub host: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DefaultsCommands {
    /// Get saved deal terms and defaults.
    Get {
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
    /// Set saved deal terms and defaults.
    Set {
        #[arg(long)]
        cancellation: String,
        #[arg(long)]
        refund: String,
        #[arg(long)]
        dispute: String,
        #[arg(long)]
        lang: String,
        #[arg(long)]
        quote_valid_hours: u64,
        #[arg(long, default_value = DEFAULT_GATEWAY_URL)]
        gateway_url: String,
        #[arg(long)]
        host: Option<String>,
    },
}

pub(super) async fn handle_eventlog(
    cmd: &EventlogCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match cmd {
        EventlogCommands::List { after_secs, after_id, limit, gateway_url, host } => {
            let mut params = json!({});
            if let Some(l) = limit {
                params["limit"] = json!(l);
            }
            match (after_secs, after_id) {
                (Some(secs), Some(id)) => {
                    params["after"] = json!({ "recorded_at_secs": secs, "id": id });
                }
                (None, None) => {}
                _ => anyhow::bail!("both --after-secs and --after-id must be provided together"),
            }
            call_and_print(ctx, gateway_url, host.as_deref(), "eventlog.list", params).await
        }
        EventlogCommands::Rebuild { gateway_url, host } => {
            call_and_print(ctx, gateway_url, host.as_deref(), "eventlog.rebuild", json!({})).await
        }
    }
}

pub(super) async fn handle_statement(
    args: &StatementArgs,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    let from_date = chrono::NaiveDate::parse_from_str(&args.from, "%Y-%m-%d")
        .with_context(|| format!("invalid --from date '{}', expected YYYY-MM-DD", args.from))?;
    let to_date = chrono::NaiveDate::parse_from_str(&args.to, "%Y-%m-%d")
        .with_context(|| format!("invalid --to date '{}', expected YYYY-MM-DD", args.to))?;
    let from_secs = from_date
        .and_hms_opt(0, 0, 0)
        .context("invalid start date midnight")?
        .and_utc()
        .timestamp();
    let to_secs =
        to_date.and_hms_opt(0, 0, 0).context("invalid end date midnight")?.and_utc().timestamp();
    if from_secs < 0 || to_secs < 0 {
        anyhow::bail!("dates before unix epoch are not supported");
    }
    let params = json!({
        "from_secs": from_secs as u64,
        "to_secs": to_secs as u64,
    });
    call_and_print(ctx, &args.gateway_url, args.host.as_deref(), "statement.export", params).await
}

pub(super) async fn handle_defaults(
    cmd: &DefaultsCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let ctx = RpcCtx { run_as, ucan_path, dir };
    match cmd {
        DefaultsCommands::Get { gateway_url, host } => {
            call_and_print(ctx, gateway_url, host.as_deref(), "deal.defaults-get", json!({})).await
        }
        DefaultsCommands::Set {
            cancellation,
            refund,
            dispute,
            lang,
            quote_valid_hours,
            gateway_url,
            host,
        } => {
            let parsed_lang = LangTag::parse(lang)?;
            let params = json!({
                "cancellation_terms": {
                    "lang": parsed_lang.as_str(),
                    "text": cancellation,
                },
                "refund_terms": {
                    "lang": parsed_lang.as_str(),
                    "text": refund,
                },
                "dispute_path": {
                    "lang": parsed_lang.as_str(),
                    "text": dispute,
                },
                "quote_valid_secs": quote_valid_hours * 3_600,
            });
            call_and_print(ctx, gateway_url, host.as_deref(), "deal.defaults-set", params).await
        }
    }
}

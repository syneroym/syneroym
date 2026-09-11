//! Directory subcommands for Roym.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{Value, json};
use tokio::{sync::Semaphore, task::JoinSet};

use super::parse_near;
use crate::DEFAULT_GATEWAY_URL;

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
}

pub(super) async fn handle_directory(
    command: &DirectoryCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    match command {
        DirectoryCommands::Sources { gateway_url, host } => {
            let v = crate::commands::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.sources",
                json!({}),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Add { did, label, gateway_url, host } => {
            let mut params = json!({ "did": did });
            if let Some(l) = label {
                params["label"] = json!(l);
            }
            let v = crate::commands::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.add-source",
                params,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Remove { did, gateway_url, host } => {
            let v = crate::commands::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.remove-source",
                json!({ "did": did }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Publish { listing_id, to, gateway_url, host } => {
            let v = crate::commands::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.publish-to-source",
                json!({ "listing_id": listing_id, "source": to }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Info { did, gateway_url, host } => {
            let v = crate::commands::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.probe-info",
                json!({ "did": did }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
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
            let v = crate::commands::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "directory.set-settings",
                params,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        DirectoryCommands::Member { command } => {
            handle_member(command, dir, run_as, ucan_path).await?
        }
        DirectoryCommands::Find { text, categories, near, limit, gateway_url, host } => {
            find(
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

async fn handle_member(
    command: &MemberCommands,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    match command {
        MemberCommands::Add { did, note, gateway_url, host } => {
            let v = crate::commands::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "member.add",
                json!({ "did": did, "note": note }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        MemberCommands::Remove { did, gateway_url, host } => {
            let v = crate::commands::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "member.remove",
                json!({ "did": did }),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        MemberCommands::List { gateway_url, host } => {
            let v = crate::commands::session::rpc_call(
                gateway_url,
                host.as_deref(),
                run_as,
                ucan_path,
                dir,
                "member.list",
                json!({}),
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
    }
    Ok(())
}

/// What one directory contributed, as this client saw it -- mirrors the
/// Hub's `SourceOutcome`. A `directory.query-source` reply is always a
/// JSON-RPC success whose `result.error` carries any per-source failure;
/// a 503 from this node's own guest-HTTP admission arrives as an `Err`
/// from `rpc_call` instead and maps to `NotStarted`.
enum SourceOutcome {
    Ok { truncated: bool },
    NotStarted,
    Failed { words: String },
}

impl SourceOutcome {
    fn from_reply(reply: anyhow::Result<Value>) -> Self {
        let result = match reply {
            Ok(v) => v,
            Err(e) => {
                // A 503 is this node's own guest-HTTP admission refusing
                // to start the call -- matched on the typed status, not
                // the error's Display text.
                if let Some(http) = e.downcast_ref::<crate::commands::session::RpcHttpError>()
                    && http.status == 503
                {
                    return SourceOutcome::NotStarted;
                }
                return SourceOutcome::Failed { words: format!("could not be reached: {e}") };
            }
        };
        let truncated = result.get("truncated").and_then(Value::as_bool).unwrap_or(false);
        match result.get("error") {
            Some(err) if !err.is_null() => {
                let kind = err.get("kind").and_then(Value::as_str).unwrap_or("unreadable");
                SourceOutcome::Failed { words: source_error_words(kind).to_string() }
            }
            _ => SourceOutcome::Ok { truncated },
        }
    }

    /// The line to print for this source, or `None` when it answered
    /// cleanly with nothing worth saying.
    fn note(&self) -> Option<String> {
        match self {
            SourceOutcome::Ok { truncated: false } => None,
            SourceOutcome::Ok { truncated: true } => {
                Some("this directory had more matches than it would return".to_string())
            }
            SourceOutcome::NotStarted => {
                Some("this installation was busy and did not start the call".to_string())
            }
            SourceOutcome::Failed { words } => Some(words.clone()),
        }
    }
}

/// Words for a merged hit's `credential` (membership) verdict. `unknown`
/// -- its only value until a membership-credential source lands -- reads
/// as "not checked"; other values pass through so a real verdict is not
/// hidden behind a constant.
fn membership_words(credential: &str) -> String {
    match credential {
        "unknown" => "not checked".to_string(),
        other => other.to_string(),
    }
}

/// The same wording the Hub shows for each `SourceError` kind.
fn source_error_words(kind: &str) -> &'static str {
    match kind {
        "not-started" => "this installation was busy and did not start the call",
        "timed-out" => "the directory did not answer in time",
        "not-found" => "no directory answers at that address",
        "refused" => "the directory refused the request",
        _ => "the directory's answer could not be read",
    }
}

#[allow(clippy::too_many_arguments)]
async fn find(
    text: Option<&str>,
    categories: &[String],
    near: Option<&str>,
    limit: u32,
    gateway_url: &str,
    host: Option<&str>,
    dir: &Path,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
) -> Result<()> {
    let mut query = json!({ "categories": categories, "limit": limit });
    if let Some(t) = text {
        query["text"] = json!(t);
    }
    if let Some(n) = near {
        query["area"] = parse_near(n)?;
    }

    let start = crate::commands::session::rpc_call(
        gateway_url,
        host,
        run_as,
        ucan_path,
        dir,
        "directory.start-run",
        json!({}),
    )
    .await?;
    let run_id =
        start.get("run_id").and_then(|v| v.as_str()).context("start-run: no run_id")?.to_string();
    let sources: Vec<String> = start
        .get("sources")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let max_concurrency =
        start.get("max_concurrency").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;

    if sources.is_empty() {
        println!("No directories added. Add one with `roymctl roym directory add <did>`,");
        println!("or reach a provider directly by link -- a directory is optional.");
    }

    // A continuous worker pool capped at `max_concurrency`, the same
    // shape the Hub's fan-out uses -- not a per-chunk barrier that idles
    // the pool while the slowest source in a chunk finishes.
    let query_one = |source: String| {
        let gateway_url = gateway_url.to_string();
        let host = host.map(str::to_string);
        let dir = dir.to_path_buf();
        let run_as = run_as.map(str::to_string);
        let ucan_path = ucan_path.map(|p| p.to_path_buf());
        let query = query.clone();
        let run_id = run_id.clone();
        async move {
            let result = crate::commands::session::rpc_call(
                &gateway_url,
                host.as_deref(),
                run_as.as_deref(),
                ucan_path.as_deref(),
                &dir,
                "directory.query-source",
                json!({ "run_id": run_id, "source": source, "query": query }),
            )
            .await;
            (source, SourceOutcome::from_reply(result))
        }
    };

    let permits = Arc::new(Semaphore::new(max_concurrency.max(1)));
    let mut set = JoinSet::new();
    for source in &sources {
        // The semaphore is never closed, so acquire only ever succeeds;
        // if it somehow did not, running the source unbounded is a safe
        // fallback.
        let permit = permits.clone().acquire_owned().await.ok();
        let fut = query_one(source.clone());
        set.spawn(async move {
            let out = fut.await;
            drop(permit);
            out
        });
    }
    let mut outcomes: Vec<(String, SourceOutcome)> = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(pair) = joined {
            outcomes.push(pair);
        }
    }

    // A source this node refused to start (a 503 from guest-HTTP
    // admission) is retried once, serially, now that the fan-out's
    // permits are free again -- the same single retry the Hub does.
    let retry: Vec<String> = outcomes
        .iter()
        .filter(|(_, o)| matches!(o, SourceOutcome::NotStarted))
        .map(|(s, _)| s.clone())
        .collect();
    for source in retry {
        let (_, again) = query_one(source.clone()).await;
        if let Some(slot) = outcomes.iter_mut().find(|(s, _)| *s == source) {
            slot.1 = again;
        }
    }

    for (source, outcome) in &outcomes {
        if let Some(line) = outcome.note() {
            println!("source {source}: {line}");
        }
    }

    let merged = crate::commands::session::rpc_call(
        gateway_url,
        host,
        run_as,
        ucan_path,
        dir,
        "directory.merge",
        json!({ "run_id": run_id }),
    )
    .await?;

    let empty = vec![];
    let hits = merged.get("hits").and_then(|v| v.as_array()).unwrap_or(&empty);
    println!("{} result(s):", hits.len());
    for hit in hits {
        let listing_id = hit.get("listing_id").and_then(|v| v.as_str()).unwrap_or("?");
        let title = hit.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let issuer = hit.get("issuer").and_then(|v| v.as_str()).unwrap_or("?");
        let age = hit.get("age_secs").and_then(|v| v.as_u64()).unwrap_or(0);
        let revocation = hit.get("revocation_status").and_then(|v| v.as_str()).unwrap_or("unknown");
        // `credential` is the membership verdict `merge` carries. "unknown"
        // is its only value today and renders as "not checked"; real values
        // can be added later without changing the field, and each gets its
        // own word here rather than a hardcoded string swallowing it.
        let membership =
            membership_words(hit.get("credential").and_then(|v| v.as_str()).unwrap_or("unknown"));
        let sources_val = hit.get("sources").cloned().unwrap_or_default();
        println!(
            "- {listing_id} \"{title}\" by {issuer}, age {age}s, revocation: {revocation}, \
             membership: {membership}, sources: {sources_val}"
        );
    }

    let empty_refused = vec![];
    let refused = merged.get("refused").and_then(|v| v.as_array()).unwrap_or(&empty_refused);
    if !refused.is_empty() {
        println!(
            "\n{} refused (never trusted, shown so you know a directory served them):",
            refused.len()
        );
        for r in refused {
            println!("- {}", serde_json::to_string(r)?);
        }
    }
    Ok(())
}

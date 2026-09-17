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
    let v = crate::commands::session::rpc_call(
        gateway_url,
        host,
        ctx.run_as,
        ctx.ucan_path,
        ctx.dir,
        method,
        params,
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
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
        DirectoryCommands::Add { did, label, gateway_url, host } => {
            let mut params = json!({ "did": did });
            if let Some(l) = label {
                params["label"] = json!(l);
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

/// Build the JSON-RPC query object `directory.query-source` and
/// `directory.merge` expect: `--text`/`--category`/`--near` folded into
/// one object, `near` converted to integer micro-degrees at this boundary,
/// never signed as a decimal.
fn build_find_query(
    text: Option<&str>,
    categories: &[String],
    near: Option<&str>,
    limit: u32,
) -> Result<Value> {
    let mut query = json!({ "categories": categories, "limit": limit });
    if let Some(t) = text {
        query["text"] = json!(t);
    }
    if let Some(n) = near {
        query["area"] = parse_near(n)?;
    }
    Ok(query)
}

/// Start a directory search run and return its id, the sources to fan out
/// to, and the server's advertised concurrency cap (at least 1).
async fn start_find_run(
    gateway_url: &str,
    host: Option<&str>,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
    dir: &Path,
) -> Result<(String, Vec<String>, usize)> {
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
    Ok((run_id, sources, max_concurrency))
}

/// Query one source for `run_id`/`query`, mapping a reachable-but-failed
/// answer, or an `Err` (a local 503 admission refusal), onto a
/// `SourceOutcome`. Takes every connection parameter owned so the future
/// satisfies `JoinSet::spawn`'s `'static` bound.
#[allow(clippy::too_many_arguments)]
async fn query_source(
    gateway_url: String,
    host: Option<String>,
    run_as: Option<String>,
    ucan_path: Option<PathBuf>,
    dir: PathBuf,
    run_id: String,
    query: Value,
    source: String,
) -> (String, SourceOutcome) {
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

/// Query every source with at most `max_concurrency` in flight at once (a
/// continuous worker pool, not a per-chunk barrier that idles while the
/// slowest source in a chunk finishes), then retry once, serially, any
/// source this node refused to start (a local 503 admission refusal) --
/// the same single retry the Hub does.
#[allow(clippy::too_many_arguments)]
async fn run_source_queries(
    sources: &[String],
    gateway_url: &str,
    host: Option<&str>,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
    dir: &Path,
    run_id: &str,
    query: &Value,
    max_concurrency: usize,
) -> Vec<(String, SourceOutcome)> {
    let permits = Arc::new(Semaphore::new(max_concurrency.max(1)));
    let mut set = JoinSet::new();
    for source in sources {
        // The semaphore is never closed, so acquire only ever succeeds;
        // if it somehow did not, running the source unbounded is a safe
        // fallback.
        let permit = permits.clone().acquire_owned().await.ok();
        let fut = query_source(
            gateway_url.to_string(),
            host.map(str::to_string),
            run_as.map(str::to_string),
            ucan_path.map(Path::to_path_buf),
            dir.to_path_buf(),
            run_id.to_string(),
            query.clone(),
            source.clone(),
        );
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
        let again = query_source(
            gateway_url.to_string(),
            host.map(str::to_string),
            run_as.map(str::to_string),
            ucan_path.map(Path::to_path_buf),
            dir.to_path_buf(),
            run_id.to_string(),
            query.clone(),
            source.clone(),
        )
        .await;
        if let Some(slot) = outcomes.iter_mut().find(|(s, _)| *s == source) {
            slot.1 = again.1;
        }
    }
    outcomes
}

/// Print each source's one-line note (if any), merge the run's results,
/// and print the verified hits and any refused evidence -- the "print only
/// the good news" failure mode this command exists to avoid.
async fn print_find_results(
    outcomes: &[(String, SourceOutcome)],
    gateway_url: &str,
    host: Option<&str>,
    run_as: Option<&str>,
    ucan_path: Option<&Path>,
    dir: &Path,
    run_id: &str,
) -> Result<()> {
    for (source, outcome) in outcomes {
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
    let query = build_find_query(text, categories, near, limit)?;

    let (run_id, sources, max_concurrency) =
        start_find_run(gateway_url, host, run_as, ucan_path, dir).await?;

    if sources.is_empty() {
        println!("No directories added. Add one with `roymctl roym directory add <did>`,");
        println!("or reach a provider directly by link -- a directory is optional.");
    }

    let outcomes = run_source_queries(
        &sources,
        gateway_url,
        host,
        run_as,
        ucan_path,
        dir,
        &run_id,
        &query,
        max_concurrency,
    )
    .await;

    print_find_results(&outcomes, gateway_url, host, run_as, ucan_path, dir, &run_id).await
}

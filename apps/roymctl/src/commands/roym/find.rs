//! `roymctl roym directory find`: fan out a search across every added
//! directory, in parallel, and print the merged, verified result -- a CLI
//! that prints only the good news hides exactly what the Hub is required
//! to show.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use syneroym_roym_core::membership;
use tokio::{sync::Semaphore, task::JoinSet};

use super::parse_near;
use crate::commands::session::{self, RpcHttpError};

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
                if let Some(http) = e.downcast_ref::<RpcHttpError>()
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

/// Words for one `sources[]` entry's `membership` value -- this node's
/// own verdict, computed from the signed evidence that source served,
/// never the source's own claim. `None` (a refused hit never reaches a
/// membership check) reads as "not checked".
fn membership_words(membership: Option<&Value>) -> String {
    let Some(v) = membership else { return "not checked".to_string() };
    let Ok(verdict) = serde_json::from_value::<membership::MembershipVerdict>(v.clone()) else {
        return "unreadable".to_string();
    };
    let word = membership::verdict_word(&verdict);
    match &verdict {
        membership::MembershipVerdict::Suspended { rule, reason, .. } => {
            format!("{word} (rule: {rule}, reason: {reason})")
        }
        membership::MembershipVerdict::Revoked { reason, .. } => format!("{word} ({reason})"),
        membership::MembershipVerdict::OutOfScope { outside, .. } => {
            format!("{word} ({})", outside.join(", "))
        }
        membership::MembershipVerdict::Unknown { reason }
        | membership::MembershipVerdict::Refused { reason } => format!("{word} ({reason})"),
        _ => word.to_string(),
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
    let start = session::rpc_call(
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
    let result = session::rpc_call(
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

    let merged = session::rpc_call(
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
        println!("- {listing_id} \"{title}\" by {issuer}, age {age}s, revocation: {revocation}");
        // Each source's own signed evidence produces its own verdict, so a
        // listing served by two directories can print two different
        // membership lines -- never one hidden behind a single field.
        let empty_sources = vec![];
        for source in hit.get("sources").and_then(|v| v.as_array()).unwrap_or(&empty_sources) {
            let directory = source.get("directory").and_then(|v| v.as_str()).unwrap_or("?");
            let words = membership_words(source.get("membership"));
            println!("  source {directory}: membership {words}");
        }
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
pub(super) async fn find(
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

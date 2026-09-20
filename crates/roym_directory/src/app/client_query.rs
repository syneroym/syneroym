//! Client half: per-source dispatch and fan-out bookkeeping.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::proxy::{CallOptions, CallTarget, ProxyError},
};
use syneroym_roym_core::{
    clock,
    directory::{
        DEFAULT_SOURCE_TIMEOUT_MS, MAX_CLIENT_CONCURRENCY, MAX_REFUSED_RESULTS,
        MAX_STORED_PER_SOURCE, RUN_RETENTION_SECS, SearchHit, SearchQuery, SourceError,
    },
    envelope::{Request, Response},
    listing::{self, ListingVerdict},
    services,
};

use super::{
    RUNS, SEARCH_RUNS, SOURCES, client_sources::SourceRow, collect_raw, ensure_coll, get_json,
    put_json, search_runs_indexes, serde_str,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunRow {
    at_secs: u64,
    sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::app) struct SearchRunRow {
    /// The run this row belongs to, its own top-level field so `merge`
    /// and `run-envelope` filter at the host instead of scanning every
    /// run ever stored. The record key still carries it too, for a
    /// stable per-row id.
    pub(in crate::app) run_id: String,
    pub(in crate::app) listing_id: String,
    pub(in crate::app) record_id: String,
    pub(in crate::app) source: String,
    pub(in crate::app) issuer: String,
    pub(in crate::app) title: String,
    pub(in crate::app) summary: String,
    pub(in crate::app) categories: Vec<String>,
    pub(in crate::app) conversation_address: String,
    pub(in crate::app) status: String,
    pub(in crate::app) verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::app) reason: Option<String>,
    pub(in crate::app) revocation_status: String,
    pub(in crate::app) credential: String,
    pub(in crate::app) issued_at_secs: u64,
    pub(in crate::app) received_at_secs: u64,
    pub(in crate::app) at_secs: u64,
    /// The provider's signed bytes, kept on a verified row so a later
    /// `run-envelope` call can hand them back. `merge` builds projections
    /// and never reads this field, so an envelope never travels through a
    /// merge result.
    pub(in crate::app) envelope: String,
    pub(in crate::app) refused: bool,
}

pub(in crate::app) async fn start_run<H: AppHost>(host: &H) -> Response {
    if let Err(e) = ensure_coll(host, RUNS, &[]).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &search_runs_indexes()).await {
        return Response::internal_error(e);
    }
    let now = clock::now_secs();
    let run_floor = now.saturating_sub(RUN_RETENTION_SECS);
    if let Err(e) = AppDataLayer::delete_many(
        host,
        RUNS.to_string(),
        json!({ "at_secs": { "$lte": run_floor } }).to_string(),
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }
    // The per-hit rows carry a full signed envelope each. Prune them on
    // the same schedule as the run rows above -- nothing else ever
    // deletes from this collection, and a person who searches every day
    // would otherwise grow it without bound.
    if let Err(e) = AppDataLayer::delete_many(
        host,
        SEARCH_RUNS.to_string(),
        json!({ "at_secs": { "$lte": run_floor } }).to_string(),
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }
    let source_rows = match collect_raw(host, SOURCES).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let mut source_dids: Vec<String> = source_rows.into_iter().map(|(id, _)| id).collect();
    source_dids.sort();
    let existing_runs = match collect_raw(host, RUNS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let run_id = format!("run_{now}_{}", existing_runs.len());
    let run = RunRow { at_secs: now, sources: source_dids.clone() };
    if let Err(e) = put_json(host, RUNS, &run_id, &run).await {
        return Response::internal_error(e);
    }
    Response::ok(
        json!({ "run_id": run_id, "sources": source_dids, "max_concurrency": MAX_CLIENT_CONCURRENCY }),
    )
}

pub(in crate::app) async fn query_source<H: AppHost>(host: &H, req: &Request) -> Response {
    let (run_id, source, query) = match parse_query_source_params(req) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    if let Err(resp) = validate_run_and_source(host, &run_id, &source).await {
        return resp;
    }

    let params = json!({ "method": "directory.search", "params": query }).to_string();
    let call_result = host
        .call(
            CallTarget::Service(source.clone()),
            services::DIRECTORY.interface.to_string(),
            "invoke".to_string(),
            json!([params]).to_string(),
            Some(CallOptions {
                protocol: None,
                idempotent: true,
                timeout_ms: Some(DEFAULT_SOURCE_TIMEOUT_MS),
                routing_key: None,
                idempotency_key: None,
            }),
        )
        .await;

    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &search_runs_indexes()).await {
        return Response::internal_error(e);
    }

    let (hits, source_truncated) = match parse_source_response(host, &source, call_result).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let now = clock::now_secs();
    let mut verified_count = 0u32;
    let mut refused_count = 0u32;
    for hit in hits {
        let verdict = listing::verify_envelope(&hit.envelope, now);
        if verdict.verified {
            if verified_count >= MAX_STORED_PER_SOURCE {
                continue;
            }
            if store_verified_hit(host, &run_id, &source, hit, verdict, now).await {
                verified_count += 1;
            }
        } else {
            if refused_count >= MAX_REFUSED_RESULTS {
                continue;
            }
            if store_refused_hit(host, &run_id, &source, hit, verdict, now).await {
                refused_count += 1;
            }
        }
    }

    mark_source_ok(host, &source, now).await;

    Response::ok(json!({
        "source": source,
        "verified": verified_count,
        "refused": refused_count,
        "truncated": source_truncated,
        "error": Value::Null,
    }))
}

fn parse_query_source_params(req: &Request) -> Result<(String, String, SearchQuery), Response> {
    let run_id = match req.params.get("run_id").and_then(Value::as_str) {
        Some(r) => r.to_string(),
        None => return Err(Response::invalid_params("run_id is required")),
    };
    let source = match req.params.get("source").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return Err(Response::invalid_params("source is required")),
    };
    let query: SearchQuery = match req.params.get("query").cloned() {
        Some(v) => match serde_json::from_value(v) {
            Ok(q) => q,
            Err(e) => return Err(Response::invalid_params(format!("invalid query: {e}"))),
        },
        None => SearchQuery::default(),
    };
    Ok((run_id, source, query))
}

/// Checks that `run_id` names a run this node minted, that `source` was
/// one of the sources that run listed, and that `source` is still in
/// this person's own registered sources (it could have been removed
/// since the run started).
async fn validate_run_and_source<H: AppHost>(
    host: &H,
    run_id: &str,
    source: &str,
) -> Result<(), Response> {
    let run: Option<RunRow> = match get_json(host, RUNS, run_id).await {
        Ok(r) => r,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let Some(run) = run else {
        return Err(Response::invalid_params("run_id does not name a run this node minted"));
    };
    if !run.sources.iter().any(|s| s.as_str() == source) {
        return Err(Response::invalid_params("source is not in this person's own sources"));
    }
    let is_registered_source: Option<SourceRow> = match get_json(host, SOURCES, source).await {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e)),
    };
    if is_registered_source.is_none() {
        return Err(Response::invalid_params("source is not in this person's own sources"));
    }
    Ok(())
}

/// Turns the raw host-proxied reply into the verified `hits` this node
/// will check, plus the source's own truncation flag -- or, on any
/// failure to reach or parse the source, a fully-formed zero-hit
/// `Response` via `refuse_source`.
async fn parse_source_response<H: AppHost>(
    host: &H,
    source: &str,
    call_result: Result<String, ProxyError>,
) -> Result<(Vec<SearchHit>, bool), Response> {
    let raw = match call_result {
        Ok(r) => r,
        Err(e) => return Err(refuse_source(host, source, map_proxy_error(&e)).await),
    };
    let resp: Response = match serde_json::from_str(&raw) {
        Ok(r) => r,
        Err(e) => {
            return Err(refuse_source(
                host,
                source,
                SourceError::Unreadable { reason: e.to_string() },
            )
            .await);
        }
    };
    let Some(result) = resp.result else {
        let source_error = SourceError::Refused {
            code: resp.error.as_ref().map(|e| e.code).unwrap_or(-32603),
            message: resp.error.map(|e| e.message).unwrap_or_default(),
        };
        return Err(refuse_source(host, source, source_error).await);
    };
    // The directory's own statement that it had more matches for this
    // query than it would return -- distinct from `merge`'s own page cap.
    // A source that returns zero rows can still set this, so it rides the
    // per-source reply rather than the run rows (a zero-row source writes
    // none).
    let source_truncated = result.get("truncated").and_then(Value::as_bool).unwrap_or(false);
    let mut hits: Vec<SearchHit> =
        match serde_json::from_value(result.get("hits").cloned().unwrap_or(json!([]))) {
            Ok(h) => h,
            Err(e) => {
                return Err(refuse_source(
                    host,
                    source,
                    SourceError::Unreadable { reason: e.to_string() },
                )
                .await);
            }
        };
    // Bound *before* verifying: a source answering with far more than it
    // could ever have stored must not get every one of them materialized
    // and signature-checked in guest memory. This dispatch has a hard
    // wall-clock budget, and verification cost is what a hostile source
    // controls directly by returning more hits.
    hits.truncate((MAX_STORED_PER_SOURCE + MAX_REFUSED_RESULTS) as usize);
    Ok((hits, source_truncated))
}

/// Records `error` against `source` and builds the zero-hit response
/// shape every early-refusal path in `query_source` shares (an unreached
/// source, an unparsable reply, a JSON-RPC error, or unreadable `hits`).
async fn refuse_source<H: AppHost>(host: &H, source: &str, error: SourceError) -> Response {
    record_source_error(host, source, &error).await;
    Response::ok(
        json!({ "source": source, "verified": 0, "refused": 0, "truncated": false, "error": error }),
    )
}

/// Stores one already-verified hit as a `search_runs` row and reports
/// whether it was written. The caller must have already checked
/// `verdict.verified` and the per-source verified cap; this only builds
/// and writes the row.
async fn store_verified_hit<H: AppHost>(
    host: &H,
    run_id: &str,
    source: &str,
    hit: SearchHit,
    verdict: ListingVerdict,
    now: u64,
) -> bool {
    let Some(payload) = verdict.payload else { return false };
    let row = SearchRunRow {
        run_id: run_id.to_string(),
        listing_id: verdict.listing_id.unwrap_or_default(),
        record_id: verdict.record_id.unwrap_or_default(),
        source: source.to_string(),
        issuer: verdict.issuer.unwrap_or_default(),
        title: payload.title,
        summary: payload.summary,
        categories: payload.categories,
        conversation_address: verdict.conversation_address.unwrap_or_default(),
        status: serde_str(&verdict.status.unwrap_or(listing::ListingStatus::Active)),
        verified: true,
        reason: None,
        revocation_status: verdict.revocation_status.unwrap_or_else(|| "unknown".to_string()),
        credential: "unknown".to_string(),
        issued_at_secs: verdict.issued_at_secs.unwrap_or(hit.issued_at_secs),
        received_at_secs: hit.received_at_secs,
        at_secs: now,
        envelope: hit.envelope,
        refused: false,
    };
    // `source` is part of the key: two directories can serve the
    // same signed envelope (same `record_id`), and without the
    // source disambiguating them one `query-source` call's row
    // would silently overwrite the other's -- `merge`'s per-hit
    // `sources[]` would then undercount, showing one directory
    // instead of two.
    let key = format!("{run_id}#{source}#{}", row.record_id);
    put_json(host, SEARCH_RUNS, &key, &row).await.is_ok()
}

/// Stores one refused hit as evidence-only (no verified payload fields,
/// only what the source itself claimed) and reports whether it was
/// written. The caller must have already checked `!verdict.verified` and
/// the per-source refused cap.
async fn store_refused_hit<H: AppHost>(
    host: &H,
    run_id: &str,
    source: &str,
    hit: SearchHit,
    verdict: ListingVerdict,
    now: u64,
) -> bool {
    let row = SearchRunRow {
        run_id: run_id.to_string(),
        listing_id: hit.listing_id.clone(),
        record_id: hit.record_id.clone(),
        source: source.to_string(),
        issuer: String::new(),
        title: String::new(),
        summary: String::new(),
        categories: vec![],
        conversation_address: String::new(),
        status: String::new(),
        verified: false,
        reason: verdict.reason,
        revocation_status: "unknown".to_string(),
        credential: "unknown".to_string(),
        issued_at_secs: hit.issued_at_secs,
        received_at_secs: hit.received_at_secs,
        at_secs: now,
        envelope: hit.envelope,
        refused: true,
    };
    let key = format!("{run_id}#refused#{source}#{}", row.record_id);
    put_json(host, SEARCH_RUNS, &key, &row).await.is_ok()
}

fn map_proxy_error(e: &ProxyError) -> SourceError {
    match e {
        ProxyError::ServiceNotFound(_) | ProxyError::DependencyNotBound(_) => SourceError::NotFound,
        ProxyError::TimedOut => SourceError::TimedOut,
        ProxyError::Callee(c) => {
            SourceError::Refused { code: c.code as i64, message: c.message.clone() }
        }
        other => SourceError::Refused { code: -32603, message: format!("{other:?}") },
    }
}

/// Loads `source`'s `sources` row, defaulting to a fresh one keyed on
/// `source` and stamped `added_at_secs: now` when it has none yet.
/// Shared by the success path (`mark_source_ok`) and the failure path
/// (`record_source_error`), which otherwise differ only in which field
/// they then set.
async fn load_source_row_or_default<H: AppHost>(host: &H, source: &str, now: u64) -> SourceRow {
    get_json(host, SOURCES, source).await.ok().flatten().unwrap_or(SourceRow {
        did: source.to_string(),
        label: String::new(),
        added_at_secs: now,
        last_ok_secs: None,
        last_error: None,
    })
}

async fn mark_source_ok<H: AppHost>(host: &H, source: &str, now: u64) {
    let mut row = load_source_row_or_default(host, source, now).await;
    row.last_ok_secs = Some(now);
    row.last_error = None;
    let _ = put_json(host, SOURCES, source, &row).await;
}

async fn record_source_error<H: AppHost>(host: &H, source: &str, error: &SourceError) {
    let now = clock::now_secs();
    let mut row = load_source_row_or_default(host, source, now).await;
    row.last_error = Some(error.clone());
    let _ = put_json(host, SOURCES, source, &row).await;
}

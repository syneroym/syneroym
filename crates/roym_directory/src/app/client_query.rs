//! Client half: fan-out, merging, and run envelope retrieval.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::proxy::{CallOptions, CallTarget, ProxyError},
};
use syneroym_roym_core::{
    clock,
    directory::{
        DEFAULT_SOURCE_TIMEOUT_MS, MAX_CLIENT_CONCURRENCY, MAX_HITS_PER_SOURCE,
        MAX_REFUSED_RESULTS, MAX_SEARCH_RESULTS, MAX_STORED_PER_SOURCE, RUN_RETENTION_SECS,
        SearchHit, SearchQuery, SourceError,
    },
    envelope::{Request, Response},
    listing, services,
};

use super::{
    RUNS, SEARCH_RUNS, SOURCES, client_sources::SourceRow, collect_raw, collect_raw_where,
    ensure_coll, get_json, put_json, search_ops::serde_str, search_runs_indexes,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunRow {
    at_secs: u64,
    sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchRunRow {
    /// The run this row belongs to, its own top-level field so `merge`
    /// and `run-envelope` filter at the host instead of scanning every
    /// run ever stored. The record key still carries it too, for a
    /// stable per-row id.
    run_id: String,
    listing_id: String,
    record_id: String,
    source: String,
    issuer: String,
    title: String,
    summary: String,
    categories: Vec<String>,
    conversation_address: String,
    status: String,
    verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    revocation_status: String,
    credential: String,
    issued_at_secs: u64,
    received_at_secs: u64,
    at_secs: u64,
    /// The provider's signed bytes, kept on a verified row so a later
    /// `run-envelope` call can hand them back. `merge` builds projections
    /// and never reads this field, so an envelope never travels through a
    /// merge result.
    envelope: String,
    refused: bool,
}

pub(crate) async fn start_run<H: AppHost>(host: &H) -> Response {
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

pub(crate) async fn query_source<H: AppHost>(host: &H, req: &Request) -> Response {
    let run_id = match req.params.get("run_id").and_then(Value::as_str) {
        Some(r) => r.to_string(),
        None => return Response::invalid_params("run_id is required"),
    };
    let source = match req.params.get("source").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return Response::invalid_params("source is required"),
    };
    let query: SearchQuery = match req.params.get("query").cloned() {
        Some(v) => match serde_json::from_value(v) {
            Ok(q) => q,
            Err(e) => return Response::invalid_params(format!("invalid query: {e}")),
        },
        None => SearchQuery::default(),
    };

    let run: Option<RunRow> = match get_json(host, RUNS, &run_id).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    };
    let Some(run) = run else {
        return Response::invalid_params("run_id does not name a run this node minted");
    };
    if !run.sources.contains(&source) {
        return Response::invalid_params("source is not in this person's own sources");
    }
    let is_registered_source: Option<SourceRow> = match get_json(host, SOURCES, &source).await {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e),
    };
    if is_registered_source.is_none() {
        return Response::invalid_params("source is not in this person's own sources");
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

    let raw = match call_result {
        Ok(r) => r,
        Err(e) => {
            let source_error = map_proxy_error(&e);
            record_source_error(host, &source, &source_error).await;
            return Response::ok(
                json!({ "source": source, "verified": 0, "refused": 0, "truncated": false, "error": source_error }),
            );
        }
    };
    let resp: Response = match serde_json::from_str(&raw) {
        Ok(r) => r,
        Err(e) => {
            let source_error = SourceError::Unreadable { reason: e.to_string() };
            record_source_error(host, &source, &source_error).await;
            return Response::ok(
                json!({ "source": source, "verified": 0, "refused": 0, "truncated": false, "error": source_error }),
            );
        }
    };
    let Some(result) = resp.result else {
        let source_error = SourceError::Refused {
            code: resp.error.as_ref().map(|e| e.code).unwrap_or(-32603),
            message: resp.error.map(|e| e.message).unwrap_or_default(),
        };
        record_source_error(host, &source, &source_error).await;
        return Response::ok(
            json!({ "source": source, "verified": 0, "refused": 0, "truncated": false, "error": source_error }),
        );
    };
    // The directory's own statement that it had more matches for this
    // query than it would return -- distinct from `merge`'s own page cap.
    // A source that returns zero rows can still set this, so it rides the
    // per-source reply rather than the run rows (a zero-row source writes
    // none).
    let source_truncated = result.get("truncated").and_then(Value::as_bool).unwrap_or(false);
    let mut hits: Vec<SearchHit> = match serde_json::from_value(
        result.get("hits").cloned().unwrap_or(json!([])),
    ) {
        Ok(h) => h,
        Err(e) => {
            let source_error = SourceError::Unreadable { reason: e.to_string() };
            record_source_error(host, &source, &source_error).await;
            return Response::ok(
                json!({ "source": source, "verified": 0, "refused": 0, "truncated": false, "error": source_error }),
            );
        }
    };
    // Bound *before* verifying: a source answering with far more than it
    // could ever have stored must not get every one of them materialized
    // and signature-checked in guest memory. This dispatch has a hard
    // wall-clock budget, and verification cost is what a hostile source
    // controls directly by returning more hits.
    hits.truncate((MAX_STORED_PER_SOURCE + MAX_REFUSED_RESULTS) as usize);

    let now = clock::now_secs();
    let mut verified_count = 0u32;
    let mut refused_count = 0u32;
    for hit in hits {
        let verdict = listing::verify_envelope(&hit.envelope, now);
        if verdict.verified {
            if verified_count >= MAX_STORED_PER_SOURCE {
                continue;
            }
            let Some(payload) = verdict.payload else { continue };
            let row = SearchRunRow {
                run_id: run_id.clone(),
                listing_id: verdict.listing_id.unwrap_or_default(),
                record_id: verdict.record_id.unwrap_or_default(),
                source: source.clone(),
                issuer: verdict.issuer.unwrap_or_default(),
                title: payload.title,
                summary: payload.summary,
                categories: payload.categories,
                conversation_address: verdict.conversation_address.unwrap_or_default(),
                status: serde_str(&verdict.status.unwrap_or(listing::ListingStatus::Active)),
                verified: true,
                reason: None,
                revocation_status: verdict
                    .revocation_status
                    .unwrap_or_else(|| "unknown".to_string()),
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
            if put_json(host, SEARCH_RUNS, &key, &row).await.is_ok() {
                verified_count += 1;
            }
        } else {
            if refused_count >= MAX_REFUSED_RESULTS {
                continue;
            }
            let row = SearchRunRow {
                run_id: run_id.clone(),
                listing_id: hit.listing_id.clone(),
                record_id: hit.record_id.clone(),
                source: source.clone(),
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
            if put_json(host, SEARCH_RUNS, &key, &row).await.is_ok() {
                refused_count += 1;
            }
        }
    }

    put_json(host, SOURCES, &source, &{
        let mut row: SourceRow =
            get_json(host, SOURCES, &source).await.ok().flatten().unwrap_or(SourceRow {
                did: source.clone(),
                label: String::new(),
                added_at_secs: now,
                last_ok_secs: None,
                last_error: None,
            });
        row.last_ok_secs = Some(now);
        row.last_error = None;
        row
    })
    .await
    .ok();

    Response::ok(json!({
        "source": source,
        "verified": verified_count,
        "refused": refused_count,
        "truncated": source_truncated,
        "error": Value::Null,
    }))
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

pub(crate) async fn record_source_error<H: AppHost>(host: &H, source: &str, error: &SourceError) {
    let now = clock::now_secs();
    let mut row: SourceRow =
        get_json(host, SOURCES, source).await.ok().flatten().unwrap_or(SourceRow {
            did: source.to_string(),
            label: String::new(),
            added_at_secs: now,
            last_ok_secs: None,
            last_error: None,
        });
    row.last_error = Some(error.clone());
    let _ = put_json(host, SOURCES, source, &row).await;
}

pub(crate) async fn merge<H: AppHost>(host: &H, req: &Request) -> Response {
    let run_id = match req.params.get("run_id").and_then(Value::as_str) {
        Some(r) => r.to_string(),
        None => return Response::invalid_params("run_id is required"),
    };
    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &search_runs_indexes()).await {
        return Response::internal_error(e);
    }
    // Rows carry `run_id` as a top-level field (the key is
    // `<run_id>#<source>#<record_id>`, or `<run_id>#refused#<source>#
    // <record_id>`, and no host filter can match a key prefix). Filter on
    // the field, so this reads one run's rows, not every run ever stored.
    let rows = match collect_raw_where(host, SEARCH_RUNS, &json!({ "run_id": run_id })).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let mut verified_rows: Vec<SearchRunRow> = Vec::new();
    let mut refused_rows: Vec<SearchRunRow> = Vec::new();
    for (_id, v) in rows {
        let Ok(row) = serde_json::from_value::<SearchRunRow>(v) else { continue };
        if row.refused {
            refused_rows.push(row);
        } else {
            verified_rows.push(row);
        }
    }

    // Per source, first: dedupe by `listing_id`, keeping the newer row.
    // A source is untrusted by construction and its response is
    // arbitrary JSON, so nothing stops it returning two genuinely-signed
    // versions of one listing in a single answer; an honest directory's
    // own `search()` collapsing to one hit per listing narrows this but
    // does not close it. It also covers the case that reliably occurs
    // even against a well-behaved source: a client calling
    // `query-source` more than once for the same `(run_id, source)`,
    // e.g. a retry, storing a different version each time. Then sort by
    // (issued_at desc, listing_id asc) and take at most
    // `MAX_HITS_PER_SOURCE`.
    let mut by_source: BTreeMap<String, BTreeMap<String, SearchRunRow>> = BTreeMap::new();
    for row in verified_rows {
        let per_listing = by_source.entry(row.source.clone()).or_default();
        match per_listing.get(&row.listing_id) {
            Some(existing) if existing.issued_at_secs >= row.issued_at_secs => {}
            _ => {
                per_listing.insert(row.listing_id.clone(), row);
            }
        }
    }
    let mut by_source: BTreeMap<String, Vec<SearchRunRow>> = by_source
        .into_iter()
        .map(|(source, per_listing)| (source, per_listing.into_values().collect()))
        .collect();
    for rows in by_source.values_mut() {
        rows.sort_by(|a, b| {
            b.issued_at_secs.cmp(&a.issued_at_secs).then(a.listing_id.cmp(&b.listing_id))
        });
        rows.truncate(MAX_HITS_PER_SOURCE as usize);
    }

    // Round-robin across sources, visited in DID order, to decide *which*
    // listings make the merged page and to enforce the per-source share
    // and the total cap. `order` is the merged page order the round-robin
    // produces -- recency within a source, interleaved across sources;
    // `seen` is only the membership check. Iterating `seen` (a
    // `BTreeSet`) instead would re-sort the page by `listing_id`, a
    // content hash a forger picks, discarding the round-robin entirely.
    // The value each selected listing carries is still computed
    // afterward, in one pass over every source's row for it, so which
    // source's turn selected it first cannot change the result.
    let mut positions: BTreeMap<String, usize> =
        by_source.keys().map(|k| (k.clone(), 0usize)).collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut order: Vec<String> = Vec::new();
    let mut merge_truncated = false;
    'outer: loop {
        let mut advanced = false;
        for (source, rows) in &by_source {
            let Some(pos) = positions.get_mut(source) else { continue };
            while *pos < rows.len() && seen.contains(&rows[*pos].listing_id) {
                *pos += 1;
            }
            if *pos >= rows.len() {
                continue;
            }
            let listing_id = rows[*pos].listing_id.clone();
            *pos += 1;
            advanced = true;
            if seen.len() as u32 >= MAX_SEARCH_RESULTS {
                merge_truncated = true;
                break 'outer;
            }
            seen.insert(listing_id.clone());
            order.push(listing_id);
        }
        if !advanced {
            break;
        }
    }

    // Every source's row for each selected listing -- at most one per
    // (source, listing_id) pair, by construction of the per-source
    // dedup/truncate above.
    let mut candidates_by_listing: BTreeMap<String, Vec<&SearchRunRow>> = BTreeMap::new();
    for rows in by_source.values() {
        for row in rows {
            if seen.contains(&row.listing_id) {
                candidates_by_listing.entry(row.listing_id.clone()).or_default().push(row);
            }
        }
    }

    let now = clock::now_secs();
    let hits: Vec<Value> = order
        .into_iter()
        .filter_map(|listing_id| {
            // Keep the row with the greatest `issued_at_secs`, ties by
            // `record_id` ascending; union every source that answered.
            let mut candidates = candidates_by_listing.remove(&listing_id)?;
            candidates.sort_by(|a, b| {
                b.issued_at_secs.cmp(&a.issued_at_secs).then(a.record_id.cmp(&b.record_id))
            });
            let winner = *candidates.first()?;
            let source_list: Vec<Value> = candidates
                .iter()
                .map(|r| {
                    json!({ "directory": r.source, "record_id": r.record_id, "received_at_secs": r.received_at_secs })
                })
                .collect();
            let distinct_record_ids: BTreeSet<&str> =
                candidates.iter().map(|r| r.record_id.as_str()).collect();
            Some(json!({
                "listing_id": winner.listing_id,
                "record_id": winner.record_id,
                "issuer": winner.issuer,
                "title": winner.title,
                "summary": winner.summary,
                "categories": winner.categories,
                "conversation_address": winner.conversation_address,
                "status": winner.status,
                "verified": true,
                "revocation_status": winner.revocation_status,
                "credential": winner.credential,
                "age_secs": now.saturating_sub(winner.issued_at_secs),
                "sources": source_list,
                "versions_differ": distinct_record_ids.len() > 1,
            }))
        })
        .collect();

    // Refused evidence: grouped by source and round-robined across
    // sources, the same way verified hits are (above), then merged by
    // listing_id. A per-source sort key alone -- the previous version
    // used the row's own `listing_id`, which a forger fully controls --
    // buys no resistance: a hostile source whose forged `listing_id`
    // (or whose DID) happens to sort first would otherwise fill every
    // slot before a second, honest source's evidence is ever reached.
    // Round-robin is what actually bounds one source's share.
    let mut refused_by_source: BTreeMap<String, Vec<SearchRunRow>> = BTreeMap::new();
    for row in refused_rows {
        refused_by_source.entry(row.source.clone()).or_default().push(row);
    }
    for rows in refused_by_source.values_mut() {
        rows.sort_by(|a, b| a.listing_id.cmp(&b.listing_id).then(a.record_id.cmp(&b.record_id)));
    }
    let mut refused_positions: BTreeMap<String, usize> =
        refused_by_source.keys().map(|k| (k.clone(), 0usize)).collect();
    let mut refused_seen: BTreeSet<String> = BTreeSet::new();
    let mut refused_order: Vec<String> = Vec::new();
    let mut refused_truncated = false;
    'refused_outer: loop {
        let mut advanced = false;
        for (source, rows) in &refused_by_source {
            let Some(pos) = refused_positions.get_mut(source) else { continue };
            while *pos < rows.len() && refused_seen.contains(&rows[*pos].listing_id) {
                *pos += 1;
            }
            if *pos >= rows.len() {
                continue;
            }
            let listing_id = rows[*pos].listing_id.clone();
            *pos += 1;
            advanced = true;
            if refused_seen.len() as u32 >= MAX_REFUSED_RESULTS {
                refused_truncated = true;
                break 'refused_outer;
            }
            refused_seen.insert(listing_id.clone());
            refused_order.push(listing_id);
        }
        if !advanced {
            break;
        }
    }

    let mut refused_candidates: BTreeMap<String, Vec<&SearchRunRow>> = BTreeMap::new();
    for rows in refused_by_source.values() {
        for row in rows {
            if refused_seen.contains(&row.listing_id) {
                refused_candidates.entry(row.listing_id.clone()).or_default().push(row);
            }
        }
    }
    let refused: Vec<Value> = refused_order
        .into_iter()
        .filter_map(|listing_id| {
            let rows = refused_candidates.remove(&listing_id)?;
            let sources: Vec<Value> = rows.iter().map(|r| json!(r.source)).collect();
            Some(json!({
                "listing_id": listing_id,
                "reason": rows.first().and_then(|r| r.reason.clone()).unwrap_or_default(),
                "sources": sources,
            }))
        })
        .collect();

    Response::ok(json!({
        "hits": hits,
        "hits_truncated": merge_truncated,
        "refused": refused,
        "refused_truncated": refused_truncated,
    }))
}

pub(crate) async fn run_envelope<H: AppHost>(host: &H, req: &Request) -> Response {
    let run_id = match req.params.get("run_id").and_then(Value::as_str) {
        Some(r) => r,
        None => return Response::invalid_params("run_id is required"),
    };
    let record_id = match req.params.get("record_id").and_then(Value::as_str) {
        Some(r) => r,
        None => return Response::invalid_params("record_id is required"),
    };
    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &search_runs_indexes()).await {
        return Response::internal_error(e);
    }
    // Filtered on the `run_id` field at the host -- one run's rows, not
    // the whole collection. Only a *verified* row is accepted: its
    // `record_id` is content-derived from the envelope, so any surviving
    // verified row for `record_id` carries the identical signed bytes. A
    // refused row's `record_id` is whatever the source claimed,
    // unverified, so a hostile source could set one to collide with a
    // genuine record -- excluded here rather than trusted to sort last.
    let rows = match collect_raw_where(host, SEARCH_RUNS, &json!({ "run_id": run_id })).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    for (_id, v) in rows {
        if let Ok(row) = serde_json::from_value::<SearchRunRow>(v)
            && !row.refused
            && row.record_id == record_id
        {
            return Response::ok(json!({ "envelope": row.envelope }));
        }
    }
    Response::ok(Value::Null)
}

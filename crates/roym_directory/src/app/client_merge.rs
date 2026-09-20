//! Client half: merging fanned-out search results into one page.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    clock,
    directory::{MAX_HITS_PER_SOURCE, MAX_REFUSED_RESULTS, MAX_SEARCH_RESULTS},
    envelope::{Request, Response},
};

use super::{
    SEARCH_RUNS, client_query::SearchRunRow, collect_raw_where, ensure_coll, search_runs_indexes,
};

pub(in crate::app) async fn merge<H: AppHost>(host: &H, req: &Request) -> Response {
    let run_id = match req.params.get("run_id").and_then(Value::as_str) {
        Some(r) => r.to_string(),
        None => return Response::invalid_params("run_id is required"),
    };
    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &search_runs_indexes()).await {
        return Response::internal_error(e);
    }
    let (verified_rows, refused_rows) = match load_run_rows(host, &run_id).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };

    let by_source = dedupe_and_cap_verified(verified_rows);
    let (order, seen, merge_truncated) = round_robin_merge(&by_source, MAX_SEARCH_RESULTS);
    let candidates = candidates_by_listing(&by_source, &seen);
    let now = clock::now_secs();
    let hits = build_hits(order, candidates, now);

    let refused_by_source = group_and_sort_refused(refused_rows);
    let (refused_order, refused_seen, refused_truncated) =
        round_robin_merge(&refused_by_source, MAX_REFUSED_RESULTS);
    let refused_candidates = candidates_by_listing(&refused_by_source, &refused_seen);
    let refused = build_refused(refused_order, refused_candidates);

    Response::ok(json!({
        "hits": hits,
        "hits_truncated": merge_truncated,
        "refused": refused,
        "refused_truncated": refused_truncated,
    }))
}

/// Loads every `search_runs` row for `run_id`, split into verified and
/// refused rows. Rows carry `run_id` as a top-level field (the key is
/// `<run_id>#<source>#<record_id>`, or `<run_id>#refused#<source>#
/// <record_id>`, and no host filter can match a key prefix). Filter on
/// the field, so this reads one run's rows, not every run ever stored.
async fn load_run_rows<H: AppHost>(
    host: &H,
    run_id: &str,
) -> Result<(Vec<SearchRunRow>, Vec<SearchRunRow>), String> {
    let rows = collect_raw_where(host, SEARCH_RUNS, &json!({ "run_id": run_id })).await?;
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
    Ok((verified_rows, refused_rows))
}

/// Per source, first: dedupe by `listing_id`, keeping the newer row.
/// A source is untrusted by construction and its response is
/// arbitrary JSON, so nothing stops it returning two genuinely-signed
/// versions of one listing in a single answer; an honest directory's
/// own `search()` collapsing to one hit per listing narrows this but
/// does not close it. It also covers the case that reliably occurs
/// even against a well-behaved source: a client calling
/// `query-source` more than once for the same `(run_id, source)`,
/// e.g. a retry, storing a different version each time. Then sort by
/// (issued_at desc, listing_id asc) and take at most
/// `MAX_HITS_PER_SOURCE`.
fn dedupe_and_cap_verified(
    verified_rows: Vec<SearchRunRow>,
) -> BTreeMap<String, Vec<SearchRunRow>> {
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
    by_source
}

/// Refused evidence: grouped by source, sorted by a key no single
/// source fully controls (`listing_id` then `record_id`). A per-source
/// sort key alone -- an earlier version used the row's own `listing_id`,
/// which a forger fully controls -- buys no resistance: a hostile source
/// whose forged `listing_id` (or whose DID) happens to sort first would
/// otherwise fill every slot before a second, honest source's evidence
/// is ever reached. `round_robin_merge` is what actually bounds one
/// source's share.
fn group_and_sort_refused(refused_rows: Vec<SearchRunRow>) -> BTreeMap<String, Vec<SearchRunRow>> {
    let mut refused_by_source: BTreeMap<String, Vec<SearchRunRow>> = BTreeMap::new();
    for row in refused_rows {
        refused_by_source.entry(row.source.clone()).or_default().push(row);
    }
    for rows in refused_by_source.values_mut() {
        rows.sort_by(|a, b| a.listing_id.cmp(&b.listing_id).then(a.record_id.cmp(&b.record_id)));
    }
    refused_by_source
}

/// Round-robin across sources, visited in DID order, to decide *which*
/// listings make the merged page (or the refused-evidence page) and to
/// enforce the per-source share and the total cap `cap`. The returned
/// order is the merged page order the round-robin produces -- recency
/// within a source, interleaved across sources; the returned set is only
/// the membership check. Iterating that set instead of the order (it is
/// a `BTreeSet`) would re-sort the page by `listing_id`, a content hash a
/// forger picks, discarding the round-robin entirely. The value each
/// selected listing carries is still computed afterward, in one pass
/// over every source's row for it (`candidates_by_listing`), so which
/// source's turn selected it first cannot change the result. Shared by
/// the verified merge and the refused-evidence merge, which round-robin
/// identically over differently-ordered, differently-capped input.
fn round_robin_merge(
    by_source: &BTreeMap<String, Vec<SearchRunRow>>,
    cap: u32,
) -> (Vec<String>, BTreeSet<String>, bool) {
    let mut positions: BTreeMap<String, usize> =
        by_source.keys().map(|k| (k.clone(), 0usize)).collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut order: Vec<String> = Vec::new();
    let mut truncated = false;
    'outer: loop {
        let mut advanced = false;
        for (source, rows) in by_source {
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
            if seen.len() as u32 >= cap {
                truncated = true;
                break 'outer;
            }
            seen.insert(listing_id.clone());
            order.push(listing_id);
        }
        if !advanced {
            break;
        }
    }
    (order, seen, truncated)
}

/// Every source's row for each selected listing -- at most one per
/// (source, listing_id) pair, by construction of the grouping that
/// produced `by_source` (`dedupe_and_cap_verified` or
/// `group_and_sort_refused`).
fn candidates_by_listing<'a>(
    by_source: &'a BTreeMap<String, Vec<SearchRunRow>>,
    seen: &BTreeSet<String>,
) -> BTreeMap<String, Vec<&'a SearchRunRow>> {
    let mut out: BTreeMap<String, Vec<&SearchRunRow>> = BTreeMap::new();
    for rows in by_source.values() {
        for row in rows {
            if seen.contains(&row.listing_id) {
                out.entry(row.listing_id.clone()).or_default().push(row);
            }
        }
    }
    out
}

fn build_hits(
    order: Vec<String>,
    mut candidates_by_listing: BTreeMap<String, Vec<&SearchRunRow>>,
    now: u64,
) -> Vec<Value> {
    order
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
        .collect()
}

fn build_refused(
    refused_order: Vec<String>,
    mut refused_candidates: BTreeMap<String, Vec<&SearchRunRow>>,
) -> Vec<Value> {
    refused_order
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
        .collect()
}

pub(in crate::app) async fn run_envelope<H: AppHost>(host: &H, req: &Request) -> Response {
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

//! Result merging and deduplication across fanned-out sources.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use syneroym_roym_core::{
    directory::{MAX_HITS_PER_SOURCE, MAX_REFUSED_RESULTS, MAX_SEARCH_RESULTS},
    membership::MembershipVerdict,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HitSource {
    pub directory: String,
    pub record_id: String,
    pub received_at_secs: u64,
    pub membership: Option<MembershipVerdict>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergedHit {
    pub listing_id: String,
    pub record_id: String,
    pub issuer: String,
    pub title: String,
    pub summary: String,
    pub categories: Vec<String>,
    pub conversation_address: String,
    pub status: String,
    pub verified: bool,
    pub revocation_status: String,
    pub age_secs: u64,
    pub sources: Vec<HitSource>,
    pub versions_differ: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefusedHit {
    pub listing_id: String,
    pub reason: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub listing_id: String,
    pub record_id: String,
    pub source: String,
    pub issuer: String,
    pub title: String,
    pub summary: String,
    pub categories: Vec<String>,
    pub conversation_address: String,
    pub status: String,
    pub reason: Option<String>,
    pub revocation_status: String,
    pub membership: Option<MembershipVerdict>,
    pub issued_at_secs: u64,
    pub received_at_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergedRows {
    pub hits: Vec<MergedHit>,
    pub hits_truncated: bool,
    pub refused: Vec<RefusedHit>,
    pub refused_truncated: bool,
}

/// Merges verified and refused result rows across sources into a single page.
pub fn merge_rows(verified: Vec<Row>, refused: Vec<Row>, now_secs: u64) -> MergedRows {
    let by_source = dedupe_and_cap_verified(verified);
    let (order, seen, merge_truncated) = round_robin_merge(&by_source, MAX_SEARCH_RESULTS);
    let candidates = candidates_by_listing(&by_source, &seen);
    let hits = build_hits(order, candidates, now_secs);

    let refused_by_source = group_and_sort_refused(refused);
    let (refused_order, refused_seen, refused_truncated) =
        round_robin_merge(&refused_by_source, MAX_REFUSED_RESULTS);
    let refused_candidates = candidates_by_listing(&refused_by_source, &refused_seen);
    let refused = build_refused(refused_order, refused_candidates);

    MergedRows { hits, hits_truncated: merge_truncated, refused, refused_truncated }
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
fn dedupe_and_cap_verified(verified_rows: Vec<Row>) -> BTreeMap<String, Vec<Row>> {
    let mut by_source: BTreeMap<String, BTreeMap<String, Row>> = BTreeMap::new();
    for row in verified_rows {
        let per_listing = by_source.entry(row.source.clone()).or_default();
        match per_listing.get(&row.listing_id) {
            Some(existing) if existing.issued_at_secs >= row.issued_at_secs => {}
            _ => {
                per_listing.insert(row.listing_id.clone(), row);
            }
        }
    }
    let mut by_source: BTreeMap<String, Vec<Row>> = by_source
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
fn group_and_sort_refused(refused_rows: Vec<Row>) -> BTreeMap<String, Vec<Row>> {
    let mut refused_by_source: BTreeMap<String, Vec<Row>> = BTreeMap::new();
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
    by_source: &BTreeMap<String, Vec<Row>>,
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
    by_source: &'a BTreeMap<String, Vec<Row>>,
    seen: &BTreeSet<String>,
) -> BTreeMap<String, Vec<&'a Row>> {
    let mut out: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
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
    mut candidates_by_listing: BTreeMap<String, Vec<&Row>>,
    now: u64,
) -> Vec<MergedHit> {
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
            let sources: Vec<HitSource> = candidates
                .iter()
                .map(|r| HitSource {
                    directory: r.source.clone(),
                    record_id: r.record_id.clone(),
                    received_at_secs: r.received_at_secs,
                    membership: r.membership.clone(),
                })
                .collect();
            let distinct_record_ids: BTreeSet<&str> =
                candidates.iter().map(|r| r.record_id.as_str()).collect();
            Some(MergedHit {
                listing_id: winner.listing_id.clone(),
                record_id: winner.record_id.clone(),
                issuer: winner.issuer.clone(),
                title: winner.title.clone(),
                summary: winner.summary.clone(),
                categories: winner.categories.clone(),
                conversation_address: winner.conversation_address.clone(),
                status: winner.status.clone(),
                verified: true,
                revocation_status: winner.revocation_status.clone(),
                age_secs: now.saturating_sub(winner.issued_at_secs),
                sources,
                versions_differ: distinct_record_ids.len() > 1,
            })
        })
        .collect()
}

fn build_refused(
    refused_order: Vec<String>,
    mut refused_candidates: BTreeMap<String, Vec<&Row>>,
) -> Vec<RefusedHit> {
    refused_order
        .into_iter()
        .filter_map(|listing_id| {
            let rows = refused_candidates.remove(&listing_id)?;
            let sources: Vec<String> = rows.iter().map(|r| r.source.clone()).collect();
            Some(RefusedHit {
                listing_id,
                reason: rows.first().and_then(|r| r.reason.clone()).unwrap_or_default(),
                sources,
            })
        })
        .collect()
}

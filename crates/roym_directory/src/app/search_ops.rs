//! Server half: search index and directory search.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{AppDataLayer, AppHost, AppSigning, types::data_layer::QueryOptions};
use syneroym_roym_core::{
    area::{self, Area},
    clock,
    directory::{
        AreaMatch, MAX_CATEGORIES, MAX_HITS_PER_QUERY, MAX_QUERY_TEXT_LEN, SearchHit, SearchQuery,
        category_tokens, normalize_category, normalize_text,
    },
    envelope::{Request, Response},
    listing,
};

use super::{
    PUBLICATIONS, SEARCH_INDEX, collect_raw, ensure_coll, get_json,
    publication_ops::{self, PublicationRow},
    put_json, serde_str, synorg,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::app) struct SearchIndexRow {
    pub(in crate::app) listing_id: String,
    record_id: String,
    pub(in crate::app) area_index: u32,
    issuer: String,
    status: String,
    issued_at_secs: u64,
    received_at_secs: u64,
    categories: String,
    text: String,
    /// Case-folded, trimmed label of this row's named service area, if it
    /// has one. Lets a named-area query push a label equality down into
    /// the host filter, the way a geometric query pushes its bounding
    /// box -- without it the sieve keeps the alphabetically-first rows and
    /// a late-hashing label can be truncated away to zero hits.
    #[serde(skip_serializing_if = "Option::is_none")]
    area_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    booking_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_lat_e6: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_lat_e6: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_lon_e6: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_lon_e6: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    area: Option<Area>,
}

pub(in crate::app) fn search_index_key(listing_id: &str, area_index: u32) -> String {
    format!("{listing_id}#{area_index}")
}

pub(in crate::app) fn build_index_rows(
    payload: &listing::ListingPayload,
    record_id: &str,
    issuer: &str,
    issued_at_secs: u64,
    received_at_secs: u64,
) -> Vec<SearchIndexRow> {
    let status = match payload.status {
        listing::ListingStatus::Active => "active",
        listing::ListingStatus::Withdrawn => "withdrawn",
        listing::ListingStatus::Draft => "draft",
    }
    .to_string();
    let categories = category_tokens(&payload.categories);
    let text = normalize_text(&format!(
        "{} {} {}",
        payload.title,
        payload.summary,
        payload.categories.join(" ")
    ));
    let open_to = payload.relationship.as_ref().map(|r| serde_str(&r.open_to));
    let booking_mode = payload.booking.as_ref().map(|b| serde_str(&b.mode));

    let areas: Vec<Area> =
        payload.location.as_ref().map(|l| l.service_area.clone()).unwrap_or_default();
    if areas.is_empty() {
        return vec![SearchIndexRow {
            listing_id: payload.listing_id.clone(),
            record_id: record_id.to_string(),
            area_index: 0,
            issuer: issuer.to_string(),
            status,
            issued_at_secs,
            received_at_secs,
            categories,
            text,
            area_label: None,
            open_to,
            booking_mode,
            min_lat_e6: None,
            max_lat_e6: None,
            min_lon_e6: None,
            max_lon_e6: None,
            area: None,
        }];
    }
    areas
        .into_iter()
        .enumerate()
        .map(|(i, a)| {
            let bbox = area::bounding_box(&a);
            let area_label = match &a {
                Area::Named { label, .. } => Some(area::normalize_label(label)),
                _ => None,
            };
            SearchIndexRow {
                listing_id: payload.listing_id.clone(),
                record_id: record_id.to_string(),
                area_index: i as u32,
                issuer: issuer.to_string(),
                status: status.clone(),
                issued_at_secs,
                received_at_secs,
                categories: categories.clone(),
                text: text.clone(),
                area_label,
                open_to: open_to.clone(),
                booking_mode: booking_mode.clone(),
                min_lat_e6: bbox.map(|b| b.min_lat_e6),
                max_lat_e6: bbox.map(|b| b.max_lat_e6),
                min_lon_e6: bbox.map(|b| b.min_lon_e6),
                max_lon_e6: bbox.map(|b| b.max_lon_e6),
                area: Some(a),
            }
        })
        .collect()
}

fn area_match_precedence(m: &AreaMatch) -> u8 {
    match m {
        AreaMatch::Geometric { .. } => 0,
        AreaMatch::Named { .. } => 1,
        AreaMatch::NoAreaStated => 2,
        AreaMatch::NotQueried => 3,
    }
}

pub(in crate::app) async fn search<H: AppHost>(host: &H, req: &Request) -> Response {
    let query: SearchQuery = match serde_json::from_value(req.params.clone()) {
        Ok(q) => q,
        Err(e) => return Response::invalid_params(format!("invalid query: {e}")),
    };
    // This is the one query shape an anonymous stranger controls end to
    // end, so every field gets checked before it reaches arithmetic or a
    // filter document: an unvalidated `Area` can drive `bounding_box`/
    // `areas_intersect` into overflow, an unbounded category list turns
    // into an unbounded `$and`, and unbounded text turns into an unbounded
    // bound-parameter list.
    if let Some(q_area) = &query.area
        && let Err(e) = q_area.validate()
    {
        return Response::invalid_params(e.to_string());
    }
    if query.categories.len() > MAX_CATEGORIES {
        return Response::invalid_params(format!(
            "more than {MAX_CATEGORIES} categories in a query"
        ));
    }
    if let Some(text) = &query.text
        && text.len() > MAX_QUERY_TEXT_LEN
    {
        return Response::invalid_params(format!(
            "query text is longer than {MAX_QUERY_TEXT_LEN} bytes"
        ));
    }
    // The text index is ASCII-folded (see `normalize_text`). A query in a
    // script that folds to nothing -- Kannada, Devanagari, CJK -- must be
    // refused, not quietly dropped: dropping the clause returns every
    // active listing, the opposite of what the person asked for. Widening
    // the index alphabet is a projection change, tracked in the backlog.
    let normalized_text = query.text.as_deref().map(normalize_text);
    if let (Some(raw), Some(norm)) = (query.text.as_deref(), normalized_text.as_deref())
        && !raw.trim().is_empty()
        && norm.is_empty()
    {
        return Response::err(
            -32602,
            "the directory's text index holds only ASCII letters, digits, spaces and hyphens; \
             this query has no searchable characters"
                .to_string(),
        );
    }
    if let Err(e) = ensure_coll(host, SEARCH_INDEX, &[]).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_coll(host, PUBLICATIONS, &[]).await {
        return Response::internal_error(e);
    }
    // Rate-gated by the stored marker (see `prune_expired_publications`),
    // so an anonymous stranger looping this verb pays one indexed marker
    // read, not a pair of delete scans, on all but one call per five
    // minutes -- and a directory nobody probes with `info` still ages out.
    if let Ok(Some(settings)) = synorg::load_settings(host).await {
        let _ = publication_ops::prune_expired_publications(host, settings.retention_secs).await;
    }

    let mut and_clauses: Vec<Value> = vec![json!({ "status": "active" })];
    for cat in &query.categories {
        let normalized = normalize_category(cat);
        and_clauses.push(json!({ "categories": { "$regex": category_tokens(&[normalized]) } }));
    }
    if let Some(normalized) = normalized_text.as_deref().filter(|n| !n.is_empty()) {
        and_clauses.push(json!({ "text": { "$regex": normalized } }));
    }
    // The index stores the enum's serde spelling (`existing-customers`,
    // not `ExistingCustomers`). Round-trip the query value through the
    // same enum so both sides speak one vocabulary; a value that is not a
    // spelling the enum declares is refused, not silently matched to
    // nothing.
    if let Some(open_to) = &query.open_to {
        match serde_json::from_value::<listing::OpenTo>(json!(open_to)) {
            Ok(v) => and_clauses.push(json!({ "open_to": serde_str(&v) })),
            Err(_) => {
                return Response::err(
                    -32602,
                    "open_to must be one of: anyone, members, referral, existing-customers"
                        .to_string(),
                );
            }
        }
    }
    if let Some(booking_mode) = &query.booking_mode {
        match serde_json::from_value::<listing::BookingMode>(json!(booking_mode)) {
            Ok(v) => and_clauses.push(json!({ "booking_mode": serde_str(&v) })),
            Err(_) => {
                return Response::err(
                    -32602,
                    "booking_mode must be one of: slots, order, enquiry".to_string(),
                );
            }
        }
    }
    let geometric_query = matches!(query.area, Some(Area::Bbox { .. }) | Some(Area::Circle { .. }));
    if let Some(q_area) = &query.area
        && geometric_query
        && let Some(bbox) = area::bounding_box(q_area)
    {
        and_clauses.push(json!({ "min_lat_e6": { "$lte": bbox.max_lat_e6 } }));
        and_clauses.push(json!({ "max_lat_e6": { "$gte": bbox.min_lat_e6 } }));
        and_clauses.push(json!({ "min_lon_e6": { "$lte": bbox.max_lon_e6 } }));
        and_clauses.push(json!({ "max_lon_e6": { "$gte": bbox.min_lon_e6 } }));
    }
    // A named-area query narrows the sieve by label, the way a geometric
    // one narrows it by bounding box -- so a directory with more matching
    // listings than the candidate ceiling cannot truncate the wanted
    // label away in hash order.
    if let Some(Area::Named { label, .. }) = &query.area {
        and_clauses.push(json!({ "area_label": area::normalize_label(label) }));
    }
    let filter = json!({ "$and": and_clauses });

    // The ceiling counts *distinct listings*, not index rows: one listing
    // holds up to `MAX_AREAS` rows, so a row count would let a page of
    // multi-area listings starve the limit.
    let ceiling = (MAX_HITS_PER_QUERY as usize) * 4;
    let mut candidates: Vec<SearchIndexRow> = Vec::new();
    let mut distinct_listings: BTreeSet<String> = BTreeSet::new();
    let mut truncated = false;
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            SEARCH_INDEX.to_string(),
            QueryOptions {
                filter: Some(filter.to_string()),
                limit: Some(500),
                cursor: cursor.clone(),
            },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(row) = serde_json::from_slice::<SearchIndexRow>(&r.payload) {
                distinct_listings.insert(row.listing_id.clone());
                candidates.push(row);
            }
        }
        if distinct_listings.len() >= ceiling {
            truncated = true;
            break;
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }

    // Refine exactly and compute each row's AreaMatch. A geometric query
    // keeps only rows whose stored area actually intersects; a named-area
    // query keeps only matching labels; no area at all keeps everything
    // the filter already admitted.
    let mut by_listing: BTreeMap<String, (SearchIndexRow, AreaMatch)> = BTreeMap::new();
    for row in candidates {
        let area_match = match &query.area {
            None => AreaMatch::NotQueried,
            Some(Area::Named { label, .. }) => match &row.area {
                Some(row_area @ Area::Named { .. })
                    if area::labels_match(
                        &Area::Named { label: label.clone(), code: None },
                        row_area,
                    ) =>
                {
                    AreaMatch::Named { label: label.clone() }
                }
                _ => continue,
            },
            Some(q_area) => match &row.area {
                Some(row_area) => match area::areas_intersect(q_area, row_area) {
                    Some(true) => AreaMatch::Geometric { area_index: row.area_index },
                    _ => continue,
                },
                None => continue,
            },
        };
        // A listing with no location block at all, under a query with no
        // area, is reported honestly as `NoAreaStated` rather than as
        // `NotQueried`.
        let effective = if query.area.is_none() && row.area.is_none() {
            AreaMatch::NoAreaStated
        } else {
            area_match
        };
        let entry = by_listing.entry(row.listing_id.clone());
        match entry {
            Entry::Vacant(v) => {
                v.insert((row, effective));
            }
            Entry::Occupied(mut o) => {
                if area_match_precedence(&effective) < area_match_precedence(&o.get().1) {
                    o.insert((row, effective));
                }
            }
        }
    }

    let mut hits: Vec<(SearchIndexRow, AreaMatch)> = by_listing.into_values().collect();
    hits.sort_by(|a, b| {
        b.0.issued_at_secs.cmp(&a.0.issued_at_secs).then(a.0.listing_id.cmp(&b.0.listing_id))
    });
    let limit = query.limit.unwrap_or(MAX_HITS_PER_QUERY).min(MAX_HITS_PER_QUERY) as usize;
    hits.truncate(limit);

    let mut out = Vec::with_capacity(hits.len());
    for (row, area_match) in hits {
        // `publications` is keyed by `record_id`, which the index row
        // already carries -- a direct get, not the full-collection scan
        // `load_publication_for_listing` does for the (rare, owner-only)
        // withdrawal/republish path.
        // A row this node cannot itself parse (e.g. an older schema left
        // over from before a field was added) drops that one hit rather
        // than failing the whole anonymous-reachable search.
        let envelope = match get_json::<H, PublicationRow>(host, PUBLICATIONS, &row.record_id).await
        {
            Ok(Some(p)) => p.envelope,
            Ok(None) | Err(_) => continue,
        };
        out.push(SearchHit {
            listing_id: row.listing_id,
            record_id: row.record_id,
            envelope,
            issued_at_secs: row.issued_at_secs,
            received_at_secs: row.received_at_secs,
            area_match,
        });
    }

    let directory_did = match AppSigning::signing_identity(host).await {
        Ok(id) => id.signing_did,
        Err(_) => String::new(),
    };
    Response::ok(json!({
        "hits": out,
        "truncated": truncated,
        "directory": directory_did,
        "answered_at_secs": clock::now_secs(),
    }))
}

pub(in crate::app) async fn reindex<H: AppHost>(host: &H) -> Response {
    match rebuild_search_index(host).await {
        Ok(rebuilt) => Response::ok(json!({ "rebuilt": rebuilt })),
        Err(e) => Response::internal_error(e),
    }
}

/// Drops and rebuilds `search_index` from `publications`. Shared by the
/// `directory.reindex` verb and `import()`: an imported bundle writes
/// `publications` directly and nothing else would ever populate the
/// projection from it, so `directory.search` would answer zero hits for
/// listings that are demonstrably present until an owner happened to
/// notice and reindex by hand.
pub(in crate::app) async fn rebuild_search_index<H: AppHost>(host: &H) -> Result<u64, String> {
    ensure_coll(host, SEARCH_INDEX, &[]).await?;
    AppDataLayer::delete_many(host, SEARCH_INDEX.to_string(), json!({}).to_string())
        .await
        .map_err(|e| e.to_string())?;
    let rows = collect_raw(host, PUBLICATIONS).await?;
    let mut rebuilt = 0u64;
    for (_, v) in rows {
        let Ok(row) = serde_json::from_value::<PublicationRow>(v) else { continue };
        let verdict = listing::verify_envelope(&row.envelope, clock::now_secs());
        let Some(payload) = verdict.payload else { continue };
        for index_row in build_index_rows(
            &payload,
            &row.record_id,
            &row.issuer,
            row.issued_at_secs,
            row.received_at_secs,
        ) {
            let key = search_index_key(&index_row.listing_id, index_row.area_index);
            if put_json(host, SEARCH_INDEX, &key, &index_row).await.is_ok() {
                rebuilt += 1;
            }
        }
    }
    Ok(rebuilt)
}

//! Directory service application logic, target-independent.
//!
//! One service, two halves, never sharing a collection: the **server**
//! half (a SynOrg's own settings, roster, publications and search) and the
//! **client** half (this person's own list of directories, the fan-out,
//! and the merge). The server half admits a stranger's signed bytes; the
//! client half never talks to a stranger directly -- it asks the node to
//! make one bounded call per directory and reads back what the node
//! verified.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::{
        data_layer::{
            CollectionSchema, IndexDefinition, IndexType, Mutation, QueryOptions, RecordWriteValue,
        },
        proxy::{CallOptions, CallTarget},
    },
};
use syneroym_roym_core::{
    admit::{self, Caller, WireRule},
    area::{self, Area},
    backup::{
        BUNDLE_VERSION, Bundle, BundleManifest, SECTION_MEMBERS, SECTION_PUBLICATION_LOG,
        SECTION_PUBLICATIONS, SECTION_SOURCES, SECTION_SYNORG,
    },
    clock,
    directory::{
        AreaMatch, DEFAULT_SOURCE_TIMEOUT_MS, DIRECTORY_SCHEMA_VERSION, MAX_CLIENT_CONCURRENCY,
        MAX_HITS_PER_QUERY, MAX_REFUSED_RESULTS, MAX_SOURCES, MAX_STORED_PER_SOURCE, Member,
        RUN_RETENTION_SECS, SearchHit, SearchQuery, SourceError, SynOrgSettings, category_tokens,
        normalize_category, normalize_text,
    },
    envelope::{Request, Response},
    listing::{self, ListingVerdict},
    safety::{self, Admission, PublicationLimits},
    services,
};

pub const SCHEMA_VERSION: u32 = DIRECTORY_SCHEMA_VERSION;

pub const SETTINGS: &str = "settings";
pub const MEMBERS: &str = "members";
pub const PUBLICATIONS: &str = "publications";
pub const SEARCH_INDEX: &str = "search_index";
pub const PUBLICATION_LOG: &str = "publication_log";
pub const SOURCES: &str = "sources";
pub const SEARCH_RUNS: &str = "search_runs";
pub const RUNS: &str = "runs";

const SETTINGS_KEY: &str = "synorg";

/// The three methods a foreign node may reach. `directory.search` and
/// `directory.info` admit a stranger with no identity at all -- reading
/// something this installation publishes on purpose costs nothing to
/// leave open. `directory.publish` records who asked, because a
/// publication is durable and must name a party.
const WIRE_REACHABLE: &[(&str, WireRule)] = &[
    ("directory.search", WireRule::Open),
    ("directory.info", WireRule::Open),
    ("directory.publish", WireRule::VerifiedOnly),
];

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::DIRECTORY.name,
        "schema_version": SCHEMA_VERSION,
    })
    .to_string())
}

async fn ensure_coll<H: AppHost>(
    host: &H,
    name: &str,
    indexes: &[IndexDefinition],
) -> Result<(), String> {
    AppDataLayer::create_collection(
        host,
        CollectionSchema { name: name.to_string(), indexes: indexes.to_vec() },
    )
    .await
    .map_err(|e| e.to_string())
}

fn idx(field: &str, ty: IndexType) -> IndexDefinition {
    IndexDefinition { field_name: field.to_string(), type_: ty }
}

/// Every row of `collection`, oldest write order, paging until the
/// data-layer's own cursor answers `None`.
async fn collect_raw<H: AppHost>(
    host: &H,
    collection: &str,
) -> Result<Vec<(String, Value)>, String> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            collection.to_string(),
            QueryOptions { filter: None, limit: Some(500), cursor: cursor.clone() },
        )
        .await
        .map_err(|e| e.to_string())?;
        for r in page.records {
            if let Ok(parsed) = serde_json::from_slice::<Value>(&r.payload) {
                out.push((r.id, parsed));
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(out)
}

async fn collect<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
    Ok(collect_raw(host, collection)
        .await?
        .into_iter()
        .map(|(id, payload)| json!({ "id": id, "payload": payload }))
        .collect())
}

async fn put_json<H: AppHost>(
    host: &H,
    collection: &str,
    id: &str,
    value: &impl Serialize,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    AppDataLayer::put(
        host,
        collection.to_string(),
        RecordWriteValue { id: id.to_string(), payload: bytes },
    )
    .await
    .map_err(|e| e.to_string())
}

async fn get_json<H: AppHost, T: for<'de> Deserialize<'de>>(
    host: &H,
    collection: &str,
    id: &str,
) -> Result<Option<T>, String> {
    let row = AppDataLayer::get(host, collection.to_string(), id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    let caller = match admit::admit(host, WIRE_REACHABLE, &req.method).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    match req.method.as_str() {
        "directory.ping" => Response::ok(json!({ "service": services::DIRECTORY.name })),
        // Server half.
        "directory.settings" => get_settings(host).await,
        "directory.set-settings" => set_settings(host, &req).await,
        "directory.info" => info(host).await,
        "member.add" => member_add(host, &req).await,
        "member.remove" => member_remove(host, &req).await,
        "member.list" => member_list(host).await,
        "directory.publish" => publish(host, &req, caller).await,
        "directory.unpublish" => unpublish(host, &req).await,
        "directory.publications" => publications(host).await,
        "directory.search" => search(host, &req).await,
        "directory.limits" => match load_publication_limits(host).await {
            Ok(l) => Response::ok(json!(l)),
            Err(e) => Response::internal_error(e),
        },
        "directory.set-limits" => set_limits(host, &req).await,
        "directory.reindex" => reindex(host).await,
        "directory.export" => export(host).await,
        "directory.import" => import(host, &req).await,
        // Client half.
        "directory.add-source" => add_source(host, &req).await,
        "directory.probe-info" => probe_info_verb(host, &req).await,
        "directory.remove-source" => remove_source(host, &req).await,
        "directory.sources" => sources(host).await,
        "directory.start-run" => start_run(host).await,
        "directory.query-source" => query_source(host, &req).await,
        "directory.merge" => merge(host, &req).await,
        "directory.run-envelope" => run_envelope(host, &req).await,
        "directory.publish-to-source" => publish_to_source(host, &req).await,
        other => Response::method_not_found(other),
    }
}

// ---------------------------------------------------------------------
// Server half: settings, roster.
// ---------------------------------------------------------------------

async fn load_settings<H: AppHost>(host: &H) -> Result<Option<SynOrgSettings>, String> {
    ensure_coll(host, SETTINGS, &[]).await?;
    get_json(host, SETTINGS, SETTINGS_KEY).await
}

async fn get_settings<H: AppHost>(host: &H) -> Response {
    match load_settings(host).await {
        Ok(Some(s)) => Response::ok(json!(s)),
        Ok(None) => Response::ok(Value::Null),
        Err(e) => Response::internal_error(e),
    }
}

async fn set_settings<H: AppHost>(host: &H, req: &Request) -> Response {
    let settings: SynOrgSettings = match serde_json::from_value(req.params.clone()) {
        Ok(s) => s,
        Err(e) => return Response::invalid_params(format!("invalid settings: {e}")),
    };
    if let Err(e) = settings.validate() {
        return Response::invalid_params(e.to_string());
    }
    if let Err(e) = ensure_coll(host, SETTINGS, &[]).await {
        return Response::internal_error(e);
    }
    if let Err(e) = put_json(host, SETTINGS, SETTINGS_KEY, &settings).await {
        return Response::internal_error(e);
    }
    Response::ok(json!(settings))
}

async fn member_count<H: AppHost>(host: &H) -> Result<u64, String> {
    ensure_coll(host, MEMBERS, &[]).await?;
    Ok(collect_raw(host, MEMBERS).await?.len() as u64)
}

async fn info<H: AppHost>(host: &H) -> Response {
    let settings = match load_settings(host).await {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e),
    };
    let Some(settings) = settings else {
        // Refusing would be indistinguishable from a network fault, which
        // is worse for the case this is actually about: someone adding an
        // address a friend gave them.
        return Response::ok(Value::Null);
    };
    let count = match member_count(host).await {
        Ok(c) => c,
        Err(e) => return Response::internal_error(e),
    };
    Response::ok(json!({
        "name": settings.name,
        "rules": settings.rules,
        "area": settings.area,
        "categories": settings.categories,
        "support_contact": settings.support_contact,
        "dispute_path": settings.dispute_path,
        "retention_secs": settings.retention_secs,
        "member_count": count,
    }))
}

async fn member_add<H: AppHost>(host: &H, req: &Request) -> Response {
    let did = match req.params.get("did").and_then(Value::as_str) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => return Response::invalid_params("did is required"),
    };
    let note = req.params.get("note").and_then(Value::as_str).unwrap_or_default().to_string();
    if let Err(e) = ensure_coll(host, MEMBERS, &[]).await {
        return Response::internal_error(e);
    }
    let member = Member { did: did.clone(), note, added_at_secs: clock::now_secs() };
    if let Err(e) = put_json(host, MEMBERS, &did, &member).await {
        return Response::internal_error(e);
    }
    Response::ok(json!(member))
}

async fn member_remove<H: AppHost>(host: &H, req: &Request) -> Response {
    let did = match req.params.get("did").and_then(Value::as_str) {
        Some(d) => d.to_string(),
        None => return Response::invalid_params("did is required"),
    };
    if let Err(e) = ensure_coll(host, MEMBERS, &[]).await {
        return Response::internal_error(e);
    }
    let existed = AppDataLayer::get(host, MEMBERS.to_string(), did.clone())
        .await
        .map(|o| o.is_some())
        .unwrap_or(false);
    if existed && let Err(e) = AppDataLayer::delete(host, MEMBERS.to_string(), did).await {
        return Response::internal_error(e.to_string());
    }
    Response::ok(json!({ "removed": existed }))
}

async fn member_list<H: AppHost>(host: &H) -> Response {
    if let Err(e) = ensure_coll(host, MEMBERS, &[]).await {
        return Response::internal_error(e);
    }
    let rows = match collect_raw(host, MEMBERS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let members: Vec<Value> = rows.into_iter().map(|(_, v)| v).collect();
    Response::ok(json!({ "members": members }))
}

// ---------------------------------------------------------------------
// Server half: publication and search.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PublicationRow {
    envelope: String,
    record_id: String,
    listing_id: String,
    issuer: String,
    published_by: String,
    received_at_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchIndexRow {
    listing_id: String,
    record_id: String,
    area_index: u32,
    issuer: String,
    status: String,
    issued_at_secs: u64,
    received_at_secs: u64,
    categories: String,
    text: String,
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

fn search_index_key(listing_id: &str, area_index: u32) -> String {
    format!("{listing_id}#{area_index}")
}

async fn load_publication_limits<H: AppHost>(host: &H) -> Result<PublicationLimits, String> {
    match load_settings(host).await? {
        Some(s) => Ok(s.publication_limits),
        None => Ok(PublicationLimits::default()),
    }
}

/// Every `publication_log` row for `issuer` strictly newer than
/// `now - window_secs`. Filtered at
/// the host, not scanned and dropped in the guest.
async fn publication_secs_in_window<H: AppHost>(
    host: &H,
    issuer: &str,
    window_secs: u64,
    now: u64,
) -> Result<Vec<u64>, String> {
    ensure_coll(
        host,
        PUBLICATION_LOG,
        &[idx("issuer", IndexType::String), idx("at_secs", IndexType::Numeric)],
    )
    .await?;
    let floor = now.saturating_sub(window_secs);
    let filter = json!({ "$and": [ { "issuer": issuer }, { "at_secs": { "$gt": floor } } ] });
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            PUBLICATION_LOG.to_string(),
            QueryOptions {
                filter: Some(filter.to_string()),
                limit: Some(500),
                cursor: cursor.clone(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        for r in page.records {
            if let Ok(v) = serde_json::from_slice::<Value>(&r.payload)
                && let Some(at) = v.get("at_secs").and_then(Value::as_u64)
            {
                out.push(at);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(out)
}

/// Deletes every `search_index` row for `listing_id`, whatever `area_index`
/// values it holds -- the fix for the stale-row bug a republish with fewer
/// areas would otherwise leave behind.
async fn delete_search_index_for<H: AppHost>(host: &H, listing_id: &str) -> Result<(), String> {
    ensure_coll(host, SEARCH_INDEX, &[]).await?;
    AppDataLayer::delete_many(
        host,
        SEARCH_INDEX.to_string(),
        json!({ "listing_id": listing_id }).to_string(),
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn build_index_rows(
    payload: &listing::ListingPayload,
    record_id: &str,
    issuer: &str,
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
    let open_to = payload.relationship.as_ref().map(|r| format!("{:?}", r.open_to).to_lowercase());
    let booking_mode = payload.booking.as_ref().map(|b| format!("{:?}", b.mode).to_lowercase());

    let areas: Vec<Area> =
        payload.location.as_ref().map(|l| l.service_area.clone()).unwrap_or_default();
    if areas.is_empty() {
        return vec![SearchIndexRow {
            listing_id: payload.listing_id.clone(),
            record_id: record_id.to_string(),
            area_index: 0,
            issuer: issuer.to_string(),
            status,
            issued_at_secs: received_at_secs,
            received_at_secs,
            categories,
            text,
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
            SearchIndexRow {
                listing_id: payload.listing_id.clone(),
                record_id: record_id.to_string(),
                area_index: i as u32,
                issuer: issuer.to_string(),
                status: status.clone(),
                issued_at_secs: received_at_secs,
                received_at_secs,
                categories: categories.clone(),
                text: text.clone(),
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

async fn publish<H: AppHost>(host: &H, req: &Request, caller: Caller) -> Response {
    let published_by = match caller {
        Caller::Verified(did) => did,
        // `admit` never returns `Internal`/`Anonymous` for a
        // `VerifiedOnly` method -- it either admits `Verified` or refuses
        // before this handler runs.
        _ => return Response::internal_error("directory.publish reached with no verified caller"),
    };
    let envelope = match req.params.get("envelope").and_then(Value::as_str) {
        Some(e) => e.to_string(),
        None => return Response::invalid_params("envelope is required"),
    };
    let now = clock::now_secs();
    let verdict: ListingVerdict = listing::verify_envelope(&envelope, now);
    if !verdict.verified {
        return Response::invalid_params(
            verdict.reason.unwrap_or_else(|| "not verified".to_string()),
        );
    }
    let (Some(payload), Some(record_id), Some(issuer)) =
        (verdict.payload, verdict.record_id, verdict.issuer)
    else {
        return Response::internal_error(
            "a verified verdict carried no payload, record id or issuer",
        );
    };

    match payload.status {
        listing::ListingStatus::Draft => {
            return Response::invalid_params("a draft listing may not be published");
        }
        listing::ListingStatus::Active | listing::ListingStatus::Withdrawn => {}
    }
    if payload.conversation_address.trim().is_empty() {
        return Response::internal_error("a verified listing had an empty conversation_address");
    }

    if let Err(e) = ensure_coll(host, PUBLICATIONS, &[idx("listing_id", IndexType::String)]).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_coll(
        host,
        SEARCH_INDEX,
        &[idx("listing_id", IndexType::String), idx("status", IndexType::String)],
    )
    .await
    {
        return Response::internal_error(e);
    }

    // Withdrawal: remove the stored publication and its index rows, consume
    // no budget.
    if matches!(payload.status, listing::ListingStatus::Withdrawn) {
        let existed = load_publication_for_listing(host, &payload.listing_id).await;
        if let Err(e) = existed {
            return Response::internal_error(e);
        }
        if let Err(e) = AppDataLayer::delete_many(
            host,
            PUBLICATIONS.to_string(),
            json!({ "listing_id": payload.listing_id }).to_string(),
        )
        .await
        {
            return Response::internal_error(e.to_string());
        }
        if let Err(e) = delete_search_index_for(host, &payload.listing_id).await {
            return Response::internal_error(e);
        }
        return Response::ok(json!({ "listing_id": payload.listing_id, "withdrawn": true }));
    }

    let settings = match load_settings(host).await {
        Ok(s) => s.unwrap_or(SynOrgSettings {
            name: String::new(),
            rules: String::new(),
            area: vec![],
            categories: vec![],
            support_contact: String::new(),
            dispute_path: String::new(),
            retention_secs: syneroym_roym_core::directory::MAX_RETENTION_SECS,
            publication_limits: PublicationLimits::default(),
        }),
        Err(e) => return Response::internal_error(e),
    };
    let limits = settings.publication_limits;
    let prior_secs = match publication_secs_in_window(host, &issuer, limits.window_secs, now).await
    {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    match safety::admit_publication(&prior_secs, &limits, now) {
        Admission::Allow => {}
        Admission::RateLimited { retry_after_secs } => {
            return Response::err(
                -32602,
                format!("publication rate limit reached; retry in {retry_after_secs}s"),
            )
            .with_data(
                json!({ "admission": "rate-limited", "retry_after_secs": retry_after_secs }),
            );
        }
        Admission::Blocked => {
            return Response::internal_error("admit_publication returned Blocked");
        }
    }

    // Prune the limiter ledger and, per the SynOrg's own retention policy,
    // publications and their index rows past their retention window --
    // in the one pass that already touches this data.
    let log_floor = now.saturating_sub(limits.window_secs);
    if let Err(e) = AppDataLayer::delete_many(
        host,
        PUBLICATION_LOG.to_string(),
        json!({ "at_secs": { "$lte": log_floor } }).to_string(),
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }
    let retention_floor = now.saturating_sub(settings.retention_secs);
    if let Err(e) = AppDataLayer::delete_many(
        host,
        PUBLICATIONS.to_string(),
        json!({ "received_at_secs": { "$lte": retention_floor } }).to_string(),
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }
    if let Err(e) = AppDataLayer::delete_many(
        host,
        SEARCH_INDEX.to_string(),
        json!({ "received_at_secs": { "$lte": retention_floor } }).to_string(),
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }

    // Replace the prior version: index rows deleted first, so a republish
    // with fewer areas never leaves an orphaned row behind.
    if let Err(e) = delete_search_index_for(host, &payload.listing_id).await {
        return Response::internal_error(e);
    }
    if let Err(e) = AppDataLayer::delete_many(
        host,
        PUBLICATIONS.to_string(),
        json!({ "listing_id": payload.listing_id }).to_string(),
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }

    let pub_row = PublicationRow {
        envelope: envelope.clone(),
        record_id: record_id.clone(),
        listing_id: payload.listing_id.clone(),
        issuer: issuer.clone(),
        published_by,
        received_at_secs: now,
    };
    if let Err(e) = put_json(host, PUBLICATIONS, &record_id, &pub_row).await {
        return Response::internal_error(e);
    }

    for row in build_index_rows(&payload, &record_id, &issuer, now) {
        let key = search_index_key(&row.listing_id, row.area_index);
        if let Err(e) = put_json(host, SEARCH_INDEX, &key, &row).await {
            return Response::internal_error(e);
        }
    }

    if let Err(e) = put_json(
        host,
        PUBLICATION_LOG,
        &format!("{issuer}:{now}:{record_id}"),
        &json!({ "issuer": issuer, "at_secs": now }),
    )
    .await
    {
        return Response::internal_error(e);
    }

    Response::ok(json!({ "listing_id": payload.listing_id, "record_id": record_id }))
}

async fn load_publication_for_listing<H: AppHost>(
    host: &H,
    listing_id: &str,
) -> Result<Option<PublicationRow>, String> {
    let rows = collect_raw(host, PUBLICATIONS).await?;
    for (_, v) in rows {
        if let Ok(row) = serde_json::from_value::<PublicationRow>(v.clone())
            && row.listing_id == listing_id
        {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

async fn unpublish<H: AppHost>(host: &H, req: &Request) -> Response {
    let listing_id = match req.params.get("listing_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return Response::invalid_params("listing_id is required"),
    };
    if let Err(e) = ensure_coll(host, PUBLICATIONS, &[]).await {
        return Response::internal_error(e);
    }
    if let Err(e) = AppDataLayer::delete_many(
        host,
        PUBLICATIONS.to_string(),
        json!({ "listing_id": listing_id }).to_string(),
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }
    if let Err(e) = delete_search_index_for(host, &listing_id).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "listing_id": listing_id, "unpublished": true }))
}

async fn publications<H: AppHost>(host: &H) -> Response {
    if let Err(e) = ensure_coll(host, PUBLICATIONS, &[]).await {
        return Response::internal_error(e);
    }
    match collect_raw(host, PUBLICATIONS).await {
        Ok(rows) => Response::ok(
            json!({ "publications": rows.into_iter().map(|(_, v)| v).collect::<Vec<_>>() }),
        ),
        Err(e) => Response::internal_error(e),
    }
}

async fn set_limits<H: AppHost>(host: &H, req: &Request) -> Response {
    let window_secs = match req.params.get("window_secs").and_then(Value::as_u64) {
        Some(w) => w,
        None => return Response::invalid_params("window_secs is required"),
    };
    let max_per_window = match req.params.get("max_per_window").and_then(Value::as_u64) {
        Some(m) => m as u32,
        None => return Response::invalid_params("max_per_window is required"),
    };
    let limits = PublicationLimits { window_secs, max_per_window };
    if let Err(e) = limits.validate() {
        return Response::invalid_params(e.to_string());
    }
    let mut settings = match load_settings(host).await {
        Ok(Some(s)) => s,
        Ok(None) => return Response::invalid_params("this installation runs no SynOrg yet"),
        Err(e) => return Response::internal_error(e),
    };
    settings.publication_limits = limits;
    if let Err(e) = put_json(host, SETTINGS, SETTINGS_KEY, &settings).await {
        return Response::internal_error(e);
    }
    Response::ok(json!(limits))
}

fn area_match_precedence(m: &AreaMatch) -> u8 {
    match m {
        AreaMatch::Geometric { .. } => 0,
        AreaMatch::Named { .. } => 1,
        AreaMatch::NoAreaStated => 2,
        AreaMatch::NotQueried => 3,
    }
}

async fn search<H: AppHost>(host: &H, req: &Request) -> Response {
    let query: SearchQuery = match serde_json::from_value(req.params.clone()) {
        Ok(q) => q,
        Err(e) => return Response::invalid_params(format!("invalid query: {e}")),
    };
    if let Err(e) = ensure_coll(host, SEARCH_INDEX, &[]).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_coll(host, PUBLICATIONS, &[]).await {
        return Response::internal_error(e);
    }

    let mut and_clauses: Vec<Value> = vec![json!({ "status": "active" })];
    for cat in &query.categories {
        let normalized = normalize_category(cat);
        and_clauses.push(json!({ "categories": { "$regex": category_tokens(&[normalized]) } }));
    }
    if let Some(text) = &query.text {
        let normalized = normalize_text(text);
        if !normalized.is_empty() {
            and_clauses.push(json!({ "text": { "$regex": normalized } }));
        }
    }
    if let Some(open_to) = &query.open_to {
        and_clauses.push(json!({ "open_to": open_to.to_lowercase() }));
    }
    if let Some(booking_mode) = &query.booking_mode {
        and_clauses.push(json!({ "booking_mode": booking_mode.to_lowercase() }));
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
    let filter = json!({ "$and": and_clauses });

    let ceiling = (MAX_HITS_PER_QUERY as usize) * 4;
    let mut candidates: Vec<SearchIndexRow> = Vec::new();
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
                candidates.push(row);
            }
        }
        if candidates.len() >= ceiling {
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
        if matches!(area_match, AreaMatch::NotQueried) && row.area.is_none() {
            // A listing with no location block at all, under a query with
            // no area: report it honestly rather than as `NotQueried`.
        }
        let effective = if query.area.is_none() && row.area.is_none() {
            AreaMatch::NoAreaStated
        } else {
            area_match
        };
        let entry = by_listing.entry(row.listing_id.clone());
        match entry {
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert((row, effective));
            }
            std::collections::btree_map::Entry::Occupied(mut o) => {
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
        let envelope = match load_publication_for_listing(host, &row.listing_id).await {
            Ok(Some(p)) => p.envelope,
            Ok(None) => continue,
            Err(e) => return Response::internal_error(e),
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

    let directory_did = match syneroym_app_host::AppSigning::signing_identity(host).await {
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

async fn reindex<H: AppHost>(host: &H) -> Response {
    if let Err(e) = ensure_coll(host, SEARCH_INDEX, &[]).await {
        return Response::internal_error(e);
    }
    if let Err(e) =
        AppDataLayer::delete_many(host, SEARCH_INDEX.to_string(), json!({}).to_string()).await
    {
        return Response::internal_error(e.to_string());
    }
    let rows = match collect_raw(host, PUBLICATIONS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let mut rebuilt = 0u64;
    for (_, v) in rows {
        let Ok(row) = serde_json::from_value::<PublicationRow>(v) else { continue };
        let verdict = listing::verify_envelope(&row.envelope, clock::now_secs());
        let Some(payload) = verdict.payload else { continue };
        for index_row in
            build_index_rows(&payload, &row.record_id, &row.issuer, row.received_at_secs)
        {
            let key = search_index_key(&index_row.listing_id, index_row.area_index);
            if put_json(host, SEARCH_INDEX, &key, &index_row).await.is_ok() {
                rebuilt += 1;
            }
        }
    }
    Response::ok(json!({ "rebuilt": rebuilt }))
}

// ---------------------------------------------------------------------
// Server half: export / import.
// ---------------------------------------------------------------------

async fn owner_did_or_node<H: AppHost>(host: &H) -> String {
    match syneroym_app_host::AppSigning::signing_identity(host).await {
        Ok(id) => id.owner_did.unwrap_or(id.signing_did),
        Err(_) => String::new(),
    }
}

async fn export<H: AppHost>(host: &H) -> Response {
    let subject = owner_did_or_node(host).await;
    let now = clock::now_secs();
    for c in [SETTINGS, MEMBERS, PUBLICATIONS, PUBLICATION_LOG, SOURCES] {
        if let Err(e) = ensure_coll(host, c, &[]).await {
            return Response::internal_error(e);
        }
    }
    let synorg = match collect(host, SETTINGS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let members = match collect(host, MEMBERS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let publications = match collect(host, PUBLICATIONS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let publication_log = match collect(host, PUBLICATION_LOG).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let sources = match collect(host, SOURCES).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let sections = BTreeMap::from([
        (SECTION_SYNORG.to_string(), synorg),
        (SECTION_MEMBERS.to_string(), members),
        (SECTION_PUBLICATIONS.to_string(), publications),
        (SECTION_PUBLICATION_LOG.to_string(), publication_log),
        (SECTION_SOURCES.to_string(), sources),
    ]);
    let mut manifest_sections = BTreeMap::new();
    for (k, v) in &sections {
        match Bundle::digest(SCHEMA_VERSION, v) {
            Ok(d) => {
                manifest_sections.insert(k.clone(), d);
            }
            Err(e) => return Response::internal_error(e.to_string()),
        }
    }
    let bundle = Bundle {
        manifest: BundleManifest {
            bundle_version: BUNDLE_VERSION,
            produced_at_secs: now,
            subject_did: subject,
            sections: manifest_sections,
        },
        sections,
    };
    match serde_json::to_value(&bundle) {
        Ok(v) => Response::ok(v),
        Err(e) => Response::internal_error(e.to_string()),
    }
}

async fn import<H: AppHost>(host: &H, req: &Request) -> Response {
    let bundle_val = match req.params.get("bundle").cloned().or_else(|| Some(req.params.clone())) {
        Some(v) => v,
        None => return Response::invalid_params("bundle is required"),
    };
    let bundle = match Bundle::from_json(&bundle_val.to_string()) {
        Ok(b) => b,
        Err(e) => return Response::invalid_params(format!("invalid bundle: {e}")),
    };
    if let Err(e) = bundle.check_integrity() {
        return Response::invalid_params(e.to_string());
    }
    let owner = owner_did_or_node(host).await;
    if !owner.is_empty() && bundle.manifest.subject_did != owner {
        return Response::invalid_params(format!(
            "bundle belongs to '{}', this node holds '{}'",
            bundle.manifest.subject_did, owner
        ));
    }
    for (name, declared) in &bundle.manifest.sections {
        if declared.schema_version != SCHEMA_VERSION {
            return Response::invalid_params(format!(
                "section '{name}' has schema version {}, this node requires {SCHEMA_VERSION}",
                declared.schema_version
            ));
        }
    }

    let mut prepared: Vec<(&'static str, Vec<Mutation>)> = Vec::new();
    for (name, records) in &bundle.sections {
        let collection = match name.as_str() {
            SECTION_SYNORG => SETTINGS,
            SECTION_MEMBERS => MEMBERS,
            SECTION_PUBLICATIONS => PUBLICATIONS,
            SECTION_PUBLICATION_LOG => PUBLICATION_LOG,
            SECTION_SOURCES => SOURCES,
            other => return Response::invalid_params(format!("unknown section '{other}'")),
        };
        let mut muts = Vec::new();
        for rec in records {
            let id = match rec.get("id").and_then(Value::as_str) {
                Some(i) => i.to_string(),
                None => return Response::invalid_params("record missing id"),
            };
            let payload_val = match rec.get("payload") {
                Some(p) => p.clone(),
                None => return Response::invalid_params("record missing payload"),
            };
            if name == SECTION_PUBLICATIONS
                && let Some(env_str) = payload_val.get("envelope").and_then(Value::as_str)
            {
                let verdict = listing::verify_envelope(env_str, clock::now_secs());
                if !verdict.verified {
                    return Response::invalid_params(format!(
                        "publication record '{id}' failed verification: {}",
                        verdict.reason.unwrap_or_default()
                    ));
                }
            }
            let payload_bytes = match serde_json::to_vec(&payload_val) {
                Ok(b) => b,
                Err(e) => return Response::internal_error(e.to_string()),
            };
            muts.push(Mutation::Put(RecordWriteValue { id, payload: payload_bytes }));
        }
        prepared.push((collection, muts));
    }

    let mut counts = Map::new();
    for (collection, muts) in prepared {
        if let Err(e) = ensure_coll(host, collection, &[]).await {
            return Response::internal_error(e);
        }
        counts.insert(collection.to_string(), json!(muts.len()));
        for chunk in muts.chunks(100) {
            if let Err(e) =
                AppDataLayer::batch_mutate(host, collection.to_string(), chunk.to_vec()).await
            {
                return Response::internal_error(e.to_string());
            }
        }
    }
    Response::ok(json!({ "imported": counts }))
}

// ---------------------------------------------------------------------
// Client half: sources, runs, merge.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SourceRow {
    did: String,
    label: String,
    added_at_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_ok_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<SourceError>,
}

/// One `directory.info` call at a chosen address, over the wire, with no
/// side effect. What `add_source`'s own probe does; also its own verb, for
/// a caller (`roymctl roym directory info`) that wants to read a
/// directory's public statement without adding it as a source.
async fn probe_info<H: AppHost>(
    host: &H,
    did: &str,
) -> Result<String, syneroym_app_host::types::proxy::ProxyError> {
    let probe_params = json!({ "method": "directory.info", "params": {} }).to_string();
    host.call(
        CallTarget::Service(did.to_string()),
        services::DIRECTORY.interface.to_string(),
        "invoke".to_string(),
        json!([probe_params]).to_string(),
        Some(CallOptions {
            protocol: None,
            idempotent: true,
            timeout_ms: Some(DEFAULT_SOURCE_TIMEOUT_MS),
            routing_key: None,
            idempotency_key: None,
        }),
    )
    .await
}

async fn probe_info_verb<H: AppHost>(host: &H, req: &Request) -> Response {
    let did = match req.params.get("did").and_then(Value::as_str) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => return Response::invalid_params("did is required"),
    };
    match probe_info(host, &did).await {
        Ok(raw) => match serde_json::from_str::<Response>(&raw) {
            Ok(resp) => resp,
            Err(e) => Response::internal_error(e.to_string()),
        },
        Err(e) => Response::internal_error(format!("{e:?}")),
    }
}

async fn add_source<H: AppHost>(host: &H, req: &Request) -> Response {
    let did = match req.params.get("did").and_then(Value::as_str) {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => return Response::invalid_params("did is required"),
    };
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Response::internal_error(e);
    }
    let existing = match collect_raw(host, SOURCES).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let already_present = existing.iter().any(|(id, _)| id == &did);
    if !already_present && existing.len() >= MAX_SOURCES {
        return Response::invalid_params(format!("at most {MAX_SOURCES} sources may be added"));
    }

    // Probe once: `directory.info` over the wire. A transport failure is
    // stored as `last_error`; a successful probe that answers `null` is
    // reported to the caller rather than silently accepted, so a person
    // does not sit waiting for results from an address that will never
    // have any.
    let probe = probe_info(host, &did).await;

    let now = clock::now_secs();
    let requested_label =
        req.params.get("label").and_then(Value::as_str).unwrap_or_default().to_string();
    let mut last_ok_secs = None;
    let mut last_error = None;
    let mut probe_note: Option<String> = None;
    let mut label = requested_label.clone();
    match probe {
        Ok(raw) => match serde_json::from_str::<Response>(&raw) {
            Ok(resp) if resp.result.as_ref().is_some_and(Value::is_null) => {
                last_ok_secs = Some(now);
                probe_note = Some("this address answered, but runs no directory".to_string());
            }
            Ok(resp) => {
                last_ok_secs = Some(now);
                if label.is_empty() {
                    label = resp
                        .result
                        .as_ref()
                        .and_then(|r| r.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
            }
            Err(e) => last_error = Some(SourceError::Unreadable { reason: e.to_string() }),
        },
        Err(_) => last_error = Some(SourceError::TimedOut),
    };

    let row = SourceRow { did: did.clone(), label, added_at_secs: now, last_ok_secs, last_error };
    if let Err(e) = put_json(host, SOURCES, &did, &row).await {
        return Response::internal_error(e);
    }
    Response::ok(json!({ "source": row, "probe": probe_note }))
}

async fn remove_source<H: AppHost>(host: &H, req: &Request) -> Response {
    let did = match req.params.get("did").and_then(Value::as_str) {
        Some(d) => d.to_string(),
        None => return Response::invalid_params("did is required"),
    };
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Response::internal_error(e);
    }
    let existed = AppDataLayer::get(host, SOURCES.to_string(), did.clone())
        .await
        .map(|o| o.is_some())
        .unwrap_or(false);
    if existed && let Err(e) = AppDataLayer::delete(host, SOURCES.to_string(), did).await {
        return Response::internal_error(e.to_string());
    }
    Response::ok(json!({ "removed": existed }))
}

async fn sources<H: AppHost>(host: &H) -> Response {
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Response::internal_error(e);
    }
    match collect_raw(host, SOURCES).await {
        Ok(rows) => {
            Response::ok(json!({ "sources": rows.into_iter().map(|(_, v)| v).collect::<Vec<_>>() }))
        }
        Err(e) => Response::internal_error(e),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunRow {
    at_secs: u64,
    sources: Vec<String>,
}

async fn start_run<H: AppHost>(host: &H) -> Response {
    if let Err(e) = ensure_coll(host, RUNS, &[]).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_coll(host, SOURCES, &[]).await {
        return Response::internal_error(e);
    }
    let now = clock::now_secs();
    if let Err(e) = AppDataLayer::delete_many(
        host,
        RUNS.to_string(),
        json!({ "at_secs": { "$lte": now.saturating_sub(RUN_RETENTION_SECS) } }).to_string(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchRunRow {
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
    /// Present only for a verified row that has not yet been fetched by
    /// `run-envelope` -- blanked (empty string) once `merge` reads the
    /// row, so a merge never carries the envelope through by accident.
    envelope: String,
    refused: bool,
}

async fn query_source<H: AppHost>(host: &H, req: &Request) -> Response {
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

    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &[]).await {
        return Response::internal_error(e);
    }

    let raw = match call_result {
        Ok(r) => r,
        Err(e) => {
            let source_error = map_proxy_error(&e);
            record_source_error(host, &source, &source_error).await;
            return Response::ok(
                json!({ "source": source, "verified": 0, "refused": 0, "error": source_error }),
            );
        }
    };
    let resp: Response = match serde_json::from_str(&raw) {
        Ok(r) => r,
        Err(e) => {
            let source_error = SourceError::Unreadable { reason: e.to_string() };
            record_source_error(host, &source, &source_error).await;
            return Response::ok(
                json!({ "source": source, "verified": 0, "refused": 0, "error": source_error }),
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
            json!({ "source": source, "verified": 0, "refused": 0, "error": source_error }),
        );
    };
    let hits: Vec<SearchHit> =
        match serde_json::from_value(result.get("hits").cloned().unwrap_or(json!([]))) {
            Ok(h) => h,
            Err(e) => {
                let source_error = SourceError::Unreadable { reason: e.to_string() };
                record_source_error(host, &source, &source_error).await;
                return Response::ok(
                    json!({ "source": source, "verified": 0, "refused": 0, "error": source_error }),
                );
            }
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
            let Some(payload) = verdict.payload else { continue };
            let row = SearchRunRow {
                listing_id: verdict.listing_id.unwrap_or_default(),
                record_id: verdict.record_id.unwrap_or_default(),
                source: source.clone(),
                issuer: verdict.issuer.unwrap_or_default(),
                title: payload.title,
                summary: payload.summary,
                categories: payload.categories,
                conversation_address: verdict.conversation_address.unwrap_or_default(),
                status: format!("{:?}", verdict.status.unwrap_or(listing::ListingStatus::Active))
                    .to_lowercase(),
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
            let key = format!("{run_id}#{}", row.record_id);
            if put_json(host, SEARCH_RUNS, &key, &row).await.is_ok() {
                verified_count += 1;
            }
        } else {
            if refused_count >= MAX_REFUSED_RESULTS {
                continue;
            }
            let row = SearchRunRow {
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
            let key = format!("{run_id}#refused#{}", row.record_id);
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

    Response::ok(
        json!({ "source": source, "verified": verified_count, "refused": refused_count, "error": Value::Null }),
    )
}

fn map_proxy_error(e: &syneroym_app_host::types::proxy::ProxyError) -> SourceError {
    use syneroym_app_host::types::proxy::ProxyError;
    match e {
        ProxyError::ServiceNotFound(_) | ProxyError::DependencyNotBound(_) => SourceError::NotFound,
        ProxyError::TimedOut => SourceError::TimedOut,
        ProxyError::Callee(c) => {
            SourceError::Refused { code: c.code as i64, message: c.message.clone() }
        }
        other => SourceError::Refused { code: -32603, message: format!("{other:?}") },
    }
}

async fn record_source_error<H: AppHost>(host: &H, source: &str, error: &SourceError) {
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

async fn merge<H: AppHost>(host: &H, req: &Request) -> Response {
    let run_id = match req.params.get("run_id").and_then(Value::as_str) {
        Some(r) => r.to_string(),
        None => return Response::invalid_params("run_id is required"),
    };
    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &[]).await {
        return Response::internal_error(e);
    }
    // `search_runs` keys are `<run_id>#<record_id>` (verified) or
    // `<run_id>#refused#<record_id>` (refused); no host-side filter can
    // match a key prefix, so this scans and filters here.
    let rows = match collect_raw(host, SEARCH_RUNS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let prefix = format!("{run_id}#");
    let mut verified_rows: Vec<SearchRunRow> = Vec::new();
    let mut refused_rows: Vec<SearchRunRow> = Vec::new();
    for (id, v) in rows {
        if !id.starts_with(&prefix) {
            continue;
        }
        let Ok(row) = serde_json::from_value::<SearchRunRow>(v) else { continue };
        if row.refused {
            refused_rows.push(row);
        } else {
            verified_rows.push(row);
        }
    }

    // Per source, first: sort by (issued_at desc, listing_id asc), take at
    // most MAX_HITS_PER_SOURCE.
    let mut by_source: BTreeMap<String, Vec<SearchRunRow>> = BTreeMap::new();
    for row in verified_rows {
        by_source.entry(row.source.clone()).or_default().push(row);
    }
    for rows in by_source.values_mut() {
        rows.sort_by(|a, b| {
            b.issued_at_secs.cmp(&a.issued_at_secs).then(a.listing_id.cmp(&b.listing_id))
        });
        rows.truncate(syneroym_roym_core::directory::MAX_HITS_PER_SOURCE as usize);
    }

    // Round-robin across sources, visited in DID order.
    let mut positions: BTreeMap<String, usize> =
        by_source.keys().map(|k| (k.clone(), 0usize)).collect();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut merged: BTreeMap<String, (SearchRunRow, Vec<Value>)> = BTreeMap::new();
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
            let row = &rows[*pos];
            *pos += 1;
            advanced = true;
            seen.insert(row.listing_id.clone());
            if merged.len() as u32 >= syneroym_roym_core::directory::MAX_SEARCH_RESULTS {
                merge_truncated = true;
                break 'outer;
            }
            merged.insert(row.listing_id.clone(), (row.clone(), vec![json!({ "directory": row.source, "record_id": row.record_id, "received_at_secs": row.received_at_secs })]));
        }
        if !advanced {
            break;
        }
    }

    // A listing more than one source returned: keep the row with the
    // greatest issued_at_secs, ties by record_id ascending; union sources.
    for rows in by_source.values() {
        for row in rows {
            if let Some((kept, source_list)) = merged.get_mut(&row.listing_id)
                && kept.source != row.source
            {
                source_list.push(json!({ "directory": row.source, "record_id": row.record_id, "received_at_secs": row.received_at_secs }));
                let better = row.issued_at_secs > kept.issued_at_secs
                    || (row.issued_at_secs == kept.issued_at_secs
                        && row.record_id < kept.record_id);
                if better {
                    *kept = row.clone();
                }
            }
        }
    }

    let now = clock::now_secs();
    let hits: Vec<Value> = merged
        .into_values()
        .map(|(row, source_list)| {
            let distinct_record_ids: std::collections::BTreeSet<&str> = source_list
                .iter()
                .filter_map(|s| s.get("record_id").and_then(Value::as_str))
                .collect();
            json!({
                "listing_id": row.listing_id,
                "record_id": row.record_id,
                "issuer": row.issuer,
                "title": row.title,
                "summary": row.summary,
                "categories": row.categories,
                "conversation_address": row.conversation_address,
                "status": row.status,
                "verified": true,
                "revocation_status": row.revocation_status,
                "credential": row.credential,
                "age_secs": now.saturating_sub(row.issued_at_secs),
                "sources": source_list,
                "versions_differ": distinct_record_ids.len() > 1,
            })
        })
        .collect();

    // Refused hits, grouped by listing_id, sorted by (directory,
    // listing_id) -- never by anything the forger controls.
    let mut refused_by_listing: BTreeMap<String, Vec<SearchRunRow>> = BTreeMap::new();
    for row in refused_rows {
        refused_by_listing.entry(row.listing_id.clone()).or_default().push(row);
    }
    let mut refused_truncated = false;
    let mut refused: Vec<Value> = Vec::new();
    let mut refused_entries: Vec<(String, Vec<SearchRunRow>)> =
        refused_by_listing.into_iter().collect();
    refused_entries.sort_by(|a, b| {
        let a_dir = a.1.first().map(|r| r.source.as_str()).unwrap_or_default();
        let b_dir = b.1.first().map(|r| r.source.as_str()).unwrap_or_default();
        a_dir.cmp(b_dir).then(a.0.cmp(&b.0))
    });
    for (listing_id, rows) in refused_entries {
        if refused.len() as u32 >= MAX_REFUSED_RESULTS {
            refused_truncated = true;
            break;
        }
        let sources: Vec<Value> = rows.iter().map(|r| json!(r.source)).collect();
        refused.push(json!({
            "listing_id": listing_id,
            "reason": rows.first().and_then(|r| r.reason.clone()).unwrap_or_default(),
            "sources": sources,
        }));
    }

    Response::ok(json!({
        "hits": hits,
        "hits_truncated": merge_truncated,
        "refused": refused,
        "refused_truncated": refused_truncated,
    }))
}

async fn run_envelope<H: AppHost>(host: &H, req: &Request) -> Response {
    let run_id = match req.params.get("run_id").and_then(Value::as_str) {
        Some(r) => r,
        None => return Response::invalid_params("run_id is required"),
    };
    let record_id = match req.params.get("record_id").and_then(Value::as_str) {
        Some(r) => r,
        None => return Response::invalid_params("record_id is required"),
    };
    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &[]).await {
        return Response::internal_error(e);
    }
    let key = format!("{run_id}#{record_id}");
    match get_json::<H, SearchRunRow>(host, SEARCH_RUNS, &key).await {
        Ok(Some(row)) => Response::ok(json!({ "envelope": row.envelope })),
        Ok(None) => Response::ok(Value::Null),
        Err(e) => Response::internal_error(e),
    }
}

async fn publish_to_source<H: AppHost>(host: &H, req: &Request) -> Response {
    let source = match req.params.get("source").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return Response::invalid_params("source is required"),
    };
    let listing_id = match req.params.get("listing_id").and_then(Value::as_str) {
        Some(l) => l.to_string(),
        None => return Response::invalid_params("listing_id is required"),
    };

    let get_req =
        json!({ "method": "listing.get", "params": { "listing_id": listing_id } }).to_string();
    let raw = match host
        .call(
            CallTarget::Dependency(services::CATALOG.name.to_string()),
            services::CATALOG.interface.to_string(),
            "invoke".to_string(),
            json!([get_req]).to_string(),
            None,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => return Response::internal_error(format!("listing.get: {e:?}")),
    };
    let resp: Response = match serde_json::from_str(&raw) {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let Some(result) = resp.result.filter(|r| !r.is_null()) else {
        return Response::invalid_params("no such listing on this installation");
    };
    let envelope = match result.get("envelope").and_then(Value::as_str) {
        Some(e) => e.to_string(),
        None => return Response::internal_error("listing.get returned no envelope"),
    };

    let publish_req =
        json!({ "method": "directory.publish", "params": { "envelope": envelope } }).to_string();
    let raw = match host
        .call(
            CallTarget::Service(source.clone()),
            services::DIRECTORY.interface.to_string(),
            "invoke".to_string(),
            json!([publish_req]).to_string(),
            Some(CallOptions {
                protocol: None,
                idempotent: false,
                timeout_ms: Some(DEFAULT_SOURCE_TIMEOUT_MS),
                routing_key: None,
                idempotency_key: None,
            }),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Response::internal_error(format!("directory.publish at '{source}': {e:?}"));
        }
    };
    match serde_json::from_str::<Response>(&raw) {
        Ok(r) => r,
        Err(e) => Response::internal_error(e.to_string()),
    }
}

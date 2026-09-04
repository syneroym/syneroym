//! Catalog service application logic, target-independent.
//!
//! The provider's offer: a signed `listing` record with a stable
//! content-derived id, edited only by producing a new version that
//! `supersedes` the last, plus unsigned availability state and the
//! catalog-side publication limiter.

use std::{cmp::Reverse, collections::BTreeMap};

use serde::Deserialize;
use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::{
            CollectionSchema, IndexDefinition, IndexType, Mutation, QueryOptions, RecordWriteValue,
        },
        proxy::CallTarget,
        signing::{Principal, RecordDraft},
    },
};
use syneroym_roym_core::{
    admit,
    backup::{BUNDLE_VERSION, Bundle, BundleManifest, SECTION_AVAILABILITY, SECTION_LISTINGS},
    clock,
    envelope::{Request, Response},
    listing::{self, ListingPayload, ListingStatus},
    person::ProfilePayload,
    record::{Envelope, RECORD_LISTING, VerifyOptions, content_digest, verify_json},
    safety::{self, Admission, PublicationLimits},
    services,
    signing::{self, CertificateError},
};

/// Bumped in this slice: the service gains its first state.
pub const SCHEMA_VERSION: u32 = 2;

pub const LISTINGS: &str = "listings";
pub const LISTING_HISTORY: &str = "listing_history";
pub const AVAILABILITY: &str = "availability";
pub const PUBLICATIONS: &str = "publications";
pub const SETTINGS: &str = "settings";
pub const PUBLICATION_LIMITS_KEY: &str = "publication_limits";

const SLOT_ID_PREFIX: &str = "slot_";

pub async fn status<H: AppHost>(_host: &H) -> Result<String, String> {
    Ok(json!({
        "service": services::CATALOG.name,
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

async fn ensure_listings<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(
        host,
        LISTINGS,
        &[idx("status", IndexType::String), idx("updated_at_secs", IndexType::Numeric)],
    )
    .await
}

async fn ensure_availability<H: AppHost>(host: &H) -> Result<(), String> {
    ensure_coll(
        host,
        AVAILABILITY,
        &[idx("listing_id", IndexType::String), idx("start_secs", IndexType::Numeric)],
    )
    .await
}

/// Every row of `collection`, as `{ id, payload }` -- the shape a `Bundle`
/// section holds and `profile.export` uses.
async fn collect<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
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
                out.push(json!({ "id": r.id, "payload": parsed }));
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(out)
}

async fn load_publication_limits<H: AppHost>(host: &H) -> Result<PublicationLimits, String> {
    ensure_coll(host, SETTINGS, &[]).await?;
    let row = AppDataLayer::get(host, SETTINGS.to_string(), PUBLICATION_LIMITS_KEY.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map_err(|e| e.to_string()),
        None => Ok(PublicationLimits::default()),
    }
}

/// The pointer row `listings/<listing_id>`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ListingRow {
    envelope: String,
    record_id: String,
    listing_id: String,
    slug: String,
    status: ListingStatus,
    updated_at_secs: u64,
    version_count: u64,
}

async fn load_listing_row<H: AppHost>(
    host: &H,
    listing_id: &str,
) -> Result<Option<ListingRow>, String> {
    ensure_listings(host).await?;
    let row = AppDataLayer::get(host, LISTINGS.to_string(), listing_id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    match row {
        Some(r) => serde_json::from_slice(&r.payload).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// The person's own conversation address, read from their `profile`
/// record through the declared `catalog -> profile` dependency.
async fn address_from_profile<H: AppHost>(host: &H) -> Result<Option<String>, String> {
    let req = json!({ "method": "profile.get", "params": {} }).to_string();
    let raw = host
        .call(
            CallTarget::Dependency(services::PROFILE.name.to_string()),
            services::PROFILE.interface.to_string(),
            "invoke".to_string(),
            json!([req]).to_string(),
            None,
        )
        .await
        .map_err(|e| format!("profile.get: {e:?}"))?;
    let resp: Response = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let Some(result) = resp.result else { return Ok(None) };
    if result.is_null() {
        return Ok(None);
    }
    let env_str =
        result.get("envelope").and_then(Value::as_str).ok_or("profile row has no envelope")?;
    let env = Envelope::from_json(env_str).map_err(|e| e.to_string())?;
    let payload: ProfilePayload = serde_json::from_value(env.payload).map_err(|e| e.to_string())?;
    Ok(Some(payload.conversation_address))
}

#[derive(Debug, Deserialize)]
struct SetListingParams {
    slug: Option<String>,
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    categories: Vec<String>,
    conversation_address: Option<String>,
    status: Option<ListingStatus>,
    #[serde(default)]
    booking: Option<listing::BookingTerms>,
    #[serde(default)]
    payment: Option<listing::PaymentTerms>,
    #[serde(default)]
    product: Option<listing::ProductDetail>,
    #[serde(default)]
    service: Option<listing::ServiceDetail>,
    #[serde(default)]
    location: Option<listing::LocationTerms>,
    #[serde(default)]
    relationship: Option<listing::RelationshipTerms>,
    #[serde(default)]
    service_record: Option<listing::ServiceRecordTerms>,
}

#[derive(Debug, Deserialize)]
struct SlotInput {
    start_secs: u64,
    end_secs: u64,
    capacity: u32,
}

pub async fn invoke<H: AppHost>(host: &H, req: Request) -> Response {
    if let Some(resp) = admit::require_internal(host).await {
        return resp;
    }
    if let Some(resp) = signing::handle_certificate_verb(host, "catalog.", &req).await {
        return resp;
    }

    match req.method.as_str() {
        "listing.ping" => Response::ok(json!({ "service": services::CATALOG.name })),
        "listing.set" => set_listing(host, &req).await,
        "listing.withdraw" => withdraw_listing(host, &req).await,
        "listing.get" => get_listing(host, &req).await,
        "listing.list" => list_listings(host, &req).await,
        "listing.history" => listing_history(host, &req).await,
        "listing.verify" => verify_listing(host, &req).await,
        "listing.limits" => match load_publication_limits(host).await {
            Ok(l) => Response::ok(json!(l)),
            Err(e) => Response::internal_error(e),
        },
        "listing.set-limits" => set_limits(host, &req).await,
        "availability.set" => availability_set(host, &req).await,
        "availability.list" => availability_list(host, &req).await,
        "availability.remove" => availability_remove(host, &req).await,
        "catalog.export" => export(host).await,
        "catalog.import" => import(host, &req).await,
        other => Response::method_not_found(other),
    }
}

async fn resolve_principal_and_owner<H: AppHost>(
    host: &H,
    now: u64,
) -> Result<(Principal, String), Response> {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(CertificateError::NoOwner) => {
            return Err(Response::invalid_params("this installation has no recorded owner"));
        }
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let (principal, _master) = match signing::person_principal(host, now).await {
        Ok(res) => res,
        Err(CertificateError::NotEnrolled) => {
            return Err(Response::invalid_params("signing-not-enrolled"));
        }
        Err(CertificateError::Expired(t)) => {
            return Err(Response::invalid_params(format!("signing-certificate-expired at {t}")));
        }
        Err(CertificateError::Stale { installed_for, current }) => {
            return Err(Response::invalid_params(format!(
                "signing-certificate-stale: {installed_for} vs {current}"
            )));
        }
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    Ok((principal, owner))
}

/// The shared body of `listing.set` and `listing.withdraw`. `withdraw`
/// forces `status = withdrawn` and skips the publication limiter
/// entirely: the limiter counts versions that put an offer *out*, and a
/// provider is never rate-limited out of taking an offer down.
async fn write_version<H: AppHost>(
    host: &H,
    payload: ListingPayload,
    count_publication: bool,
    now: u64,
) -> Response {
    let (principal, owner) = match resolve_principal_and_owner(host, now).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    if let Err(e) = payload.validate() {
        return Response::invalid_params(e.to_string());
    }

    let prior = match load_listing_row(host, &payload.listing_id).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e),
    };
    let supersedes = prior.as_ref().map(|r| r.record_id.clone());
    let next_count = prior.as_ref().map(|r| r.version_count).unwrap_or(0) + 1;

    if count_publication {
        let limits = match load_publication_limits(host).await {
            Ok(l) => l,
            Err(e) => return Response::internal_error(e),
        };
        let prior_secs = match publication_secs_in_window(host, &limits, now).await {
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
                .with_data(json!({
                    "admission": "rate-limited",
                    "retry_after_secs": retry_after_secs,
                }));
            }
            // `admit_publication` never blocks -- naming the arm rather
            // than collapsing it into a catch-all.
            Admission::Blocked => {
                return Response::internal_error("admit_publication returned Blocked");
            }
        }
        // Pruned in the same pass that already reads this collection --
        // the ledger otherwise grows without bound (deferred-backlog's
        // "publications never pruned" row).
        let floor = now.saturating_sub(limits.window_secs);
        if let Err(e) = AppDataLayer::delete_many(
            host,
            PUBLICATIONS.to_string(),
            json!({ "at_secs": { "$lte": floor } }).to_string(),
        )
        .await
        {
            return Response::internal_error(e.to_string());
        }
    }

    let payload_json = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let draft = RecordDraft {
        version: listing::LISTING_VERSION,
        record_type: RECORD_LISTING.to_string(),
        subject: payload.listing_id.clone(),
        payload: payload_json,
        expires_at_secs: None,
        supersedes,
    };
    let envelope_json = match AppSigning::sign_record(host, draft, principal).await {
        Ok(j) => j,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let envelope = match Envelope::from_json(&envelope_json) {
        Ok(e) => e,
        Err(e) => {
            return Response::internal_error(format!(
                "the host returned an envelope this build cannot parse: {e}"
            ));
        }
    };
    if envelope.issuer != owner {
        return Response::internal_error(
            "the host signed under an issuer this service did not ask for",
        );
    }
    let record_id = match envelope.record_id() {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    if let Err(e) = ensure_listings(host).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_coll(host, LISTING_HISTORY, &[]).await {
        return Response::internal_error(e);
    }

    let row = ListingRow {
        envelope: envelope_json.clone(),
        record_id: record_id.clone(),
        listing_id: payload.listing_id.clone(),
        slug: payload.slug.clone(),
        status: payload.status,
        updated_at_secs: now,
        version_count: next_count,
    };
    let row_bytes = match serde_json::to_vec(&row) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    // The pointer first: a crash between the two writes leaves the pointer
    // on the previous valid version, and an unreferenced history row is
    // harmless -- `profile.set`'s own rule.
    if let Err(e) = AppDataLayer::put(
        host,
        LISTINGS.to_string(),
        RecordWriteValue { id: payload.listing_id.clone(), payload: row_bytes },
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }
    if let Err(e) = AppDataLayer::put(
        host,
        LISTING_HISTORY.to_string(),
        RecordWriteValue { id: record_id.clone(), payload: envelope_json.into_bytes() },
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }

    if count_publication {
        if let Err(e) = ensure_coll(host, PUBLICATIONS, &[idx("at_secs", IndexType::Numeric)]).await
        {
            return Response::internal_error(e);
        }
        // Keyed by `record_id` (unique per signed version), not by a
        // counter: two concurrent `listing.set` calls both read the same
        // `version_count`, so a `{listing_id}:{next_count}` key would let
        // the second write overwrite the first's publication row and one
        // unit of the flood budget would cover two published versions.
        let pub_id = format!("{}:{}", payload.listing_id, record_id);
        let pub_row = json!({ "listing_id": payload.listing_id, "at_secs": now });
        if let Err(e) = AppDataLayer::put(
            host,
            PUBLICATIONS.to_string(),
            RecordWriteValue {
                id: pub_id,
                payload: serde_json::to_vec(&pub_row).unwrap_or_default(),
            },
        )
        .await
        {
            return Response::internal_error(e.to_string());
        }
    }

    Response::ok(json!({
        "listing_id": payload.listing_id,
        "record_id": record_id,
        "version_count": next_count,
    }))
}

async fn publication_secs_in_window<H: AppHost>(
    host: &H,
    limits: &PublicationLimits,
    now: u64,
) -> Result<Vec<u64>, String> {
    ensure_coll(host, PUBLICATIONS, &[idx("at_secs", IndexType::Numeric)]).await?;
    let floor = now.saturating_sub(limits.window_secs);
    // Filtered at the host rather than scanned and dropped in the guest. `F4` still
    // applies -- a filter is not an indexed scan -- but it is fewer rows
    // crossing the host boundary.
    let filter = json!({ "at_secs": { "$gt": floor } });
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            PUBLICATIONS.to_string(),
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

async fn build_payload<H: AppHost>(
    host: &H,
    p: SetListingParams,
    owner: &str,
    forced_status: Option<ListingStatus>,
) -> Result<ListingPayload, Response> {
    let slug = match p.slug {
        Some(s) => s,
        None => match listing::slug_from_title(&p.title) {
            Some(s) => s,
            None => {
                return Err(Response::invalid_params(
                    "slug is required: the title has no usable characters",
                ));
            }
        },
    };
    let listing_id = match listing::derive_listing_id(owner, &slug) {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let address = match p.conversation_address {
        Some(a) => a,
        None => match address_from_profile(host).await {
            Ok(Some(a)) => a,
            Ok(None) => {
                return Err(Response::invalid_params(
                    "conversation_address is required and no profile record carries one",
                ));
            }
            Err(e) => return Err(Response::internal_error(e)),
        },
    };
    // A `set` that names no status keeps the prior version's status rather
    // than defaulting to `Active` -- editing the title of a withdrawn
    // listing must not silently republish it. `Active` is the default only
    // for a brand-new listing.
    let prior_status = load_listing_row(host, &listing_id).await.ok().flatten().map(|r| r.status);
    Ok(ListingPayload {
        listing_id,
        slug,
        title: p.title,
        summary: p.summary,
        categories: p.categories,
        conversation_address: address,
        status: forced_status.or(p.status).or(prior_status).unwrap_or(ListingStatus::Active),
        booking: p.booking,
        payment: p.payment,
        product: p.product,
        service: p.service,
        location: p.location,
        relationship: p.relationship,
        service_record: p.service_record,
    })
}

async fn set_listing<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let params: SetListingParams = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return Response::invalid_params(format!("invalid listing params: {e}")),
    };
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(CertificateError::NoOwner) => {
            return Response::invalid_params("this installation has no recorded owner");
        }
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let payload = match build_payload(host, params, &owner, None).await {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    write_version(host, payload, true, now).await
}

async fn withdraw_listing<H: AppHost>(host: &H, req: &Request) -> Response {
    let now = clock::now_secs();
    let listing_id = match req.params.get("listing_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return Response::invalid_params("listing_id is required"),
    };
    let prior = match load_listing_row(host, &listing_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return Response::invalid_params("no such listing"),
        Err(e) => return Response::internal_error(e),
    };
    let mut payload: ListingPayload = match serde_json::from_str::<Envelope>(&prior.envelope)
        .ok()
        .and_then(|e| serde_json::from_value(e.payload).ok())
    {
        Some(p) => p,
        None => return Response::internal_error("stored listing envelope is unreadable"),
    };
    payload.status = ListingStatus::Withdrawn;
    write_version(host, payload, false, now).await
}

async fn get_listing<H: AppHost>(host: &H, req: &Request) -> Response {
    let listing_id = match req.params.get("listing_id").and_then(Value::as_str) {
        Some(id) => id,
        None => return Response::invalid_params("listing_id is required"),
    };
    match load_listing_row(host, listing_id).await {
        Ok(Some(r)) => Response::ok(json!({
            "envelope": r.envelope,
            "record_id": r.record_id,
            "listing_id": r.listing_id,
            "status": r.status,
            "updated_at_secs": r.updated_at_secs,
        })),
        Ok(None) => Response::ok(Value::Null),
        Err(e) => Response::internal_error(e),
    }
}

async fn list_listings<H: AppHost>(host: &H, req: &Request) -> Response {
    if let Err(e) = ensure_listings(host).await {
        return Response::internal_error(e);
    }
    let status = req.params.get("status").and_then(Value::as_str);
    let offset = req.params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = req.params.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
    // The host sieves by status when the caller asked for one,
    // rather than every row crossing the boundary to be dropped here.
    let filter = status.map(|s| json!({ "status": s }).to_string());

    let mut rows: Vec<ListingRow> = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            LISTINGS.to_string(),
            QueryOptions { filter: filter.clone(), limit: Some(500), cursor: cursor.clone() },
        )
        .await
        {
            Ok(p) => p,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        for r in page.records {
            if let Ok(row) = serde_json::from_slice::<ListingRow>(&r.payload) {
                rows.push(row);
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    rows.sort_by_key(|r| Reverse(r.updated_at_secs));
    let out: Vec<Value> = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|r| {
            // The title lives inside the signed envelope; a listing row that
            // could not be parsed still lists, with an empty title.
            let title = serde_json::from_str::<Envelope>(&r.envelope)
                .ok()
                .and_then(|e| e.payload.get("title").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            json!({
                "listing_id": r.listing_id,
                "slug": r.slug,
                "title": title,
                "status": r.status,
                "record_id": r.record_id,
                "updated_at_secs": r.updated_at_secs,
                "version_count": r.version_count,
            })
        })
        .collect();
    Response::ok(json!({ "listings": out }))
}

async fn listing_history<H: AppHost>(host: &H, req: &Request) -> Response {
    let listing_id = match req.params.get("listing_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return Response::invalid_params("listing_id is required"),
    };
    if let Err(e) = ensure_coll(host, LISTING_HISTORY, &[]).await {
        return Response::internal_error(e);
    }
    // The history rows are keyed by record_id, so gather every envelope
    // whose payload names this listing_id and order them oldest-first by
    // `issued_at_secs`. Two versions minted in the same second keep store
    // order; the `supersedes` chain in each payload is the exact order if
    // a consumer needs it.
    //
    // Filtered at the host on the payload's own `listing_id`
    // field, rather than parsing every envelope in the collection to find
    // the ones that match (`F4`: still a scan, but far fewer rows cross
    // the host boundary).
    let history_filter = json!({ "payload.listing_id": listing_id }).to_string();
    let mut envelopes: Vec<(u64, String)> = Vec::new();
    let mut cursor = None;
    loop {
        let page = match AppDataLayer::query(
            host,
            LISTING_HISTORY.to_string(),
            QueryOptions {
                filter: Some(history_filter.clone()),
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
            let env_str = String::from_utf8_lossy(&r.payload).into_owned();
            if let Ok(env) = Envelope::from_json(&env_str) {
                let matches = env
                    .payload
                    .get("listing_id")
                    .and_then(Value::as_str)
                    .map(|id| id == listing_id)
                    .unwrap_or(false);
                if matches {
                    envelopes.push((env.issued_at_secs, env_str));
                }
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    envelopes.sort_by_key(|(t, _)| *t);
    let out: Vec<Value> = envelopes.into_iter().map(|(_, e)| Value::String(e)).collect();
    Response::ok(json!({ "history": out }))
}

/// A thin wrapper over `roym_core::listing::verify_envelope` -- the one
/// verification body this handler and the directory client both call, so a
/// stranger's listing is never verified twice by two copies of the same
/// logic that could quietly disagree.
async fn verify_listing<H: AppHost>(host: &H, req: &Request) -> Response {
    let _ = host;
    let now = clock::now_secs();
    let env_val = match req.params.get("envelope") {
        Some(v) => v,
        None => return Response::invalid_params("envelope is required"),
    };
    let env_str = match env_val {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let verdict = listing::verify_envelope(&env_str, now);
    Response::ok(json!(verdict))
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
    if let Err(e) = ensure_coll(host, SETTINGS, &[]).await {
        return Response::internal_error(e);
    }
    let payload = match serde_json::to_vec(&limits) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if let Err(e) = AppDataLayer::put(
        host,
        SETTINGS.to_string(),
        RecordWriteValue { id: PUBLICATION_LIMITS_KEY.to_string(), payload },
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }
    Response::ok(json!(limits))
}

fn slot_id(listing_id: &str, start_secs: u64, end_secs: u64) -> Result<String, String> {
    content_digest(
        SLOT_ID_PREFIX,
        &json!({ "listing_id": listing_id, "start_secs": start_secs, "end_secs": end_secs }),
    )
    .map_err(|e| e.to_string())
}

async fn availability_set<H: AppHost>(host: &H, req: &Request) -> Response {
    let listing_id = match req.params.get("listing_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return Response::invalid_params("listing_id is required"),
    };
    let slots: Vec<SlotInput> = match req.params.get("slots").cloned() {
        Some(v) => match serde_json::from_value(v) {
            Ok(s) => s,
            Err(e) => return Response::invalid_params(format!("invalid slots: {e}")),
        },
        None => return Response::invalid_params("slots is required"),
    };
    if let Err(e) = ensure_availability(host).await {
        return Response::internal_error(e);
    }
    let mut ids = Vec::new();
    for s in slots {
        if s.end_secs <= s.start_secs {
            return Response::invalid_params("slot end_secs must be after start_secs");
        }
        let id = match slot_id(&listing_id, s.start_secs, s.end_secs) {
            Ok(id) => id,
            Err(e) => return Response::internal_error(e),
        };
        let row = json!({
            "slot_id": id,
            "listing_id": listing_id,
            "start_secs": s.start_secs,
            "end_secs": s.end_secs,
            "capacity": s.capacity,
        });
        if let Err(e) = AppDataLayer::put(
            host,
            AVAILABILITY.to_string(),
            RecordWriteValue {
                id: id.clone(),
                payload: serde_json::to_vec(&row).unwrap_or_default(),
            },
        )
        .await
        {
            return Response::internal_error(e.to_string());
        }
        ids.push(id);
    }
    Response::ok(json!({ "listing_id": listing_id, "slot_ids": ids }))
}

async fn availability_list<H: AppHost>(host: &H, req: &Request) -> Response {
    let listing_id = match req.params.get("listing_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return Response::invalid_params("listing_id is required"),
    };
    let from = req.params.get("from_secs").and_then(Value::as_u64);
    let to = req.params.get("to_secs").and_then(Value::as_u64);
    if let Err(e) = ensure_availability(host).await {
        return Response::internal_error(e);
    }
    let rows = match collect(host, AVAILABILITY).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let mut slots: Vec<Value> = rows
        .into_iter()
        .filter_map(|r| r.get("payload").cloned())
        .filter(|p| p.get("listing_id").and_then(Value::as_str) == Some(listing_id.as_str()))
        .filter(|p| {
            let start = p.get("start_secs").and_then(Value::as_u64).unwrap_or(0);
            from.map(|f| start >= f).unwrap_or(true) && to.map(|t| start <= t).unwrap_or(true)
        })
        .collect();
    slots.sort_by_key(|p| p.get("start_secs").and_then(Value::as_u64).unwrap_or(0));
    Response::ok(json!({ "slots": slots }))
}

async fn availability_remove<H: AppHost>(host: &H, req: &Request) -> Response {
    let slot_id = match req.params.get("slot_id").and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => return Response::invalid_params("slot_id is required"),
    };
    if let Err(e) = ensure_availability(host).await {
        return Response::internal_error(e);
    }
    let existed = AppDataLayer::get(host, AVAILABILITY.to_string(), slot_id.clone())
        .await
        .map(|o| o.is_some())
        .unwrap_or(false);
    if existed && let Err(e) = AppDataLayer::delete(host, AVAILABILITY.to_string(), slot_id).await {
        return Response::internal_error(e.to_string());
    }
    Response::ok(json!({ "removed": existed }))
}

async fn export<H: AppHost>(host: &H) -> Response {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let now = clock::now_secs();
    if let Err(e) = ensure_listings(host).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_availability(host).await {
        return Response::internal_error(e);
    }
    let listings = match collect(host, LISTINGS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let availability = match collect(host, AVAILABILITY).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let sections = BTreeMap::from([
        (SECTION_LISTINGS.to_string(), listings),
        (SECTION_AVAILABILITY.to_string(), availability),
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
            subject_did: owner,
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
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if bundle.manifest.subject_did != owner {
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

    let now = clock::now_secs();
    let mut prepared: Vec<(&'static str, Vec<Mutation>)> = Vec::new();
    for (name, records) in &bundle.sections {
        let collection = match name.as_str() {
            SECTION_LISTINGS => LISTINGS,
            SECTION_AVAILABILITY => AVAILABILITY,
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
            if name == SECTION_LISTINGS {
                let env_str = match payload_val.get("envelope").and_then(Value::as_str) {
                    Some(s) => s,
                    None => return Response::invalid_params("listing record missing envelope"),
                };
                let verified = match verify_json(env_str, &VerifyOptions::new(now)) {
                    Ok(v) => v,
                    Err(e) => {
                        return Response::invalid_params(format!(
                            "listing record '{id}' failed verification: {e}"
                        ));
                    }
                };
                if verified.record_type != RECORD_LISTING
                    || verified.version != listing::LISTING_VERSION
                {
                    return Response::invalid_params("record is not a listing");
                }
                if verified.issuer != owner {
                    return Response::invalid_params(format!(
                        "listing record '{id}' was signed by '{}', not this node's owner",
                        verified.issuer
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
        // Re-ensure the collection; indexes are added lazily on first
        // regular write, so an import needs none of its own.
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

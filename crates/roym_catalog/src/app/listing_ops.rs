use super::*;

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
        let limits = match limits::load_publication_limits(host).await {
            Ok(l) => l,
            Err(e) => return Response::internal_error(e),
        };
        let prior_secs = match limits::publication_secs_in_window(host, &limits, now).await {
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

pub(crate) async fn set_listing<H: AppHost>(host: &H, req: &Request) -> Response {
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

pub(crate) async fn withdraw_listing<H: AppHost>(host: &H, req: &Request) -> Response {
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

pub(crate) async fn get_listing<H: AppHost>(host: &H, req: &Request) -> Response {
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

pub(crate) async fn list_listings<H: AppHost>(host: &H, req: &Request) -> Response {
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

pub(crate) async fn listing_history<H: AppHost>(host: &H, req: &Request) -> Response {
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
    // Filtered at the host on the payload's own `listing_id` field,
    // rather than parsing every envelope in the collection to find the
    // ones that match -- still a scan (no expression index on a JSON
    // path), but far fewer rows cross the host boundary.
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
/// logic that could quietly disagree. The response is the whole
/// `ListingVerdict` (record id, revocation status, issued-at, supersedes
/// and the full payload), not a five-field subset; an underivable
/// listing_id comes back as `verified: false`, not an internal error.
pub(crate) async fn verify_listing<H: AppHost>(host: &H, req: &Request) -> Response {
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

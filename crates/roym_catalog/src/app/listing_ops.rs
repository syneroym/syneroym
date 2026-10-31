use std::cmp::Reverse;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost, AppSigning,
    types::{
        data_layer::{IndexType, RecordWriteValue},
        signing::{Principal, RecordDraft},
    },
};
use syneroym_roym_core::{
    clock,
    envelope::Response,
    listing::{self, ListingPayload, ListingStatus, ListingVerdict},
    method::{
        EnvelopeHistoryOutput, Reply, SiblingError, VerifyInput, call_sibling,
        catalog::{ListingGetOutput, ListingKeyInput},
        profile::{ProfileGetInput, ProfileRecord},
        verify_with,
    },
    paging,
    person::ProfilePayload,
    record::{Envelope, RECORD_LISTING},
    safety::{self, Admission},
    services,
    signing::{self, CertificateError},
};

use super::{LISTING_HISTORY, LISTINGS, PUBLICATIONS, ensure_coll, ensure_listings, idx, limits};

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
    let profile_res = call_sibling::<_, ProfileGetInput, Option<ProfileRecord>>(
        host,
        services::PROFILE,
        "profile.get",
        &ProfileGetInput::default(),
    )
    .await;
    let maybe_record = match profile_res {
        Ok(rec) => rec,
        Err(SiblingError::Refused(_)) => return Ok(None),
        Err(SiblingError::Transport(e) | SiblingError::BadReply(e)) => return Err(e),
    };
    let Some(record) = maybe_record else { return Ok(None) };
    let env = Envelope::from_json(&record.envelope).map_err(|e| e.to_string())?;
    let payload: ProfilePayload = serde_json::from_value(env.payload).map_err(|e| e.to_string())?;
    Ok(Some(payload.conversation_address))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetListingInput {
    pub slug: Option<String>,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub categories: Vec<String>,
    pub conversation_address: Option<String>,
    pub status: Option<ListingStatus>,
    #[serde(default)]
    pub booking: Option<listing::BookingTerms>,
    #[serde(default)]
    pub payment: Option<listing::PaymentTerms>,
    #[serde(default)]
    pub product: Option<listing::ProductDetail>,
    #[serde(default)]
    pub service: Option<listing::ServiceDetail>,
    #[serde(default)]
    pub location: Option<listing::LocationTerms>,
    #[serde(default)]
    pub relationship: Option<listing::RelationshipTerms>,
    #[serde(default)]
    pub service_record: Option<listing::ServiceRecordTerms>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListingWriteOutput {
    pub listing_id: String,
    pub record_id: String,
    pub version_count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListingListInput {
    pub status: Option<ListingStatus>,
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListingListItem {
    pub listing_id: String,
    pub slug: String,
    pub title: String,
    pub status: ListingStatus,
    pub record_id: String,
    pub updated_at_secs: u64,
    pub version_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListingListOutput {
    pub listings: Vec<ListingListItem>,
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

async fn check_and_prune_publication_limits<H: AppHost>(
    host: &H,
    now: u64,
) -> Result<(), Response> {
    let limits = match limits::load_publication_limits(host).await {
        Ok(l) => l,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let prior_secs = match limits::publication_secs_in_window(host, &limits, now).await {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    match safety::admit_publication(&prior_secs, &limits, now) {
        Admission::Allow => {}
        Admission::RateLimited { retry_after_secs } => {
            return Err(Response::err(
                -32602,
                format!("publication rate limit reached; retry in {retry_after_secs}s"),
            )
            .with_data(json!({
                "admission": "rate-limited",
                "retry_after_secs": retry_after_secs,
            })));
        }
        // `admit_publication` never blocks -- naming the arm rather
        // than collapsing it into a catch-all.
        Admission::Blocked => {
            return Err(Response::internal_error("admit_publication returned Blocked"));
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
        return Err(Response::internal_error(e.to_string()));
    }
    Ok(())
}

async fn sign_listing_draft<H: AppHost>(
    host: &H,
    payload: &ListingPayload,
    supersedes: Option<String>,
    principal: Principal,
    owner: &str,
) -> Result<(String, String), Response> {
    let payload_json = match serde_json::to_string(payload) {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e.to_string())),
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
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let envelope = match Envelope::from_json(&envelope_json) {
        Ok(e) => e,
        Err(e) => {
            return Err(Response::internal_error(format!(
                "the host returned an envelope this build cannot parse: {e}"
            )));
        }
    };
    if envelope.issuer != owner {
        return Err(Response::internal_error(
            "the host signed under an issuer this service did not ask for",
        ));
    }
    let record_id = match envelope.record_id() {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    Ok((envelope_json, record_id))
}

async fn persist_listing_records<H: AppHost>(
    host: &H,
    payload: &ListingPayload,
    record_id: &str,
    envelope_json: String,
    next_count: u64,
    count_publication: bool,
    now: u64,
) -> Result<(), Response> {
    if let Err(e) = ensure_listings(host).await {
        return Err(Response::internal_error(e));
    }
    if let Err(e) = ensure_coll(host, LISTING_HISTORY, &[]).await {
        return Err(Response::internal_error(e));
    }

    let row = ListingRow {
        envelope: envelope_json.clone(),
        record_id: record_id.to_string(),
        listing_id: payload.listing_id.clone(),
        slug: payload.slug.clone(),
        status: payload.status,
        updated_at_secs: now,
        version_count: next_count,
    };
    let row_bytes = match serde_json::to_vec(&row) {
        Ok(b) => b,
        Err(e) => return Err(Response::internal_error(e.to_string())),
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
        return Err(Response::internal_error(e.to_string()));
    }
    if let Err(e) = AppDataLayer::put(
        host,
        LISTING_HISTORY.to_string(),
        RecordWriteValue { id: record_id.to_string(), payload: envelope_json.into_bytes() },
    )
    .await
    {
        return Err(Response::internal_error(e.to_string()));
    }

    if count_publication {
        if let Err(e) = ensure_coll(host, PUBLICATIONS, &[idx("at_secs", IndexType::Numeric)]).await
        {
            return Err(Response::internal_error(e));
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
            return Err(Response::internal_error(e.to_string()));
        }
    }

    Ok(())
}

/// The shared body of `listing.set` and `listing.withdraw`. `withdraw`
/// forces `status = withdrawn` and skips the publication limiter
/// entirely: the limiter counts versions that put an offer *out*, and a
/// listing owner is never rate-limited out of taking an offer down.
async fn write_version<H: AppHost>(
    host: &H,
    payload: ListingPayload,
    count_publication: bool,
    now: u64,
) -> Reply<ListingWriteOutput> {
    let (principal, owner) = match resolve_principal_and_owner(host, now).await {
        Ok(v) => v,
        Err(resp) => return Err(resp),
    };

    if let Err(e) = payload.validate() {
        return Err(Response::invalid_params(e.to_string()));
    }

    let prior = match load_listing_row(host, &payload.listing_id).await {
        Ok(r) => r,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let supersedes = prior.as_ref().map(|r| r.record_id.clone());
    let next_count = prior.as_ref().map(|r| r.version_count).unwrap_or(0) + 1;

    if count_publication {
        check_and_prune_publication_limits(host, now).await?;
    }

    let (envelope_json, record_id) =
        sign_listing_draft(host, &payload, supersedes, principal, &owner).await?;

    persist_listing_records(
        host,
        &payload,
        &record_id,
        envelope_json,
        next_count,
        count_publication,
        now,
    )
    .await?;

    Ok(ListingWriteOutput { listing_id: payload.listing_id, record_id, version_count: next_count })
}

async fn build_payload<H: AppHost>(
    host: &H,
    p: SetListingInput,
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

pub(crate) async fn set<H: AppHost>(host: &H, input: SetListingInput) -> Reply<ListingWriteOutput> {
    let now = clock::now_secs();
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(CertificateError::NoOwner) => {
            return Err(Response::invalid_params("this installation has no recorded owner"));
        }
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let payload = build_payload(host, input, &owner, None).await?;
    write_version(host, payload, true, now).await
}

pub(crate) async fn withdraw<H: AppHost>(
    host: &H,
    input: ListingKeyInput,
) -> Reply<ListingWriteOutput> {
    let now = clock::now_secs();
    let prior = match load_listing_row(host, &input.listing_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(Response::invalid_params("no such listing")),
        Err(e) => return Err(Response::internal_error(e)),
    };
    let mut payload: ListingPayload = match serde_json::from_str::<Envelope>(&prior.envelope)
        .ok()
        .and_then(|e| serde_json::from_value(e.payload).ok())
    {
        Some(p) => p,
        None => return Err(Response::internal_error("stored listing envelope is unreadable")),
    };
    payload.status = ListingStatus::Withdrawn;
    write_version(host, payload, false, now).await
}

pub(crate) async fn get<H: AppHost>(
    host: &H,
    input: ListingKeyInput,
) -> Reply<Option<ListingGetOutput>> {
    match load_listing_row(host, &input.listing_id).await {
        Ok(Some(r)) => Ok(Some(ListingGetOutput {
            envelope: r.envelope,
            record_id: r.record_id,
            listing_id: r.listing_id,
            status: r.status,
            updated_at_secs: r.updated_at_secs,
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(Response::internal_error(e)),
    }
}

pub(crate) async fn list<H: AppHost>(
    host: &H,
    input: ListingListInput,
) -> Reply<ListingListOutput> {
    if let Err(e) = ensure_listings(host).await {
        return Err(Response::internal_error(e));
    }
    let offset = usize::try_from(input.offset.unwrap_or(0)).unwrap_or(usize::MAX);
    let limit = usize::try_from(input.limit.unwrap_or(100)).unwrap_or(usize::MAX);
    // The host sieves by status when the caller asked for one,
    // rather than every row crossing the boundary to be dropped here.
    let filter = input.status.map(|s| json!({ "status": s }).to_string());

    let mut rows: Vec<ListingRow> = match paging::query_all(host, LISTINGS, filter).await {
        Ok(r) => r,
        Err(e) => return Err(Response::internal_error(e)),
    };
    rows.sort_by_key(|r| Reverse(r.updated_at_secs));
    let listings: Vec<ListingListItem> = rows
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
            ListingListItem {
                listing_id: r.listing_id,
                slug: r.slug,
                title,
                status: r.status,
                record_id: r.record_id,
                updated_at_secs: r.updated_at_secs,
                version_count: r.version_count,
            }
        })
        .collect();
    Ok(ListingListOutput { listings })
}

pub(crate) async fn history<H: AppHost>(
    host: &H,
    input: ListingKeyInput,
) -> Reply<EnvelopeHistoryOutput> {
    if let Err(e) = ensure_coll(host, LISTING_HISTORY, &[]).await {
        return Err(Response::internal_error(e));
    }
    // The history rows are keyed by record_id, so this gathers every
    // envelope whose payload names this listing_id, oldest first. The
    // `supersedes` chain in each payload is the exact order if a searcher
    // needs to separate two versions minted in the same second.
    let history = match paging::envelope_history(
        host,
        LISTING_HISTORY,
        "listing_id",
        &input.listing_id,
    )
    .await
    {
        Ok(h) => h,
        Err(e) => return Err(Response::internal_error(e)),
    };
    Ok(EnvelopeHistoryOutput { history })
}

pub(crate) async fn verify<H: AppHost>(_host: &H, input: VerifyInput) -> Reply<ListingVerdict> {
    verify_with(input, listing::verify_envelope)
}

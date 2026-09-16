//! Server half: publication ingestion, safety limits, retention, and deletion.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{IndexType, QueryOptions},
};
use syneroym_roym_core::{
    admit::Caller,
    clock,
    envelope::{Request, Response},
    listing::{self, ListingVerdict},
    safety::{self, Admission, PublicationLimits},
};

use super::{
    NODE_STATE, PUBLICATION_LOG, PUBLICATIONS, SEARCH_INDEX, SETTINGS, SETTINGS_KEY,
    backup::owner_did_or_node,
    collect_raw, ensure_coll, get_json, idx, put_json,
    search_ops::{build_index_rows, search_index_key},
    synorg::load_settings,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PublicationRow {
    pub(crate) envelope: String,
    pub(crate) record_id: String,
    pub(crate) listing_id: String,
    pub(crate) issuer: String,
    pub(crate) published_by: String,
    /// The envelope's own signed clock -- never the directory's
    /// `received_at_secs` -- so a later publish can tell whether an
    /// incoming envelope is actually newer.
    pub(crate) issued_at_secs: u64,
    pub(crate) received_at_secs: u64,
}

const PRUNE_MARKER_KEY: &str = "prune_marker";
/// The retention prune runs at most this often. A read verb that finds
/// the stored marker fresher than this skips the prune entirely, so an
/// anonymous caller looping `directory.info` or `directory.search`
/// cannot turn every call into a pair of unindexed delete scans.
const PRUNE_MIN_INTERVAL_SECS: u64 = 300;

pub(crate) async fn load_publication_limits<H: AppHost>(
    host: &H,
) -> Result<PublicationLimits, String> {
    match load_settings(host).await? {
        Some(s) => Ok(s.publication_limits),
        None => Ok(PublicationLimits::default()),
    }
}

/// Every `publication_log` row for `published_by` -- the identity the
/// router verified for the connection, never the envelope's own `issuer`
/// -- strictly newer than `now - window_secs`. Filtered at the host, not
/// scanned and dropped in the guest.
async fn publication_secs_in_window<H: AppHost>(
    host: &H,
    published_by: &str,
    window_secs: u64,
    now: u64,
) -> Result<Vec<u64>, String> {
    ensure_coll(
        host,
        PUBLICATION_LOG,
        &[idx("published_by", IndexType::String), idx("at_secs", IndexType::Numeric)],
    )
    .await?;
    let floor = now.saturating_sub(window_secs);
    let filter =
        json!({ "$and": [ { "published_by": published_by }, { "at_secs": { "$gt": floor } } ] });
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
pub(crate) async fn delete_search_index_for<H: AppHost>(
    host: &H,
    listing_id: &str,
) -> Result<(), String> {
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

pub(crate) async fn publish<H: AppHost>(host: &H, req: &Request, caller: Caller) -> Response {
    // A local dispatch (this node's own owner, through the Hub or
    // `roymctl`, or a same-node `directory.publish-to-source` loopback)
    // arrives `Caller::Internal` -- `admit()` short-circuits to it
    // regardless of the wire table, by design (a local dispatch is
    // trusted for where it came from). It is a real, supported path, not
    // a state that "never happens": it publishes under this
    // installation's own recorded owner, never from a caller-supplied
    // value. `Caller::Anonymous` cannot reach a `VerifiedOnly` method
    // from the wire (`admit()` refuses it before this handler runs); the
    // arm exists only so the match stays exhaustive against a future
    // change to that contract.
    let published_by = match caller {
        Caller::Verified(did) => did,
        Caller::Internal => owner_did_or_node(host).await,
        Caller::Anonymous => {
            return Response::internal_error(
                "directory.publish reached with an anonymous caller, which admit() must never \
                 allow",
            );
        }
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
    let (Some(payload), Some(record_id), Some(issuer), Some(issued_at_secs)) =
        (verdict.payload, verdict.record_id, verdict.issuer, verdict.issued_at_secs)
    else {
        return Response::internal_error(
            "a verified verdict carried no payload, record id or issuer",
        );
    };
    let supersedes = verdict.supersedes;

    match payload.status {
        listing::ListingStatus::Draft => {
            return Response::invalid_params("a draft listing may not be published");
        }
        listing::ListingStatus::Active | listing::ListingStatus::Withdrawn => {}
    }
    if payload.conversation_address.trim().is_empty() {
        return Response::internal_error("a verified listing had an empty conversation_address");
    }

    // A directory that has never declared itself (no `settings` row) is
    // not a SynOrg -- `directory.info` already answers `null` for it, and
    // `directory.publish` must refuse rather than silently accept a
    // stranger's bytes onto a disk with no stated retention policy to
    // bound them.
    let settings = match load_settings(host).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Response::invalid_params("this installation runs no SynOrg yet");
        }
        Err(e) => return Response::internal_error(e),
    };

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

    // Freshness: refuse an envelope that is neither strictly newer than,
    // nor a declared edit of, whatever this directory already holds for
    // the listing. Without this, replaying an old signed envelope -- of
    // any status, withdrawal included -- silently rewrites or deletes a
    // provider's current, live listing. Strict `issued_at_secs` alone is
    // not enough: the signing clock's resolution is one second, so a
    // second, legitimate edit issued in the same second as the one it
    // replaces would tie on timestamp -- `supersedes` naming the stored
    // `record_id` is what tells the two cases apart.
    let existing_row = match load_publication_for_listing(host, &payload.listing_id).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    if let Some(existing) = &existing_row
        && existing.record_id != record_id
        && issued_at_secs <= existing.issued_at_secs
        && supersedes.as_deref() != Some(existing.record_id.as_str())
    {
        return Response::invalid_params(
            "a newer or equal version of this listing is already published here",
        );
    }

    // Withdrawal: remove the stored publication and its index rows, consume
    // no budget.
    if matches!(payload.status, listing::ListingStatus::Withdrawn) {
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

    let limits = settings.publication_limits;
    // Keyed on `published_by` -- the identity the router verified for
    // this connection -- never on the envelope's own `issuer`. An issuer
    // key is self-minted and rotatable by whoever holds it, and the
    // envelope's bytes are served back verbatim by `directory.search`, so
    // keying on `issuer` would let any caller either mint a fresh budget
    // by rotating keys, or exhaust a stranger's budget by replaying a
    // signed envelope that names them. `published_by` is stable per
    // connection (a person's own owner DID locally, an instance DID over
    // the wire) and is exactly the party a rate limit is supposed to
    // bind.
    let prior_secs =
        match publication_secs_in_window(host, &published_by, limits.window_secs, now).await {
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

    // The ledger row is written immediately on admission, not after the
    // several other awaited writes below: the read (above) and this
    // write are still two separate host calls, not one atomic operation
    // -- the data layer offers no compare-and-swap this call could use
    // instead -- but writing right away narrows the window a second,
    // concurrent `publish` could race through to the smallest span
    // available rather than the whole rest of this function.
    let log_key = format!("{published_by}:{now}:{record_id}");
    if let Err(e) = put_json(
        host,
        PUBLICATION_LOG,
        &log_key,
        &json!({ "published_by": published_by, "at_secs": now }),
    )
    .await
    {
        return Response::internal_error(e);
    }

    // Prune the limiter ledger and, per the SynOrg's own retention policy,
    // publications and their index rows past their retention window --
    // in the one pass that already touches this data. Unconditional here
    // (owner-gated, already writing), unlike the rate-gated read paths.
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
    if let Err(e) = prune_expired_publications_now(host, settings.retention_secs, now).await {
        return Response::internal_error(e);
    }
    // Record this prune so a read verb in the next few minutes skips its
    // own -- the marker means "last time any path pruned", not "last read".
    let _ = ensure_coll(host, NODE_STATE, &[]).await;
    let _ = put_json(host, NODE_STATE, PRUNE_MARKER_KEY, &json!({ "at_secs": now })).await;

    // Replace the prior version, new row written before the old is
    // deleted: a crash between the two steps then leaves both the old and
    // the new publication briefly present (a later publish or `reindex`
    // reconciles), never neither -- losing the row outright would also
    // have spent one of the provider's daily publications on nothing.
    let pub_row = PublicationRow {
        envelope: envelope.clone(),
        record_id: record_id.clone(),
        listing_id: payload.listing_id.clone(),
        issuer: issuer.clone(),
        published_by: published_by.clone(),
        issued_at_secs,
        received_at_secs: now,
    };
    if let Err(e) = put_json(host, PUBLICATIONS, &record_id, &pub_row).await {
        return Response::internal_error(e);
    }
    if let Err(e) = AppDataLayer::delete_many(
        host,
        PUBLICATIONS.to_string(),
        json!({ "$and": [
            { "listing_id": payload.listing_id },
            { "record_id": { "$ne": record_id } },
        ] })
        .to_string(),
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }

    // Index rows deleted before the rebuild, so a republish with fewer
    // areas never leaves an orphaned row behind.
    if let Err(e) = delete_search_index_for(host, &payload.listing_id).await {
        return Response::internal_error(e);
    }
    for row in build_index_rows(&payload, &record_id, &issuer, issued_at_secs, now) {
        let key = search_index_key(&row.listing_id, row.area_index);
        if let Err(e) = put_json(host, SEARCH_INDEX, &key, &row).await {
            return Response::internal_error(e);
        }
    }

    Response::ok(json!({ "listing_id": payload.listing_id, "record_id": record_id }))
}

/// Deletes publications and their index rows past the SynOrg's stated
/// retention window -- but at most once per `PRUNE_MIN_INTERVAL_SECS`,
/// gated by a `node_state` marker, so it is cheap to call on every read
/// path (`info`, `publications`, `search`) including the
/// anonymous-reachable ones and a quiet directory still ages its rows
/// out. `publish` prunes unconditionally instead: that path is
/// owner-gated and already writing.
pub(crate) async fn prune_expired_publications<H: AppHost>(
    host: &H,
    retention_secs: u64,
) -> Result<(), String> {
    let now = clock::now_secs();
    ensure_coll(host, NODE_STATE, &[]).await?;
    let last_at = get_json::<H, Value>(host, NODE_STATE, PRUNE_MARKER_KEY)
        .await?
        .as_ref()
        .and_then(|v| v.get("at_secs"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if now.saturating_sub(last_at) < PRUNE_MIN_INTERVAL_SECS {
        return Ok(());
    }
    prune_expired_publications_now(host, retention_secs, now).await?;
    put_json(host, NODE_STATE, PRUNE_MARKER_KEY, &json!({ "at_secs": now })).await
}

/// The prune itself, with no rate gate -- the publish path calls this
/// directly in the pass that already touches these collections.
pub(crate) async fn prune_expired_publications_now<H: AppHost>(
    host: &H,
    retention_secs: u64,
    now: u64,
) -> Result<(), String> {
    let floor = now.saturating_sub(retention_secs);
    ensure_coll(host, PUBLICATIONS, &[]).await?;
    ensure_coll(host, SEARCH_INDEX, &[]).await?;
    AppDataLayer::delete_many(
        host,
        PUBLICATIONS.to_string(),
        json!({ "received_at_secs": { "$lte": floor } }).to_string(),
    )
    .await
    .map_err(|e| e.to_string())?;
    AppDataLayer::delete_many(
        host,
        SEARCH_INDEX.to_string(),
        json!({ "received_at_secs": { "$lte": floor } }).to_string(),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
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

pub(crate) async fn unpublish<H: AppHost>(host: &H, req: &Request) -> Response {
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

pub(crate) async fn publications<H: AppHost>(host: &H) -> Response {
    if let Err(e) = ensure_coll(host, PUBLICATIONS, &[]).await {
        return Response::internal_error(e);
    }
    if let Ok(Some(settings)) = load_settings(host).await {
        let _ = prune_expired_publications(host, settings.retention_secs).await;
    }
    match collect_raw(host, PUBLICATIONS).await {
        Ok(rows) => Response::ok(
            json!({ "publications": rows.into_iter().map(|(_, v)| v).collect::<Vec<_>>() }),
        ),
        Err(e) => Response::internal_error(e),
    }
}

pub(crate) async fn set_limits<H: AppHost>(host: &H, req: &Request) -> Response {
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

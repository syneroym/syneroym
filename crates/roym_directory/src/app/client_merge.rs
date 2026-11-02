//! Client half: merging fanned-out search results into one page.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{clock, envelope::Response, method::Reply, paging};
use syneroym_roym_federated_query::{MergedHit, RefusedHit, Row, merge_rows};

use super::{SEARCH_RUNS, client_query::SearchRunRow, ensure_coll, search_runs_indexes};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunInput {
    pub run_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunEnvelopeInput {
    pub run_id: String,
    pub record_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RunEnvelopeOutput {
    pub envelope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MergeOutput {
    pub hits: Vec<MergedHit>,
    pub hits_truncated: bool,
    pub refused: Vec<RefusedHit>,
    pub refused_truncated: bool,
}

pub(crate) async fn merge<H: AppHost>(host: &H, input: RunInput) -> Reply<MergeOutput> {
    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &search_runs_indexes()).await {
        return Err(Response::internal_error(e));
    }
    let (verified_rows, refused_rows) = match load_run_rows(host, &input.run_id).await {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };

    let verified = verified_rows.into_iter().map(to_row).collect();
    let refused = refused_rows.into_iter().map(to_row).collect();
    let now = clock::now_secs();
    let merged = merge_rows(verified, refused, now);

    Ok(MergeOutput {
        hits: merged.hits,
        hits_truncated: merged.hits_truncated,
        refused: merged.refused,
        refused_truncated: merged.refused_truncated,
    })
}

fn to_row(row: SearchRunRow) -> Row {
    Row {
        listing_id: row.listing_id,
        record_id: row.record_id,
        source: row.source,
        issuer: row.issuer,
        title: row.title,
        summary: row.summary,
        categories: row.categories,
        conversation_address: row.conversation_address,
        status: row.status,
        reason: row.reason,
        revocation_status: row.revocation_status,
        membership: row.membership,
        issued_at_secs: row.issued_at_secs,
        received_at_secs: row.received_at_secs,
    }
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
    let rows: Vec<SearchRunRow> =
        paging::query_all(host, SEARCH_RUNS, Some(json!({ "run_id": run_id }).to_string())).await?;
    let mut verified_rows: Vec<SearchRunRow> = Vec::new();
    let mut refused_rows: Vec<SearchRunRow> = Vec::new();
    for row in rows {
        if row.refused {
            refused_rows.push(row);
        } else {
            verified_rows.push(row);
        }
    }
    Ok((verified_rows, refused_rows))
}

pub(crate) async fn run_envelope<H: AppHost>(
    host: &H,
    input: RunEnvelopeInput,
) -> Reply<Option<RunEnvelopeOutput>> {
    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &search_runs_indexes()).await {
        return Err(Response::internal_error(e));
    }
    // Filtered on the `run_id` field at the host -- one run's rows, not
    // the whole collection. Only a *verified* row is accepted: its
    // `record_id` is content-derived from the envelope, so any surviving
    // verified row for `record_id` carries the identical signed bytes. A
    // refused row's `record_id` is whatever the source claimed,
    // unverified, so a hostile source could set one to collide with a
    // genuine record -- excluded here rather than trusted to sort last.
    let rows: Vec<SearchRunRow> = match paging::query_all(
        host,
        SEARCH_RUNS,
        Some(json!({ "run_id": input.run_id }).to_string()),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    for row in rows {
        if !row.refused && row.record_id == input.record_id {
            return Ok(Some(RunEnvelopeOutput { envelope: row.envelope }));
        }
    }
    Ok(None)
}

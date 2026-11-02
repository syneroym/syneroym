//! Client half: federated run lifecycle, hit envelope storage, and source
//! status updates.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_app_host::{AppDataLayer, AppHost};
use syneroym_roym_core::{
    clock,
    directory::{RUN_RETENTION_SECS, SourceError},
    envelope::Response,
    method::Reply,
    paging,
};
use syneroym_roym_federated_query::{HeldEvidence, SourceReport};

use super::{
    RUNS, SEARCH_RUNS, SOURCES, client_sources::SourceRow, ensure_coll, get_json, held, put_json,
    search_runs_indexes,
};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunRow {
    at_secs: u64,
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
pub(crate) struct RunEnvelopeRow {
    pub(crate) run_id: String,
    pub(crate) record_id: String,
    pub(crate) envelope: String,
    pub(crate) at_secs: u64,
    #[serde(default)]
    pub(crate) refused: bool,
}

pub(crate) async fn begin_run<H: AppHost>(host: &H) -> Reply<(String, u64)> {
    if let Err(e) = ensure_coll(host, RUNS, &[]).await {
        return Err(Response::internal_error(e));
    }
    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &search_runs_indexes()).await {
        return Err(Response::internal_error(e));
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
        return Err(Response::internal_error(e.to_string()));
    }

    if let Err(e) = AppDataLayer::delete_many(
        host,
        SEARCH_RUNS.to_string(),
        json!({ "at_secs": { "$lte": run_floor } }).to_string(),
    )
    .await
    {
        return Err(Response::internal_error(e.to_string()));
    }

    let existing_runs = match paging::count(host, RUNS, None).await {
        Ok(n) => n,
        Err(e) => return Err(Response::internal_error(e)),
    };
    let run_id = format!("run_{now}_{existing_runs}");
    let run = RunRow { at_secs: now };
    if let Err(e) = put_json(host, RUNS, &run_id, &run).await {
        return Err(Response::internal_error(e));
    }

    Ok((run_id, now))
}

pub(crate) fn errors_to_record(outcomes: &[SourceReport]) -> Vec<(&str, &SourceError)> {
    outcomes
        .iter()
        .filter_map(|r| match &r.error {
            Some(SourceError::NotStarted) | None => None,
            Some(err) => Some((r.source.as_str(), err)),
        })
        .collect()
}

pub(crate) async fn persist_find_results<H: AppHost>(
    host: &H,
    run_id: &str,
    now: u64,
    reports: &[SourceReport],
    evidence: &[HeldEvidence],
    page_envelopes: &[(String, String)],
) {
    for report in reports {
        if report.error.is_none() {
            mark_source_ok(host, &report.source, now).await;
        }
    }

    for (source, error) in errors_to_record(reports) {
        record_source_error(host, source, error).await;
    }

    let mut last_by_key = std::collections::BTreeMap::new();
    for item in evidence {
        last_by_key.insert((item.source.as_str(), item.member_did.as_str()), item);
    }
    for item in last_by_key.values() {
        let _ = held::remember(
            host,
            &item.source,
            &item.member_did,
            item.pinned_issuer.as_deref(),
            &item.evidence,
            now,
        )
        .await;
    }

    for (record_id, envelope) in page_envelopes {
        let row = RunEnvelopeRow {
            run_id: run_id.to_string(),
            record_id: record_id.clone(),
            envelope: envelope.clone(),
            at_secs: now,
            refused: false,
        };
        let key = format!("{run_id}#{record_id}");
        let _ = put_json(host, SEARCH_RUNS, &key, &row).await;
    }
}

pub(crate) async fn run_envelope<H: AppHost>(
    host: &H,
    input: RunEnvelopeInput,
) -> Reply<Option<RunEnvelopeOutput>> {
    if let Err(e) = ensure_coll(host, SEARCH_RUNS, &search_runs_indexes()).await {
        return Err(Response::internal_error(e));
    }
    let rows: Vec<RunEnvelopeRow> = match paging::query_all(
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

async fn load_source_row_or_default<H: AppHost>(host: &H, source: &str, now: u64) -> SourceRow {
    get_json(host, SOURCES, source).await.ok().flatten().unwrap_or(SourceRow {
        did: source.to_string(),
        label: String::new(),
        added_at_secs: now,
        last_ok_secs: None,
        last_error: None,
        issuer_did: None,
    })
}

pub(crate) async fn mark_source_ok<H: AppHost>(host: &H, source: &str, now: u64) {
    let mut row = load_source_row_or_default(host, source, now).await;
    row.last_ok_secs = Some(now);
    row.last_error = None;
    let _ = put_json(host, SOURCES, source, &row).await;
}

pub(crate) async fn record_source_error<H: AppHost>(host: &H, source: &str, error: &SourceError) {
    let now = clock::now_secs();
    let mut row = load_source_row_or_default(host, source, now).await;
    row.last_error = Some(error.clone());
    let _ = put_json(host, SOURCES, source, &row).await;
}

//! Client half: federated find across registered directory sources.

use federated_query::{Page, Source};
use serde::{Deserialize, Serialize};
use syneroym_app_host::AppHost;
use syneroym_roym_core::{directory::SearchQuery, envelope::Response, method::Reply, paging};
use syneroym_roym_federated_query as federated_query;

use super::{
    SOURCES,
    client_runs::{begin_run, persist_find_results},
    client_sources::SourceRow,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindInput {
    #[serde(default)]
    pub query: SearchQuery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FindOutput {
    pub run_id: String,
    #[serde(flatten)]
    pub page: Page,
}

pub(crate) async fn find<H: AppHost>(host: &H, input: FindInput) -> Reply<FindOutput> {
    let (run_id, now) = begin_run(host).await?;

    if let Err(e) = super::ensure_coll(host, SOURCES, &[]).await {
        return Err(Response::internal_error(e));
    }

    let mut source_rows: Vec<SourceRow> = match paging::query_all(host, SOURCES, None).await {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    source_rows.sort_by(|a, b| a.did.cmp(&b.did));

    let sources: Vec<Source> = source_rows
        .into_iter()
        .map(|r| Source { did: r.did, pinned_issuer: r.issuer_did })
        .collect();

    let gathered = match federated_query::find(host, &sources, &input.query, now).await {
        Ok(g) => g,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    persist_find_results(
        host,
        &run_id,
        now,
        &gathered.page.outcomes,
        &gathered.evidence,
        &gathered.page_envelopes,
    )
    .await;

    Ok(FindOutput { run_id, page: gathered.page })
}

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
//! Federated query library: fan-out, verification, and result merging across
//! sources.

mod gather;
mod merge;
mod verify;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

pub use gather::{MAX_SEARCH_REPLY_BYTES, Source};
pub use merge::{HitSource, MergedHit, MergedRows, RefusedHit, Row, merge_rows};
use serde::{Deserialize, Serialize};
use syneroym_app_host::AppProxy;
use syneroym_roym_core::directory::{MAX_SOURCES, SearchQuery, SourceError};
use thiserror::Error;
pub use verify::HeldEvidence;

/// Errors returned by [`find`]: `Err` only when `sources` is longer than
/// `MAX_SOURCES`.
#[derive(Debug, Error)]
pub enum FindError {
    #[error("too many sources: requested {count}, maximum is {MAX_SOURCES}")]
    TooManySources { count: usize },
}

/// Per-source query outcome and hit verification counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceReport {
    pub source: String,
    pub verified: u32,
    pub refused: u32,
    pub truncated: bool,
    pub error: Option<SourceError>,
}

/// Merged search results, refusal list, and per-source outcome reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub outcomes: Vec<SourceReport>,
    pub hits: Vec<MergedHit>,
    pub hits_truncated: bool,
    pub refused: Vec<RefusedHit>,
    pub refused_truncated: bool,
}

/// Find output including the merged page, envelopes for records on the page
/// (`page_envelopes`), and `evidence` to remember.
#[derive(Debug, Clone, PartialEq)]
pub struct Gathered {
    pub page: Page,
    pub page_envelopes: Vec<(String, String)>,
    pub evidence: Vec<HeldEvidence>,
}

/// Run federated search fan-out across sources, verifying envelopes, evaluating
/// membership, and merging results into a page.
pub async fn find<P: AppProxy>(
    proxy: &P,
    sources: &[Source],
    query: &SearchQuery,
    now_secs: u64,
) -> Result<Gathered, FindError> {
    if sources.len() > MAX_SOURCES {
        return Err(FindError::TooManySources { count: sources.len() });
    }
    if sources.is_empty() {
        return Ok(Gathered {
            page: Page {
                outcomes: Vec::new(),
                hits: Vec::new(),
                hits_truncated: false,
                refused: Vec::new(),
                refused_truncated: false,
            },
            page_envelopes: Vec::new(),
            evidence: Vec::new(),
        });
    }

    let requests = gather::build_requests(sources, query);
    let outcomes = gather::execute_and_parse(proxy, sources, requests).await;
    let verified = verify::verify_outcomes(outcomes, now_secs);

    let merged = merge_rows(verified.verified_rows, verified.refused_rows, now_secs);

    let mut page_envelopes = Vec::new();
    let mut seen_records = BTreeSet::new();
    for hit in &merged.hits {
        for s in &hit.sources {
            if !seen_records.insert(s.record_id.clone()) {
                continue;
            }
            if let Some(env) = verified.envelopes_by_record_id.get(&s.record_id) {
                page_envelopes.push((s.record_id.clone(), env.clone()));
            }
        }
    }

    Ok(Gathered {
        page: Page {
            outcomes: verified.reports,
            hits: merged.hits,
            hits_truncated: merged.hits_truncated,
            refused: merged.refused,
            refused_truncated: merged.refused_truncated,
        },
        page_envelopes,
        evidence: verified.evidence,
    })
}

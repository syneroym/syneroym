//! Statement export operations.

use serde::Deserialize;
use syneroym_app_host::AppHost;
use syneroym_roym_core::{
    clock,
    envelope::Response,
    eventlog::EventLogEntry,
    paging, signing,
    statement::{AgreementSource, Statement, StatementError, build},
};

use crate::app::{
    AGREEMENTS, AgreementRow, EVENT_LOG, ensure_collections, eventlog_ops, refusal::refusal,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatementExportInput {
    pub from_secs: u64,
    pub to_secs: u64,
}

pub async fn export<H: AppHost>(
    host: &H,
    input: StatementExportInput,
) -> Result<Statement, Response> {
    export_at(host, input, clock::now_secs()).await
}

pub async fn export_at<H: AppHost>(
    host: &H,
    input: StatementExportInput,
    now: u64,
) -> Result<Statement, Response> {
    if input.from_secs >= input.to_secs {
        return Err(refusal("bad-period"));
    }

    ensure_collections(host).await.map_err(Response::internal_error)?;
    eventlog_ops::derive_all(host, now).await.map_err(Response::internal_error)?;

    let owner_did =
        signing::owner_did(host).await.map_err(|e| Response::internal_error(e.to_string()))?;

    let agr_rows: Vec<AgreementRow> =
        paging::query_all(host, AGREEMENTS, None).await.map_err(Response::internal_error)?;

    let agr_sources: Vec<AgreementSource<'_>> = agr_rows
        .iter()
        .map(|row| {
            let (owner_role, counterparty_did) = row.owner_role_and_counterparty(&owner_did);
            AgreementSource {
                agreement: &row.quote_record_id,
                counterparty_did,
                owner_role,
                terms: &row.terms,
            }
        })
        .collect();

    let entries: Vec<EventLogEntry> =
        paging::query_all(host, EVENT_LOG, None).await.map_err(Response::internal_error)?;

    build(&owner_did, input.from_secs, input.to_secs, now, &entries, &agr_sources).map_err(|err| {
        match err {
            StatementError::BadPeriod => refusal("bad-period"),
            StatementError::TotalOverflow => refusal("statement-total-overflow"),
        }
    })
}

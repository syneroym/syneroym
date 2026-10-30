//! Moderation operations: blocks and safety reports.

use serde::{Deserialize, Serialize};
use serde_json::json;
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{IndexDefinition, IndexType, RecordWriteValue},
};
use syneroym_roym_core::{
    clock,
    envelope::Response,
    method::{
        Reply,
        profile::{BlockCheckOutput, BlockTargetInput, BlockedSince},
    },
    paging,
    record::content_digest,
};

use super::{BLOCKS, REPORTS, ensure_coll};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BlockRow {
    pub key: String,
    pub person_did: Option<String>,
    pub address: Option<String>,
    pub reason: Option<String>,
    pub at_secs: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ReportRow {
    pub report_id: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub category: String,
    pub details: Option<String>,
    pub status: String,
    pub at_secs: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockAddInput {
    pub person_did: Option<String>,
    pub address: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BlockAddOutput {
    pub key: String,
}

#[derive(Debug, Serialize)]
pub struct BlockRemoveOutput {
    pub removed: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockListInput {
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportSubject {
    Person,
    Listing,
    Message,
}

impl ReportSubject {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Listing => "listing",
            Self::Message => "message",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportCategory {
    Impersonation,
    Fraud,
    Harassment,
    UnsafeService,
    IllegalContent,
}

impl ReportCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Impersonation => "impersonation",
            Self::Fraud => "fraud",
            Self::Harassment => "harassment",
            Self::UnsafeService => "unsafe-service",
            Self::IllegalContent => "illegal-content",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportCreateInput {
    pub subject_kind: ReportSubject,
    pub subject_id: String,
    pub category: ReportCategory,
    pub details: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReportCreateOutput {
    pub report_id: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportListInput {
    pub status: Option<String>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportIdInput {
    pub report_id: String,
}

pub(crate) fn block_keys(person_did: Option<&str>, address: Option<&str>) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(d) = person_did {
        keys.push(format!("did:{d}"));
    }
    if let Some(a) = address {
        keys.push(format!("addr:{a}"));
    }
    keys
}

pub(crate) async fn block_add<H: AppHost>(host: &H, input: BlockAddInput) -> Reply<BlockAddOutput> {
    let keys = block_keys(input.person_did.as_deref(), input.address.as_deref());
    if keys.is_empty() {
        return Err(Response::invalid_params("at least one of person_did or address is required"));
    }

    let primary_key = keys[0].clone();
    let now = clock::now_secs();

    if let Err(e) = ensure_coll(
        host,
        BLOCKS,
        &[IndexDefinition { field_name: "at_secs".to_string(), type_: IndexType::Numeric }],
    )
    .await
    {
        return Err(Response::internal_error(e));
    }

    for key in keys {
        let row = BlockRow {
            key: key.clone(),
            person_did: input.person_did.clone(),
            address: input.address.clone(),
            reason: input.reason.clone(),
            at_secs: now,
        };
        let payload = match serde_json::to_vec(&row) {
            Ok(b) => b,
            Err(e) => return Err(Response::internal_error(e.to_string())),
        };
        if let Err(e) =
            AppDataLayer::put(host, BLOCKS.to_string(), RecordWriteValue { id: key, payload }).await
        {
            return Err(Response::internal_error(e.to_string()));
        }
    }

    Ok(BlockAddOutput { key: primary_key })
}

pub(crate) async fn block_remove<H: AppHost>(
    host: &H,
    input: BlockTargetInput,
) -> Reply<BlockRemoveOutput> {
    let mut keys_to_delete = block_keys(input.person_did.as_deref(), input.address.as_deref());
    if keys_to_delete.is_empty() {
        return Err(Response::invalid_params("at least one of person_did or address is required"));
    }

    if let Err(e) = ensure_coll(host, BLOCKS, &[]).await {
        return Err(Response::internal_error(e));
    }

    let primary_key = keys_to_delete[0].clone();

    // Look up existing rows so complementary keys (e.g. addr: when given did:) are
    // also removed
    for key in &keys_to_delete.clone() {
        if let Ok(Some(row_val)) = AppDataLayer::get(host, BLOCKS.to_string(), key.clone()).await
            && let Ok(row) = serde_json::from_slice::<BlockRow>(&row_val.payload)
        {
            for extra_key in block_keys(row.person_did.as_deref(), row.address.as_deref()) {
                if !keys_to_delete.contains(&extra_key) {
                    keys_to_delete.push(extra_key);
                }
            }
        }
    }

    for key in keys_to_delete {
        if let Err(e) = AppDataLayer::delete(host, BLOCKS.to_string(), key).await {
            return Err(Response::internal_error(e.to_string()));
        }
    }

    Ok(BlockRemoveOutput { removed: primary_key })
}

pub(crate) async fn block_list<H: AppHost>(
    host: &H,
    input: BlockListInput,
) -> Reply<Vec<BlockRow>> {
    if let Err(e) = ensure_coll(
        host,
        BLOCKS,
        &[IndexDefinition { field_name: "at_secs".to_string(), type_: IndexType::Numeric }],
    )
    .await
    {
        return Err(Response::internal_error(e));
    }
    let mut list: Vec<BlockRow> = match paging::query_all(host, BLOCKS, None).await {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    // Skip secondary addr: alias rows for blocks that have a primary did: row
    list.retain(|row| !(row.key.starts_with("addr:") && row.person_did.is_some()));
    let offset = input.offset.unwrap_or(0) as usize;
    let limit = input.limit.map(|v| v as usize);
    let paged: Vec<_> = match limit {
        Some(lim) => list.into_iter().skip(offset).take(lim).collect(),
        None => list.into_iter().skip(offset).collect(),
    };
    Ok(paged)
}

pub(crate) async fn block_check<H: AppHost>(
    host: &H,
    input: BlockTargetInput,
) -> Reply<BlockCheckOutput> {
    let keys = block_keys(input.person_did.as_deref(), input.address.as_deref());
    if keys.is_empty() {
        return Err(Response::invalid_params("at least one of person_did or address is required"));
    }

    if let Err(e) = ensure_coll(host, BLOCKS, &[]).await {
        return Err(Response::internal_error(e));
    }

    for key in keys {
        match AppDataLayer::get(host, BLOCKS.to_string(), key).await {
            Ok(Some(row)) => {
                let parsed = serde_json::from_slice::<BlockRow>(&row.payload).ok();
                return Ok(BlockCheckOutput {
                    blocked: true,
                    since: Some(BlockedSince {
                        reason: parsed.as_ref().and_then(|b| b.reason.clone()),
                        since_secs: parsed.as_ref().map(|b| b.at_secs),
                    }),
                });
            }
            Ok(None) => {}
            Err(e) => return Err(Response::internal_error(e.to_string())),
        }
    }

    Ok(BlockCheckOutput { blocked: false, since: None })
}

pub(crate) async fn report_create<H: AppHost>(
    host: &H,
    input: ReportCreateInput,
) -> Reply<ReportCreateOutput> {
    let subject_kind_str = input.subject_kind.as_str().to_string();
    let category_str = input.category.as_str().to_string();

    let content_val = json!({
        "subject_kind": subject_kind_str,
        "subject_id": input.subject_id,
        "category": category_str,
        "details": input.details,
    });

    let report_id = match content_digest("rep_", &content_val) {
        Ok(id) => id,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let now = clock::now_secs();

    if let Err(e) = ensure_coll(
        host,
        REPORTS,
        &[
            IndexDefinition { field_name: "status".to_string(), type_: IndexType::String },
            IndexDefinition { field_name: "at_secs".to_string(), type_: IndexType::Numeric },
        ],
    )
    .await
    {
        return Err(Response::internal_error(e));
    }

    // Check whether this content was already reported or withdrawn.
    // `report_id` is content-derived, so the same content hits the same row.
    // Re-filing a withdrawn report is not permitted: the original decision and
    // timestamp must be preserved.
    if let Ok(Some(existing_row)) =
        AppDataLayer::get(host, REPORTS.to_string(), report_id.clone()).await
        && let Ok(existing) = serde_json::from_slice::<ReportRow>(&existing_row.payload)
    {
        if existing.status == "withdrawn" {
            return Err(Response::invalid_params(
                "this report was withdrawn; re-filing the same content is not permitted",
            ));
        }
        // Already recorded — return idempotently, preserving the original timestamp.
        return Ok(ReportCreateOutput { report_id: existing.report_id, status: existing.status });
    }

    let row = ReportRow {
        report_id: report_id.clone(),
        subject_kind: subject_kind_str,
        subject_id: input.subject_id,
        category: category_str,
        details: input.details,
        status: "recorded".to_string(),
        at_secs: now,
    };

    let payload = match serde_json::to_vec(&row) {
        Ok(b) => b,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    if let Err(e) = AppDataLayer::put(
        host,
        REPORTS.to_string(),
        RecordWriteValue { id: report_id.clone(), payload },
    )
    .await
    {
        return Err(Response::internal_error(e.to_string()));
    }

    Ok(ReportCreateOutput { report_id, status: "recorded".to_string() })
}

pub(crate) async fn report_list<H: AppHost>(
    host: &H,
    input: ReportListInput,
) -> Reply<Vec<ReportRow>> {
    let status_filter = input.status.as_deref();
    if let Err(e) = ensure_coll(
        host,
        REPORTS,
        &[
            IndexDefinition { field_name: "status".to_string(), type_: IndexType::String },
            IndexDefinition { field_name: "at_secs".to_string(), type_: IndexType::Numeric },
        ],
    )
    .await
    {
        return Err(Response::internal_error(e));
    }
    let mut list: Vec<ReportRow> = match paging::query_all(host, REPORTS, None).await {
        Ok(v) => v,
        Err(e) => return Err(Response::internal_error(e)),
    };
    list.retain(|row| status_filter.is_none_or(|s| row.status == s));
    let offset = input.offset.unwrap_or(0) as usize;
    let limit = input.limit.map(|v| v as usize);
    let paged: Vec<_> = match limit {
        Some(lim) => list.into_iter().skip(offset).take(lim).collect(),
        None => list.into_iter().skip(offset).collect(),
    };
    Ok(paged)
}

pub(crate) async fn report_get<H: AppHost>(
    host: &H,
    input: ReportIdInput,
) -> Reply<Option<ReportRow>> {
    if let Err(e) = ensure_coll(host, REPORTS, &[]).await {
        return Err(Response::internal_error(e));
    }
    match AppDataLayer::get(host, REPORTS.to_string(), input.report_id).await {
        Ok(Some(row)) => match serde_json::from_slice::<ReportRow>(&row.payload) {
            Ok(r) => Ok(Some(r)),
            Err(e) => Err(Response::internal_error(e.to_string())),
        },
        Ok(None) => Ok(None),
        Err(e) => Err(Response::internal_error(e.to_string())),
    }
}

pub(crate) async fn report_withdraw<H: AppHost>(
    host: &H,
    input: ReportIdInput,
) -> Reply<ReportCreateOutput> {
    let report_id = input.report_id;
    if let Err(e) = ensure_coll(host, REPORTS, &[]).await {
        return Err(Response::internal_error(e));
    }
    let row_opt = match AppDataLayer::get(host, REPORTS.to_string(), report_id.clone()).await {
        Ok(r) => r,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    let Some(row) = row_opt else {
        return Err(Response::invalid_params(format!("report '{report_id}' not found")));
    };

    let mut parsed: ReportRow = match serde_json::from_slice(&row.payload) {
        Ok(p) => p,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    parsed.status = "withdrawn".to_string();
    let payload = match serde_json::to_vec(&parsed) {
        Ok(b) => b,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };

    if let Err(e) = AppDataLayer::put(
        host,
        REPORTS.to_string(),
        RecordWriteValue { id: report_id.clone(), payload },
    )
    .await
    {
        return Err(Response::internal_error(e.to_string()));
    }

    Ok(ReportCreateOutput { report_id, status: "withdrawn".to_string() })
}

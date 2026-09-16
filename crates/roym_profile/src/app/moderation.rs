//! Moderation operations: blocks and safety reports.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{IndexDefinition, IndexType, RecordWriteValue},
};
use syneroym_roym_core::{
    clock,
    envelope::{Request, Response},
    record::content_digest,
};

use super::{BLOCKS, REPORTS, backup::collect, ensure_coll};

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

pub(crate) async fn block_add<H: AppHost>(host: &H, req: &Request) -> Response {
    let person_did = req.params.get("person_did").and_then(|v| v.as_str()).map(String::from);
    let address = req.params.get("address").and_then(|v| v.as_str()).map(String::from);
    let reason = req.params.get("reason").and_then(|v| v.as_str()).map(String::from);

    let keys = block_keys(person_did.as_deref(), address.as_deref());
    if keys.is_empty() {
        return Response::invalid_params("at least one of person_did or address is required");
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
        return Response::internal_error(e);
    }

    for key in keys {
        let row = BlockRow {
            key: key.clone(),
            person_did: person_did.clone(),
            address: address.clone(),
            reason: reason.clone(),
            at_secs: now,
        };
        let payload = match serde_json::to_vec(&row) {
            Ok(b) => b,
            Err(e) => return Response::internal_error(e.to_string()),
        };
        if let Err(e) =
            AppDataLayer::put(host, BLOCKS.to_string(), RecordWriteValue { id: key, payload }).await
        {
            return Response::internal_error(e.to_string());
        }
    }

    Response::ok(json!({ "key": primary_key }))
}

pub(crate) async fn block_remove<H: AppHost>(host: &H, req: &Request) -> Response {
    let person_did = req.params.get("person_did").and_then(|v| v.as_str());
    let address = req.params.get("address").and_then(|v| v.as_str());

    let mut keys_to_delete = block_keys(person_did, address);
    if keys_to_delete.is_empty() {
        return Response::invalid_params("at least one of person_did or address is required");
    }

    if let Err(e) = ensure_coll(host, BLOCKS, &[]).await {
        return Response::internal_error(e);
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
            return Response::internal_error(e.to_string());
        }
    }

    Response::ok(json!({ "removed": primary_key }))
}

pub(crate) async fn block_list<H: AppHost>(host: &H, req: &Request) -> Response {
    if let Err(e) = ensure_coll(
        host,
        BLOCKS,
        &[IndexDefinition { field_name: "at_secs".to_string(), type_: IndexType::Numeric }],
    )
    .await
    {
        return Response::internal_error(e);
    }
    let records = match collect(host, BLOCKS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };

    let mut list = Vec::new();
    for item in records {
        if let Some(p) = item.get("payload")
            && let Ok(row) = serde_json::from_value::<BlockRow>(p.clone())
        {
            // Skip secondary addr: alias rows for blocks that have a primary did: row
            if row.key.starts_with("addr:") && row.person_did.is_some() {
                continue;
            }
            list.push(row);
        }
    }
    let offset = req.params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = req.params.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
    let paged: Vec<_> = match limit {
        Some(lim) => list.into_iter().skip(offset).take(lim).collect(),
        None => list.into_iter().skip(offset).collect(),
    };
    Response::ok(json!(paged))
}

pub(crate) async fn block_check<H: AppHost>(host: &H, req: &Request) -> Response {
    let person_did = req.params.get("person_did").and_then(|v| v.as_str());
    let address = req.params.get("address").and_then(|v| v.as_str());

    let keys = block_keys(person_did, address);
    if keys.is_empty() {
        return Response::invalid_params("at least one of person_did or address is required");
    }

    if let Err(e) = ensure_coll(host, BLOCKS, &[]).await {
        return Response::internal_error(e);
    }

    for key in keys {
        match AppDataLayer::get(host, BLOCKS.to_string(), key).await {
            Ok(Some(row)) => {
                let parsed = serde_json::from_slice::<BlockRow>(&row.payload).ok();
                return Response::ok(json!({
                    "blocked": true,
                    "reason": parsed.as_ref().and_then(|b| b.reason.clone()),
                    "since_secs": parsed.as_ref().map(|b| b.at_secs),
                }));
            }
            Ok(None) => {}
            Err(e) => return Response::internal_error(e.to_string()),
        }
    }

    Response::ok(json!({ "blocked": false }))
}

pub(crate) async fn report_create<H: AppHost>(host: &H, req: &Request) -> Response {
    let subject_kind = match req.params.get("subject_kind").and_then(|v| v.as_str()) {
        Some(k) if matches!(k, "person" | "listing" | "message") => k.to_string(),
        _ => {
            return Response::invalid_params(
                "subject_kind must be 'person', 'listing', or 'message'",
            );
        }
    };
    let subject_id = match req.params.get("subject_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return Response::invalid_params("subject_id is required"),
    };
    let category = match req.params.get("category").and_then(|v| v.as_str()) {
        Some(c)
            if matches!(
                c,
                "impersonation" | "fraud" | "harassment" | "unsafe-service" | "illegal-content"
            ) =>
        {
            c.to_string()
        }
        _ => {
            return Response::invalid_params("category must be one of the five valid categories");
        }
    };
    let details = req.params.get("details").and_then(|v| v.as_str()).map(String::from);

    let content_val = json!({
        "subject_kind": subject_kind,
        "subject_id": subject_id,
        "category": category,
        "details": details,
    });

    let report_id = match content_digest("rep_", &content_val) {
        Ok(id) => id,
        Err(e) => return Response::internal_error(e.to_string()),
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
        return Response::internal_error(e);
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
            return Response::invalid_params(
                "this report was withdrawn; re-filing the same content is not permitted",
            );
        }
        // Already recorded — return idempotently, preserving the original timestamp.
        return Response::ok(json!({
            "report_id": existing.report_id,
            "status": existing.status,
        }));
    }

    let row = ReportRow {
        report_id: report_id.clone(),
        subject_kind,
        subject_id,
        category,
        details,
        status: "recorded".to_string(),
        at_secs: now,
    };

    let payload = match serde_json::to_vec(&row) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    if let Err(e) = AppDataLayer::put(
        host,
        REPORTS.to_string(),
        RecordWriteValue { id: report_id.clone(), payload },
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }

    Response::ok(json!({ "report_id": report_id, "status": "recorded" }))
}

pub(crate) async fn report_list<H: AppHost>(host: &H, req: &Request) -> Response {
    let status_filter = req.params.get("status").and_then(|v| v.as_str());
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
        return Response::internal_error(e);
    }
    let records = match collect(host, REPORTS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };

    let mut list = Vec::new();
    for item in records {
        if let Some(p) = item.get("payload")
            && let Ok(row) = serde_json::from_value::<ReportRow>(p.clone())
            && status_filter.is_none_or(|s| row.status == s)
        {
            list.push(row);
        }
    }
    let offset = req.params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let limit = req.params.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
    let paged: Vec<_> = match limit {
        Some(lim) => list.into_iter().skip(offset).take(lim).collect(),
        None => list.into_iter().skip(offset).collect(),
    };
    Response::ok(json!(paged))
}

pub(crate) async fn report_get<H: AppHost>(host: &H, req: &Request) -> Response {
    let report_id = match req.params.get("report_id").and_then(|v| v.as_str()) {
        Some(r) => r,
        None => return Response::invalid_params("report_id is required"),
    };
    if let Err(e) = ensure_coll(host, REPORTS, &[]).await {
        return Response::internal_error(e);
    }
    match AppDataLayer::get(host, REPORTS.to_string(), report_id.to_string()).await {
        Ok(Some(row)) => match serde_json::from_slice::<ReportRow>(&row.payload) {
            Ok(r) => Response::ok(json!(r)),
            Err(e) => Response::internal_error(e.to_string()),
        },
        Ok(None) => Response::ok(Value::Null),
        Err(e) => Response::internal_error(e.to_string()),
    }
}

pub(crate) async fn report_withdraw<H: AppHost>(host: &H, req: &Request) -> Response {
    let report_id = match req.params.get("report_id").and_then(|v| v.as_str()) {
        Some(r) => r.to_string(),
        None => return Response::invalid_params("report_id is required"),
    };
    if let Err(e) = ensure_coll(host, REPORTS, &[]).await {
        return Response::internal_error(e);
    }
    let row_opt = match AppDataLayer::get(host, REPORTS.to_string(), report_id.clone()).await {
        Ok(r) => r,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    let Some(row) = row_opt else {
        return Response::invalid_params(format!("report '{report_id}' not found"));
    };

    let mut parsed: ReportRow = match serde_json::from_slice(&row.payload) {
        Ok(p) => p,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    parsed.status = "withdrawn".to_string();
    let payload = match serde_json::to_vec(&parsed) {
        Ok(b) => b,
        Err(e) => return Response::internal_error(e.to_string()),
    };

    if let Err(e) = AppDataLayer::put(
        host,
        REPORTS.to_string(),
        RecordWriteValue { id: report_id.clone(), payload },
    )
    .await
    {
        return Response::internal_error(e.to_string());
    }

    Response::ok(json!({ "report_id": report_id, "status": "withdrawn" }))
}

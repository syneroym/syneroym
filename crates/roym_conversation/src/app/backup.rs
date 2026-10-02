//! Conversation backup export and import using host export/import and
//! admissions.

use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppConversation, AppDataLayer, AppHost, types::data_layer::RecordWriteValue,
};
use syneroym_roym_core::{
    backup::{self, Bundle},
    clock,
    envelope::{Request, Response},
    signing,
};

use super::{ADMISSIONS, SCHEMA_VERSION, ensure_admissions};

pub const SECTION_HISTORY: &str = "conversation_history";
pub const SECTION_ADMISSIONS: &str = "admissions";

pub(crate) async fn export<H: AppHost>(host: &H) -> Response {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let now = clock::now_secs();
    if let Err(e) = ensure_admissions(host).await {
        return Response::internal_error(e);
    }

    // Export history chunks from host
    let mut history_records = Vec::new();
    let mut cursor = None;
    let mut idx = 0u64;
    loop {
        let chunk = match AppConversation::export_history(host, cursor).await {
            Ok(c) => c,
            Err(e) => return Response::internal_error(format!("export_history: {e:?}")),
        };
        history_records.push(json!({
            "id": format!("chunk:{idx}"),
            "payload": {
                "data": chunk.data,
            }
        }));
        idx += 1;
        cursor = chunk.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    let mut sections =
        match backup::collect_sections(host, &[(SECTION_ADMISSIONS, ADMISSIONS)]).await {
            Ok(s) => s,
            Err(e) => return Response::internal_error(e),
        };
    sections.insert(SECTION_HISTORY.to_string(), history_records);

    backup::export_signed(host, owner, SCHEMA_VERSION, sections, now).await
}

pub(crate) async fn import<H: AppHost>(host: &H, req: &Request) -> Response {
    let bundle_val = match req.params.get("bundle").cloned().or_else(|| Some(req.params.clone())) {
        Some(v) => v,
        None => return Response::invalid_params("bundle is required"),
    };
    let bundle = match Bundle::from_json(&bundle_val.to_string()) {
        Ok(b) => b,
        Err(e) => return Response::invalid_params(format!("invalid bundle: {e}")),
    };
    let now = clock::now_secs();
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if let Err(e) = backup::check_signed_bundle(&bundle, &owner, now) {
        return Response::invalid_params(e.to_string());
    }
    for (name, declared) in &bundle.manifest.sections {
        if declared.schema_version != SCHEMA_VERSION {
            return Response::invalid_params(format!(
                "section '{name}' has schema version {}, this node requires {SCHEMA_VERSION}",
                declared.schema_version
            ));
        }
    }

    let mut counts = Map::new();
    let mut message_count = 0u64;
    let mut admission_count = 0u64;
    for (name, records) in &bundle.sections {
        match name.as_str() {
            SECTION_HISTORY => {
                for rec in records {
                    let data_val = match rec.get("payload").and_then(|p| p.get("data")) {
                        Some(d) => d,
                        None => return Response::invalid_params("history chunk missing data"),
                    };
                    let bytes: Vec<u8> = match serde_json::from_value(data_val.clone()) {
                        Ok(b) => b,
                        Err(e) => {
                            return Response::invalid_params(format!("invalid chunk bytes: {e}"));
                        }
                    };
                    let count = match AppConversation::import_history(host, bytes).await {
                        Ok(n) => n,
                        Err(e) => {
                            return Response::internal_error(format!("import_history: {e:?}"));
                        }
                    };
                    message_count += count as u64;
                }
            }
            SECTION_ADMISSIONS => {
                if let Err(e) = ensure_admissions(host).await {
                    return Response::internal_error(e);
                }
                for rec in records {
                    let id = match rec.get("id").and_then(Value::as_str) {
                        Some(i) => i.to_string(),
                        None => return Response::invalid_params("record missing id"),
                    };
                    let payload_val = match rec.get("payload") {
                        Some(p) => p.clone(),
                        None => return Response::invalid_params("record missing payload"),
                    };
                    let bytes = match serde_json::to_vec(&payload_val) {
                        Ok(b) => b,
                        Err(e) => return Response::internal_error(e.to_string()),
                    };
                    if let Err(e) = AppDataLayer::put(
                        host,
                        ADMISSIONS.to_string(),
                        RecordWriteValue { id, payload: bytes },
                    )
                    .await
                    {
                        return Response::internal_error(e.to_string());
                    }
                    admission_count += 1;
                }
            }
            other => return Response::invalid_params(format!("unknown section '{other}'")),
        }
    }

    counts.insert("messages".to_string(), json!(message_count));
    counts.insert("admissions".to_string(), json!(admission_count));
    Response::ok(json!({ "imported": counts }))
}

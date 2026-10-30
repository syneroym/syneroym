//! Conversation backup export and import using host export/import and
//! admissions.

use serde::Serialize;
use serde_json::{Value, json};
use syneroym_app_host::{
    AppConversation, AppDataLayer, AppHost, types::data_layer::RecordWriteValue,
};
use syneroym_roym_core::{
    backup::{self, Bundle, ImportInput},
    clock,
    envelope::Response,
    method::{NoParams, Reply},
    signing,
};

use super::{ADMISSIONS, SCHEMA_VERSION, ensure_admissions};

pub const SECTION_HISTORY: &str = "conversation_history";
pub const SECTION_ADMISSIONS: &str = "admissions";

#[derive(Debug, Clone, Serialize)]
pub struct ConversationImportCounts {
    pub messages: u64,
    pub admissions: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationImportOutput {
    pub imported: ConversationImportCounts,
}

pub(crate) async fn export<H: AppHost>(host: &H, _: NoParams) -> Reply<Bundle> {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let now = clock::now_secs();
    if let Err(e) = ensure_admissions(host).await {
        return Err(Response::internal_error(e));
    }

    // Export history chunks from host
    let mut history_records = Vec::new();
    let mut cursor = None;
    let mut idx = 0u64;
    loop {
        let chunk = match AppConversation::export_history(host, cursor).await {
            Ok(c) => c,
            Err(e) => return Err(Response::internal_error(format!("export_history: {e:?}"))),
        };
        history_records.push(json!({
            "id": format!("chunk:{idx}"),
            "payload": {
                "data": backup::bytes_to_hex(&chunk.data),
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
            Err(e) => return Err(Response::internal_error(e)),
        };
    sections.insert(SECTION_HISTORY.to_string(), history_records);

    backup::export_signed_bundle(host, owner, SCHEMA_VERSION, sections, now).await
}

pub(crate) async fn import<H: AppHost>(
    host: &H,
    input: ImportInput,
) -> Reply<ConversationImportOutput> {
    let bundle = input.bundle;
    let now = clock::now_secs();
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    if let Err(e) = backup::check_signed_bundle(&bundle, &owner, now) {
        return Err(Response::invalid_params(e.to_string()));
    }
    for (name, declared) in &bundle.manifest.sections {
        if declared.schema_version != SCHEMA_VERSION {
            return Err(Response::invalid_params(format!(
                "section '{name}' has schema version {}, this node requires {SCHEMA_VERSION}",
                declared.schema_version
            )));
        }
    }

    let mut message_count = 0u64;
    let mut admission_count = 0u64;
    for (name, records) in &bundle.sections {
        match name.as_str() {
            SECTION_HISTORY => {
                for rec in records {
                    let data_val = match rec.get("payload").and_then(|p| p.get("data")) {
                        Some(d) => d,
                        None => return Err(Response::invalid_params("history chunk missing data")),
                    };
                    let Some(bytes) = data_val.as_str().and_then(backup::bytes_from_hex) else {
                        return Err(Response::invalid_params("history chunk data is not hex"));
                    };
                    let count = match AppConversation::import_history(host, bytes).await {
                        Ok(n) => n,
                        Err(e) => {
                            // Importing skips what is already stored, so running the
                            // same import again resumes after this failure.
                            return Err(Response::internal_error(format!(
                                "import_history failed ({e:?}); run the same import again to \
                                 resume"
                            )));
                        }
                    };
                    message_count += count as u64;
                }
            }
            SECTION_ADMISSIONS => {
                if let Err(e) = ensure_admissions(host).await {
                    return Err(Response::internal_error(e));
                }
                for rec in records {
                    let id = match rec.get("id").and_then(Value::as_str) {
                        Some(i) => i.to_string(),
                        None => return Err(Response::invalid_params("record missing id")),
                    };
                    let payload_val = match rec.get("payload") {
                        Some(p) => p.clone(),
                        None => return Err(Response::invalid_params("record missing payload")),
                    };
                    let bytes = match serde_json::to_vec(&payload_val) {
                        Ok(b) => b,
                        Err(e) => return Err(Response::internal_error(e.to_string())),
                    };
                    if let Err(e) = AppDataLayer::put(
                        host,
                        ADMISSIONS.to_string(),
                        RecordWriteValue { id, payload: bytes },
                    )
                    .await
                    {
                        return Err(Response::internal_error(e.to_string()));
                    }
                    admission_count += 1;
                }
            }
            other => return Err(Response::invalid_params(format!("unknown section '{other}'"))),
        }
    }

    Ok(ConversationImportOutput {
        imported: ConversationImportCounts { messages: message_count, admissions: admission_count },
    })
}

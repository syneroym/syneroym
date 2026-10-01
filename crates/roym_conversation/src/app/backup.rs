//! Conversation backup export and import.

use serde_json::{Map, Value, json};
use syneroym_app_host::{AppDataLayer, AppHost, types::data_layer::RecordWriteValue};
use syneroym_roym_core::{
    backup::{self, Bundle, SECTION_CONVERSATIONS, SECTION_MESSAGES},
    clock,
    envelope::{Request, Response},
    signing,
};

use super::{
    CONVERSATIONS, MESSAGES, SCHEMA_VERSION, ensure_coll, ensure_conversations, ensure_messages,
};

pub(crate) async fn export<H: AppHost>(host: &H) -> Response {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let now = clock::now_secs();
    if let Err(e) = ensure_conversations(host).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_messages(host).await {
        return Response::internal_error(e);
    }
    let sections = match backup::collect_sections(
        host,
        &[(SECTION_CONVERSATIONS, CONVERSATIONS), (SECTION_MESSAGES, MESSAGES)],
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return Response::internal_error(e),
    };
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
    for (name, records) in &bundle.sections {
        let collection = match name.as_str() {
            SECTION_CONVERSATIONS => CONVERSATIONS,
            SECTION_MESSAGES => MESSAGES,
            other => return Response::invalid_params(format!("unknown section '{other}'")),
        };
        if let Err(e) = ensure_coll(host, collection, &[]).await {
            return Response::internal_error(e);
        }
        let mut n = 0u64;
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
                collection.to_string(),
                RecordWriteValue { id, payload: bytes },
            )
            .await
            {
                return Response::internal_error(e.to_string());
            }
            n += 1;
        }
        counts.insert(collection.to_string(), json!(n));
    }
    Response::ok(json!({ "imported": counts }))
}

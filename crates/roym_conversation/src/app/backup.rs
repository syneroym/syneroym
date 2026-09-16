//! Conversation backup export and import.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{QueryOptions, RecordWriteValue},
};
use syneroym_roym_core::{
    backup::{BUNDLE_VERSION, Bundle, BundleManifest, SECTION_CONVERSATIONS, SECTION_MESSAGES},
    clock,
    envelope::{Request, Response},
    signing,
};

use super::{
    CONVERSATIONS, MESSAGES, SCHEMA_VERSION, ensure_coll, ensure_conversations, ensure_messages,
};

async fn collect<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = AppDataLayer::query(
            host,
            collection.to_string(),
            QueryOptions { filter: None, limit: Some(500), cursor: cursor.clone() },
        )
        .await
        .map_err(|e| e.to_string())?;
        for r in page.records {
            if let Ok(parsed) = serde_json::from_slice::<Value>(&r.payload) {
                out.push(json!({ "id": r.id, "payload": parsed }));
            }
        }
        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(out)
}

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
    let conversations = match collect(host, CONVERSATIONS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let messages = match collect(host, MESSAGES).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let sections = BTreeMap::from([
        (SECTION_CONVERSATIONS.to_string(), conversations),
        (SECTION_MESSAGES.to_string(), messages),
    ]);
    let mut manifest_sections = BTreeMap::new();
    for (k, v) in &sections {
        match Bundle::digest(SCHEMA_VERSION, v) {
            Ok(d) => {
                manifest_sections.insert(k.clone(), d);
            }
            Err(e) => return Response::internal_error(e.to_string()),
        }
    }
    let bundle = Bundle {
        manifest: BundleManifest {
            bundle_version: BUNDLE_VERSION,
            produced_at_secs: now,
            subject_did: owner,
            sections: manifest_sections,
        },
        sections,
    };
    match serde_json::to_value(&bundle) {
        Ok(v) => Response::ok(v),
        Err(e) => Response::internal_error(e.to_string()),
    }
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
    if let Err(e) = bundle.check_integrity() {
        return Response::invalid_params(e.to_string());
    }
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    if bundle.manifest.subject_did != owner {
        return Response::invalid_params(format!(
            "bundle belongs to '{}', this node holds '{}'",
            bundle.manifest.subject_did, owner
        ));
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

//! Profile backup export and import.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{Mutation, QueryOptions, RecordWriteValue},
};
use syneroym_roym_core::{
    backup::{
        BUNDLE_VERSION, Bundle, BundleManifest, SECTION_BLOCKS, SECTION_CONTACTS, SECTION_PROFILE,
        SECTION_REPORTS,
    },
    clock,
    envelope::{Request, Response},
    record::{RECORD_PROFILE, VerifyOptions, verify_json},
    signing,
};

use super::{BLOCKS, CONTACTS, PROFILES, REPORTS, SCHEMA_VERSION, ensure_coll};

pub(crate) async fn collect<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
    ensure_coll(host, collection, &[]).await?;
    let mut results = Vec::new();
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
                results.push(json!({ "id": r.id, "payload": parsed }));
            }
        }

        if page.next_cursor.is_none() || page.next_cursor == cursor {
            break;
        }
        cursor = page.next_cursor;
    }
    Ok(results)
}

async fn reverify_profiles<H: AppHost>(host: &H, now: u64) -> Result<u64, String> {
    ensure_coll(host, PROFILES, &[]).await?;
    let records = collect(host, PROFILES).await?;
    let mut verified = 0;
    for rec in records {
        if let Some(payload) = rec.get("payload")
            && let Some(env_str) = payload.get("envelope").and_then(|v| v.as_str())
        {
            let did = rec.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            if verify_json(env_str, &VerifyOptions::new(now).expecting(did)).is_ok() {
                verified += 1;
            }
        }
    }
    Ok(verified)
}

pub(crate) async fn export<H: AppHost>(host: &H) -> Response {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let now = clock::now_secs();

    let p_sec = match collect(host, PROFILES).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let c_sec = match collect(host, CONTACTS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let b_sec = match collect(host, BLOCKS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let r_sec = match collect(host, REPORTS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };

    let sections = BTreeMap::from([
        (SECTION_PROFILE.to_string(), p_sec),
        (SECTION_CONTACTS.to_string(), c_sec),
        (SECTION_BLOCKS.to_string(), b_sec),
        (SECTION_REPORTS.to_string(), r_sec),
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

    let manifest = BundleManifest {
        bundle_version: BUNDLE_VERSION,
        produced_at_secs: now,
        subject_did: owner,
        sections: manifest_sections,
    };

    let bundle = Bundle { manifest, sections };
    match serde_json::to_value(&bundle) {
        Ok(v) => Response::ok(v),
        Err(e) => Response::internal_error(e.to_string()),
    }
}

fn validate_bundle_manifest(bundle: &Bundle, owner: &str) -> Result<(), Response> {
    if bundle.manifest.subject_did != owner {
        return Err(Response::invalid_params(format!(
            "bundle belongs to '{}', this node holds '{}'",
            bundle.manifest.subject_did, owner
        )));
    }

    for (name, declared) in &bundle.manifest.sections {
        if declared.schema_version != SCHEMA_VERSION {
            return Err(Response::invalid_params(format!(
                "section '{name}' has schema version {}, this node requires {SCHEMA_VERSION}",
                declared.schema_version
            )));
        }
    }
    Ok(())
}

fn prepare_section_record(section_name: &str, rec: &Value, now: u64) -> Result<Mutation, Response> {
    let id = match rec.get("id").and_then(|v| v.as_str()) {
        Some(i) => i.to_string(),
        None => return Err(Response::invalid_params("record missing id")),
    };
    let mut payload_val = match rec.get("payload") {
        Some(p) => p.clone(),
        None => return Err(Response::invalid_params("record missing payload")),
    };
    if section_name == SECTION_PROFILE {
        let env_str = match payload_val.get("envelope").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return Err(Response::invalid_params("profile record missing envelope"));
            }
        };
        let verified_rec = match verify_json(env_str, &VerifyOptions::new(now).expecting(&id)) {
            Ok(vr) => vr,
            Err(e) => {
                return Err(Response::invalid_params(format!(
                    "profile record '{id}' failed verification: {e}"
                )));
            }
        };
        if verified_rec.record_type != RECORD_PROFILE {
            return Err(Response::invalid_params("record is not a profile"));
        }
        if verified_rec.version != 1 {
            return Err(Response::invalid_params("unsupported profile record version"));
        }
        if verified_rec.subject != id {
            return Err(Response::invalid_params(format!(
                "profile record subject '{}' does not match id '{id}'",
                verified_rec.subject
            )));
        }
        if let Some(obj) = payload_val.as_object_mut() {
            obj.insert("verified_at_secs".to_string(), json!(now));
        }
    }
    let payload_bytes = match serde_json::to_vec(&payload_val) {
        Ok(b) => b,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    Ok(Mutation::Put(RecordWriteValue { id, payload: payload_bytes }))
}

async fn apply_prepared_writes<H: AppHost>(
    host: &H,
    prepared_writes: Vec<(&'static str, Vec<Mutation>)>,
) -> Result<(), Response> {
    for (collection, muts) in prepared_writes {
        if let Err(e) = ensure_coll(host, collection, &[]).await {
            return Err(Response::internal_error(e));
        }
        for chunk in muts.chunks(100) {
            if let Err(e) =
                AppDataLayer::batch_mutate(host, collection.to_string(), chunk.to_vec()).await
            {
                return Err(Response::internal_error(e.to_string()));
            }
        }
    }
    Ok(())
}

pub(crate) async fn import<H: AppHost>(host: &H, req: &Request) -> Response {
    let bundle_val = match req.params.get("bundle").cloned().or_else(|| Some(req.params.clone())) {
        Some(v) => v,
        None => return Response::invalid_params("bundle is required"),
    };
    let bundle_str = bundle_val.to_string();
    let bundle = match Bundle::from_json(&bundle_str) {
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

    if let Err(resp) = validate_bundle_manifest(&bundle, &owner) {
        return resp;
    }

    let now = clock::now_secs();
    let mut prepared_writes: Vec<(&'static str, Vec<Mutation>)> = Vec::new();

    for (name, records) in &bundle.sections {
        let collection = match name.as_str() {
            SECTION_PROFILE => PROFILES,
            SECTION_CONTACTS => CONTACTS,
            SECTION_BLOCKS => BLOCKS,
            SECTION_REPORTS => REPORTS,
            other => return Response::invalid_params(format!("unknown section '{other}'")),
        };

        let mut section_muts = Vec::new();
        for rec in records {
            match prepare_section_record(name, rec, now) {
                Ok(mutation) => section_muts.push(mutation),
                Err(resp) => return resp,
            }
        }
        prepared_writes.push((collection, section_muts));
    }

    // Phase 2: All records and sections verified clean -- apply mutations
    if let Err(resp) = apply_prepared_writes(host, prepared_writes).await {
        return resp;
    }

    let verified = match reverify_profiles(host, clock::now_secs()).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };

    Response::ok(json!({ "sections": bundle.sections.len(), "profiles_verified": verified }))
}

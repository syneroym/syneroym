//! Server half: export and import.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{Mutation, RecordWriteValue},
};
use syneroym_roym_core::{
    backup::{
        BUNDLE_VERSION, Bundle, BundleManifest, SECTION_MEMBERS, SECTION_PUBLICATION_LOG,
        SECTION_PUBLICATIONS, SECTION_SOURCES, SECTION_SYNORG,
    },
    clock,
    envelope::{Request, Response},
    listing,
};

use super::{
    MEMBERS, PUBLICATION_LOG, PUBLICATIONS, SCHEMA_VERSION, SETTINGS, SOURCES, collect,
    ensure_coll, owner_did_or_node, search_ops,
};

pub(in crate::app) async fn export<H: AppHost>(host: &H) -> Response {
    let subject = owner_did_or_node(host).await;
    let now = clock::now_secs();
    for c in [SETTINGS, MEMBERS, PUBLICATIONS, PUBLICATION_LOG, SOURCES] {
        if let Err(e) = ensure_coll(host, c, &[]).await {
            return Response::internal_error(e);
        }
    }
    let synorg = match collect(host, SETTINGS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let members = match collect(host, MEMBERS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let publications = match collect(host, PUBLICATIONS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let publication_log = match collect(host, PUBLICATION_LOG).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let sources = match collect(host, SOURCES).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let sections = BTreeMap::from([
        (SECTION_SYNORG.to_string(), synorg),
        (SECTION_MEMBERS.to_string(), members),
        (SECTION_PUBLICATIONS.to_string(), publications),
        (SECTION_PUBLICATION_LOG.to_string(), publication_log),
        (SECTION_SOURCES.to_string(), sources),
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
            subject_did: subject,
            sections: manifest_sections,
        },
        sections,
    };
    match serde_json::to_value(&bundle) {
        Ok(v) => Response::ok(v),
        Err(e) => Response::internal_error(e.to_string()),
    }
}

pub(in crate::app) async fn import<H: AppHost>(host: &H, req: &Request) -> Response {
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
    let owner = owner_did_or_node(host).await;
    if !owner.is_empty() && bundle.manifest.subject_did != owner {
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

    let mut prepared: Vec<(&'static str, Vec<Mutation>)> = Vec::new();
    for (name, records) in &bundle.sections {
        let collection = match name.as_str() {
            SECTION_SYNORG => SETTINGS,
            SECTION_MEMBERS => MEMBERS,
            SECTION_PUBLICATIONS => PUBLICATIONS,
            SECTION_PUBLICATION_LOG => PUBLICATION_LOG,
            SECTION_SOURCES => SOURCES,
            other => return Response::invalid_params(format!("unknown section '{other}'")),
        };
        let mut muts = Vec::new();
        for rec in records {
            let id = match rec.get("id").and_then(Value::as_str) {
                Some(i) => i.to_string(),
                None => return Response::invalid_params("record missing id"),
            };
            let payload_val = match rec.get("payload") {
                Some(p) => p.clone(),
                None => return Response::invalid_params("record missing payload"),
            };
            if name == SECTION_PUBLICATIONS
                && let Some(env_str) = payload_val.get("envelope").and_then(Value::as_str)
            {
                let verdict = listing::verify_envelope(env_str, clock::now_secs());
                if !verdict.verified {
                    return Response::invalid_params(format!(
                        "publication record '{id}' failed verification: {}",
                        verdict.reason.unwrap_or_default()
                    ));
                }
            }
            let payload_bytes = match serde_json::to_vec(&payload_val) {
                Ok(b) => b,
                Err(e) => return Response::internal_error(e.to_string()),
            };
            muts.push(Mutation::Put(RecordWriteValue { id, payload: payload_bytes }));
        }
        prepared.push((collection, muts));
    }

    let mut counts = Map::new();
    for (collection, muts) in prepared {
        if let Err(e) = ensure_coll(host, collection, &[]).await {
            return Response::internal_error(e);
        }
        counts.insert(collection.to_string(), json!(muts.len()));
        for chunk in muts.chunks(100) {
            if let Err(e) =
                AppDataLayer::batch_mutate(host, collection.to_string(), chunk.to_vec()).await
            {
                return Response::internal_error(e.to_string());
            }
        }
    }
    // `search_index` is derived from `publications`, and nothing else
    // populates it -- an import that skipped this would leave a fresh
    // node answering zero hits for listings it demonstrably holds.
    let rebuilt = match search_ops::rebuild_search_index(host).await {
        Ok(n) => n,
        Err(e) => return Response::internal_error(e),
    };
    Response::ok(json!({ "imported": counts, "reindexed": rebuilt }))
}

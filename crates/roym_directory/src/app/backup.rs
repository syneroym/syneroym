//! Server half: export and import. The bundle is signed (C9): a
//! directory's own statements about its members are as much this
//! installation's signed word as anything else it produces.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{Mutation, RecordWriteValue},
};
use syneroym_roym_core::{
    backup::{
        self, BUNDLE_VERSION, Bundle, BundleManifest, SECTION_CREDENTIALS, SECTION_DECISIONS,
        SECTION_HELD_MEMBERSHIPS, SECTION_MEMBERS, SECTION_PUBLICATION_LOG, SECTION_PUBLICATIONS,
        SECTION_REVOCATIONS, SECTION_SOURCES, SECTION_SYNORG,
    },
    clock,
    envelope::{Request, Response},
    listing,
    signing::{self, CertificateError},
};

use super::{
    CREDENTIALS, DECISIONS, HELD_MEMBERSHIPS, MEMBERS, PUBLICATION_LOG, PUBLICATIONS, REVOCATIONS,
    SCHEMA_VERSION, SETTINGS, SOURCES, collect, ensure_coll, owner_did_or_node, search_ops,
    standing,
};

pub(in crate::app) async fn export<H: AppHost>(host: &H) -> Response {
    let subject = owner_did_or_node(host).await;
    let now = clock::now_secs();
    let collections = [
        SETTINGS,
        MEMBERS,
        PUBLICATIONS,
        PUBLICATION_LOG,
        SOURCES,
        CREDENTIALS,
        REVOCATIONS,
        DECISIONS,
        HELD_MEMBERSHIPS,
    ];
    for c in collections {
        if let Err(e) = ensure_coll(host, c, &[]).await {
            return Response::internal_error(e);
        }
    }
    let mut sections = BTreeMap::new();
    let names = [
        (SECTION_SYNORG, SETTINGS),
        (SECTION_MEMBERS, MEMBERS),
        (SECTION_PUBLICATIONS, PUBLICATIONS),
        (SECTION_PUBLICATION_LOG, PUBLICATION_LOG),
        (SECTION_SOURCES, SOURCES),
        (SECTION_CREDENTIALS, CREDENTIALS),
        (SECTION_REVOCATIONS, REVOCATIONS),
        (SECTION_DECISIONS, DECISIONS),
        (SECTION_HELD_MEMBERSHIPS, HELD_MEMBERSHIPS),
    ];
    for (section, collection) in names {
        match collect(host, collection).await {
            Ok(v) => {
                sections.insert(section.to_string(), v);
            }
            Err(e) => return Response::internal_error(e),
        }
    }
    // `standing` is derived from `credentials`/`revocations`/
    // `moderation_decisions` and is not exported -- `import` rebuilds it.

    let mut manifest_sections = BTreeMap::new();
    for (k, v) in &sections {
        match Bundle::digest(SCHEMA_VERSION, v) {
            Ok(d) => {
                manifest_sections.insert(k.clone(), d);
            }
            Err(e) => return Response::internal_error(e.to_string()),
        }
    }
    let mut bundle = Bundle {
        manifest: BundleManifest {
            bundle_version: BUNDLE_VERSION,
            subject_did: subject,
            sections: manifest_sections,
        },
        sections,
        manifest_signature: None,
    };
    if let Err(e) = signing::sign_bundle(host, &mut bundle, now).await {
        if matches!(e, CertificateError::NotEnrolled) {
            return Response::invalid_params("signing-not-enrolled");
        }
        return Response::internal_error(e.to_string());
    }
    match serde_json::to_value(&bundle) {
        Ok(v) => Response::ok(v),
        Err(e) => Response::internal_error(e.to_string()),
    }
}

fn collection_for(section: &str) -> Option<&'static str> {
    match section {
        SECTION_SYNORG => Some(SETTINGS),
        SECTION_MEMBERS => Some(MEMBERS),
        SECTION_PUBLICATIONS => Some(PUBLICATIONS),
        SECTION_PUBLICATION_LOG => Some(PUBLICATION_LOG),
        SECTION_SOURCES => Some(SOURCES),
        SECTION_CREDENTIALS => Some(CREDENTIALS),
        SECTION_REVOCATIONS => Some(REVOCATIONS),
        SECTION_DECISIONS => Some(DECISIONS),
        SECTION_HELD_MEMBERSHIPS => Some(HELD_MEMBERSHIPS),
        _ => None,
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
    let now = clock::now_secs();
    let owner = owner_did_or_node(host).await;
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

    let mut prepared: Vec<(&'static str, Vec<Mutation>)> = Vec::new();
    for (name, records) in &bundle.sections {
        let Some(collection) = collection_for(name) else {
            return Response::invalid_params(format!("unknown section '{name}'"));
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
                let verdict = listing::verify_envelope(env_str, now);
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
    // `standing` and `search_index` are both derived, and nothing else
    // populates them from an imported bundle. Standing first: the index
    // rows read the standing to compute their listed window.
    let standing_rebuilt = match standing::rebuild_all(host).await {
        Ok(n) => n,
        Err(e) => return Response::internal_error(e),
    };
    let rebuilt = match search_ops::rebuild_search_index(host).await {
        Ok(n) => n,
        Err(e) => return Response::internal_error(e),
    };
    Response::ok(json!({ "imported": counts, "reindexed": rebuilt, "standing": standing_rebuilt }))
}

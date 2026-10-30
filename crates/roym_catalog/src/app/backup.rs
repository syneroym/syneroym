use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use syneroym_app_host::{
    AppDataLayer, AppHost,
    types::data_layer::{Mutation, RecordWriteValue},
};
use syneroym_roym_core::{
    backup::{self, Bundle, ImportInput, SECTION_AVAILABILITY, SECTION_LISTINGS},
    clock,
    envelope::Response,
    listing,
    method::{NoParams, Reply},
    record::{RECORD_LISTING, VerifyOptions, verify_json},
    signing,
};

use super::{
    AVAILABILITY, LISTINGS, SCHEMA_VERSION, ensure_availability, ensure_coll, ensure_listings,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogImportOutput {
    pub imported: BTreeMap<String, usize>,
}

pub(crate) async fn export<H: AppHost>(host: &H, _: NoParams) -> Reply<Bundle> {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Err(Response::internal_error(e.to_string())),
    };
    let now = clock::now_secs();
    if let Err(e) = ensure_listings(host).await {
        return Err(Response::internal_error(e));
    }
    if let Err(e) = ensure_availability(host).await {
        return Err(Response::internal_error(e));
    }
    let sections = match backup::collect_sections(
        host,
        &[(SECTION_LISTINGS, LISTINGS), (SECTION_AVAILABILITY, AVAILABILITY)],
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return Err(Response::internal_error(e)),
    };
    backup::export_signed_bundle(host, owner, SCHEMA_VERSION, sections, now).await
}

pub(crate) async fn import<H: AppHost>(host: &H, input: ImportInput) -> Reply<CatalogImportOutput> {
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

    let mut prepared: Vec<(&'static str, Vec<Mutation>)> = Vec::new();
    for (name, records) in &bundle.sections {
        let collection = match name.as_str() {
            SECTION_LISTINGS => LISTINGS,
            SECTION_AVAILABILITY => AVAILABILITY,
            other => return Err(Response::invalid_params(format!("unknown section '{other}'"))),
        };
        let mut muts = Vec::new();
        for rec in records {
            let id = match rec.get("id").and_then(Value::as_str) {
                Some(i) => i.to_string(),
                None => return Err(Response::invalid_params("record missing id")),
            };
            let payload_val = match rec.get("payload") {
                Some(p) => p.clone(),
                None => return Err(Response::invalid_params("record missing payload")),
            };
            if name == SECTION_LISTINGS {
                let env_str = match payload_val.get("envelope").and_then(Value::as_str) {
                    Some(s) => s,
                    None => {
                        return Err(Response::invalid_params("listing record missing envelope"));
                    }
                };
                let verified = match verify_json(env_str, &VerifyOptions::new(now)) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(Response::invalid_params(format!(
                            "listing record '{id}' failed verification: {e}"
                        )));
                    }
                };
                if verified.record_type != RECORD_LISTING
                    || verified.version != listing::LISTING_VERSION
                {
                    return Err(Response::invalid_params("record is not a listing"));
                }
                if verified.issuer != owner {
                    return Err(Response::invalid_params(format!(
                        "listing record '{id}' was signed by '{}', not this node's owner",
                        verified.issuer
                    )));
                }
            }
            let payload_bytes = match serde_json::to_vec(&payload_val) {
                Ok(b) => b,
                Err(e) => return Err(Response::internal_error(e.to_string())),
            };
            muts.push(Mutation::Put(RecordWriteValue { id, payload: payload_bytes }));
        }
        prepared.push((collection, muts));
    }

    let mut counts = BTreeMap::new();
    for (collection, muts) in prepared {
        if let Err(e) = ensure_coll(host, collection, &[]).await {
            return Err(Response::internal_error(e));
        }
        counts.insert(collection.to_string(), muts.len());
        for chunk in muts.chunks(100) {
            if let Err(e) =
                AppDataLayer::batch_mutate(host, collection.to_string(), chunk.to_vec()).await
            {
                return Err(Response::internal_error(e.to_string()));
            }
        }
    }
    Ok(CatalogImportOutput { imported: counts })
}

use syneroym_roym_core::backup;

use super::*;

pub(crate) async fn export<H: AppHost>(host: &H) -> Response {
    let owner = match signing::owner_did(host).await {
        Ok(o) => o,
        Err(e) => return Response::internal_error(e.to_string()),
    };
    let now = clock::now_secs();
    if let Err(e) = ensure_listings(host).await {
        return Response::internal_error(e);
    }
    if let Err(e) = ensure_availability(host).await {
        return Response::internal_error(e);
    }
    let sections = match backup::collect_sections(
        host,
        &[(SECTION_LISTINGS, LISTINGS), (SECTION_AVAILABILITY, AVAILABILITY)],
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

    let now = clock::now_secs();
    let mut prepared: Vec<(&'static str, Vec<Mutation>)> = Vec::new();
    for (name, records) in &bundle.sections {
        let collection = match name.as_str() {
            SECTION_LISTINGS => LISTINGS,
            SECTION_AVAILABILITY => AVAILABILITY,
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
            if name == SECTION_LISTINGS {
                let env_str = match payload_val.get("envelope").and_then(Value::as_str) {
                    Some(s) => s,
                    None => return Response::invalid_params("listing record missing envelope"),
                };
                let verified = match verify_json(env_str, &VerifyOptions::new(now)) {
                    Ok(v) => v,
                    Err(e) => {
                        return Response::invalid_params(format!(
                            "listing record '{id}' failed verification: {e}"
                        ));
                    }
                };
                if verified.record_type != RECORD_LISTING
                    || verified.version != listing::LISTING_VERSION
                {
                    return Response::invalid_params("record is not a listing");
                }
                if verified.issuer != owner {
                    return Response::invalid_params(format!(
                        "listing record '{id}' was signed by '{}', not this node's owner",
                        verified.issuer
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
        // Re-ensure the collection; indexes are added lazily on first
        // regular write, so an import needs none of its own.
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
    Response::ok(json!({ "imported": counts }))
}

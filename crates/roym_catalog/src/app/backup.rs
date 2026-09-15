use super::*;

/// Every row of `collection`, as `{ id, payload }` -- the shape a `Bundle`
/// section holds and `profile.export` uses.
pub(crate) async fn collect<H: AppHost>(host: &H, collection: &str) -> Result<Vec<Value>, String> {
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

pub(super) async fn export<H: AppHost>(host: &H) -> Response {
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
    let listings = match collect(host, LISTINGS).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let availability = match collect(host, AVAILABILITY).await {
        Ok(v) => v,
        Err(e) => return Response::internal_error(e),
    };
    let sections = BTreeMap::from([
        (SECTION_LISTINGS.to_string(), listings),
        (SECTION_AVAILABILITY.to_string(), availability),
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

pub(super) async fn import<H: AppHost>(host: &H, req: &Request) -> Response {
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

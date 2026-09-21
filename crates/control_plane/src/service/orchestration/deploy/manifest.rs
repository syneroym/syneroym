use super::*;

/// Canonical idempotency hash over the manifest and app-context, excluding
/// fields that record *who is writing* or mint-time freshness rather than
/// *what gets installed*: `generation`, each binding's `epoch`, and the
/// instance/registry certificates' fresh-every-mint fields (their `not_
/// after`/signing timestamp would otherwise make the hash differ on every
/// apply regardless of content, since both are re-minted fresh every call).
/// Each certificate is hashed on its *stable* identity fields only.
pub(super) fn compute_deploy_idempotency_hash(
    manifest: &DeployManifest,
    app_context: Option<&AppContext>,
    installed_instance_cert: Option<&DelegationCertificate>,
) -> Result<String, String> {
    let context_for_hash = app_context.map(|c| {
        let bindings: Vec<_> = c
            .bindings
            .iter()
            .map(|b| (&b.dependency_name, &b.app_instance_id, &b.mode, &b.members, b.cache_ttl_ms))
            .collect();
        (&c.app_instance_id, &c.service_name, bindings)
    });
    let instance_cert_for_hash =
        installed_instance_cert.map(|c| (&c.master_did, &c.temporary_did, &c.scope));
    let registry_cert_for_hash =
        manifest.registry_certificate.as_deref().map(stable_registry_certificate_for_hash);
    let manifest_for_hash =
        (&manifest.config, &manifest.service_type, instance_cert_for_hash, registry_cert_for_hash);
    let canonical = serde_json::to_string(&(&manifest_for_hash, &context_for_hash))
        .map_err(|e| format!("Failed to canonicalize deploy manifest for dedup: {e}"))?;
    Ok(blake3::hash(canonical.as_bytes()).to_hex().to_string())
}

/// Flattens `manifest.config.custom_config`'s JSON into dotted-path
/// key/value pairs for storage, and parses its reserved `http_routes` key
/// (see `crate::http_routes`). A declared `schema` is validated against
/// `custom_config` on a blocking thread; the deploy is a hard failure on
/// either a malformed schema or a violation.
pub(super) async fn build_flat_config_and_routes(
    manifest: &DeployManifest,
) -> Result<(BTreeMap<String, String>, Vec<HttpRoute>), String> {
    let mut flat_config = BTreeMap::new();
    let mut http_routes = Vec::new();
    let Some(custom_config_str) = &manifest.config.custom_config else {
        return Ok((flat_config, http_routes));
    };
    let custom_json: Value = serde_json::from_str(custom_config_str)
        .map_err(|e| format!("custom_config is not valid JSON: {e}"))?;
    http_routes = http_routes::parse_http_routes(&custom_json)?;

    if let Some(schema_source) = &manifest.config.schema {
        let schema_str = resolve_document(schema_source, "schema").await?;
        let custom_json_clone = custom_json.clone();
        task::spawn_blocking(move || -> Result<(), String> {
            let schema_json: Value = serde_json::from_str(&schema_str)
                .map_err(|e| format!("JSON schema is not valid JSON: {e}"))?;
            let compiled_schema = jsonschema::validator_for(&schema_json)
                .map_err(|e| format!("Invalid JSON schema: {e}"))?;
            if let Err(error) = compiled_schema.validate(&custom_json_clone) {
                return Err(format!(
                    "Configuration validation failed: {} at {}",
                    error,
                    error.instance_path()
                ));
            }
            Ok(())
        })
        .await
        .map_err(|e| format!("Failed to spawn blocking task: {e}"))??;
    }

    config_utils::flatten_json_config(&custom_json, "", &mut flat_config);
    Ok((flat_config, http_routes))
}

/// A probe kind that cannot address `service_type` is a manifest error --
/// checked before any engine work runs, so it fails at deploy rather than
/// producing a permanently `failing` probe indistinguishable from a real
/// outage.
pub(super) fn validate_health_check_matches_service_type(
    check: &WitHealthCheck,
    service_type: AppServiceType,
) -> Result<(), String> {
    let model = model_health_check(check)?;
    if !model.valid_for().contains(&service_type) {
        return Err(format!(
            "health check '{}' cannot address a '{service_type:?}' service; it is valid for {:?}",
            model.kind_name(),
            model.valid_for()
        ));
    }
    if let HealthCheck::HttpGet(p) = &model
        && !p.path.starts_with('/')
    {
        return Err(format!("http-get probe path '{}' must start with '/'", p.path));
    }
    Ok(())
}

/// An asset bundle, or an http_routes entry targeting `guest`/`websocket`,
/// is only reachable through a `Wasm` service's own HTTP bridge -- a
/// `Tcp`/`Container` service's endpoint is `SubstrateEndpoint::
/// TcpHostPort`, routed to raw `io::copy_bidirectional` passthrough
/// regardless of what the client sends, so neither path is ever reached for
/// one. Declaring either for a non-`Wasm` service would otherwise be
/// silent dead configuration: an unpacked-but-unservable bundle, or a route
/// that never triggers.
pub(super) fn validate_wasm_only_features(
    service_id: &str,
    manifest: &DeployManifest,
    http_routes: &[HttpRoute],
    service_type: AppServiceType,
) -> Result<(), String> {
    if manifest.config.assets.is_some() && service_type != AppServiceType::Wasm {
        return Err(format!(
            "service '{service_id}': an asset bundle is only servable for a 'Wasm' service; a \
             '{service_type:?}' service's endpoint is raw TCP passthrough, which never reaches \
             the asset-serving HTTP path"
        ));
    }
    if http_routes.iter().any(|r| r.target == "guest" || r.target == "websocket")
        && service_type != AppServiceType::Wasm
    {
        return Err(format!(
            "service '{service_id}': an http_routes entry with target=guest is only servable for \
             a 'Wasm' service; a '{service_type:?}' service's endpoint is raw TCP passthrough, \
             which never reaches the guest HTTP path"
        ));
    }
    Ok(())
}

/// Resolves and validates `manifest.config.fdae_policy`, if any. A parse/
/// validation failure is logged in full server-side but reported to the
/// caller only generically -- the underlying `PolicyError`'s `Display`
/// embeds the offending JSON *instance*, which for a policy can be the
/// document's own content (it can now arrive inline from that same
/// caller), so it must never cross back out.
pub(super) async fn resolve_fdae_policy_for_deploy(
    service_id: &str,
    manifest: &DeployManifest,
) -> Result<Option<(String, Arc<Policy>)>, String> {
    let Some(policy_source) = &manifest.config.fdae_policy else { return Ok(None) };
    let doc = resolve_document(policy_source, "fdae_policy").await?;
    let policy = syneroym_fdae::parse_and_validate(&doc).map_err(|e| {
        tracing::warn!("FDAE policy validation failed for service {}: {}", service_id, e);
        "FDAE policy validation failed: invalid policy document".to_string()
    })?;
    Ok(Some((doc, Arc::new(policy))))
}

/// Builds `manifest`'s flattened config/http_routes, then validates its
/// health-check and `Wasm`-only declarations and resolves its FDAE policy --
/// the manifest-driven checks that must all pass before any config
/// generation write happens. Grouped into one call so `deploy_with_context`
/// only needs to thread `service_id`/`manifest`/`service_type` through it,
/// not each of the four steps individually.
pub(super) async fn validate_and_build_deploy_config(
    service_id: &str,
    manifest: &DeployManifest,
    service_type: AppServiceType,
) -> Result<(BTreeMap<String, String>, Vec<HttpRoute>, Option<(String, Arc<Policy>)>), String> {
    let (flat_config, http_routes) = build_flat_config_and_routes(manifest).await?;

    // A probe kind that cannot address this service type is a manifest
    // error, checked before any engine work runs (`service_type` was
    // already computed by the caller, for the idempotency dedup check).
    if let Some(check) = &manifest.config.health_check {
        validate_health_check_matches_service_type(check, service_type)?;
    }

    validate_wasm_only_features(service_id, manifest, &http_routes, service_type)?;

    // FDAE policy: independent of `custom_config` (unlike `schema` above,
    // which is only resolved when a `custom_config` is present). Validation
    // is a hard deploy failure (ADR-0017 §1's "validated at deploy... the
    // Cedar lesson").
    let fdae_policy = resolve_fdae_policy_for_deploy(service_id, manifest).await?;

    Ok((flat_config, http_routes, fdae_policy))
}

/// A `public` guest route is reachable with no verified caller identity
/// over a direct anonymous connection -- logged loudly so an author who
/// didn't mean to leave a route open still has one place to notice, the
/// same treatment the asset bundle's own visibility gets in
/// `unpack_new_assets`.
pub(super) fn log_public_guest_routes(service_id: &str, http_routes: &[HttpRoute]) {
    for route in http_routes.iter().filter(|r| r.target == "guest" && r.public) {
        info!(
            "guest HTTP route for '{service_id}': {} {} declared public -- reachable with no \
             verified caller identity, and its handler still runs with the service's own storage \
             rights",
            route.method, route.path
        );
    }
}

use super::*;

impl ControlPlaneService {
    /// The `deploy` trait method's entire body -- the `deploy` <->
    /// `deploy_with_context` split lets `deploy` pass no app context, so no
    /// bindings, unchanged for every existing caller including the JSON-RPC
    /// `deploy` dispatch. `deploy_plan` calls this directly, passing
    /// `service.app_context`.
    pub(super) async fn deploy_with_context(
        &self,
        service_id: String,
        manifest: DeployManifest,
        app_context: Option<AppContext>,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // `service_id` is joined verbatim into `hosted_apps_dir/<service_id>.json`
        // below (write, and also delete on a private redeploy) --
        // reject anything that could walk that join out of the directory
        // before it is used for anything, including the ownership/capability
        // checks that follow.
        if !is_safe_service_id_for_path(&service_id) {
            return Err(format!(
                "service_id '{service_id}' is not a valid deploy target: it must be non-empty and \
                 contain no '/', '\\\\', or '..' -- it is joined into a stored-record filename"
            ));
        }
        // A deploy may not claim a `service_id` this substrate already uses
        // as a fixed `native_dispatch` key: the node's own DID (`RouteHandler
        // ::init` registers `ControlPlaneService` itself there, so claiming
        // it hijacks every `orchestrator`/`security` call this node ever
        // receives) or the literal `"supervisor"` (the supervisor role's own
        // dispatch id, whose vault a deploy under that name would also open
        // via `open_service_db`). Neither `SERVICE_ID_REGEX` nor
        // `validate_service_id` reserves either string -- both are ordinary,
        // deployable-looking ids otherwise. Checked before ownership/
        // capability below so the rejection reason is unambiguous.
        if service_id == self.node_did
            || service_id == SUPERVISOR_RESERVED_SERVICE_ID
            || service_id == AUTH_RESERVED_SERVICE_ID
        {
            return Err(format!(
                "service_id '{service_id}' is reserved for this substrate's own dispatch and \
                 cannot be deployed to"
            ));
        }

        // A service_id already owned by someone else may
        // not be re-deployed into. An unowned substrate holds no node-wide
        // orchestrator authority, so this always
        // enforces the takeover check there -- only an owned substrate's
        // owner can override it, and today's overwrite-on-redeploy behavior
        // is preserved exactly for that case. Checks ORCHESTRATOR_DEPLOY
        // specifically, not a single catch-all ability: a caller who holds
        // only `orchestrator/status` must not be able to override someone
        // else's takeover protection just because they can also list every
        // app.
        //
        // TOCTOU note (reviewed, accepted): this read and the terminal
        // `set_owner` write below are separated by the whole deploy body,
        // not atomic. Two concurrent *first* deploys of the same brand-new
        // `service_id` from different DIDs can both observe `owner_of ==
        // None` and both proceed -- whichever `set_owner` call lands last
        // wins attribution. This cannot defeat an *existing* owner's
        // protection (a service that already has a recorded owner is
        // rejected deterministically regardless of timing, since the row
        // predates both racing calls), so it is an attribution race on a
        // service_id nobody owns yet, not a takeover-check bypass. Not fixed
        // here: closing it fully needs a per-service_id lock or an atomic
        // claim-then-verify around the entire (non-atomic, pre-existing)
        // deploy flow, which is a larger change than this one.
        if let Some(existing) = self.registry.owner_of(&service_id)
            && existing != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {existing}; redeploy must come from its owner \
                 or a substrate owner"
            ));
        }

        // Tier-1 deploy admission. The caller must
        // hold `orchestrator/deploy` covering this app. No owner/unowned
        // branch and no separate substrate-owner bypass here: a bare
        // `substrate:<node>` capability (the owner's `substrate/admin`) is
        // `is_substrate_scope`, so `grants` wildcards the resource and only
        // `entails` has to hold -- that passes here for free. An app-scoped
        // grantee is prefix-covered instead. One check, two principals,
        // no branch. (An unowned substrate holds neither shape of
        // capability, so this denies unconditionally
        // there unless the caller holds an app-scoped grant -- which
        // nothing can issue on an unowned substrate either, so deploy is
        // simply unreachable until ownership is established.)
        let deploy_resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{service_id}' on this substrate",
                caller.caller_did
            ));
        }

        // ADR-0020 §1 install-time verification, placed before any artifact
        // work below so a bad certificate is rejected at deploy rather than
        // discovered later as a routing failure. `None` leaves the service
        // its own master -- the pre-existing fallback, unchanged.
        let installed_instance_cert: Option<DelegationCertificate> =
            match &manifest.instance_certificate {
                Some(cert_json) => Some(self.verify_installed_instance_cert(
                    &caller.caller_did,
                    &service_id,
                    cert_json,
                )?),
                None => None,
            };

        let validated_visibility = validate_publication(
            &service_id,
            manifest.config.visibility,
            manifest.registry_certificate.as_deref(),
        )?;

        // ADR-0021 §2: validated here, before the artifact work,
        // for the same reason the certificate is -- a malformed or
        // unauthorized binding is a deploy failure, not a routing failure
        // discovered later. The write itself is deferred past every other
        // fallible step below (schema validation, FDAE policy, artifact
        // delivery, `deploy_wasm_service`/`deploy_tcp_service`/
        // `deploy_container_service`): see `install_app_context`, called
        // near owner attribution. A deploy that fails after validating here
        // but before that call must not leave a binding installed for a
        // service that never actually started.
        let prepared_app_context: Option<PreparedAppContext> = if let Some(ctx) = &app_context {
            // Validate before anything touches storage, so a later read of
            // these rows can only fail on real corruption rather than on
            // something a deploy caller sent. The registry itself stores
            // plain `String`s, so this is the only place the shape
            // can be enforced on the way in.
            let instance_id = AppInstanceId::try_new(&ctx.app_instance_id)
                .map_err(|e| format!("app context names an invalid app instance id: {e}"))?;
            LogicalServiceName::try_new(&ctx.service_name)
                .map_err(|e| format!("app context names an invalid service name: {e}"))?;

            // An app instance's first successful deploy becomes its owner
            // (first-write-wins, the same shape `service_id` ownership uses
            // just above -- including that check's own F7 note: an unowned
            // substrate holds no node-wide orchestrator authority, so
            // `has_node_wide_ability` only short-circuits
            // this for an *owned* substrate's owner, same as the
            // `service_id` check above. Also the same accepted
            // TOCTOU gap: this read
            // and the generation-gate persist just below are separated by
            // the whole deploy body, so two concurrent *first* deploys
            // claiming the same brand-new app instance id can both observe
            // `app_instance_management_of == None` and race -- whichever
            // write lands last wins. Same reasoning as the `service_id`
            // note: this cannot defeat an *existing* owner's protection,
            // only decide attribution on an app instance nobody owns yet).
            // Without the check itself, the equality check above is not
            // enough on its own: any caller authorized to deploy *some*
            // service could still name an existing, unrelated app instance
            // in its own `app_context` and overwrite the bindings that
            // instance's other services resolve -- it would just have to
            // also lie about which app instance its own service belongs
            // to, which costs it nothing.
            if let Some(existing) =
                self.registry.app_instance_management_of(&ctx.app_instance_id).map(|m| m.owner_did)
                && existing != caller.caller_did
                && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
            {
                return Err(format!(
                    "app instance '{}' is owned by {existing}; a deploy that joins it must come \
                     from its owner or a substrate owner",
                    ctx.app_instance_id
                ));
            }

            // ADR-0021 §4's generation gate: persisted
            // immediately, before binding validation or any artifact work,
            // so a manager is recorded even if a later step in this deploy
            // fails -- this write records *who is writing*, not
            // what was installed.
            let management = self.check_generation(&ctx.app_instance_id, caller, ctx.generation)?;
            self.registry
                .set_app_instance_management(ctx.app_instance_id.clone(), management)
                .await
                .map_err(|e| e.to_string())?;

            let mut bindings = Vec::with_capacity(ctx.bindings.len());
            for binding in &ctx.bindings {
                let (dependency_name, entry) = prepare_binding(binding, &ctx.app_instance_id)?;
                bindings.push((binding.dependency_name.clone(), dependency_name, entry));
            }

            Some(PreparedAppContext {
                instance_id,
                raw_instance_id: ctx.app_instance_id.clone(),
                raw_service_name: ctx.service_name.clone(),
                bindings,
            })
        } else {
            None
        };

        // Deploy idempotency, distinct from the epoch guard (dedups
        // binding writes) and the generation gate (picks between writers)
        // -- ADR-0021 §3 says explicitly that neither covers the other.
        // Canonical hash over
        // (manifest, app_context-minus-generation); the generation is
        // excluded deliberately, since bumping it is a change of *writer*,
        // not a change to the deployed service, and hashing it would make
        // an `adopt` force a pointless reinstall of every service.
        let service_type = app_service_type(&manifest.service_type);
        // Every rollback below re-enters through
        // `self.undeploy`, now generation-gated -- send the same
        // generation this deploy itself presented, so a rollback of the
        // supervisor's own deploy is never rejected by its own gate.
        let generation = app_context.as_ref().map_or(0, |c| c.generation);
        // `epoch` is `generation`'s sibling, not its
        // opposite -- it too records who is writing (the supervisor
        // advances it before every apply), not what gets
        // installed. Hashing it raw meant every re-apply changed the
        // hash and forced a genuine reinstall of every dependent, which
        // is exactly the restart `write-bindings` exists to avoid. Each
        // binding is hashed minus its `epoch`, by the same reasoning the
        // idempotency hash already applies to `generation` above.
        let context_for_hash = app_context.as_ref().map(|c| {
            let bindings: Vec<_> = c
                .bindings
                .iter()
                .map(|b| {
                    (&b.dependency_name, &b.app_instance_id, &b.mode, &b.members, b.cache_ttl_ms)
                })
                .collect();
            (&c.app_instance_id, &c.service_name, bindings)
        });
        // The earlier `epoch` fix was inert on its own: `manifest.instance_
        // certificate`/`registry_certificate` are minted fresh on every
        // apply -- `certify_placed_members` calls `certify_instance` and
        // builds an `EndpointInfo` whose `not_after` is derived from
        // `SystemTime::now()`, both landing in this same manifest
        // (`sdk::mapper`). Hashing the raw blobs meant those two fields
        // alone made the hash differ on every apply, epoch notwithstanding
        // -- the no-op branch below was unreachable from either real
        // deploy path (`roymctl app deploy` mints per call too). Dropping
        // both fields entirely is not the fix either: a certificate going
        // from installed to absent (or naming a different master/key) is
        // a real content change, not freshness churn, and must still
        // reinstall -- `a_redeploy_without_a_certificate_clears_a_
        // previously_installed_one` pins exactly that. So each is hashed
        // on its *stable* identity fields only: `installed_instance_cert`
        // is already parsed and verified above, so its `master_did`/
        // `temporary_did`/`scope` are reused directly rather than
        // re-parsing the raw JSON; `registry_certificate` has no earlier
        // parse to reuse, so `stable_registry_certificate_for_hash` does
        // its own, falling back to the raw string (never less safe, only
        // less deduplicating) if it does not parse.
        let instance_cert_for_hash =
            installed_instance_cert.as_ref().map(|c| (&c.master_did, &c.temporary_did, &c.scope));
        let registry_cert_for_hash =
            manifest.registry_certificate.as_deref().map(stable_registry_certificate_for_hash);
        let manifest_for_hash = (
            &manifest.config,
            &manifest.service_type,
            instance_cert_for_hash,
            registry_cert_for_hash,
        );
        let incoming_hash = {
            let canonical = serde_json::to_string(&(&manifest_for_hash, &context_for_hash))
                .map_err(|e| format!("Failed to canonicalize deploy manifest for dedup: {e}"))?;
            blake3::hash(canonical.as_bytes()).to_hex().to_string()
        };
        // The owner check: the idempotency case is "a retry after a lost
        // response" -- the *same* caller re-sending a request whose
        // response never arrived. A *different* caller presenting
        // byte-identical content is a takeover, not a retry, and
        // `set_owner` below must still run unconditionally for it -- a
        // dedup that skipped straight to `Ok(())` here would silently
        // leave the service owned by whoever deployed it first.
        // `full_deploy_completed` is the witness that *this* process (not
        // just a past one) already registered this service's routing. The
        // route tables it stands for are process-local and empty on every
        // boot, and the sandbox warm-up restores only the WASM instance --
        // so without this clause a redeploy right after a substrate
        // restart (matching persisted `manifest_hash`, warmed `Running`
        // instance) would dedup into a no-op and leave guest `POST /rpc`
        // and every native-capability call unrouted until some later
        // *content* change forced a real deploy. A fresh boot has no
        // entry, so its redeploy falls through and re-registers every
        // table below. See the field's doc comment.
        if self.full_deploy_completed.contains_key(&service_id)
            && self.registry.deploy_facts(&service_id).and_then(|(_, _, hash, _)| hash).as_deref()
                == Some(incoming_hash.as_str())
            && self.registry.owner_of(&service_id).as_deref().is_none_or(|o| o == caller.caller_did)
            && !matches!(
                self.instance_phase(&service_id, Some(service_type_str(service_type))).await,
                InstancePhase::NotRunning(_) | InstancePhase::NotFound
            )
        {
            // A retry after a lost response: nothing changed and the
            // instance is up, so this is a no-op that reports success --
            // not a reinstall that restarts a healthy service.
            info!("deploy for '{service_id}' is identical to what is installed and running; no-op");
            return Ok(());
        }

        match &manifest.registry_certificate {
            Some(cert) => {
                let cert_path = self.hosted_apps_dir.join(format!("{service_id}.json"));
                if let Err(e) = fs::write(&cert_path, cert) {
                    tracing::warn!("Failed to save registry certificate for {}: {}", service_id, e);
                } else {
                    tracing::debug!(
                        "Saved registry certificate for {} at {}",
                        service_id,
                        cert_path.display()
                    );
                }
            }
            None => {
                let cert_path = self.hosted_apps_dir.join(format!("{service_id}.json"));
                if cert_path.exists()
                    && let Err(e) = fs::remove_file(&cert_path)
                {
                    tracing::warn!(
                        "failed to remove the stored endpoint record for {service_id} after a \
                         private redeploy; it may keep being republished: {e}"
                    );
                }
            }
        }

        // Configuration Generation & Validation
        let mut flat_config = BTreeMap::new();
        // `http_routes` is a reserved top-level key inside
        // `custom_config`'s JSON (see `crate::http_routes`) -- parsed here,
        // alongside the existing flatten step, since this is already the
        // one place `custom_config` gets interpreted rather than treated as
        // opaque. A malformed `http_routes` value fails deploy the same way
        // a schema violation does, rather than silently discarding routes.
        let mut http_routes = Vec::new();
        if let Some(custom_config_str) = &manifest.config.custom_config {
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
        }

        // A probe kind that cannot address this service type is a
        // manifest error, checked before any engine work runs. Accepting it
        // would produce a permanently `failing` probe that is
        // indistinguishable, at the supervisor, from a real outage.
        // (`service_type` was already computed above, for the idempotency
        // dedup check.)
        if let Some(check) = &manifest.config.health_check {
            let model = model_health_check(check)?;
            if !model.valid_for().contains(&service_type) {
                return Err(format!(
                    "health check '{}' cannot address a '{service_type:?}' service; it is valid \
                     for {:?}",
                    model.kind_name(),
                    model.valid_for()
                ));
            }
            if let HealthCheck::HttpGet(p) = &model
                && !p.path.starts_with('/')
            {
                return Err(format!("http-get probe path '{}' must start with '/'", p.path));
            }
        }

        // An asset bundle is only reachable through a `Wasm`
        // service's `NativeService` HTTP path (`try_handle_asset`,
        // `crates/router/src/route_handler/http.rs`) -- a `Tcp`/`Container`
        // service's endpoint is registered as `SubstrateEndpoint::
        // TcpHostPort`, which `dispatch.rs`'s `(_, TcpHostPort { .. })` arm
        // unconditionally routes to raw `io::copy_bidirectional` passthrough
        // regardless of what protocol the client actually speaks -- the
        // asset-serving HTTP bridge is never reached for one, even when the
        // client sends literal HTTP bytes. Without this check, a `Tcp`/
        // `Container` deploy with `assets` set unpacked and stored a bundle
        // that could never be served, silently: a wasted blob write with no
        // signal to the caller. Also matches the CLI's existing
        // `--asset-visibility requires --assets requires --wasm` chain
        // (`apps/roymctl/src/commands/svc.rs`) and this fact:
        // `Tcp`/`Container` services already run their own web
        // server outside the substrate, which is exactly what asset
        // bundles exist to stop being the only way to serve a web app --
        // they have no need
        // for this feature, not just no support for it yet.
        if manifest.config.assets.is_some() && service_type != AppServiceType::Wasm {
            return Err(format!(
                "service '{service_id}': an asset bundle is only servable for a 'Wasm' service; a \
                 '{service_type:?}' service's endpoint is raw TCP passthrough, which never \
                 reaches the asset-serving HTTP path"
            ));
        }

        // Same reasoning as the asset-bundle check above, for a `guest` or
        // `websocket` route -- a `Tcp`/`Container` service's endpoint is
        // `SubstrateEndpoint::TcpHostPort`, routed to raw `copy_bidirectional`
        // passthrough regardless of what the client sends, so the guest HTTP/
        // WebSocket bridge is structurally unreachable for one. Without this a
        // declared `guest` or `websocket` route would be silent dead configuration.
        if http_routes.iter().any(|r| r.target == "guest" || r.target == "websocket")
            && service_type != AppServiceType::Wasm
        {
            return Err(format!(
                "service '{service_id}': an http_routes entry with target=guest is only servable \
                 for a 'Wasm' service; a '{service_type:?}' service's endpoint is raw TCP \
                 passthrough, which never reaches the guest HTTP path"
            ));
        }

        // FDAE policy: independent of `custom_config` (unlike `schema`
        // above, which is only resolved when a `custom_config` is present) --
        // deliberately not nested inside the block above, since a policy has
        // nothing to do with config-schema validation. Validation is a hard
        // deploy failure (ADR-0017 §1's "validated at deploy... the Cedar
        // lesson").
        let fdae_policy: Option<(String, Arc<Policy>)> = if let Some(policy_source) =
            &manifest.config.fdae_policy
        {
            let doc = resolve_document(policy_source, "fdae_policy").await?;
            // The underlying `PolicyError` embeds the offending JSON
            // *instance* (jsonschema's `ValidationError::Display`) --
            // for a policy that instance can be the document's own
            // content (unlike `schema`, where the instance is always the
            // caller's own `custom_config`), so it must never cross back
            // out to the remote deploy caller. This matters more now that
            // the document can arrive inline from that same caller.
            // Logged in full server-side; the caller gets a generic
            // failure.
            let policy = syneroym_fdae::parse_and_validate(&doc).map_err(|e| {
                tracing::warn!("FDAE policy validation failed for service {}: {}", service_id, e);
                "FDAE policy validation failed: invalid policy document".to_string()
            })?;
            Some((doc, Arc::new(policy)))
        } else {
            None
        };

        let config_blob = serde_json::to_string(&flat_config)
            .map_err(|e| format!("Failed to serialize flattened config: {e}"))?;

        let new_gen = self
            .storage_provider
            .save_config_generation(&service_id, &config_blob)
            .await
            .map_err(|e| format!("Failed to save config generation: {e}"))?;
        tracing::info!("Saved configuration generation {} for service {}", new_gen, service_id);

        // Persist before the service is actually instantiated below, so the
        // `init`/`migrate` lifecycle hook's first read already sees the row.
        // Last-write-wins (no generation ladder, unlike config generations
        // above) -- a policy edit binds late by design.
        //
        // `previous_fdae_policy` captures whatever was there *before* this
        // deploy's write, for `rollback_fdae_policy` below, unconditionally
        // and in both directions (a new/changed policy, or the manifest
        // dropping the block entirely). Unlike config generations
        // (append-only, so rolling back a failed attempt's row never
        // touches an earlier one), `fdae_policies` is a single
        // last-write-wins row per service -- on a re-deploy, a later step
        // failing must restore the *previous* policy exactly, or an
        // already-running previous version loses its policy to an
        // unrelated failed re-deploy attempt the next time its engine cache
        // re-resolves from storage. This applies just as much when the new
        // manifest drops the policy block: capturing `previous` only in the
        // save branch would let a later-step failure leave a deleted policy
        // deleted, silently reopening the previous version's enforcement.
        let previous_fdae_policy = self
            .storage_provider
            .load_fdae_policy(&service_id)
            .await
            .map_err(|e| format!("Failed to check existing FDAE policy: {e}"))?;
        if let Some((policy_doc, _)) = &fdae_policy {
            self.storage_provider
                .save_fdae_policy(&service_id, policy_doc)
                .await
                .map_err(|e| format!("Failed to save FDAE policy: {e}"))?;
        } else {
            // A manifest that no longer declares `fdae_policy` clears
            // any previously-declared policy -- a deploy's `config` fully
            // declares this service's policy state, so absence means
            // explicit removal, not "leave whatever was there" (the F2
            // resurrection bug: without this, `AppSandboxEngine::
            // resolve_fdae_policy` would reload the stale row on its next
            // cache miss even though native dispatch has correctly gone
            // unfiltered).
            self.storage_provider
                .delete_fdae_policy(&service_id)
                .await
                .map_err(|e| format!("Failed to clear FDAE policy: {e}"))?;
        }

        // Static asset bundle unpack, before the wasm/tcp/container
        // dispatch below so a bad archive fails deploy the same way a bad
        // FDAE policy does -- before anything guest-visible has started.
        //
        // `old_assets` is read now, before any mutation: it is the only
        // point that can see the still-live previous generation, which the
        // backward rollback below (any failure between here and the
        // registry commit further down) must keep, and which the forward
        // cleanup at the commit point must diff against.
        let old_assets = self.assets.get(&service_id).map(|entry| entry.value().clone());
        let mut written_asset_hashes = BTreeSet::new();
        let new_assets: Option<ServiceAssets> = if let Some(bundle) = &manifest.config.assets {
            let archive = match resolve_asset_archive(&bundle.archive) {
                Ok(a) => a,
                Err(e) => {
                    // Nothing has been written yet at this point, but the
                    // FDAE policy and config generation above already have
                    // been -- roll those back the same as every later
                    // failure branch in this block, or a redeploy whose
                    // manifest merely points at an unsupported archive
                    // source silently drops the still-running previous
                    // version's policy.
                    self.rollback_asset_bundle(
                        &service_id,
                        &written_asset_hashes,
                        old_assets.as_ref(),
                    )
                    .await;
                    self.rollback_config_generation(&service_id, new_gen).await;
                    self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
                    return Err(e);
                }
            };
            let dek =
                match self.storage_provider.load_service_dek(&service_id, &self.key_store).await {
                    Ok(d) => d,
                    Err(e) => {
                        self.rollback_asset_bundle(
                            &service_id,
                            &written_asset_hashes,
                            old_assets.as_ref(),
                        )
                        .await;
                        self.rollback_config_generation(&service_id, new_gen).await;
                        self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
                        return Err(format!("Failed to resolve service DEK: {e}"));
                    }
                };
            let unpacked = assets::unpack_asset_bundle(
                &service_id,
                &archive,
                bundle.hash.as_deref(),
                &http_routes,
                &self.blob_provider,
                dek.clone(),
                &mut written_asset_hashes,
            )
            .await;
            let asset_manifest = match unpacked {
                Ok(m) => m,
                Err(e) => {
                    self.rollback_asset_bundle(
                        &service_id,
                        &written_asset_hashes,
                        old_assets.as_ref(),
                    )
                    .await;
                    self.rollback_config_generation(&service_id, new_gen).await;
                    self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
                    return Err(format!("Asset bundle unpack failed: {e}"));
                }
            };
            let manifest_hash = match assets::store_manifest(
                &service_id,
                &asset_manifest,
                &self.blob_provider,
                dek,
            )
            .await
            {
                Ok(h) => h,
                Err(e) => {
                    self.rollback_asset_bundle(
                        &service_id,
                        &written_asset_hashes,
                        old_assets.as_ref(),
                    )
                    .await;
                    self.rollback_config_generation(&service_id, new_gen).await;
                    self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
                    return Err(format!("Asset manifest storage failed: {e}"));
                }
            };
            written_asset_hashes.insert(manifest_hash.clone());
            let public = matches!(bundle.visibility.as_ref(), Some(WitVisibility::Public));
            // A caller who forgets to declare `public` gets 404s
            // with no signal anywhere unless this is logged -- absence of
            // an explicit `visibility` defaults to `private` by
            // construction (the wire's `option<visibility>`), which is
            // deliberately silent at the *serving* layer (a miss
            // and a non-public bundle look identical to a caller), so the
            // one place left to say so is here, at deploy time.
            info!(
                "asset bundle for '{service_id}': {} entries, visibility {}",
                asset_manifest.entries.len(),
                match bundle.visibility.as_ref() {
                    Some(WitVisibility::Public) => "public",
                    Some(WitVisibility::Internal) => "internal",
                    Some(WitVisibility::Private) | None => "private",
                }
            );
            Some(ServiceAssets { manifest: Arc::new(asset_manifest), public, manifest_hash })
        } else {
            None
        };

        // A `public` guest route is reachable with no verified
        // caller identity over a direct anonymous connection -- the same
        // loud-signal treatment the asset bundle's own visibility gets
        // above, so an author who didn't mean to leave a route open still
        // has one place to notice.
        for route in http_routes.iter().filter(|r| r.target == "guest" && r.public) {
            info!(
                "guest HTTP route for '{service_id}': {} {} declared public -- reachable with no \
                 verified caller identity, and its handler still runs with the service's own \
                 storage rights (M06A D-A2-7)",
                route.method, route.path
            );
        }

        let new_fdae_policy = fdae_policy.as_ref().map(|(_, policy)| policy.as_ref());
        match &manifest.service_type {
            WitServiceType::Wasm(wasm_manifest) => {
                if let Err(e) = self
                    .deploy_wasm_service(
                        &service_id,
                        &manifest,
                        wasm_manifest,
                        new_gen,
                        &previous_fdae_policy,
                        new_fdae_policy,
                        &http_routes,
                    )
                    .await
                {
                    // The wasm/tcp/container helpers already roll back the
                    // config generation and FDAE policy themselves before
                    // returning `Err` -- only the asset-bundle rollback is
                    // new here, so it must not be duplicated inside them.
                    self.rollback_asset_bundle(
                        &service_id,
                        &written_asset_hashes,
                        old_assets.as_ref(),
                    )
                    .await;
                    return Err(e);
                }
            }
            WitServiceType::Tcp(tcp_manifest) => {
                if let Err(e) = self
                    .deploy_tcp_service(
                        &service_id,
                        tcp_manifest,
                        new_gen,
                        &previous_fdae_policy,
                        new_fdae_policy,
                    )
                    .await
                {
                    self.rollback_asset_bundle(
                        &service_id,
                        &written_asset_hashes,
                        old_assets.as_ref(),
                    )
                    .await;
                    return Err(e);
                }
            }
            WitServiceType::Container(container_manifest) => {
                if let Err(e) = self
                    .deploy_container_service(
                        &service_id,
                        &manifest,
                        container_manifest,
                        new_gen,
                        &previous_fdae_policy,
                        new_fdae_policy,
                    )
                    .await
                {
                    self.rollback_asset_bundle(
                        &service_id,
                        &written_asset_hashes,
                        old_assets.as_ref(),
                    )
                    .await;
                    return Err(e);
                }
            }
        }

        // Author-time `strict:` warning: the service's own
        // database is the collection inventory (a manifest declares no
        // collection list -- collections come from the guest's `init()` or
        // native calls), so this is the first point at which a first
        // deploy's `init()` has created its tables. Warn-only in both
        // directions, never a deploy failure.
        if let Some((_, policy)) = &fdae_policy {
            warn_on_ambiguous_public_permission(&service_id, policy);
            match self.storage_provider.open_service_db(&service_id, &self.key_store).await {
                Ok(store) => match store.list_collections().await {
                    Ok(collections) => {
                        warn_on_policy_collection_mismatch(&service_id, policy, &collections)
                    }
                    Err(e) => tracing::warn!(
                        "Failed to list collections for FDAE strict-mode check on {}: {}",
                        service_id,
                        e
                    ),
                },
                Err(e) => tracing::warn!(
                    "Failed to open service db for FDAE strict-mode check on {}: {}",
                    service_id,
                    e
                ),
            }
        }

        // Data-layer/vault/app-config/blob-store access is a host-provided
        // capability orthogonal to how the service's own business logic
        // runs (wasm/container/tcp), so every deployed service gets a
        // native-callable channel for it regardless of type.
        for interface in NATIVE_CAPABILITY_INTERFACES {
            if let Err(e) = self
                .registry
                .register(
                    service_id.clone(),
                    interface.to_string(),
                    SubstrateEndpoint::NativeHostChannel { service_id: service_id.clone() },
                )
                .await
            {
                if let Err(undeploy_err) =
                    self.undeploy(service_id.clone(), generation, caller).await
                {
                    tracing::error!(
                        "Failed to roll back partially deployed service {} after native \
                         capability registration error: {}",
                        service_id,
                        undeploy_err
                    );
                }
                // `undeploy` above only cleans up whatever the registry
                // already held (the *old* generation, if any) -- it knows
                // nothing about this attempt's own writes, so they need
                // their own rollback here too.
                self.rollback_asset_bundle(&service_id, &written_asset_hashes, old_assets.as_ref())
                    .await;
                self.rollback_config_generation(&service_id, new_gen).await;
                self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
                return Err(format!("Native capability registration failed: {e}"));
            }
        }
        if let Some(native_dispatch) = self.native_dispatch.upgrade() {
            let native_service = Arc::new(SynSvcNativeService::new(
                service_id.clone(),
                self.key_store.clone(),
                self.storage_provider.clone(),
                self.blob_provider.clone(),
                self.messaging_broker.clone(),
                fdae_policy.as_ref().map(|(_, policy)| policy.clone()),
                self.node_identity.clone(),
                &caller.caller_did,
                self.current_service_proxy(),
                self.current_row_authorizer(),
                installed_instance_cert.clone(),
            ));
            native_service.set_conversation(self.current_conversation());
            native_service.set_record_signer_from(self);
            native_dispatch.insert(service_id.clone(), native_service as Arc<dyn NativeService>);
        } else {
            tracing::error!(
                "Native dispatch registry unavailable for service {}: registered its native \
                 capability endpoints but could not insert a dispatch entry, so calls into them \
                 will fail",
                service_id
            );
        }
        if http_routes.is_empty() {
            self.http_routes.remove(&service_id);
        } else {
            self.http_routes.insert(service_id.clone(), http_routes);
        }
        match &new_assets {
            Some(sa) => {
                self.assets.insert(service_id.clone(), sa.clone());
            }
            None => {
                self.assets.remove(&service_id);
            }
        }
        // Forward cleanup: remove whatever the *old* manifest held
        // that the *new* one (if any) no longer references -- never a
        // wholesale delete of the old bundle, since unchanged files share
        // hashes across generations. Best-effort: a GC failure here must
        // not fail an otherwise-successful deploy.
        if let Some(old) = &old_assets {
            let remove = assets::hashes_of(&old.manifest, Some(&old.manifest_hash));
            let keep = new_assets
                .as_ref()
                .map(|sa| assets::hashes_of(&sa.manifest, Some(&sa.manifest_hash)))
                .unwrap_or_default();
            if let Err(e) =
                assets::delete_hashes(&service_id, &remove, &keep, &self.blob_provider).await
            {
                tracing::warn!(
                    "Failed to garbage-collect the previous asset bundle for service {}: {}",
                    service_id,
                    e
                );
            }
        }

        // Record the owner last, after every other step
        // succeeded. Every earlier failure path above either never reached
        // this line, or calls `undeploy` (whose rollback is itself safe --
        // see the doc comment there), so a crash/failure before this point
        // never leaves a stale owner row. Writing it first would leak an
        // owner row on the `deploy_wasm_service`/`deploy_container_service`
        // failure paths, which only roll back the config generation and any
        // FDAE policy this deploy touched.
        //
        // Reviewed: on a *re-deploy* of an already-owned, already-running
        // service, a `set_owner` failure here rolls back via a full
        // `undeploy` -- tearing the service down entirely rather than
        // restoring the previous running version, since the new
        // wasm/container/tcp version was already swapped in above before
        // this line ever runs. This is not a new gap this slice introduces:
        // the native-capability-registration failure branch a few lines up
        // (`self.undeploy(...)` after the `registry.register` loop) already
        // does the exact same full-teardown rollback for the exact same
        // reason, and predates the ownership gate. `deploy` has never been
        // transactional across config-generation / engine / registry
        // writes; making a re-deploy's late failure preserve the prior
        // running version would need a genuinely versioned/staged deploy
        // (keep the old instance live until the new one fully commits),
        // which is a materially larger change -- not attempted here.
        if let Err(e) = self.registry.set_owner(service_id.clone(), caller.caller_did.clone()).await
        {
            if let Err(undeploy_err) = self.undeploy(service_id.clone(), generation, caller).await {
                tracing::error!(
                    "rollback after owner-attribution failure also failed: {undeploy_err}"
                );
            }
            self.rollback_config_generation(&service_id, new_gen).await;
            self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
            return Err(format!("Owner attribution failed: {e}"));
        }

        // Installed right after the owner row, under the same rollback:
        // already verified above, so `Some` only fails on a storage error.
        // `None` clears any certificate a previous deploy of this
        // service_id installed -- the WIT contract's "absent leaves the
        // service its own master" (control-plane.wit) must hold on every
        // deploy, not only the first, or a redeploy that drops `--master`
        // silently keeps presenting the stale certificate's now-mismatched
        // `temporary_did` on outbound guest calls.
        let cert_result = match installed_instance_cert {
            Some(cert) => self.registry.set_instance_cert(service_id.clone(), cert).await,
            None => self.registry.remove_instance_cert(&service_id).await,
        };
        if let Err(e) = cert_result {
            if let Err(undeploy_err) = self.undeploy(service_id.clone(), generation, caller).await {
                tracing::error!(
                    "rollback after instance-certificate installation failure also failed: \
                     {undeploy_err}"
                );
            }
            self.rollback_config_generation(&service_id, new_gen).await;
            self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
            return Err(format!("Instance certificate installation failed: {e}"));
        }

        // What this deploy said the service is, and its declared
        // probe if any. Stored as the **wire** variant's own JSON (not
        // `model_check`, which only exists for the `valid_for`/`kind_name`
        // validation above and serializes under a different serde config) --
        // `run_probe` deserializes back into the same wire type it reads
        // here, so the two must agree on shape. No upsert-or-clear branch
        // like the certificate above -- the type is always present, and a
        // redeploy that drops the probe writes a row with a `NULL`
        // `health_check_json`, clearing it by construction.
        let health_check_json = manifest
            .config
            .health_check
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| format!("Failed to serialize health check: {e}"))?;
        if let Err(e) = self
            .registry
            .set_deploy_facts(
                service_id.clone(),
                service_type_str(service_type).to_string(),
                health_check_json,
                Some(incoming_hash.clone()),
                Some(validated_visibility.as_str().to_string()),
            )
            .await
        {
            if let Err(undeploy_err) = self.undeploy(service_id.clone(), generation, caller).await {
                tracing::error!(
                    "rollback after deploy-facts installation failure also failed: {undeploy_err}"
                );
            }
            self.rollback_config_generation(&service_id, new_gen).await;
            self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
            return Err(format!("Deploy facts installation failed: {e}"));
        }

        info!(
            "service '{service_id}' deployed with visibility '{}'",
            validated_visibility.as_str()
        );

        // A2 write (finding 03/post-review fix), deferred until every
        // fallible step above -- schema validation, FDAE policy, artifact
        // delivery, the wasm/tcp/container deploy itself, native capability
        // registration, owner attribution, instance-certificate install --
        // has succeeded. Under the same undeploy+rollback idiom as the
        // failure branches just above: nothing here can run for a deploy
        // that is about to fail, so nothing here can leave a binding
        // installed for a service that never actually started.
        if let Some(prepared) = &prepared_app_context
            && let Err(e) = self.install_app_context(&service_id, prepared).await
        {
            if let Err(undeploy_err) = self.undeploy(service_id.clone(), generation, caller).await {
                tracing::error!(
                    "rollback after app-context/binding installation failure also failed: \
                     {undeploy_err}"
                );
            }
            self.rollback_config_generation(&service_id, new_gen).await;
            self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
            return Err(e);
        }

        // Publish now rather than at the next heartbeat: a member
        // reinstantiated here has to become resolvable under its unchanged
        // master DID promptly, and the heartbeat runs hourly. Never fatal --
        // a registry that is down must not fail a deploy, and the heartbeat
        // sweep repairs it.
        if let Some(publisher) = self.endpoint_publisher.get()
            && let Err(e) = publisher.publish_service(&service_id).await
        {
            tracing::warn!("Failed to publish endpoint record for {}: {}", service_id, e);
        }

        // Every route table above is now written for this service in this
        // process -- record it so a later identical redeploy can dedup,
        // while a redeploy in a fresh process (post-restart) cannot.
        self.full_deploy_completed.insert(service_id.clone(), ());

        Ok(())
    }
}

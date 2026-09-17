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
        self.validate_deploy_admission(&service_id, caller)?;

        let (installed_instance_cert, validated_visibility) =
            self.validate_deploy_certs_and_visibility(&service_id, caller, &manifest)?;

        let prepared_app_context =
            self.prepare_app_context_for_deploy(&app_context, caller).await?;

        // Deploy idempotency, distinct from the epoch guard (dedups
        // binding writes) and the generation gate (picks between writers)
        // -- ADR-0021 §3 says explicitly that neither covers the other.
        let service_type = app_service_type(&manifest.service_type);
        // Every rollback below re-enters through `self.undeploy`, now
        // generation-gated -- send the same generation this deploy itself
        // presented, so a rollback of the supervisor's own deploy is never
        // rejected by its own gate.
        let generation = app_context.as_ref().map_or(0, |c| c.generation);
        let incoming_hash = compute_deploy_idempotency_hash(
            &manifest,
            app_context.as_ref(),
            installed_instance_cert.as_ref(),
        )?;
        if self.deploy_is_redundant_noop(&service_id, caller, service_type, &incoming_hash).await {
            // A retry after a lost response: nothing changed and the
            // instance is up, so this is a no-op that reports success --
            // not a reinstall that restarts a healthy service.
            info!("deploy for '{service_id}' is identical to what is installed and running; no-op");
            return Ok(());
        }

        let registry_cert = manifest.registry_certificate.as_deref();
        self.persist_or_clear_registry_cert_file(&service_id, registry_cert);

        // Configuration Generation & Validation
        let (flat_config, http_routes) = build_flat_config_and_routes(&manifest).await?;

        // A probe kind that cannot address this service type is a manifest
        // error, checked before any engine work runs (`service_type` was
        // already computed above, for the idempotency dedup check).
        if let Some(check) = &manifest.config.health_check {
            validate_health_check_matches_service_type(check, service_type)?;
        }

        validate_wasm_only_features(&service_id, &manifest, &http_routes, service_type)?;

        // FDAE policy: independent of `custom_config` (unlike `schema`
        // above, which is only resolved when a `custom_config` is present).
        // Validation is a hard deploy failure (ADR-0017 §1's "validated at
        // deploy... the Cedar lesson").
        let fdae_policy = resolve_fdae_policy_for_deploy(&service_id, &manifest).await?;

        // Persisted before the service is actually instantiated below, so
        // the `init`/`migrate` lifecycle hook's first read already sees
        // both rows.
        let (new_gen, previous_fdae_policy) =
            self.persist_config_and_fdae_policy(&service_id, &flat_config, &fdae_policy).await?;

        // `old_assets` is read now, before any mutation: it is the only
        // point that can see the still-live previous generation, which the
        // backward rollback (any failure from here on) must keep, and which
        // the forward cleanup at the commit point must diff against.
        let old_assets = self.assets.get(&service_id).map(|entry| entry.value().clone());
        let new_fdae_policy = fdae_policy.as_ref().map(|(_, policy)| policy.as_ref());
        let (new_assets, written_asset_hashes) = self
            .unpack_assets_and_deploy_service(
                &service_id,
                &manifest,
                &http_routes,
                new_fdae_policy,
                new_gen,
                &previous_fdae_policy,
                old_assets.as_ref(),
            )
            .await?;

        self.warn_on_fdae_strict_mode_issues(&service_id, &fdae_policy).await;

        // Data-layer/vault/app-config/blob-store access is a host-provided
        // capability orthogonal to how the service's own business logic
        // runs (wasm/container/tcp), so every deployed service gets a
        // native-callable channel for it regardless of type.
        self.register_native_capabilities_or_rollback(
            &service_id,
            generation,
            caller,
            new_gen,
            &previous_fdae_policy,
            &written_asset_hashes,
            old_assets.as_ref(),
        )
        .await?;
        self.register_native_dispatch_entry(
            &service_id,
            &caller.caller_did,
            &fdae_policy,
            installed_instance_cert.as_ref(),
        );
        self.commit_routes_and_gc_old_assets(
            &service_id,
            http_routes,
            new_assets.as_ref(),
            old_assets.as_ref(),
        )
        .await;

        // Owner attribution, instance-certificate install, and deploy-facts
        // recording -- the three registry writes that commit the deploy
        // after every earlier fallible step has succeeded. Every earlier
        // failure path above either never reached this line, or calls
        // `undeploy` itself (whose rollback is safe -- see
        // `undeploy_and_rollback_late_failure`'s doc comment), so a crash/
        // failure before this point never leaves a stale owner row.
        self.commit_deploy_registry_writes(
            &service_id,
            generation,
            caller,
            new_gen,
            &previous_fdae_policy,
            installed_instance_cert,
            &manifest,
            &incoming_hash,
            validated_visibility,
        )
        .await?;

        // A2 write (post-review fix), deferred until every fallible step
        // above -- schema validation, FDAE policy, artifact delivery, the
        // wasm/tcp/container deploy itself, native capability registration,
        // owner attribution, instance-certificate install -- has succeeded.
        // Nothing here can run for a deploy that is about to fail, so
        // nothing here can leave a binding installed for a service that
        // never actually started.
        if let Some(prepared) = &prepared_app_context
            && let Err(e) = self.install_app_context(&service_id, prepared).await
        {
            self.undeploy_and_rollback_late_failure(
                &service_id,
                generation,
                caller,
                new_gen,
                &previous_fdae_policy,
                "app-context/binding installation",
            )
            .await;
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

    /// The service_id-path-safety, reservation, ownership/takeover, and
    /// deploy-capability checks every deploy must pass before any other
    /// work starts. Pure reads -- nothing here writes anything, so
    /// returning `Err` leaves no state to roll back.
    fn validate_deploy_admission(
        &self,
        service_id: &str,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // `service_id` is joined verbatim into `hosted_apps_dir/<service_id>
        // .json` (write, and also delete on a private redeploy) -- reject
        // anything that could walk that join out of the directory before it
        // is used for anything, including the ownership/capability checks
        // that follow.
        if !is_safe_service_id_for_path(service_id) {
            return Err(format!(
                "service_id '{service_id}' is not a valid deploy target: it must be non-empty and \
                 contain no '/', '\\\\', or '..' -- it is joined into a stored-record filename"
            ));
        }
        // A deploy may not claim a `service_id` this substrate already uses
        // as a fixed `native_dispatch` key: the node's own DID (claiming it
        // hijacks every `orchestrator`/`security` call this node ever
        // receives) or the literal `"supervisor"`/auth alias (whose vault a
        // deploy under that name would also open). Neither `SERVICE_ID_
        // REGEX` nor `validate_service_id` reserves either string -- both
        // are ordinary, deployable-looking ids otherwise.
        if service_id == self.node_did
            || service_id == SUPERVISOR_RESERVED_SERVICE_ID
            || service_id == AUTH_RESERVED_SERVICE_ID
        {
            return Err(format!(
                "service_id '{service_id}' is reserved for this substrate's own dispatch and \
                 cannot be deployed to"
            ));
        }

        // A service_id already owned by someone else may not be re-deployed
        // into. An unowned substrate holds no node-wide orchestrator
        // authority, so this always enforces the takeover check there --
        // only an owned substrate's owner can override it.
        //
        // TOCTOU note (reviewed, accepted): this read and the terminal
        // `set_owner` write far below are separated by the whole deploy
        // body, not atomic. Two concurrent *first* deploys of the same
        // brand-new `service_id` from different DIDs can both observe
        // `owner_of == None` and both proceed -- whichever `set_owner` call
        // lands last wins attribution. This cannot defeat an *existing*
        // owner's protection (a service that already has a recorded owner
        // is rejected deterministically regardless of timing), so it is an
        // attribution race on a service_id nobody owns yet, not a
        // takeover-check bypass. Not fixed here: closing it fully needs a
        // per-service_id lock or an atomic claim-then-verify around the
        // entire (non-atomic, pre-existing) deploy flow, which is a larger
        // change than this one.
        if let Some(existing) = self.registry.owner_of(service_id)
            && existing != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {existing}; redeploy must come from its owner \
                 or a substrate owner"
            ));
        }

        // Tier-1 deploy admission. The caller must hold `orchestrator/
        // deploy` covering this app. No owner/unowned branch and no
        // separate substrate-owner bypass here: a bare `substrate:<node>`
        // capability (the owner's `substrate/admin`) is `is_substrate_
        // scope`, so `grants` wildcards the resource and only `entails` has
        // to hold -- that passes here for free. An app-scoped grantee is
        // prefix-covered instead. One check, two principals, no branch.
        let deploy_resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{service_id}' on this substrate",
                caller.caller_did
            ));
        }
        Ok(())
    }

    /// ADR-0020 §1's install-time instance-certificate verification, plus
    /// ADR-0018 §4's publication/visibility check -- both placed before any
    /// artifact work runs, so a bad certificate or an unauthorized
    /// visibility declaration is rejected at deploy rather than discovered
    /// later as a routing failure.
    fn validate_deploy_certs_and_visibility(
        &self,
        service_id: &str,
        caller: &CallerContext,
        manifest: &DeployManifest,
    ) -> Result<(Option<DelegationCertificate>, AppVisibility), String> {
        let installed_instance_cert = match &manifest.instance_certificate {
            Some(cert_json) => Some(self.verify_installed_instance_cert(
                &caller.caller_did,
                service_id,
                cert_json,
            )?),
            None => None,
        };
        let validated_visibility = validate_publication(
            service_id,
            manifest.config.visibility,
            manifest.registry_certificate.as_deref(),
        )?;
        Ok((installed_instance_cert, validated_visibility))
    }

    /// ADR-0021 §2: validates and prepares `app_context`'s bindings, and
    /// persists the ADR-0021 §4 generation-gate stamp immediately (that
    /// write records *who is writing*, not what was installed, so it is not
    /// behind the defer-until-everything-succeeds rule the returned
    /// `PreparedAppContext` itself is subject to -- see `install_app_context`
    /// and the call site's own comment).
    async fn prepare_app_context_for_deploy(
        &self,
        app_context: &Option<AppContext>,
        caller: &CallerContext,
    ) -> Result<Option<PreparedAppContext>, String> {
        let Some(ctx) = app_context else { return Ok(None) };
        // Validate before anything touches storage, so a later read of
        // these rows can only fail on real corruption rather than on
        // something a deploy caller sent.
        let instance_id = AppInstanceId::try_new(&ctx.app_instance_id)
            .map_err(|e| format!("app context names an invalid app instance id: {e}"))?;
        LogicalServiceName::try_new(&ctx.service_name)
            .map_err(|e| format!("app context names an invalid service name: {e}"))?;

        // An app instance's first successful deploy becomes its owner
        // (first-write-wins, the same shape `service_id` ownership uses in
        // `validate_deploy_admission`, including that check's own TOCTOU
        // note). Without this, any caller authorized to deploy *some*
        // service could name an existing, unrelated app instance in its own
        // `app_context` and overwrite the bindings that instance's other
        // services resolve.
        if let Some(existing) =
            self.registry.app_instance_management_of(&ctx.app_instance_id).map(|m| m.owner_did)
            && existing != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "app instance '{}' is owned by {existing}; a deploy that joins it must come from \
                 its owner or a substrate owner",
                ctx.app_instance_id
            ));
        }

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

        Ok(Some(PreparedAppContext {
            instance_id,
            raw_instance_id: ctx.app_instance_id.clone(),
            raw_service_name: ctx.service_name.clone(),
            bindings,
        }))
    }

    /// Whether this deploy is a retry after a lost response: same caller,
    /// byte-identical content already installed and running.
    /// `full_deploy_completed` is the witness that *this* process (not just
    /// a past one) already registered this service's routing -- those route
    /// tables are process-local and empty on every boot, so a fresh boot's
    /// redeploy always falls through here and re-registers everything, even
    /// when the persisted `manifest_hash` still matches. See the field's own
    /// doc comment.
    async fn deploy_is_redundant_noop(
        &self,
        service_id: &str,
        caller: &CallerContext,
        service_type: AppServiceType,
        incoming_hash: &str,
    ) -> bool {
        self.full_deploy_completed.contains_key(service_id)
            && self.registry.deploy_facts(service_id).and_then(|(_, _, hash, _)| hash).as_deref()
                == Some(incoming_hash)
            && self.registry.owner_of(service_id).as_deref().is_none_or(|o| o == caller.caller_did)
            && !matches!(
                self.instance_phase(service_id, Some(service_type_str(service_type))).await,
                InstancePhase::NotRunning(_) | InstancePhase::NotFound
            )
    }

    /// Saves `cert` (a registry certificate) to `hosted_apps_dir/
    /// <service_id>.json`, or removes that file when `cert` is absent -- a
    /// private redeploy must not leave a stale endpoint record around to
    /// keep being republished. Best-effort: a filesystem error here is
    /// logged, never propagated -- the deploy itself does not depend on
    /// this file existing.
    fn persist_or_clear_registry_cert_file(&self, service_id: &str, cert: Option<&str>) {
        let cert_path = self.hosted_apps_dir.join(format!("{service_id}.json"));
        match cert {
            Some(cert) => {
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
    }

    /// Saves this deploy's flattened config as a new generation, and
    /// last-write-wins the FDAE policy row -- both before the service is
    /// actually instantiated, so the `init`/`migrate` hook's first read
    /// already sees them. Returns the new config generation and whatever
    /// policy (if any) was there *before* this write, for the caller's own
    /// rollback should a later step fail: a redeploy's later-step failure
    /// must restore the *previous* policy exactly (including the manifest
    /// dropping the block entirely, i.e. `None`), or an already-running
    /// previous version loses its policy to an unrelated failed attempt the
    /// next time its engine cache re-resolves from storage.
    async fn persist_config_and_fdae_policy(
        &self,
        service_id: &str,
        flat_config: &BTreeMap<String, String>,
        fdae_policy: &Option<(String, Arc<Policy>)>,
    ) -> Result<(u64, Option<String>), String> {
        let config_blob = serde_json::to_string(flat_config)
            .map_err(|e| format!("Failed to serialize flattened config: {e}"))?;
        let new_gen = self
            .storage_provider
            .save_config_generation(service_id, &config_blob)
            .await
            .map_err(|e| format!("Failed to save config generation: {e}"))?;
        tracing::info!("Saved configuration generation {} for service {}", new_gen, service_id);

        let previous_fdae_policy = self
            .storage_provider
            .load_fdae_policy(service_id)
            .await
            .map_err(|e| format!("Failed to check existing FDAE policy: {e}"))?;
        if let Some((policy_doc, _)) = fdae_policy {
            self.storage_provider
                .save_fdae_policy(service_id, policy_doc)
                .await
                .map_err(|e| format!("Failed to save FDAE policy: {e}"))?;
        } else {
            // A manifest that no longer declares `fdae_policy` clears any
            // previously-declared policy -- a deploy's `config` fully
            // declares this service's policy state, so absence means
            // explicit removal, not "leave whatever was there" (without
            // this, the WASM engine's resolved-policy cache would reload
            // the stale row on its next cache miss even though native
            // dispatch has correctly gone unfiltered).
            self.storage_provider
                .delete_fdae_policy(service_id)
                .await
                .map_err(|e| format!("Failed to clear FDAE policy: {e}"))?;
        }
        Ok((new_gen, previous_fdae_policy))
    }

    /// Unpacks and stores the deploy manifest's asset bundle. On failure,
    /// undoes any blob writes this attempt itself made (restoring
    /// `old_assets`'s hashes) before returning `Err` -- the caller is still
    /// responsible for the config-generation/FDAE-policy rollback, since
    /// those were not created by this step. On success, also returns the
    /// set of newly-written hashes: a *later* deploy step can still fail,
    /// and its own rollback needs to know what this step wrote.
    async fn unpack_new_assets(
        &self,
        service_id: &str,
        bundle: &AssetBundle,
        http_routes: &[HttpRoute],
        old_assets: Option<&ServiceAssets>,
    ) -> Result<(ServiceAssets, BTreeSet<String>), String> {
        let mut written_asset_hashes = BTreeSet::new();
        let archive = match resolve_asset_archive(&bundle.archive) {
            Ok(a) => a,
            Err(e) => {
                self.rollback_asset_bundle(service_id, &written_asset_hashes, old_assets).await;
                return Err(e);
            }
        };
        let dek = match self.storage_provider.load_service_dek(service_id, &self.key_store).await {
            Ok(d) => d,
            Err(e) => {
                self.rollback_asset_bundle(service_id, &written_asset_hashes, old_assets).await;
                return Err(format!("Failed to resolve service DEK: {e}"));
            }
        };
        let unpacked = assets::unpack_asset_bundle(
            service_id,
            &archive,
            bundle.hash.as_deref(),
            http_routes,
            &self.blob_provider,
            dek.clone(),
            &mut written_asset_hashes,
        )
        .await;
        let asset_manifest = match unpacked {
            Ok(m) => m,
            Err(e) => {
                self.rollback_asset_bundle(service_id, &written_asset_hashes, old_assets).await;
                return Err(format!("Asset bundle unpack failed: {e}"));
            }
        };
        let manifest_hash =
            match assets::store_manifest(service_id, &asset_manifest, &self.blob_provider, dek)
                .await
            {
                Ok(h) => h,
                Err(e) => {
                    self.rollback_asset_bundle(service_id, &written_asset_hashes, old_assets).await;
                    return Err(format!("Asset manifest storage failed: {e}"));
                }
            };
        written_asset_hashes.insert(manifest_hash.clone());
        let public = matches!(bundle.visibility.as_ref(), Some(WitVisibility::Public));
        // A caller who forgets to declare `public` gets 404s with no signal
        // anywhere unless this is logged -- absence of an explicit
        // `visibility` defaults to `private` by construction, which is
        // deliberately silent at the *serving* layer, so the one place left
        // to say so is here, at deploy time.
        info!(
            "asset bundle for '{service_id}': {} entries, visibility {}",
            asset_manifest.entries.len(),
            match bundle.visibility.as_ref() {
                Some(WitVisibility::Public) => "public",
                Some(WitVisibility::Internal) => "internal",
                Some(WitVisibility::Private) | None => "private",
            }
        );
        Ok((
            ServiceAssets { manifest: Arc::new(asset_manifest), public, manifest_hash },
            written_asset_hashes,
        ))
    }

    /// Unpacks the deploy manifest's optional asset bundle, then dispatches
    /// to `deploy_wasm_service`/`deploy_tcp_service`/`deploy_container_
    /// service` for the manifest's own service type. On any failure,
    /// performs that specific step's own rollback (`unpack_new_assets`
    /// already rolls back its own partial asset writes on its own failure;
    /// the wasm/tcp/container helpers already roll back the config
    /// generation and FDAE policy on theirs, so only the asset-bundle
    /// rollback is added here) before returning `Err`. On success, also
    /// returns the set of newly-written asset hashes, which a still-later
    /// deploy step needs for its own rollback.
    #[allow(clippy::too_many_arguments)]
    async fn unpack_assets_and_deploy_service(
        &self,
        service_id: &str,
        manifest: &DeployManifest,
        http_routes: &[HttpRoute],
        new_fdae_policy: Option<&Policy>,
        new_gen: u64,
        previous_fdae_policy: &Option<String>,
        old_assets: Option<&ServiceAssets>,
    ) -> Result<(Option<ServiceAssets>, BTreeSet<String>), String> {
        let mut written_asset_hashes = BTreeSet::new();
        let new_assets = if let Some(bundle) = &manifest.config.assets {
            match self.unpack_new_assets(service_id, bundle, http_routes, old_assets).await {
                Ok((assets, hashes)) => {
                    written_asset_hashes = hashes;
                    Some(assets)
                }
                Err(e) => {
                    self.rollback_config_generation(service_id, new_gen).await;
                    self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
                    return Err(e);
                }
            }
        } else {
            None
        };

        log_public_guest_routes(service_id, http_routes);

        let deploy_result = match &manifest.service_type {
            WitServiceType::Wasm(wasm_manifest) => {
                self.deploy_wasm_service(
                    service_id,
                    manifest,
                    wasm_manifest,
                    new_gen,
                    previous_fdae_policy,
                    new_fdae_policy,
                    http_routes,
                )
                .await
            }
            WitServiceType::Tcp(tcp_manifest) => {
                self.deploy_tcp_service(
                    service_id,
                    tcp_manifest,
                    new_gen,
                    previous_fdae_policy,
                    new_fdae_policy,
                )
                .await
            }
            WitServiceType::Container(container_manifest) => {
                self.deploy_container_service(
                    service_id,
                    manifest,
                    container_manifest,
                    new_gen,
                    previous_fdae_policy,
                    new_fdae_policy,
                )
                .await
            }
        };
        if let Err(e) = deploy_result {
            self.rollback_asset_bundle(service_id, &written_asset_hashes, old_assets).await;
            return Err(e);
        }
        Ok((new_assets, written_asset_hashes))
    }

    /// Registers this deploy's owner attribution, installs (or clears) its
    /// instance certificate, and records its deploy facts -- the three
    /// registry writes that commit the deploy once every earlier fallible
    /// step has succeeded. Each runs only if the one before it did, and
    /// each failure tears the whole deploy down via `undeploy_and_
    /// rollback_late_failure` before returning `Err`.
    #[allow(clippy::too_many_arguments)]
    async fn commit_deploy_registry_writes(
        &self,
        service_id: &str,
        generation: u64,
        caller: &CallerContext,
        new_gen: u64,
        previous_fdae_policy: &Option<String>,
        installed_instance_cert: Option<DelegationCertificate>,
        manifest: &DeployManifest,
        incoming_hash: &str,
        validated_visibility: AppVisibility,
    ) -> Result<(), String> {
        let service_type = app_service_type(&manifest.service_type);
        if let Err(e) =
            self.registry.set_owner(service_id.to_string(), caller.caller_did.clone()).await
        {
            self.undeploy_and_rollback_late_failure(
                service_id,
                generation,
                caller,
                new_gen,
                previous_fdae_policy,
                "owner attribution",
            )
            .await;
            return Err(format!("Owner attribution failed: {e}"));
        }

        // Installed right after the owner row, under the same rollback:
        // already verified in `validate_deploy_certs_and_visibility`, so
        // `Some` only fails on a storage error. `None` clears any
        // certificate a previous deploy of this service_id installed -- the
        // WIT contract's "absent leaves the service its own master"
        // (control-plane.wit) must hold on every deploy, not only the
        // first.
        let cert_result = match installed_instance_cert {
            Some(cert) => self.registry.set_instance_cert(service_id.to_string(), cert).await,
            None => self.registry.remove_instance_cert(service_id).await,
        };
        if let Err(e) = cert_result {
            self.undeploy_and_rollback_late_failure(
                service_id,
                generation,
                caller,
                new_gen,
                previous_fdae_policy,
                "instance-certificate installation",
            )
            .await;
            return Err(format!("Instance certificate installation failed: {e}"));
        }

        // What this deploy said the service is, and its declared probe if
        // any. No upsert-or-clear branch like the certificate above -- the
        // type is always present, and a redeploy that drops the probe
        // writes a row with a `NULL` `health_check_json`, clearing it by
        // construction.
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
                service_id.to_string(),
                service_type_str(service_type).to_string(),
                health_check_json,
                Some(incoming_hash.to_string()),
                Some(validated_visibility.as_str().to_string()),
            )
            .await
        {
            self.undeploy_and_rollback_late_failure(
                service_id,
                generation,
                caller,
                new_gen,
                previous_fdae_policy,
                "deploy-facts installation",
            )
            .await;
            return Err(format!("Deploy facts installation failed: {e}"));
        }

        info!(
            "service '{service_id}' deployed with visibility '{}'",
            validated_visibility.as_str()
        );
        Ok(())
    }

    /// Author-time `strict:` warning pass: the service's own database is
    /// the collection inventory (a manifest declares no collection list;
    /// that comes from the guest's `init()` or native calls), so this is
    /// the first point after a first deploy's `init()` has created its
    /// tables. Warn-only in both directions, never a deploy failure.
    async fn warn_on_fdae_strict_mode_issues(
        &self,
        service_id: &str,
        fdae_policy: &Option<(String, Arc<Policy>)>,
    ) {
        let Some((_, policy)) = fdae_policy else { return };
        warn_on_ambiguous_public_permission(service_id, policy);
        match self.storage_provider.open_service_db(service_id, &self.key_store).await {
            Ok(store) => match store.list_collections().await {
                Ok(collections) => {
                    warn_on_policy_collection_mismatch(service_id, policy, &collections);
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

    /// Registers every `NATIVE_CAPABILITY_INTERFACES` endpoint for
    /// `service_id`. On a registration failure, tears the whole deploy
    /// down: `undeploy` first (there is now a partially-registered service
    /// to clean up, unlike every earlier failure branch, which never got
    /// this far), then the asset/config/FDAE rollbacks every other late
    /// failure also performs.
    #[allow(clippy::too_many_arguments)]
    async fn register_native_capabilities_or_rollback(
        &self,
        service_id: &str,
        generation: u64,
        caller: &CallerContext,
        new_gen: u64,
        previous_fdae_policy: &Option<String>,
        written_asset_hashes: &BTreeSet<String>,
        old_assets: Option<&ServiceAssets>,
    ) -> Result<(), String> {
        for interface in NATIVE_CAPABILITY_INTERFACES {
            if let Err(e) = self
                .registry
                .register(
                    service_id.to_string(),
                    interface.to_string(),
                    SubstrateEndpoint::NativeHostChannel { service_id: service_id.to_string() },
                )
                .await
            {
                if let Err(undeploy_err) =
                    self.undeploy(service_id.to_string(), generation, caller).await
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
                self.rollback_asset_bundle(service_id, written_asset_hashes, old_assets).await;
                self.rollback_config_generation(service_id, new_gen).await;
                self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
                return Err(format!("Native capability registration failed: {e}"));
            }
        }
        Ok(())
    }

    /// Wires `service_id`'s native-capability dispatch entry into the
    /// shared registry, if one is still available -- absent only when the
    /// substrate is shutting down, in which case the endpoints just
    /// registered above are left unroutable and logged, not treated as a
    /// deploy failure.
    fn register_native_dispatch_entry(
        &self,
        service_id: &str,
        caller_did: &str,
        fdae_policy: &Option<(String, Arc<Policy>)>,
        installed_instance_cert: Option<&DelegationCertificate>,
    ) {
        let Some(native_dispatch) = self.native_dispatch.upgrade() else {
            tracing::error!(
                "Native dispatch registry unavailable for service {}: registered its native \
                 capability endpoints but could not insert a dispatch entry, so calls into them \
                 will fail",
                service_id
            );
            return;
        };
        let native_service = Arc::new(SynSvcNativeService::new(
            service_id.to_string(),
            self.key_store.clone(),
            self.storage_provider.clone(),
            self.blob_provider.clone(),
            self.messaging_broker.clone(),
            fdae_policy.as_ref().map(|(_, policy)| policy.clone()),
            self.node_identity.clone(),
            caller_did,
            self.current_service_proxy(),
            self.current_row_authorizer(),
            installed_instance_cert.cloned(),
        ));
        native_service.set_conversation(self.current_conversation());
        native_service.set_record_signer_from(self);
        native_dispatch.insert(service_id.to_string(), native_service as Arc<dyn NativeService>);
    }

    /// Commits this deploy's http_routes/assets into their live
    /// process-local maps, then garbage-collects whatever blob the *old*
    /// manifest held that the new one no longer references -- never a
    /// wholesale delete, since unchanged files share hashes across
    /// generations. Best-effort: a GC failure here must not fail an
    /// otherwise-successful deploy.
    async fn commit_routes_and_gc_old_assets(
        &self,
        service_id: &str,
        http_routes: Vec<HttpRoute>,
        new_assets: Option<&ServiceAssets>,
        old_assets: Option<&ServiceAssets>,
    ) {
        if http_routes.is_empty() {
            self.http_routes.remove(service_id);
        } else {
            self.http_routes.insert(service_id.to_string(), http_routes);
        }
        match new_assets {
            Some(sa) => {
                self.assets.insert(service_id.to_string(), sa.clone());
            }
            None => {
                self.assets.remove(service_id);
            }
        }
        if let Some(old) = old_assets {
            let remove = assets::hashes_of(&old.manifest, Some(&old.manifest_hash));
            let keep = new_assets
                .map(|sa| assets::hashes_of(&sa.manifest, Some(&sa.manifest_hash)))
                .unwrap_or_default();
            if let Err(e) =
                assets::delete_hashes(service_id, &remove, &keep, &self.blob_provider).await
            {
                tracing::warn!(
                    "Failed to garbage-collect the previous asset bundle for service {}: {}",
                    service_id,
                    e
                );
            }
        }
    }

    /// Tears a deploy down after a *late* failure -- owner attribution,
    /// instance-certificate install, deploy-facts install, or app-context/
    /// binding install all run only after the wasm/tcp/container version is
    /// already live, so recovering means a full `undeploy`, not just
    /// restoring the two rows this attempt itself wrote. This is not a new
    /// gap this helper introduces: every one of its call sites already did
    /// this same full-teardown rollback before extraction, and it predates
    /// all of them -- `deploy` has never been transactional across
    /// config-generation / engine / registry writes, so a re-deploy's late
    /// failure tears down rather than restoring the prior running version.
    /// `context` names which late step failed, only for the log line if the
    /// undeploy itself also fails.
    async fn undeploy_and_rollback_late_failure(
        &self,
        service_id: &str,
        generation: u64,
        caller: &CallerContext,
        new_gen: u64,
        previous_fdae_policy: &Option<String>,
        context: &str,
    ) {
        if let Err(undeploy_err) = self.undeploy(service_id.to_string(), generation, caller).await {
            tracing::error!("rollback after {context} failure also failed: {undeploy_err}");
        }
        self.rollback_config_generation(service_id, new_gen).await;
        self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
    }
}

/// Canonical idempotency hash over the manifest and app-context, excluding
/// fields that record *who is writing* or mint-time freshness rather than
/// *what gets installed*: `generation`, each binding's `epoch`, and the
/// instance/registry certificates' fresh-every-mint fields (their `not_
/// after`/signing timestamp would otherwise make the hash differ on every
/// apply regardless of content, since both are re-minted fresh every call).
/// Each certificate is hashed on its *stable* identity fields only.
fn compute_deploy_idempotency_hash(
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
async fn build_flat_config_and_routes(
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
fn validate_health_check_matches_service_type(
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
fn validate_wasm_only_features(
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
async fn resolve_fdae_policy_for_deploy(
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

/// A `public` guest route is reachable with no verified caller identity
/// over a direct anonymous connection -- logged loudly so an author who
/// didn't mean to leave a route open still has one place to notice, the
/// same treatment the asset bundle's own visibility gets in
/// `unpack_new_assets`.
fn log_public_guest_routes(service_id: &str, http_routes: &[HttpRoute]) {
    for route in http_routes.iter().filter(|r| r.target == "guest" && r.public) {
        info!(
            "guest HTTP route for '{service_id}': {} {} declared public -- reachable with no \
             verified caller identity, and its handler still runs with the service's own storage \
             rights",
            route.method, route.path
        );
    }
}

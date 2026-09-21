use super::*;

impl ControlPlaneService {
    /// Registers this deploy's owner attribution, installs (or clears) its
    /// instance certificate, and records its deploy facts -- the three
    /// registry writes that commit the deploy once every earlier fallible
    /// step has succeeded. Each runs only if the one before it did, and
    /// each failure tears the whole deploy down via `undeploy_and_
    /// rollback_late_failure` before returning `Err`.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn commit_deploy_registry_writes(
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
    pub(super) async fn warn_on_fdae_strict_mode_issues(
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
    pub(super) async fn register_native_capabilities_or_rollback(
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
    pub(super) fn register_native_dispatch_entry(
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
    pub(super) async fn commit_routes_and_gc_old_assets(
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
    pub(super) async fn undeploy_and_rollback_late_failure(
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

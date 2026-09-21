use super::*;

impl ControlPlaneService {
    /// Saves `cert` (a registry certificate) to `hosted_apps_dir/
    /// <service_id>.json`, or removes that file when `cert` is absent -- a
    /// private redeploy must not leave a stale endpoint record around to
    /// keep being republished. Best-effort: a filesystem error here is
    /// logged, never propagated -- the deploy itself does not depend on
    /// this file existing.
    pub(super) fn persist_or_clear_registry_cert_file(&self, service_id: &str, cert: Option<&str>) {
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
    pub(super) async fn persist_config_and_fdae_policy(
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
    pub(super) async fn unpack_assets_and_deploy_service(
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

        super::manifest::log_public_guest_routes(service_id, http_routes);

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
}

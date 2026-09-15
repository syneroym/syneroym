use super::*;

impl AppSandboxEngine {
    /// Whether `service_id`'s compiled component exports `function` on
    /// `interface`. Cheap: reads the cached `InstancePre`'s static component
    /// type, no instantiation. `interface` is matched exactly, which is what
    /// dispatch itself does (`get_wasm_func`), so a name that passes here is
    /// a name a call can reach.
    #[must_use]
    pub fn exports_function(&self, service_id: &str, interface: &str, function: &str) -> bool {
        let Some(entry) = self.components.get(service_id) else { return false };
        let ct = entry.value().0.component().component_type();
        let Some(export) = ct.get_export(&self.engine, interface) else { return false };
        let ComponentItem::ComponentInstance(instance) = export.ty else { return false };
        instance.get_export(&self.engine, function).is_some()
    }

    /// Every function `interface` exports, or `None` when the component does
    /// not export that interface at all. Backs the deploy-time saga
    /// compensation gate: a `saga-undo-x` with no `x` beside it.
    #[must_use]
    pub fn exported_functions(&self, service_id: &str, interface: &str) -> Option<Vec<String>> {
        let entry = self.components.get(service_id)?;
        let ct = entry.value().0.component().component_type();
        let export = ct.get_export(&self.engine, interface)?;
        let ComponentItem::ComponentInstance(instance) = export.ty else { return None };
        Some(instance.exports(&self.engine).map(|(name, _)| name.to_string()).collect())
    }

    /// Whether `service_id`'s compiled component exports the stage-4
    /// after-step. Cheap: inspects the cached `InstancePre`'s static
    /// component type, no instantiation -- used by the deploy-time gate
    /// (`validate_stage4_export`) to reject a policy that opts a permission
    /// into `authorize_rows: true` against a component that could never
    /// satisfy it.
    #[must_use]
    pub fn exports_authorize_rows(&self, service_id: &str) -> bool {
        self.exports_function(service_id, Self::AUTHORIZER_INTERFACE, "authorize-rows")
    }

    /// Whether `service_id`'s compiled component exports the guest HTTP
    /// handler. Cheap (static component type, no instantiation) --
    /// exactly `exports_authorize_rows`' shape and deploy-gate role.
    #[must_use]
    pub fn exports_http_handler(&self, service_id: &str) -> bool {
        self.exports_function(service_id, http::HTTP_HANDLER_INTERFACE, "handle-request")
    }

    /// Whether `service_id`'s compiled component exports the guest WebSocket
    /// handler (on-open, on-message, on-close).
    #[must_use]
    pub fn exports_websocket_handler(&self, service_id: &str) -> bool {
        self.exports_function(service_id, "syneroym:http/websocket-handler@0.1.0", "on-open")
            && self.exports_function(
                service_id,
                "syneroym:http/websocket-handler@0.1.0",
                "on-message",
            )
            && self.exports_function(
                service_id,
                "syneroym:http/websocket-handler@0.1.0",
                "on-close",
            )
    }

    /// Whether a compiled component is loaded for `service_id` -- the only
    /// liveness a wasm service has, since nothing runs between calls.
    #[must_use]
    pub fn is_deployed(&self, service_id: &str) -> bool {
        self.components.contains_key(service_id)
    }

    /// Deploy and compile a WASM component for a given service
    pub async fn deploy_wasm(&self, service_id: &str, manifest: &DeployManifest) -> Result<()> {
        Self::validate_service_id(service_id)?;
        info!("AppSandboxEngine: Deploying Wasm component for {}", service_id);

        let ServiceType::Wasm(wasm_manifest) = &manifest.service_type else {
            return Err(anyhow::anyhow!("Expected Wasm manifest"));
        };

        // 1. Fetch bytes
        let bytes = Self::fetch_wasm_bytes(&wasm_manifest.source).await?;

        // 2. Verify hash
        Self::verify_wasm_hash(&bytes, wasm_manifest.hash.as_deref())?;

        // 3. Store locally in blobs_dir
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        tokio_fs::write(&file_path, &bytes).await.context("Failed to save WASM binary locally")?;

        info!("WASM binary stored at {:?}", file_path);

        let quota = manifest.config.quota.as_ref().map(|q| WasmResourceQuota {
            max_instructions: q.max_instructions,
            max_memory_bytes: q.max_memory_bytes,
        });

        if let Some(ref q) = quota {
            let quota_path = self.blobs_dir.join(format!("{service_id}.quota.json"));
            if let Ok(quota_json) = serde_json::to_string(q) {
                let _ = tokio_fs::write(&quota_path, quota_json).await;
            }
        }

        // 4. Compile and cache the component; the raw bytes are moved in and dropped by
        //    the compile itself, freeing the memory
        self.compile_and_cache_wasm(service_id, bytes, quota).await?;

        // 5. Invoke the guest's schema lifecycle hook: `init()` on a fresh service (no
        //    existing database), `migrate()` on a re-deploy of a service with existing
        //    state. Checked here, before anything else can lazily open the service DB
        //    and thereby create it.
        let is_first_deploy = !self
            .storage_provider
            .service_exists(service_id)
            .await
            .context("failed to check for pre-existing service state")?;
        let hook = if is_first_deploy {
            "init"
        } else {
            // TODO(M5): full snapshot/rollback safety net for migrate() is
            // deferred to M5 [LFC-VER]. migrate() may execute destructive
            // DDL; there is no automatic rollback on partial failure in M3A.
            "migrate"
        };
        self.invoke_lifecycle_hook(service_id, hook)
            .await
            .with_context(|| format!("{hook}() lifecycle hook failed for service {service_id}"))?;

        Ok(())
    }

    /// Invokes a guest lifecycle export (`init` or `migrate`) declared
    /// directly on the `data-layer-guest` world, if the deployed component
    /// exports it. Components that don't declare the export (e.g. a plain
    /// component with no data-layer usage, like the `greeter` test
    /// component) are left untouched -- this makes it safe to call
    /// unconditionally on every deploy.
    async fn invoke_lifecycle_hook(&self, service_id: &str, hook: &str) -> Result<()> {
        let (mut store, instance, _max_instructions) = self
            .build_store_and_instantiate(
                service_id,
                CallerContext::local_elevated(service_id),
                self.lifecycle_hook_epoch_ticks,
                InstanceOptions::default(),
            )
            .await?;

        if instance.get_export(&mut store, None, hook).is_none() {
            debug!(service_id, hook, "component does not export lifecycle hook, skipping");
            return Ok(());
        }

        let (func, results_len, _item) = Self::get_wasm_func(&mut store, &instance, None, hook)?;
        let mut results = vec![Val::Bool(false); results_len];
        func.call_async(&mut store, &[], &mut results).await?;

        if let Some(msg) = Self::wasm_result_err(&results) {
            return Err(anyhow::anyhow!("{hook}() failed: {msg}"));
        }
        Ok(())
    }

    /// Stop and evict a running Wasm component from the in-memory cache.
    pub async fn stop_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        info!(service_id = %service_id, "AppSandboxEngine: stopping Wasm component");
        self.components.remove(service_id);
        self.fdae_policies.remove(service_id);
        self.bump_fdae_policy_generation(service_id);
        self.abort_streams(service_id);
        self.forget_websocket_senders(service_id);
        self.forget_guest_http_permits(service_id);
        metrics::gauge!("substrate.wasm.component_cache_size").set(self.components.len() as f64);
        Ok(())
    }

    /// Remove a stopped Wasm component's binary from disk.
    pub async fn remove_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        info!(service_id = %service_id, "AppSandboxEngine: removing Wasm component");
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        if file_path.exists() {
            tokio_fs::remove_file(&file_path)
                .await
                .with_context(|| format!("Failed to remove WASM file {file_path:?}"))?;
        }
        let quota_path = self.blobs_dir.join(format!("{service_id}.quota.json"));
        if quota_path.exists() {
            let _ = tokio_fs::remove_file(&quota_path).await;
        }
        Ok(())
    }

    /// Evict and recompile `service_id` from the artifact `deploy_wasm`
    /// persisted to `blobs_dir`. The remediation half of
    /// restart-in-place: a wasm service has no process, so "restart" means
    /// dropping the cached `InstancePre` (and with it the resolved FDAE
    /// policy) and rebuilding it from disk.
    ///
    /// Fails -- rather than `load_cached_wasm`'s `warn!` -- when no
    /// artifact is on disk: a supervisor must be able to tell "restarted"
    /// from "there was nothing to restart", or bounded remediation counts
    /// a no-op as an attempt and exhausts its budget against a service it
    /// never touched.
    ///
    /// **No identity work.** The instance key is HKDF-derived from the
    /// node identity and the calling DID, so a restart on the same node
    /// under the same caller yields the *same* key and the installed
    /// certificate stays valid; the endpoint record is unchanged, so there
    /// is nothing to republish.
    pub async fn reload_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        if !file_path.exists() {
            anyhow::bail!("no WASM artifact on disk for {service_id}; redeploy it");
        }
        self.stop_wasm(service_id).await?;
        self.load_cached_wasm(service_id).await
    }

    /// Helper to load a cached WASM component from disk and compile it
    pub(crate) async fn load_cached_wasm(&self, service_id: &str) -> Result<()> {
        Self::validate_service_id(service_id)?;
        let file_path = self.blobs_dir.join(format!("{service_id}.wasm"));
        if file_path.exists() {
            let bytes = tokio_fs::read(&file_path)
                .await
                .context(format!("Failed to read WASM file {file_path:?}"))?;
            let quota_path = self.blobs_dir.join(format!("{service_id}.quota.json"));
            let quota = if quota_path.exists() {
                if let Ok(quota_json) = tokio_fs::read_to_string(&quota_path).await {
                    serde_json::from_str::<WasmResourceQuota>(&quota_json).ok()
                } else {
                    None
                }
            } else {
                None
            };
            self.compile_and_cache_wasm(service_id, bytes, quota).await?;
        } else {
            warn!("WASM file not found on disk for service: {:?}", file_path);
        }
        Ok(())
    }

    /// Helper to compile a WASM binary and store it in the cache.
    ///
    /// The compile runs on a blocking thread, not the calling task's own.
    /// Cranelift is synchronous and CPU-bound: a component of a few
    /// hundred kilobytes takes over a second here, and several seconds
    /// when wasmtime itself is built unoptimized (any test profile). Left
    /// on an async worker it stalls every other task on that thread,
    /// including this node's iroh endpoint -- and iroh caps a QUIC path's
    /// idle timeout at 6.5s with no way to raise it, so a compile that
    /// outlasts that makes the peer abandon the path and the very deploy
    /// RPC that asked for the compile dies with "connection lost".
    pub async fn compile_and_cache_wasm(
        &self,
        service_id: &str,
        bytes: Vec<u8>,
        quota: Option<WasmResourceQuota>,
    ) -> Result<()> {
        let engine = self.engine.clone();
        let linker = self.linker.clone();
        let instance_pre = task::spawn_blocking(move || {
            let component = Component::new(&engine, &bytes)
                .map_err(|e| anyhow!("Failed to compile WASM component: {e}"))?;
            linker
                .instantiate_pre(&component)
                .map_err(|e| anyhow!("Failed to pre-link WASM component: {e}"))
        })
        .await
        .context("WASM compile task failed to complete")??;

        self.components.insert(service_id.to_string(), (instance_pre, quota));
        // A re-deploy compiles and re-caches the component here; evict any
        // previously resolved policy so the next instantiation re-resolves
        // from `substrate.db` rather than serving a stale one.
        self.fdae_policies.remove(service_id);
        self.bump_fdae_policy_generation(service_id);
        info!("WASM component compiled and cached for {}", service_id);
        metrics::gauge!("substrate.wasm.component_cache_size").set(self.components.len() as f64);
        Ok(())
    }

    /// Spin up a new Podman instance
    pub async fn deploy_podman(&self, _service_id: &str, _manifest: &[u8]) -> Result<()> {
        info!("AppSandboxEngine: Deploying Podman container for {}", _service_id);
        Ok(())
    }
}

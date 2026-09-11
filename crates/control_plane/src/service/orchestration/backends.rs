use super::*;

impl ControlPlaneService {
    pub(super) async fn register_wasm_endpoints(
        &self,
        service_id: &str,
        interfaces: Vec<String>,
    ) -> Result<()> {
        for interface in interfaces {
            self.registry
                .register(
                    service_id.to_string(),
                    interface,
                    SubstrateEndpoint::WasmChannel { service_id: service_id.to_string() },
                )
                .await?;
        }
        Ok(())
    }

    /// Writes `prepared`'s app-context and binding rows. Called only once
    /// every earlier fallible step in `deploy_with_context` has already
    /// succeeded -- see the call site's own comment -- so a storage error
    /// here is the *only* way this can fail, never a validation problem
    /// (`prepared`'s fields already passed `try_new`). The app-instance
    /// management stamp is *not* written here -- `deploy_with_context`
    /// persists it right after `check_generation` succeeds, before this
    /// method ever runs, since it records who is writing rather than what
    /// was installed.
    pub(super) async fn install_app_context(
        &self,
        service_id: &str,
        prepared: &PreparedAppContext,
    ) -> Result<(), String> {
        // A redeploy fully declares this service's app context, so its
        // previous rows go first -- a dependency dropped from the manifest
        // must not survive as a stale row (the same "absence means
        // removal" rule `fdae_policy` follows above). Safe to do here,
        // unlike at the deploy's original early call site: every field of
        // `prepared` already passed `try_new` before this method is ever
        // called, so this removal can only be followed by a fresh write,
        // never by a validation failure that leaves the old rows gone and
        // nothing in their place.
        self.registry.remove_app_context(service_id).await.map_err(|e| e.to_string())?;
        self.registry
            .set_app_context(
                service_id.to_string(),
                prepared.raw_instance_id.clone(),
                prepared.raw_service_name.clone(),
            )
            .await
            .map_err(|e| e.to_string())?;

        for (raw_dependency_name, dependency_name, entry) in &prepared.bindings {
            let entry_json = serde_json::to_string(entry).map_err(|e| e.to_string())?;
            self.registry
                .save_binding(
                    service_id,
                    &prepared.raw_instance_id,
                    raw_dependency_name,
                    &entry_json,
                )
                .await
                .map_err(|e| e.to_string())?;
            // Last-write-wins. ADR-0021 §3's four-case epoch
            // guard -- lower rejects, equal+identical no-ops, equal+
            // different is a reported conflict, higher applies -- belongs
            // at exactly this call site.
            self.logical_resolver.register(
                TopologyKey::local(prepared.instance_id.clone(), dependency_name.clone()),
                entry.clone(),
            );
        }

        Ok(())
    }

    /// ADR-0021 §4's single-writer rule, applied to every write that
    /// changes an app instance: `deploy_with_context`, `write_bindings`,
    /// `restart`, `undeploy`, `release_app_instance`.
    ///
    /// The generation is a tiebreaker, so an *unadopted* instance
    /// (`supervisor_did: None`) accepts any authorized writer -- that is
    /// what keeps the operator-driven `app deploy` working unchanged,
    /// and what `release-app-instance` restores. The
    /// returned value is what the caller must persist immediately
    /// (`set_app_instance_management`), before anything else it does
    /// -- it records *who is writing*, not what was installed, so it is
    /// not behind the defer-until-everything-succeeds rule that governs
    /// bindings.
    ///
    /// `presented == 0` never claims supervision, regardless of whether a
    /// row already exists: the WIT `app-context.generation` doc is
    /// explicit that `0` means unmanaged, which is what every
    /// operator-driven `roymctl app deploy` sends. Without this, the
    /// instance's *first* deploy -- by anyone, including a node-wide
    /// caller redeploying over a different owner -- would stamp itself in
    /// as supervisor and lock out every later un-adopted deploy, which is
    /// exactly the "unadopted instance accepts any authorized writer"
    /// invariant this function exists to uphold.
    pub(super) fn check_generation(
        &self,
        app_instance_id: &str,
        caller: &CallerContext,
        presented: u64,
    ) -> Result<AppInstanceManagement, String> {
        let held = self.registry.app_instance_management_of(app_instance_id);
        match held {
            None => Ok(AppInstanceManagement {
                owner_did: caller.caller_did.clone(),
                supervisor_did: (presented != 0).then(|| caller.caller_did.clone()),
                generation: presented,
            }),
            Some(m) if m.supervisor_did.is_none() && presented == 0 => Ok(m),
            Some(m) if m.supervisor_did.is_none() => Ok(AppInstanceManagement {
                supervisor_did: Some(caller.caller_did.clone()),
                generation: presented,
                ..m
            }),
            Some(m) => match presented.cmp(&m.generation) {
                Ordering::Greater => Ok(AppInstanceManagement {
                    supervisor_did: Some(caller.caller_did.clone()),
                    generation: presented,
                    ..m
                }),
                Ordering::Equal
                    if m.supervisor_did.as_deref() == Some(caller.caller_did.as_str()) =>
                {
                    Ok(m)
                }
                Ordering::Equal => Err(format!(
                    "app instance '{app_instance_id}' is managed at generation {} by {}; a second \
                     writer at the same generation is rejected (ADR-0021 §4)",
                    m.generation,
                    m.supervisor_did.as_deref().unwrap_or("<unknown>"),
                )),
                Ordering::Less => Err(format!(
                    "app instance '{app_instance_id}' is managed at generation {} by {}; this \
                     write presented generation {presented}. Stop managing this instance and \
                     alert -- never self-increment (ADR-0021 §4).",
                    m.generation,
                    m.supervisor_did.as_deref().unwrap_or("<unknown>"),
                )),
            },
        }
    }

    /// Logs (but does not propagate) a failure to roll back a config
    /// generation saved just before a deploy that then failed. Best-effort:
    /// the deploy error itself is what gets returned to the caller.
    pub(super) async fn rollback_config_generation(&self, service_id: &str, generation: u64) {
        if let Err(rollback_err) =
            self.storage_provider.delete_config_generation(service_id, generation).await
        {
            tracing::error!(
                "Failed to rollback config generation {} for service {} after deploy error: {}",
                generation,
                service_id,
                rollback_err
            );
        }
    }

    /// Rolls back an in-progress deploy's asset-bundle work (the backward
    /// direction of asset cleanup): deletes every blob this attempt itself
    /// wrote, keeping any hash the still-live previous generation (`old`)
    /// still references. A no-op when `written` is empty, so calling this
    /// unconditionally on every failure branch above the registry commit
    /// costs nothing for a deploy that declares no assets at all.
    /// Best-effort, same as `rollback_config_generation`.
    pub(super) async fn rollback_asset_bundle(
        &self,
        service_id: &str,
        written: &BTreeSet<String>,
        old: Option<&ServiceAssets>,
    ) {
        if written.is_empty() {
            return;
        }
        let keep =
            old.map(|a| assets::hashes_of(&a.manifest, Some(&a.manifest_hash))).unwrap_or_default();
        if let Err(e) = assets::delete_hashes(service_id, written, &keep, &self.blob_provider).await
        {
            tracing::error!(
                "Failed to roll back asset bundle blobs for service {} after deploy error: {}",
                service_id,
                e
            );
        }
    }

    /// Restores whatever FDAE policy (or absence) `service_id` had before
    /// this deploy attempt's `save_fdae_policy`/`delete_fdae_policy` call --
    /// see `previous_fdae_policy`'s capture in `deploy` for why this must
    /// restore the previous value rather than unconditionally delete, in
    /// both directions (a new/changed policy, or the manifest dropping the
    /// block entirely). Best-effort, same as `rollback_config_generation`.
    ///
    /// Also evicts the WASM engine's own resolved-policy cache
    /// (`stop_wasm`'s side effect, alongside the component cache it exists
    /// to evict) for `service_id`. A failed `deploy_wasm_service` attempt
    /// can reach this point *after* `compile_and_cache_wasm`/
    /// `resolve_fdae_policy` already cached the new (about-to-be-rolled-
    /// back) policy -- restoring the DB row alone would leave the engine
    /// serving that cached policy for the rest of the process's uptime,
    /// diverging from what storage now says. Safe to call unconditionally:
    /// `stop_wasm` no-ops for a `service_id` the engine never cached
    /// anything for (the TCP/container rollback paths, and the ordinary
    /// case of nothing having been cached yet).
    pub(super) async fn rollback_fdae_policy(&self, service_id: &str, previous: &Option<String>) {
        let result = match previous {
            Some(doc) => self.storage_provider.save_fdae_policy(service_id, doc).await,
            None => self.storage_provider.delete_fdae_policy(service_id).await,
        };
        if let Err(e) = result {
            tracing::error!(
                "Failed to roll back FDAE policy for service {} after deploy error: {}",
                service_id,
                e
            );
        }
        if let Err(e) = self.app_sandbox_engine.stop_wasm(service_id).await {
            tracing::error!(
                "Failed to evict cached FDAE policy for service {} after deploy error: {}",
                service_id,
                e
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn deploy_wasm_service(
        &self,
        service_id: &str,
        manifest: &DeployManifest,
        wasm_manifest: &WasmManifest,
        new_gen: u64,
        previous_fdae_policy: &Option<String>,
        new_fdae_policy: Option<&Policy>,
        http_routes: &[HttpRoute],
    ) -> Result<(), String> {
        if let Err(e) = self.app_sandbox_engine.deploy_wasm(service_id, manifest).await {
            self.rollback_config_generation(service_id, new_gen).await;
            self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
            return Err(format!("WASM deployment failed: {e}"));
        }

        // `validate_stage4_export` (ADR-0017 §8): a policy that opts a
        // permission into the stage-4 after-step but whose compiled
        // component doesn't export `syneroym:data-layer/authorizer#
        // authorize-rows` would deny **every** read through that permission
        // at runtime (fail-closed) -- failing the deploy here, once the
        // component is actually compiled and its exports are knowable, is
        // strictly better than shipping a service that silently returns
        // nothing. Placed after `deploy_wasm` (which compiles/caches the
        // component) so `exports_authorize_rows` has a real answer.
        if let Some(policy) = new_fdae_policy
            && policy_declares_stage4(policy)
            && !self.app_sandbox_engine.exports_authorize_rows(service_id)
        {
            self.rollback_config_generation(service_id, new_gen).await;
            self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
            return Err(format!(
                "FDAE policy for service {service_id} opts a permission into the stage-4 \
                 after-step (authorize_rows: true), but the deployed component does not export \
                 syneroym:data-layer/authorizer#authorize-rows"
            ));
        }

        // A declared `guest` route whose compiled component
        // doesn't export the handler would 500 on every request it ever
        // gets, discoverable only in production -- same reasoning, and
        // placed right after, the stage-4 export check above. Must run
        // after `deploy_wasm` (just above) has compiled the component,
        // which is where `exports_http_handler` has a real answer.
        if http_routes.iter().any(|r| r.target == "guest")
            && !self.app_sandbox_engine.exports_http_handler(service_id)
        {
            self.rollback_config_generation(service_id, new_gen).await;
            self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
            return Err(format!(
                "service {service_id} declares an http_routes entry with target=guest, but the \
                 deployed component does not export syneroym:http/incoming-handler#handle-request"
            ));
        }

        if http_routes.iter().any(|r| r.target == "websocket")
            && !self.app_sandbox_engine.exports_websocket_handler(service_id)
        {
            self.rollback_config_generation(service_id, new_gen).await;
            self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
            return Err(format!(
                "service {service_id} declares an http_routes entry with target=websocket, but \
                 the deployed component does not export syneroym:http/websocket-handler#on-open"
            ));
        }

        // One rule, over the interfaces the manifest already declares. Sound
        // only because `saga-undo-` is reserved (ADR-0023 §7, as amended): a
        // component that exports it is unambiguously claiming a saga
        // compensation, so a missing counterpart is a defect and never a
        // legal business name.
        for iface in &wasm_manifest.interfaces {
            let Some(exports) = self.app_sandbox_engine.exported_functions(service_id, iface)
            else {
                continue;
            };
            for function in &exports {
                let Some(forward) = compensated_operation(function) else { continue };
                if !self.app_sandbox_engine.exports_function(service_id, iface, forward) {
                    self.rollback_config_generation(service_id, new_gen).await;
                    self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
                    return Err(format!(
                        "component exports '{function}' on '{iface}' but no '{forward}' beside \
                         it: a saga compensation must name an operation this component actually \
                         has"
                    ));
                }
            }
        }

        if let Err(e) =
            self.register_wasm_endpoints(service_id, wasm_manifest.interfaces.clone()).await
        {
            self.rollback_config_generation(service_id, new_gen).await;
            self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
            return Err(format!("Endpoint registration failed: {e}"));
        }
        Ok(())
    }

    pub(super) async fn deploy_tcp_service(
        &self,
        service_id: &str,
        tcp_manifest: &TcpManifest,
        new_gen: u64,
        previous_fdae_policy: &Option<String>,
        new_fdae_policy: Option<&Policy>,
    ) -> Result<(), String> {
        // No guest to call at all -- a TCP service can never satisfy a
        // stage-4 opt-in, so reject up front rather than deploying a
        // service that would deny every such read.
        if let Some(policy) = new_fdae_policy
            && policy_declares_stage4(policy)
        {
            self.rollback_config_generation(service_id, new_gen).await;
            self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
            return Err(format!(
                "FDAE policy for service {service_id} opts a permission into the stage-4 \
                 after-step (authorize_rows: true), but a TCP service has no guest component to \
                 export it"
            ));
        }
        for endpoint in &tcp_manifest.endpoints {
            info!(
                "Deploying TCP service {} endpoint {}: {}:{}",
                service_id, endpoint.interface_name, endpoint.host, endpoint.port
            );
            if let Err(e) = self
                .registry
                .register(
                    service_id.to_string(),
                    endpoint.interface_name.clone(),
                    SubstrateEndpoint::TcpHostPort {
                        host: endpoint.host.clone(),
                        port: endpoint.port,
                    },
                )
                .await
            {
                self.rollback_config_generation(service_id, new_gen).await;
                self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
                return Err(format!("Endpoint registration failed: {e}"));
            }
        }
        Ok(())
    }

    pub(super) async fn deploy_container_service(
        &self,
        service_id: &str,
        manifest: &DeployManifest,
        container_manifest: &ContainerManifest,
        new_gen: u64,
        previous_fdae_policy: &Option<String>,
        new_fdae_policy: Option<&Policy>,
    ) -> Result<(), String> {
        // Same reasoning as `deploy_tcp_service`: no guest component to
        // export the after-step.
        if let Some(policy) = new_fdae_policy
            && policy_declares_stage4(policy)
        {
            self.rollback_config_generation(service_id, new_gen).await;
            self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
            return Err(format!(
                "FDAE policy for service {service_id} opts a permission into the stage-4 \
                 after-step (authorize_rows: true), but a container service has no guest \
                 component to export it"
            ));
        }
        info!("Deploying container service {}: image={}", service_id, container_manifest.image);
        let actual_mappings = match self.podman_sandbox_engine.deploy(service_id, manifest).await {
            Ok(mappings) => mappings,
            Err(e) => {
                self.rollback_config_generation(service_id, new_gen).await;
                self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
                return Err(format!("Container deployment failed: {e}"));
            }
        };

        for (interface_name, host_port) in actual_mappings {
            if let Err(e) = self
                .registry
                .register(
                    service_id.to_string(),
                    interface_name,
                    SubstrateEndpoint::TcpHostPort {
                        host: "127.0.0.1".to_string(),
                        port: host_port,
                    },
                )
                .await
            {
                self.rollback_config_generation(service_id, new_gen).await;
                self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
                return Err(format!("Endpoint registration failed: {e}"));
            }
        }
        Ok(())
    }
}

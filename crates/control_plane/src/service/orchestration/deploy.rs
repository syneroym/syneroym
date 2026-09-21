use super::*;

mod admission;
mod commit;
mod manifest;
mod provision;

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
        let incoming_hash = self::manifest::compute_deploy_idempotency_hash(
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

        // Configuration/http_routes generation, plus the health-check,
        // wasm-only-feature, and FDAE-policy validation that must all pass
        // before any config generation write happens.
        let (flat_config, http_routes, fdae_policy) =
            self::manifest::validate_and_build_deploy_config(&service_id, &manifest, service_type)
                .await?;

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
}

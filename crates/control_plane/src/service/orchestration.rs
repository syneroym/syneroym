//! Deploy/undeploy/list lifecycle for the orchestrator.
//!
//! Handles validating and applying a `DeployManifest` (wasm/container/tcp),
//! wiring up the native-capability endpoints and dispatch registration every
//! deployed service gets, and tearing all of that back down on undeploy.
//! Distinct from `service`'s own concern (`NativeService::dispatch`'s JSON-RPC
//! routing table and the KEK/secret management calls it handles directly).

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Component, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use serde_json::Value;
use syneroym_core::{
    deploy_docs,
    local_registry::{NATIVE_CAPABILITY_INTERFACES, SubstrateEndpoint},
    util,
};
use syneroym_fdae::Policy;
use syneroym_identity::{
    DelegationCertificate, delegation::SCOPE_SERVICE_INSTANCE, substrate::derive_did_key,
};
use syneroym_rpc::{Ability, CallerContext, NativeService, ResourceUri};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    ArtifactSource, ContainerManifest, DeployManifest, DeployedService, DeploymentPlan,
    DocumentSource, InstanceIdentity, ServiceType as WitServiceType, TcpManifest, WasmManifest,
};
use tokio::task;
use tracing::info;

use super::ControlPlaneService;
use crate::{config_utils, http_routes, synsvc_native::SynSvcNativeService};

#[async_trait::async_trait]
pub trait OrchestratorInterface {
    async fn readyz(&self, service_id: String, caller: &CallerContext) -> Result<(), String>;
    /// The instance signing key this substrate would derive for `service_id`
    /// under `caller`'s identity, answerable before the service is deployed
    /// (ADR-0020 §3): the master holder certifies this key without the
    /// substrate ever holding the master.
    async fn instance_identity(
        &self,
        service_id: String,
        caller: &CallerContext,
    ) -> Result<InstanceIdentity, String>;
    async fn deploy(
        &self,
        service_id: String,
        manifest: DeployManifest,
        caller: &CallerContext,
    ) -> Result<(), String>;
    async fn undeploy(&self, service_id: String, caller: &CallerContext) -> Result<(), String>;
    async fn list(&self, caller: &CallerContext) -> Result<Vec<DeployedService>, String>;
    async fn deploy_plan(&self, plan: DeploymentPlan, caller: &CallerContext)
    -> Result<(), String>;
}

/// Resolves a manifest document to its content. `Inline` arrives with the
/// deploy call itself; `Path` is read from the substrate host's own
/// filesystem, under `deploy_docs`' traversal and size guards, on a blocking
/// thread since it touches the disk.
async fn resolve_document(
    source: &DocumentSource,
    field_name: &'static str,
) -> Result<String, String> {
    match source {
        DocumentSource::Inline(content) => {
            deploy_docs::check_inline_size(content, field_name)?;
            Ok(content.clone())
        }
        DocumentSource::Path(path) => {
            let path = PathBuf::from(path);
            task::spawn_blocking(move || deploy_docs::read_host_document(&path, field_name))
                .await
                .map_err(|e| format!("Failed to spawn blocking task: {}", e))?
        }
    }
}

/// D-04-02-c's deploy-time author-time warning: compares a deployed policy's
/// `definitions:` against the service's actual collections (its own tables
/// are the collection inventory -- a manifest declares no collection list of
/// its own). Warn-only in both directions, never a hard failure:
/// 1. a table with no matching `definitions:` entry is unfiltered today and
///    would be denied under `strict: true`;
/// 2. a `definitions:` entry whose `table` doesn't exist yet is expected for a
///    TCP/container service whose collections are created lazily on first use,
///    so it must not read as an error.
fn warn_on_policy_collection_mismatch(service_id: &str, policy: &Policy, collections: &[String]) {
    let defined_tables: BTreeSet<&str> =
        policy.definitions.values().map(|d| d.table.as_str()).collect();
    for collection in collections {
        if !defined_tables.contains(collection.as_str()) {
            tracing::warn!(
                service_id,
                collection,
                "collection has no FDAE definition; it is unfiltered today and would be denied \
                 under `strict: true`"
            );
        }
    }
    for (type_name, def) in &policy.definitions {
        if !collections.iter().any(|c| c == &def.table) {
            tracing::warn!(
                service_id,
                definition = type_name.as_str(),
                table = def.table.as_str(),
                "policy defines a collection but no such collection exists yet -- expected for a \
                 TCP/container service whose collections are created lazily on first use"
            );
        }
    }
}

/// Author-time lint, in the same additive warn-only class as
/// `warn_on_policy_collection_mismatch`: flags a definition where an
/// unconditionally-public permission (`paths: []`, compiles to `1=1`) shares
/// a covering ability with a path-restricted sibling permission, and the two
/// aren't linked by `includes`. The compiler ORs every covering permission
/// together (`applicable_permissions`), so a caller holding a generic
/// ability-scoped capability -- not a named `app/<type>.<permission>` grant
/// -- that satisfies the restricted permission's ability is also admitted
/// through the public one, silently widening access past the restricted
/// permission's own `paths`. Sometimes intended (that's what `includes` is
/// for, to make it explicit); often a policy-authoring mistake, so it's
/// worth a loud warning even though nothing here justifies failing the
/// deploy.
fn warn_on_ambiguous_public_permission(service_id: &str, policy: &Policy) {
    for (type_name, def) in &policy.definitions {
        for (public_name, public_perm) in &def.permissions {
            if !public_perm.paths.is_empty() {
                continue;
            }
            for (restricted_name, restricted_perm) in &def.permissions {
                if public_name == restricted_name || restricted_perm.paths.is_empty() {
                    continue;
                }
                if public_perm.includes.contains(restricted_name)
                    || restricted_perm.includes.contains(public_name)
                {
                    continue;
                }
                let shares_covering_ability = public_perm.allows.iter().any(|a| {
                    restricted_perm.allows.iter().any(|b| {
                        let (a, b) = (Ability(a.clone()), Ability(b.clone()));
                        a.0 == b.0 || a.entails(&b) || b.entails(&a)
                    })
                });
                if shares_covering_ability {
                    tracing::warn!(
                        service_id,
                        definition = type_name.as_str(),
                        public_permission = public_name.as_str(),
                        restricted_permission = restricted_name.as_str(),
                        "an unconditionally public permission (paths: []) shares a covering \
                         ability with a path-restricted sibling permission and the two aren't \
                         linked by `includes` -- any capability admitted for the restricted \
                         permission is also admitted for the public one, silently granting \
                         unrestricted access unless callers only ever hold a named \
                         app/<type>.<permission> capability; link them with `includes` if this is \
                         intended"
                    );
                }
            }
        }
    }
}

/// Whether any permission in `policy` opts into the stage-4 after-step
/// (ADR-0017 §7, `authorize_rows: true`). Whole-policy, unlike
/// `syneroym_fdae::definition_has_abac`'s per-collection question -- the
/// deploy-time gate below needs to know before a single component/service
/// type is chosen, since a TCP/container service has no guest to call for
/// *any* collection.
fn policy_declares_stage4(policy: &Policy) -> bool {
    policy.definitions.values().any(|def| def.permissions.values().any(|p| p.authorize_rows))
}

impl ControlPlaneService {
    async fn register_wasm_endpoints(
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

    /// Logs (but does not propagate) a failure to roll back a config
    /// generation saved just before a deploy that then failed. Best-effort:
    /// the deploy error itself is what gets returned to the caller.
    async fn rollback_config_generation(&self, service_id: &str, generation: u64) {
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
    async fn rollback_fdae_policy(&self, service_id: &str, previous: &Option<String>) {
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

    async fn deploy_wasm_service(
        &self,
        service_id: &str,
        manifest: &DeployManifest,
        wasm_manifest: &WasmManifest,
        new_gen: u64,
        previous_fdae_policy: &Option<String>,
        new_fdae_policy: Option<&Policy>,
    ) -> Result<(), String> {
        if let Err(e) = self.app_sandbox_engine.deploy_wasm(service_id, manifest).await {
            self.rollback_config_generation(service_id, new_gen).await;
            self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
            return Err(format!("WASM deployment failed: {e}"));
        }

        // D-B4-1/validate_stage4_export (ADR-0017 §8): a policy that opts a
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

        if let Err(e) =
            self.register_wasm_endpoints(service_id, wasm_manifest.interfaces.clone()).await
        {
            self.rollback_config_generation(service_id, new_gen).await;
            self.rollback_fdae_policy(service_id, previous_fdae_policy).await;
            return Err(format!("Endpoint registration failed: {e}"));
        }
        Ok(())
    }

    async fn deploy_tcp_service(
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

    async fn deploy_container_service(
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

#[async_trait::async_trait]
impl OrchestratorInterface for ControlPlaneService {
    /// M04A Slice B7b (§2.4.1): `readyz` has two forms, and only one is a
    /// status-check in the ownership sense. Empty `service_id` is a
    /// substrate-liveness ping -- `SyneroymClient::wait_for_ready` calls it
    /// pre-capability during `connect()`, so gating it would break connect
    /// for every ordinary client; it stays open, as a health probe (design
    /// §6.1.2's spirit: liveness is not an authorization surface). A
    /// non-empty `service_id` is a per-service readiness check (task.md item
    /// 1's "status-check") and is gated on `orchestrator/status`, exactly
    /// like `deploy`/`undeploy` gate on their own abilities below --
    /// node-wide authority (the owner, or on an unowned substrate, anyone --
    /// F4) passes for free; otherwise the caller needs a grant covering this
    /// app.
    async fn readyz(&self, service_id: String, caller: &CallerContext) -> Result<(), String> {
        if !service_id.is_empty() {
            if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_STATUS) {
                let resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
                if !caller
                    .has_capability(&resource, &Ability(Ability::ORCHESTRATOR_STATUS.to_string()))
                {
                    return Err(format!(
                        "caller {} holds no orchestrator/status grant for '{service_id}'",
                        caller.caller_did
                    ));
                }
            }

            let endpoints = self.registry.lookup_by_service(&service_id);
            let mut is_container = false;
            for (_, endpoint) in endpoints {
                if matches!(endpoint, SubstrateEndpoint::TcpHostPort { .. }) {
                    is_container = true;
                    break;
                }
            }
            if is_container {
                self.podman_sandbox_engine
                    .readyz(&service_id)
                    .await
                    .map_err(|e| format!("Container readiness check failed: {e}"))?;
            }
        }
        Ok(())
    }

    /// Gated like `readyz`'s per-service form: this returns a public key, not
    /// an authority, but it is still `ORCHESTRATOR_STATUS`-scoped rather than
    /// open, since enumerating `(owner, service_id)` pairs is otherwise free
    /// reconnaissance of every derived instance key on the node.
    async fn instance_identity(
        &self,
        service_id: String,
        caller: &CallerContext,
    ) -> Result<InstanceIdentity, String> {
        if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_STATUS) {
            let resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
            if !caller.has_capability(&resource, &Ability(Ability::ORCHESTRATOR_STATUS.to_string()))
            {
                return Err(format!(
                    "caller {} holds no orchestrator/status grant for '{service_id}'",
                    caller.caller_did
                ));
            }
        }

        let instance = self.node_identity.derive_service_identity(&caller.caller_did, &service_id);
        Ok(InstanceIdentity {
            instance_did: derive_did_key(&instance.public_key()),
            pubkey_hex: hex::encode(instance.public_key().to_bytes()),
        })
    }

    async fn deploy(
        &self,
        service_id: String,
        manifest: DeployManifest,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // M04A Slice B7a / F7: a service_id already owned by someone else may
        // not be re-deployed into. On an unowned substrate every caller
        // holds node-wide orchestrator authority (F4), so this never fires
        // and today's overwrite-on-redeploy behavior is preserved exactly --
        // without a mode branch. Checks ORCHESTRATOR_DEPLOY specifically
        // (post-review fix, not the old single-ability
        // `has_node_wide_orchestrator_authority`): a caller who holds only
        // `orchestrator/status` must not be able to override someone else's
        // takeover protection just because they can also list every app.
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
        // deploy flow, which is a larger change than this slice's scope.
        if let Some(existing) = self.registry.owner_of(&service_id)
            && existing != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {existing}; redeploy must come from its owner \
                 or a substrate owner"
            ));
        }

        // M04A Slice B7b (§3.2): Tier-1 deploy admission. The caller must
        // hold `orchestrator/deploy` covering this app. No owner/unowned
        // branch and no separate substrate-owner bypass here: a bare
        // `substrate:<node>` capability (the owner's `substrate/admin`, or
        // the unowned grant of F4) is `is_substrate_scope`, so `grants`
        // wildcards the resource and only `entails` has to hold -- both
        // pass here for free. An app-scoped B7b grantee is prefix-covered
        // instead. One check, three principals, no branch.
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
                Some(cert_json) => {
                    let cert = DelegationCertificate::from_json(cert_json)
                        .map_err(|e| format!("Invalid instance certificate: {e}"))?;
                    // (1) the certificate is for *this* member.
                    if cert.master_did != service_id {
                        return Err(format!(
                            "instance certificate master_did '{}' does not name this deploy's \
                             service_id '{service_id}'",
                            cert.master_did
                        ));
                    }
                    // (2) it certifies *this node's* derived key, not some
                    // other key the client chose.
                    let derived_did = derive_did_key(
                        &self
                            .node_identity
                            .derive_service_identity(&caller.caller_did, &service_id)
                            .public_key(),
                    );
                    if cert.temporary_did != derived_did {
                        return Err(format!(
                            "instance certificate certifies '{}', not the key this substrate \
                             would derive ('{derived_did}') for this caller and service_id",
                            cert.temporary_did
                        ));
                    }
                    // (3) signature, validity window, and the narrow scope.
                    cert.verify(&service_id, &[SCOPE_SERVICE_INSTANCE])
                        .map_err(|e| format!("Invalid instance certificate: {e}"))?;
                    Some(cert)
                }
                None => None,
            };

        if let Some(cert) = &manifest.registry_certificate {
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

        // Configuration Generation & Validation
        let mut flat_config = BTreeMap::new();
        // M3B Slice 7: `http_routes` is a reserved top-level key inside
        // `custom_config`'s JSON (see `crate::http_routes`) -- parsed here,
        // alongside the existing flatten step, since this is already the
        // one place `custom_config` gets interpreted rather than treated as
        // opaque. A malformed `http_routes` value fails deploy the same way
        // a schema violation does, rather than silently discarding routes.
        let mut http_routes = Vec::new();
        if let Some(custom_config_str) = &manifest.config.custom_config {
            let custom_json: Value = serde_json::from_str(custom_config_str)
                .map_err(|e| format!("custom_config is not valid JSON: {}", e))?;
            http_routes = http_routes::parse_http_routes(&custom_json)?;

            if let Some(schema_source) = &manifest.config.schema {
                let schema_str = resolve_document(schema_source, "schema").await?;

                let custom_json_clone = custom_json.clone();
                task::spawn_blocking(move || -> Result<(), String> {
                    let schema_json: Value = serde_json::from_str(&schema_str)
                        .map_err(|e| format!("JSON schema is not valid JSON: {}", e))?;

                    let compiled_schema = jsonschema::validator_for(&schema_json)
                        .map_err(|e| format!("Invalid JSON schema: {}", e))?;

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
                .map_err(|e| format!("Failed to spawn blocking task: {}", e))??;
            }

            config_utils::flatten_json_config(&custom_json, "", &mut flat_config);
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
            .map_err(|e| format!("Failed to serialize flattened config: {}", e))?;

        let new_gen = self
            .storage_provider
            .save_config_generation(&service_id, &config_blob)
            .await
            .map_err(|e| format!("Failed to save config generation: {}", e))?;
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
            .map_err(|e| format!("Failed to check existing FDAE policy: {}", e))?;
        if let Some((policy_doc, _)) = &fdae_policy {
            self.storage_provider
                .save_fdae_policy(&service_id, policy_doc)
                .await
                .map_err(|e| format!("Failed to save FDAE policy: {}", e))?;
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
                .map_err(|e| format!("Failed to clear FDAE policy: {}", e))?;
        }

        let new_fdae_policy = fdae_policy.as_ref().map(|(_, policy)| policy.as_ref());
        match &manifest.service_type {
            WitServiceType::Wasm(wasm_manifest) => {
                self.deploy_wasm_service(
                    &service_id,
                    &manifest,
                    wasm_manifest,
                    new_gen,
                    &previous_fdae_policy,
                    new_fdae_policy,
                )
                .await?;
            }
            WitServiceType::Tcp(tcp_manifest) => {
                self.deploy_tcp_service(
                    &service_id,
                    tcp_manifest,
                    new_gen,
                    &previous_fdae_policy,
                    new_fdae_policy,
                )
                .await?;
            }
            WitServiceType::Container(container_manifest) => {
                self.deploy_container_service(
                    &service_id,
                    &manifest,
                    container_manifest,
                    new_gen,
                    &previous_fdae_policy,
                    new_fdae_policy,
                )
                .await?;
            }
        }

        // D-04-02-c's author-time `strict:` warning: the service's own
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
                if let Err(undeploy_err) = self.undeploy(service_id.clone(), caller).await {
                    tracing::error!(
                        "Failed to roll back partially deployed service {} after native \
                         capability registration error: {}",
                        service_id,
                        undeploy_err
                    );
                }
                self.rollback_config_generation(&service_id, new_gen).await;
                self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
                return Err(format!("Native capability registration failed: {e}"));
            }
        }
        if let Some(native_dispatch) = self.native_dispatch.upgrade() {
            native_dispatch.insert(
                service_id.clone(),
                Arc::new(SynSvcNativeService::new(
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
                )) as Arc<dyn NativeService>,
            );
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

        // M04A Slice B7a: record the owner last, after every other step
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
        // reason, predating B7a. `deploy` has never been transactional
        // across config-generation / engine / registry writes (plan §2.3,
        // "Known non-atomicity... B7a does not make this worse"); making a
        // re-deploy's late failure preserve the prior running version would
        // need a genuinely versioned/staged deploy (keep the old instance
        // live until the new one fully commits), which is a materially
        // larger change than this slice's scope -- not attempted here.
        if let Err(e) = self.registry.set_owner(service_id.clone(), caller.caller_did.clone()).await
        {
            if let Err(undeploy_err) = self.undeploy(service_id.clone(), caller).await {
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
            if let Err(undeploy_err) = self.undeploy(service_id.clone(), caller).await {
                tracing::error!(
                    "rollback after instance-certificate installation failure also failed: \
                     {undeploy_err}"
                );
            }
            self.rollback_config_generation(&service_id, new_gen).await;
            self.rollback_fdae_policy(&service_id, &previous_fdae_policy).await;
            return Err(format!("Instance certificate installation failed: {e}"));
        }

        Ok(())
    }

    /// M04A Slice B7a / F7: gates on ownership before tearing anything down
    /// -- a non-owner undeploying someone else's service is the same
    /// escalation as taking it over via redeploy. Checks
    /// `ORCHESTRATOR_UNDEPLOY` specifically (post-review fix -- see
    /// `has_node_wide_ability`'s doc comment): a status-only grantee must
    /// not be able to undeploy someone else's app.
    ///
    /// Safe to call from `deploy`'s own rollback path (§2.3): at that point
    /// `owner_of` is one of (a) `None` (the native-capability-registration
    /// failure path, reached before `set_owner` ever ran), (b) already
    /// `caller.caller_did` (the happy-path retry: this same `deploy` call
    /// already ran `set_owner` successfully once, or this is an ordinary
    /// owner re-deploying their own service), or (c) a *different* DID that
    /// `caller` is redeploying over while holding node-wide authority -- in
    /// which case this gate passes via that authority, not because the row
    /// matches `caller.caller_did`. All three pass; there is no branch where
    /// `deploy`'s own rollback gets rejected by this check.
    async fn undeploy(&self, service_id: String, caller: &CallerContext) -> Result<(), String> {
        if let Some(owner) = self.registry.owner_of(&service_id)
            && owner != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_UNDEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {owner}; only its owner or a substrate owner \
                 may undeploy it"
            ));
        }

        // M04A Slice B7b (§3.2): Tier-1 undeploy admission, the same shape
        // as `deploy`'s -- the caller must hold `orchestrator/undeploy`
        // covering this app.
        //
        // Interaction with `deploy`'s own rollback path (§2.3): `deploy`
        // calls `self.undeploy(service_id.clone(), caller)` with the *same*
        // `caller` on two failure paths. F4's unowned-substrate grant issues
        // all three `orchestrator/*` abilities together, so this never trips
        // there; on a real *owned* substrate it could, in principle, reject
        // a rollback for a caller who legitimately holds `orchestrator/
        // deploy` on this app but was never separately granted `orchestrator/
        // undeploy` for it -- abilities are deliberately flat and
        // independently grantable (§3.1 A2), so "deploy but not undeploy" is
        // a real, supported shape. That is inert today for the same reason
        // §6.1 records for the gate as a whole: nothing can create a
        // `ControllerAgreement` yet, so every substrate is unowned and every
        // verified caller holds all three abilities together. Revisit if a
        // deploy-only grantee becomes real before the ownership tooling
        // lands.
        let undeploy_resource =
            ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller.has_capability(
            &undeploy_resource,
            &Ability(Ability::ORCHESTRATOR_UNDEPLOY.to_string()),
        ) {
            return Err(format!(
                "caller {} holds no orchestrator/undeploy grant for '{service_id}' on this \
                 substrate",
                caller.caller_did
            ));
        }

        info!("Undeploying service: {}", service_id);

        let cert_path = self.hosted_apps_dir.join(format!("{service_id}.json"));
        if cert_path.exists()
            && let Err(e) = fs::remove_file(&cert_path)
        {
            tracing::warn!("Failed to remove registry certificate for {}: {}", service_id, e);
        }

        let endpoints = self.registry.lookup_by_service(&service_id);
        let mut is_wasm = false;
        let mut is_container = false;

        for (interface_name, endpoint) in endpoints {
            if matches!(endpoint, SubstrateEndpoint::WasmChannel { .. }) {
                is_wasm = true;
            } else if matches!(endpoint, SubstrateEndpoint::TcpHostPort { .. }) {
                is_container = true;
            }
            if let Err(e) = self.registry.remove(&service_id, &interface_name).await {
                tracing::warn!(
                    "Failed to remove endpoint {} for service {}: {}",
                    interface_name,
                    service_id,
                    e
                );
            }
        }

        if is_wasm {
            if let Err(e) = self.app_sandbox_engine.stop_wasm(&service_id).await {
                tracing::warn!("Failed to stop WASM engine for service {}: {}", service_id, e);
            }
            if let Err(e) = self.app_sandbox_engine.remove_wasm(&service_id).await {
                tracing::warn!("Failed to remove WASM file for service {}: {}", service_id, e);
            }
        }

        if is_container {
            if let Err(e) = self.podman_sandbox_engine.stop(&service_id).await {
                tracing::warn!("Failed to stop Container engine for service {}: {}", service_id, e);
            }
            if let Err(e) = self.podman_sandbox_engine.remove(&service_id).await {
                tracing::warn!("Failed to remove Container for service {}: {}", service_id, e);
            }
        }

        // Messaging subscriptions have no analogue among the other 4 native
        // capabilities: they're a long-lived stateful subsystem (persisted
        // rows plus live broker registrations), not pure request/response,
        // so they need an explicit "forget this service" step the
        // endpoint-registry loop above doesn't cover.
        if let Err(e) =
            self.storage_provider.delete_all_messaging_subscriptions_for_service(&service_id).await
        {
            tracing::warn!(
                "Failed to remove messaging subscriptions for service {}: {}",
                service_id,
                e
            );
        }
        if is_wasm {
            self.app_sandbox_engine.unsubscribe_all(&service_id);
        }

        // An `fdae_policies` row has no in-memory analogue that gets torn
        // down for free elsewhere in this function -- `stop_wasm` above only
        // evicts the WASM engine's *cache* of it, and native dispatch's copy
        // dies with the `SynSvcNativeService` removed below. Without this, a
        // later re-deploy of the same `service_id` with no `fdae` block
        // would still have `AppSandboxEngine::resolve_fdae_policy` resurrect
        // this row from storage on its next cache miss.
        if let Err(e) = self.storage_provider.delete_fdae_policy(&service_id).await {
            tracing::warn!("Failed to remove FDAE policy for service {}: {}", service_id, e);
        }

        // The endpoint-registry loop above already removed the 6 native
        // capability interfaces generically (it iterates every registered
        // interface for this service_id); just drop the in-memory dispatch
        // entry too.
        if let Some(native_dispatch) = self.native_dispatch.upgrade() {
            native_dispatch.remove(&service_id);
        } else {
            tracing::error!(
                "Native dispatch registry unavailable while undeploying service {}: its in-memory \
                 dispatch entry, if any, was left behind",
                service_id
            );
        }
        self.http_routes.remove(&service_id);

        // Warn-not-fail, matching every other teardown step above (endpoints,
        // subscriptions, http_routes are all best-effort).
        if let Err(e) = self.registry.remove_owner(&service_id).await {
            tracing::warn!("Failed to remove owner record for service {}: {}", service_id, e);
        }
        if let Err(e) = self.registry.remove_instance_cert(&service_id).await {
            tracing::warn!(
                "Failed to remove instance certificate for service {}: {}",
                service_id,
                e
            );
        }

        Ok(())
    }

    async fn list(&self, caller: &CallerContext) -> Result<Vec<DeployedService>, String> {
        let endpoints = self.registry.get_all_endpoints();
        let mut services: HashMap<String, DeployedService> = HashMap::new();

        for (service_id, interface, endpoint) in endpoints {
            // The native-capability interfaces (data-layer/vault/app-config/
            // blob-store/messaging/http) are host-provided plumbing registered
            // on every deployed service regardless of type -- they must not be
            // mistaken for the service's own declared interfaces, nor
            // influence `endpoint_type` (every deployed service also always
            // has its real wasm/container/tcp endpoint registered).
            if NATIVE_CAPABILITY_INTERFACES.contains(&interface.as_str()) {
                continue;
            }
            let registry = &self.registry;
            let entry = services.entry(service_id.clone()).or_insert_with(|| DeployedService {
                service_id: service_id.clone(),
                interfaces: Vec::new(),
                endpoint_type: match endpoint {
                    SubstrateEndpoint::WasmChannel { .. } => "wasm".to_string(),
                    SubstrateEndpoint::PodmanSocket { .. } => "podman".to_string(),
                    SubstrateEndpoint::NativeHostChannel { .. } => "native".to_string(),
                    SubstrateEndpoint::TcpHostPort { .. } => "tcp".to_string(),
                },
                instance_certificate_expires_at: registry
                    .instance_cert(&service_id)
                    .map(|cert| cert.expires_at_secs),
            });
            entry.interfaces.push(interface);
        }

        let mut result: Vec<DeployedService> = services.into_values().collect();
        result.sort_by(|a, b| a.service_id.cmp(&b.service_id));

        // M04A Slice B7a: node-wide orchestrator authority sees everything --
        // the substrate owner, or on an unowned substrate, everyone (F4),
        // preserving today's behavior with no mode branch. Checks
        // ORCHESTRATOR_STATUS specifically (unlike deploy/undeploy's checks
        // above): a status-only monitoring grantee is meant to see the
        // list -- that is what the ability names -- without thereby gaining
        // any deploy/undeploy override, which the two checks above enforce
        // independently.
        if self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_STATUS) {
            return Ok(result);
        }
        // A service owner sees only their own. `owner_of` == None (deployed
        // pre-B7a, or the §2.3 crash window) filters OUT: an unattributed
        // app is not "everyone's", and defaulting it visible would make that
        // window a disclosure bug. The substrate owner still sees it above.
        Ok(result
            .into_iter()
            .filter(|s| {
                self.registry.owner_of(&s.service_id).as_deref() == Some(caller.caller_did.as_str())
            })
            .collect())
    }

    async fn deploy_plan(
        &self,
        plan: DeploymentPlan,
        caller: &CallerContext,
    ) -> Result<(), String> {
        for service in plan.services {
            let service_id = service.service_id.clone();

            // Only allow WASM sources that do not use path traversal and stay within an
            // allowed directory Note: Since deploy-plan is handled over RPC, we
            // restrict file source reads to the current directory
            // or an explicit sandbox.
            let mut deploy_manifest = service.manifest.clone();

            match &mut deploy_manifest.service_type {
                WitServiceType::Wasm(wasm_manifest) => {
                    if let ArtifactSource::Binary(_) = &wasm_manifest.source {
                        // Binary is fine, it was passed directly
                    } else if let ArtifactSource::Url(url_or_path) = &wasm_manifest.source
                        && !url_or_path.starts_with("http://")
                        && !url_or_path.starts_with("https://")
                    {
                        // It's a local file path
                        let path = PathBuf::from(url_or_path);

                        // Path traversal check
                        if path.components().any(|c| matches!(c, Component::ParentDir))
                            || path.is_absolute()
                        {
                            return Err(format!(
                                "Arbitrary file read prevented: Path traversal or absolute paths \
                                 are not allowed in deploy-plan: {:?}",
                                path
                            ));
                        }

                        let bytes = util::read_local_artifact(&path).map_err(|e| {
                            format!("Failed to read WASM file at {:?}: {}", path, e)
                        })?;
                        wasm_manifest.source = ArtifactSource::Binary(bytes);
                    }
                }
                WitServiceType::Tcp(_) | WitServiceType::Container(_) => {
                    // TCP and Container don't read host files directly in
                    // deploy_plan logic for sources
                }
            }

            self.deploy(service_id, deploy_manifest, caller).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use dashmap::DashMap;
    use syneroym_core::{
        config::SubstrateConfig,
        http_routes::HttpRouteRegistry,
        local_registry::EndpointRegistry,
        storage::{EndpointStorage, MockStorage},
    };
    use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
    use syneroym_data_db::{SqliteStorageProvider, traits::StorageProvider};
    use syneroym_data_keystore::KeyStore;
    use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
    use syneroym_rpc::NativeDispatchRegistry;
    use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
        NetworkEndpoint, PlannedService, ServiceConfig,
    };

    use super::*;
    use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

    /// M04A Slice B7b: a caller holding node-wide orchestrator authority on
    /// `"did:key:zTestNode"` (every test in this module inits
    /// `ControlPlaneService` with that node DID) -- the shape `build_caller`
    /// issues for the F4 unowned-substrate bootstrap grant. Deploy/undeploy
    /// now gate on an explicit `orchestrator/{deploy,undeploy}` capability
    /// (§3.2), so every test below that exercises `deploy`/`deploy_plan`/
    /// `undeploy` and expects to get *past* that gate (to reach a
    /// path-traversal/schema/rollback/ownership assertion further in) needs
    /// a caller that holds it -- `CallerContext::service_system` (zero
    /// capabilities) no longer suffices on its own.
    fn node_wide_caller(caller_did: &str) -> CallerContext {
        use syneroym_rpc::{AuthLevel, Capability, SessionContext};

        let resource = ResourceUri::substrate("did:key:zTestNode");
        CallerContext {
            caller_did: caller_did.to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: caller_did.to_string(),
                capabilities: vec![
                    Capability {
                        with: resource.clone(),
                        can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
                        caveats: None,
                    },
                    Capability {
                        with: resource,
                        can: Ability(Ability::ORCHESTRATOR_UNDEPLOY.to_string()),
                        caveats: None,
                    },
                ],
                ..Default::default()
            },
            auth: AuthLevel::Delegated,
            proof: None,
        }
    }

    #[tokio::test]
    async fn test_deploy_plan_path_traversal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        // Create a deployment plan with path traversal in source
        let plan = DeploymentPlan {
            app_instance_id: "test-instance".to_string(),
            blueprint_id: "test-blueprint".to_string(),
            version: "0.1.0".to_string(),
            services: vec![PlannedService {
                service_id: "did:key:test".to_string(),
                logical_ref: "test/main".to_string(),
                manifest: DeployManifest {
                    config: ServiceConfig {
                        env: vec![],
                        args: vec![],
                        custom_config: None,
                        quota: None,
                        schema: None,
                        rotation_policy: None,
                        fdae_policy: None,
                    },
                    service_type: WitServiceType::Wasm(WasmManifest {
                        source: ArtifactSource::Url("../../../../../etc/passwd".to_string()),
                        hash: None,
                        interfaces: vec![],
                    }),
                    registry_certificate: None,
                    instance_certificate: None,
                },
            }],
        };

        let result = service.deploy_plan(plan, &CallerContext::service_system("test-caller")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Arbitrary file read prevented: Path traversal"));
    }

    #[tokio::test]
    async fn test_deploy_plan_absolute_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let plan = DeploymentPlan {
            app_instance_id: "test-instance".to_string(),
            blueprint_id: "test-blueprint".to_string(),
            version: "0.1.0".to_string(),
            services: vec![PlannedService {
                service_id: "did:key:test".to_string(),
                logical_ref: "test/main".to_string(),
                manifest: DeployManifest {
                    config: ServiceConfig {
                        env: vec![],
                        args: vec![],
                        custom_config: None,
                        quota: None,
                        schema: None,
                        rotation_policy: None,
                        fdae_policy: None,
                    },
                    service_type: WitServiceType::Wasm(WasmManifest {
                        source: ArtifactSource::Url("/etc/passwd".to_string()),
                        hash: None,
                        interfaces: vec![],
                    }),
                    registry_certificate: None,
                    instance_certificate: None,
                },
            }],
        };

        let result = service.deploy_plan(plan, &CallerContext::service_system("test-caller")).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Arbitrary file read prevented: Path traversal or absolute paths")
        );
    }

    #[tokio::test]
    async fn test_deploy_config_schema_rejection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        // Write a schema file with a relative path
        let schema_filename = format!("test_schema_{}.json", std::process::id());
        fs::write(
            &schema_filename,
            r#"{"type": "object", "properties": {"port": {"type": "integer"}}}"#,
        )
        .unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: Some(r#"{"port": "8080"}"#.to_string()), // string instead of int
                quota: None,
                schema: Some(DocumentSource::Path(schema_filename.clone())),
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy("test_service".to_string(), manifest, &node_wide_caller("test-caller"))
            .await;

        let _ = fs::remove_file(&schema_filename);

        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(err_msg.contains("Configuration validation failed"), "{}", err_msg);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_deploy_schema_symlink_escape_rejected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        // A symlink under the working directory whose target lives outside
        // it. No `..` component and not absolute, so the component check
        // alone would let it through; only canonicalizing the resolved path
        // catches it.
        let outside_dir = tempfile::tempdir().unwrap();
        let outside_schema = outside_dir.path().join("schema.json");
        fs::write(&outside_schema, r#"{"type": "object"}"#).unwrap();

        let symlink_name = format!("test_schema_symlink_{}.json", std::process::id());
        std::os::unix::fs::symlink(&outside_schema, &symlink_name).unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: Some(r#"{"port": 8080}"#.to_string()),
                quota: None,
                schema: Some(DocumentSource::Path(symlink_name.clone())),
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy(
                "symlink_schema_service".to_string(),
                manifest,
                &node_wide_caller("test-caller"),
            )
            .await;

        let _ = fs::remove_file(&symlink_name);

        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("resolves outside the working directory via a symlink"),
            "{}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_deploy_config_generation_rollback() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        // Deliberately malformed WasmManifest source to cause a deployment failure
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: Some(r#"{"key": "value"}"#.to_string()),
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Url("/does_not_exist.wasm".to_string()),
                hash: None,
                interfaces: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy("rollback_service".to_string(), manifest, &node_wide_caller("test-caller"))
            .await;
        assert!(result.is_err()); // deployment must fail

        // Config generation should not exist
        let latest =
            storage_provider.get_latest_config_generation("rollback_service").await.unwrap();
        assert!(latest.is_none());
    }

    #[tokio::test]
    async fn test_deploy_fdae_policy_validates_persists_and_is_loadable() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        // A policy with no `custom_config` on the manifest -- the regression
        // test for the FDAE block's placement outside the `custom_config`
        // block (unlike `schema`, which is only read inside it).
        let policy_filename = format!("test_fdae_policy_{}.json", std::process::id());
        fs::write(&policy_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_filename.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy("fdae_test_service".to_string(), manifest, &node_wide_caller("test-caller"))
            .await;

        let _ = fs::remove_file(&policy_filename);

        assert!(result.is_ok(), "{:?}", result);
        let loaded = storage_provider.load_fdae_policy("fdae_test_service").await.unwrap();
        assert_eq!(loaded, Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string()));
    }

    /// A minimal, real component that compiles successfully but exports
    /// nothing under `syneroym:data-layer/authorizer` -- the same
    /// `wat` shape
    /// `test_deploy_failure_after_successful_wasm_compile_rolls_back_gen_and_policy`
    /// uses to get a real, cheap-to-build component without a
    /// `cargo-component`-built fixture.
    const WASM_WITHOUT_AUTHORIZE_ROWS_EXPORT: &str = r#"
(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface (export "greet" (func $noop)))
  (export "test-interface" (instance $interface))
)
"#;

    /// A policy opting a single permission into the stage-4 after-step
    /// (`authorize_rows: true`, ADR-0017 §7).
    const STAGE4_POLICY: &str = r#"{
        "version": "fdae/v1",
        "definitions": {
            "items": {
                "table": "items",
                "principal_column": "creator_id",
                "permissions": {
                    "view": {
                        "allows": ["data-layer/read"],
                        "paths": [["caller"]],
                        "authorize_rows": true
                    }
                }
            }
        }
    }"#;

    /// D-B4-1/`validate_stage4_export` (ADR-0017 §8): a policy that opts
    /// into the stage-4 after-step but whose compiled WASM component does
    /// not export `syneroym:data-layer/authorizer#authorize-rows` must fail
    /// the deploy, not ship a service that silently denies every read
    /// through that permission at runtime.
    #[tokio::test]
    async fn test_stage4_policy_without_the_export_fails_deploy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Inline(STAGE4_POLICY.to_string())),
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(
                    WASM_WITHOUT_AUTHORIZE_ROWS_EXPORT.as_bytes().to_vec(),
                ),
                hash: None,
                interfaces: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy(
                "stage4_missing_export_svc".to_string(),
                manifest,
                &node_wide_caller("test-caller"),
            )
            .await;
        assert!(
            result.is_err(),
            "a stage-4-opted policy on a component without the export must fail"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("authorize_rows: true") && err.contains("does not export"),
            "expected the validate_stage4_export error, got: {err}"
        );
    }

    /// Same shape as `deploy_wasm_service`'s gate above, for the two
    /// service types that have no guest component to call at all: a TCP
    /// service can never satisfy a stage-4 opt-in, so it is rejected up
    /// front, before any endpoint registration.
    #[tokio::test]
    async fn test_stage4_policy_on_a_tcp_service_fails_deploy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Inline(STAGE4_POLICY.to_string())),
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy("stage4_tcp_svc".to_string(), manifest, &node_wide_caller("test-caller"))
            .await;
        assert!(result.is_err(), "a stage-4-opted policy on a TCP service must fail deploy");
        let err = result.unwrap_err();
        assert!(
            err.contains("authorize_rows: true") && err.contains("no guest component"),
            "expected the TCP-service stage-4 rejection, got: {err}"
        );
    }

    /// A rejected stage-4 deploy must not leave its (already-persisted, per
    /// `deploy`'s save-then-validate ordering) policy row in force --
    /// `rollback_fdae_policy` must restore whatever was there before (here,
    /// nothing at all: `stage4_rollback_svc` has never deployed before).
    #[tokio::test]
    async fn test_stage4_policy_rejection_rolls_back_the_policy_row() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Inline(STAGE4_POLICY.to_string())),
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(
                    WASM_WITHOUT_AUTHORIZE_ROWS_EXPORT.as_bytes().to_vec(),
                ),
                hash: None,
                interfaces: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy("stage4_rollback_svc".to_string(), manifest, &node_wide_caller("test-caller"))
            .await;
        assert!(result.is_err(), "the deploy must still be rejected: {result:?}");
        assert_eq!(
            storage_provider.load_fdae_policy("stage4_rollback_svc").await.unwrap(),
            None,
            "a rejected stage-4 deploy must roll back the policy row it saved before validating \
             the export, not leave the rejected policy in force"
        );
    }

    #[tokio::test]
    async fn test_undeploy_removes_fdae_policy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let policy_filename = format!("test_fdae_undeploy_policy_{}.json", std::process::id());
        fs::write(&policy_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_filename.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let caller = node_wide_caller("test-caller");
        service.deploy("undeploy_fdae_svc".to_string(), manifest, &caller).await.unwrap();
        let _ = fs::remove_file(&policy_filename);
        assert!(storage_provider.load_fdae_policy("undeploy_fdae_svc").await.unwrap().is_some());

        service.undeploy("undeploy_fdae_svc".to_string(), &caller).await.unwrap();
        assert_eq!(
            storage_provider.load_fdae_policy("undeploy_fdae_svc").await.unwrap(),
            None,
            "undeploy must clear a service's persisted FDAE policy"
        );
    }

    #[tokio::test]
    async fn test_redeploy_without_fdae_block_clears_previous_policy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let policy_filename = format!("test_fdae_redeploy_policy_{}.json", std::process::id());
        fs::write(&policy_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();

        let caller = node_wide_caller("test-caller");
        let with_policy = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_filename.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("redeploy_fdae_svc".to_string(), with_policy, &caller).await.unwrap();
        let _ = fs::remove_file(&policy_filename);
        assert!(storage_provider.load_fdae_policy("redeploy_fdae_svc").await.unwrap().is_some());

        // Re-deploy the same service_id with no `fdae` block at all.
        let without_policy = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("redeploy_fdae_svc".to_string(), without_policy, &caller).await.unwrap();
        assert_eq!(
            storage_provider.load_fdae_policy("redeploy_fdae_svc").await.unwrap(),
            None,
            "a re-deploy whose manifest drops the fdae block must clear the previous policy, not \
             leave it for the WASM engine to resurrect from storage"
        );
    }

    #[tokio::test]
    async fn test_deploy_failure_restores_previous_fdae_policy_not_the_new_one() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let caller = node_wide_caller("test-caller");

        // First, a successful deploy with policy P1.
        let policy_1_filename = format!("test_fdae_rollback_p1_{}.json", std::process::id());
        fs::write(&policy_1_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
        let first = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_1_filename.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("rollback_fdae_svc".to_string(), first, &caller).await.unwrap();
        let _ = fs::remove_file(&policy_1_filename);
        assert_eq!(
            storage_provider.load_fdae_policy("rollback_fdae_svc").await.unwrap(),
            Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string())
        );

        // Re-deploy the same service_id as WASM, with a new policy P2 and a
        // WASM source that doesn't exist -- `deploy_wasm` fails, which must
        // restore P1, not leave P2 (already persisted before the failure)
        // or an empty row in place.
        let policy_2_filename = format!("test_fdae_rollback_p2_{}.json", std::process::id());
        fs::write(
            &policy_2_filename,
            r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#,
        )
        .unwrap();
        let second = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_2_filename.clone())),
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Url("/does_not_exist.wasm".to_string()),
                hash: None,
                interfaces: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let result = service.deploy("rollback_fdae_svc".to_string(), second, &caller).await;
        let _ = fs::remove_file(&policy_2_filename);
        assert!(result.is_err(), "the WASM deploy must fail: {result:?}");

        assert_eq!(
            storage_provider.load_fdae_policy("rollback_fdae_svc").await.unwrap(),
            Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string()),
            "a failed re-deploy must restore the previous policy, not leave the new one in force \
             or drop the row entirely -- the still-running previous version's engine cache would \
             otherwise resurrect the failed deploy's policy on its next miss"
        );
    }

    #[tokio::test]
    async fn test_deploy_failure_restores_a_policy_the_new_manifest_dropped() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let caller = node_wide_caller("test-caller");

        // First, a successful deploy with a policy.
        let policy_filename = format!("test_fdae_dropped_rollback_{}.json", std::process::id());
        fs::write(&policy_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
        let first = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_filename.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("dropped_rollback_svc".to_string(), first, &caller).await.unwrap();
        let _ = fs::remove_file(&policy_filename);
        assert_eq!(
            storage_provider.load_fdae_policy("dropped_rollback_svc").await.unwrap(),
            Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string())
        );

        // Re-deploy the same service_id as WASM, with no `fdae` block at all
        // (the new manifest's `config` fully declares this deploy's policy
        // state, so absence deletes the previous row up front) and a WASM
        // source that doesn't exist, so `deploy_wasm` fails after the
        // deletion already happened. The failure must restore the policy
        // that was there before this deploy attempt, not leave the row
        // deleted -- an already-running previous version must not lose its
        // policy to an unrelated failed re-deploy.
        let second = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Url("/does_not_exist.wasm".to_string()),
                hash: None,
                interfaces: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let result = service.deploy("dropped_rollback_svc".to_string(), second, &caller).await;
        assert!(result.is_err(), "the WASM deploy must fail: {result:?}");

        assert_eq!(
            storage_provider.load_fdae_policy("dropped_rollback_svc").await.unwrap(),
            Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string()),
            "a failed re-deploy whose manifest dropped the fdae block must restore the policy \
             that existed before this attempt, not leave it deleted -- the still-running previous \
             version's engine cache would otherwise resolve no policy on its next miss"
        );
    }

    /// Wraps `MockStorage`, failing `save` for one specific interface name --
    /// lets a test deterministically fail `EndpointRegistry::register`
    /// (used by `register_wasm_endpoints`/`deploy_container_service`'s
    /// registration loop) without needing a real network/podman failure.
    struct FailingEndpointStorage {
        inner: MockStorage,
        fail_interface: String,
    }

    #[async_trait::async_trait]
    impl EndpointStorage for FailingEndpointStorage {
        async fn load_all(&self) -> Result<Vec<(String, String, SubstrateEndpoint)>> {
            self.inner.load_all().await
        }
        async fn save(
            &self,
            service_id: &str,
            interface_name: &str,
            endpoint: &SubstrateEndpoint,
        ) -> Result<()> {
            if interface_name == self.fail_interface {
                anyhow::bail!("simulated registry storage failure for {interface_name}");
            }
            self.inner.save(service_id, interface_name, endpoint).await
        }
        async fn remove(&self, service_id: &str, interface_name: &str) -> Result<()> {
            self.inner.remove(service_id, interface_name).await
        }
        async fn load_all_owners(&self) -> Result<Vec<(String, String)>> {
            self.inner.load_all_owners().await
        }
        async fn save_owner(&self, service_id: &str, owner_did: &str) -> Result<()> {
            self.inner.save_owner(service_id, owner_did).await
        }
        async fn remove_owner(&self, service_id: &str) -> Result<()> {
            self.inner.remove_owner(service_id).await
        }
        async fn load_all_certs(&self) -> Result<Vec<(String, String)>> {
            self.inner.load_all_certs().await
        }
        async fn save_cert(&self, service_id: &str, certificate_json: &str) -> Result<()> {
            self.inner.save_cert(service_id, certificate_json).await
        }
        async fn remove_cert(&self, service_id: &str) -> Result<()> {
            self.inner.remove_cert(service_id).await
        }
    }

    #[tokio::test]
    async fn test_deploy_failure_after_successful_wasm_compile_rolls_back_gen_and_policy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        // The endpoint registry itself fails to persist one specific
        // interface -- simulating `register_wasm_endpoints` (called *after*
        // `deploy_wasm` has already compiled/cached the component and run
        // its lifecycle hook) hitting a real storage error.
        let registry = EndpointRegistry::new(Arc::new(FailingEndpointStorage {
            inner: MockStorage::new(),
            fail_interface: "fails-to-register".to_string(),
        }))
        .await
        .unwrap();

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let caller = node_wide_caller("test-caller");

        // First, a successful TCP deploy with policy P1, establishing a
        // baseline config generation and policy for the same service_id.
        let policy_1_filename = format!("test_fdae_endpoint_reg_p1_{}.json", std::process::id());
        fs::write(&policy_1_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
        let first = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_1_filename.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("endpoint_reg_svc".to_string(), first, &caller).await.unwrap();
        let _ = fs::remove_file(&policy_1_filename);
        let (gen_before, _) = storage_provider
            .get_latest_config_generation("endpoint_reg_svc")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            storage_provider.load_fdae_policy("endpoint_reg_svc").await.unwrap(),
            Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string())
        );

        // Re-deploy as WASM with a real, minimal, valid component (so
        // `deploy_wasm` itself succeeds) and a new policy P2, but declaring
        // the interface name the registry is rigged to reject -- so the
        // failure happens in `register_wasm_endpoints`, *after* the
        // component was already compiled/cached and P2 already persisted.
        let wat = r#"
(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface (export "greet" (func $noop)))
  (export "test-interface" (instance $interface))
)
"#;
        let policy_2_filename = format!("test_fdae_endpoint_reg_p2_{}.json", std::process::id());
        fs::write(
            &policy_2_filename,
            r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#,
        )
        .unwrap();
        let second = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_2_filename.clone())),
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wat.as_bytes().to_vec()),
                hash: None,
                interfaces: vec!["fails-to-register".to_string()],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let result = service.deploy("endpoint_reg_svc".to_string(), second, &caller).await;
        let _ = fs::remove_file(&policy_2_filename);
        assert!(result.is_err(), "endpoint registration must fail: {result:?}");
        assert!(result.unwrap_err().contains("Endpoint registration failed"));

        assert_eq!(
            storage_provider.load_fdae_policy("endpoint_reg_svc").await.unwrap(),
            Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string()),
            "a register_wasm_endpoints failure -- after the component was already compiled and \
             the new policy already persisted -- must restore the previous policy, not leave the \
             new one (P2) in force"
        );
        let (gen_after, _) = storage_provider
            .get_latest_config_generation("endpoint_reg_svc")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            gen_after, gen_before,
            "a register_wasm_endpoints failure must roll back the config generation this deploy \
             attempt saved, not leave it in force alongside a rolled-back policy"
        );
    }

    #[tokio::test]
    async fn test_deploy_tcp_endpoint_registration_failure_rolls_back_gen_and_policy() {
        // Regression: `deploy_tcp_service` used to have no rollback at all
        // -- a failed TCP redeploy left the new policy (P2) persisted and
        // the config generation bumped, with the previous, still-running
        // version's policy row silently replaced. Same shape as the
        // already-covered WASM/container arms, using the same
        // `FailingEndpointStorage` fixture to force the failure
        // deterministically instead of a real network error.
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new(Arc::new(FailingEndpointStorage {
            inner: MockStorage::new(),
            fail_interface: "fails-to-register".to_string(),
        }))
        .await
        .unwrap();

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let caller = node_wide_caller("test-caller");

        // First, a successful TCP deploy with policy P1, using an interface
        // name the registry accepts.
        let policy_1_filename = format!("test_tcp_rollback_p1_{}.json", std::process::id());
        fs::write(&policy_1_filename, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();
        let first = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_1_filename.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest {
                endpoints: vec![NetworkEndpoint {
                    interface_name: "safe-interface".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: 9000,
                }],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("tcp_rollback_svc".to_string(), first, &caller).await.unwrap();
        let _ = fs::remove_file(&policy_1_filename);
        assert_eq!(
            storage_provider.load_fdae_policy("tcp_rollback_svc").await.unwrap(),
            Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string())
        );
        let (gen_before, _) = storage_provider
            .get_latest_config_generation("tcp_rollback_svc")
            .await
            .unwrap()
            .unwrap();

        // Re-deploy the same TCP service with a new policy P2, declaring the
        // interface name the registry is rigged to reject.
        let policy_2_filename = format!("test_tcp_rollback_p2_{}.json", std::process::id());
        fs::write(
            &policy_2_filename,
            r#"{"version": "fdae/v1", "strict": true, "definitions": {}}"#,
        )
        .unwrap();
        let second = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_2_filename.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest {
                endpoints: vec![NetworkEndpoint {
                    interface_name: "fails-to-register".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: 9001,
                }],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let result = service.deploy("tcp_rollback_svc".to_string(), second, &caller).await;
        let _ = fs::remove_file(&policy_2_filename);
        assert!(result.is_err(), "TCP endpoint registration must fail: {result:?}");
        assert!(result.unwrap_err().contains("Endpoint registration failed"));

        assert_eq!(
            storage_provider.load_fdae_policy("tcp_rollback_svc").await.unwrap(),
            Some(r#"{"version": "fdae/v1", "definitions": {}}"#.to_string()),
            "a failed TCP redeploy must restore the previous policy, not leave the new one (P2) \
             in force"
        );
        let (gen_after, _) = storage_provider
            .get_latest_config_generation("tcp_rollback_svc")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            gen_after, gen_before,
            "a failed TCP redeploy must roll back the config generation this attempt saved"
        );
    }

    #[tokio::test]
    async fn test_deploy_fdae_policy_schema_invalid_rejected_and_not_persisted() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let policy_filename = format!("test_fdae_bad_policy_{}.json", std::process::id());
        // Missing required "definitions" key -- fails JSON-Schema validation.
        fs::write(&policy_filename, r#"{"version": "fdae/v1"}"#).unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_filename.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy("fdae_bad_service".to_string(), manifest, &node_wide_caller("test-caller"))
            .await;

        let _ = fs::remove_file(&policy_filename);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("FDAE policy validation failed"));
        assert_eq!(
            storage_provider.load_fdae_policy("fdae_bad_service").await.unwrap(),
            None,
            "an invalid policy must never reach fdae_policies"
        );
    }

    /// A schema-invalid document that is itself sensitive-looking content
    /// (not a policy at all) must not have that content echoed back to the
    /// remote deploy caller. `jsonschema::ValidationError`'s `Display` embeds
    /// the offending JSON *instance* -- for a top-level type mismatch, that
    /// instance is the whole file -- so `PolicyError::Schema`'s `to_string()`
    /// must never be forwarded verbatim into the returned error.
    #[tokio::test]
    async fn test_deploy_fdae_policy_error_does_not_echo_file_contents() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let policy_filename = format!("test_fdae_secret_leak_{}.json", std::process::id());
        let secret = "SUPER_SECRET_API_KEY_abc123";
        fs::write(&policy_filename, format!("\"{secret}\"")).unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(policy_filename.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy("fdae_leak_service".to_string(), manifest, &node_wide_caller("test-caller"))
            .await;

        let _ = fs::remove_file(&policy_filename);

        let err = result.unwrap_err();
        assert!(err.contains("FDAE policy validation failed"), "{err}");
        assert!(!err.contains(secret), "policy file content leaked into the deploy error: {err}");
    }

    #[tokio::test]
    async fn test_deploy_fdae_policy_traversal_and_absolute_rejected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        for bad_path in ["../../../../etc/fdae-policy.json", "/etc/fdae-policy.json"] {
            let manifest = DeployManifest {
                config: ServiceConfig {
                    env: vec![],
                    args: vec![],
                    custom_config: None,
                    quota: None,
                    schema: None,
                    rotation_policy: None,
                    fdae_policy: Some(DocumentSource::Path(bad_path.to_string())),
                },
                service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
                registry_certificate: None,
                instance_certificate: None,
            };

            let result = service
                .deploy("fdae_traversal_service".to_string(), manifest, &node_wide_caller("t"))
                .await;
            assert!(result.is_err(), "{bad_path} should be rejected");
            assert!(
                result.unwrap_err().contains("Arbitrary file read prevented: Path traversal"),
                "{bad_path} should fail on the traversal guard"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_deploy_fdae_policy_symlink_escape_rejected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        // Same symlink-escape gap as the schema guard, on the
        // fdae_policy guard: no `..` component, not absolute, but the
        // symlink target lives outside the working directory.
        let outside_dir = tempfile::tempdir().unwrap();
        let outside_policy = outside_dir.path().join("fdae-policy.json");
        fs::write(&outside_policy, r#"{"version": "fdae/v1", "definitions": {}}"#).unwrap();

        let symlink_name = format!("test_fdae_policy_symlink_{}.json", std::process::id());
        std::os::unix::fs::symlink(&outside_policy, &symlink_name).unwrap();

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: Some(DocumentSource::Path(symlink_name.clone())),
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let result = service
            .deploy(
                "symlink_fdae_policy_service".to_string(),
                manifest,
                &node_wide_caller("test-caller"),
            )
            .await;

        let _ = fs::remove_file(&symlink_name);

        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("resolves outside the working directory via a symlink"),
            "{}",
            err_msg
        );
    }

    #[test]
    fn test_warn_on_policy_collection_mismatch_fires_in_both_directions() {
        use std::io;

        use tracing_subscriber::prelude::*;

        let logs = Arc::new(Mutex::new(Vec::new()));
        let logs_clone = logs.clone();

        struct MockWriter {
            logs: Arc<Mutex<Vec<u8>>>,
        }
        impl io::Write for MockWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.logs.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let make_writer = move || MockWriter { logs: logs_clone.clone() };
        let layer = tracing_subscriber::fmt::layer().with_writer(make_writer).with_ansi(false);
        let subscriber = tracing_subscriber::registry().with(layer);

        // "widget" -> "widgets" is present in `collections` (no warning
        // expected). "gizmo" -> "gizmos" is a `definitions:` entry whose
        // table doesn't exist yet (direction 2). "orphan_table" exists in
        // `collections` with no matching definition (direction 1).
        let policy = syneroym_fdae::parse_and_validate(
            r#"{
                "version": "fdae/v1",
                "definitions": {
                    "widget": { "table": "widgets" },
                    "gizmo": { "table": "gizmos" }
                }
            }"#,
        )
        .unwrap();

        tracing::subscriber::with_default(subscriber, || {
            warn_on_policy_collection_mismatch(
                "svc-a",
                &policy,
                &["widgets".to_string(), "orphan_table".to_string()],
            );
        });

        let output = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
        assert!(
            output.contains("orphan_table") && output.contains("has no FDAE definition"),
            "direction 1 (table with no definition) should warn: {output}"
        );
        assert!(
            output.contains("gizmos") && output.contains("no such collection exists"),
            "direction 2 (definition with no table) should warn: {output}"
        );
        assert!(
            !output.contains("collection=\"widgets\""),
            "a collection with a matching definition must not warn: {output}"
        );
    }

    #[test]
    fn test_warn_on_ambiguous_public_permission() {
        use std::io;

        use tracing_subscriber::prelude::*;

        struct MockWriter {
            logs: Arc<Mutex<Vec<u8>>>,
        }
        impl io::Write for MockWriter {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.logs.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let run = |policy_json: &str| -> String {
            let logs = Arc::new(Mutex::new(Vec::new()));
            let logs_clone = logs.clone();
            let make_writer = move || MockWriter { logs: logs_clone.clone() };
            let layer = tracing_subscriber::fmt::layer().with_writer(make_writer).with_ansi(false);
            let subscriber = tracing_subscriber::registry().with(layer);
            let policy = syneroym_fdae::parse_and_validate(policy_json).unwrap();
            tracing::subscriber::with_default(subscriber, || {
                warn_on_ambiguous_public_permission("svc-a", &policy);
            });
            String::from_utf8(logs.lock().unwrap().clone()).unwrap()
        };

        // "audit" is unconditionally public and shares `data-layer/read`
        // with the path-restricted "view", with no `includes` link between
        // them -- exactly the shape that silently widens "view" for any
        // caller holding a generic read capability.
        let ambiguous = run(r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]},
                            "audit": {"allows": ["data-layer/read"], "paths": []}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#);
        assert!(
            ambiguous.contains("public_permission=\"audit\"")
                && ambiguous.contains("restricted_permission=\"view\""),
            "an unlinked public/restricted pair sharing an ability should warn: {ambiguous}"
        );

        // Same shape, but "audit" declares `includes: ["view"]` -- the
        // author made the relationship explicit, so no warning.
        let linked = run(r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["data-layer/read"], "paths": [["creator", "caller"]]},
                            "audit": {"allows": ["data-layer/read"], "paths": [], "includes": ["view"]}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#);
        assert!(linked.is_empty(), "an explicit `includes` link must not warn: {linked}");

        // "audit" and "view" don't share a covering ability at all (write
        // vs. read, and neither entails the other) -- no warning.
        let disjoint = run(r#"{
                "version": "fdae/v1",
                "definitions": {
                    "document": {
                        "table": "documents",
                        "relations": {"creator": {"target": "user", "join_column": "creator_uuid"}},
                        "permissions": {
                            "view": {"allows": ["rpc/move"], "paths": [["creator", "caller"]]},
                            "audit": {"allows": ["data-layer/read"], "paths": []}
                        }
                    },
                    "user": {"table": "users", "principal_column": "did"}
                }
            }"#);
        assert!(disjoint.is_empty(), "disjoint abilities must not warn: {disjoint}");
    }

    /// M3B Slice 7: `deploy()` parses `http_routes` out of `custom_config`
    /// and populates the shared `HttpRouteRegistry` (the same `Arc` handed
    /// to `RouteHandlerInner` in production); `undeploy()` clears it. A TCP
    /// manifest is enough -- `http_routes` parsing/storage is independent
    /// of `service_type`.
    #[tokio::test]
    async fn test_http_routes_populated_on_deploy_and_cleared_on_undeploy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            native_dispatch,
            http_routes.clone(),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let service_id = "http-routes-svc".to_string();
        let custom_config = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/orders/{id}", "target": "data-layer",
                 "operation": "get", "collection": "orders"},
                {"method": "POST", "path": "/orders", "target": "data-layer",
                 "operation": "put", "collection": "orders"},
            ]
        })
        .to_string();
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: Some(custom_config),
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let caller = node_wide_caller("test-caller");
        service.deploy(service_id.clone(), manifest, &caller).await.unwrap();

        let routes = http_routes.get(&service_id).expect("http_routes populated on deploy");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].collection.as_deref(), Some("orders"));
        drop(routes);

        service.undeploy(service_id.clone(), &caller).await.unwrap();
        assert!(
            http_routes.get(&service_id).is_none(),
            "http_routes entry must be removed on undeploy"
        );
    }

    /// M3B Slice 7: a service deployed with no `http_routes` key gets no
    /// entry in the shared registry at all (not an empty-`Vec` entry) --
    /// keeps the registry from growing with a no-op entry per ordinary
    /// deployed service.
    #[tokio::test]
    async fn test_no_http_routes_entry_when_custom_config_has_none() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            native_dispatch,
            http_routes.clone(),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let service_id = "no-http-routes-svc".to_string();
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service
            .deploy(service_id.clone(), manifest, &node_wide_caller("test-caller"))
            .await
            .unwrap();

        assert!(http_routes.get(&service_id).is_none());
    }

    fn owner_test_manifest() -> DeployManifest {
        DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest {
                endpoints: vec![NetworkEndpoint {
                    interface_name: "default".to_string(),
                    host: "127.0.0.1".to_string(),
                    port: 9100,
                }],
            }),
            registry_certificate: None,
            instance_certificate: None,
        }
    }

    /// M04A Slice B7a (§2.3, F11): `deploy` records `caller.caller_did` as
    /// the owner -- the same DID `build_caller` resolves to the
    /// `DelegationCertificate`'s `master_did`, never the ephemeral
    /// `temporary_did`. `crates/router/src/route_handler/io.rs`'s
    /// `build_caller_uses_master_did_not_temporary_did_as_caller_did`
    /// (added on post-commit review -- every other `build_caller` test
    /// constructed `master_did == temporary_did`, so none could actually
    /// distinguish the two) proves that resolution; this test covers what
    /// `ControlPlaneService` does with whatever `caller_did` it is handed.
    #[tokio::test]
    async fn deploy_records_owner_as_caller_did() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider =
            Arc::new(SqliteStorageProvider::new(temp_dir.path(), false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry.clone(),
            temp_dir.path().to_path_buf(),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            native_dispatch,
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap();

        let caller = node_wide_caller("did:key:zOwnerDid");
        let service_id = "owner-attribution-svc".to_string();
        service.deploy(service_id.clone(), owner_test_manifest(), &caller).await.unwrap();

        assert_eq!(registry.owner_of(&service_id), Some(caller.caller_did.clone()));

        service.undeploy(service_id.clone(), &caller).await.unwrap();
        assert_eq!(registry.owner_of(&service_id), None);
    }

    /// Like `node_wide_caller`, plus `orchestrator/status` -- needed by the
    /// `instance_identity` tests below, which `deploy`/`undeploy` never gate
    /// on.
    fn status_capable_caller(caller_did: &str) -> CallerContext {
        use syneroym_rpc::{AuthLevel, Capability, SessionContext};

        let resource = ResourceUri::substrate("did:key:zTestNode");
        CallerContext {
            caller_did: caller_did.to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: caller_did.to_string(),
                capabilities: vec![
                    Capability {
                        with: resource.clone(),
                        can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
                        caveats: None,
                    },
                    Capability {
                        with: resource.clone(),
                        can: Ability(Ability::ORCHESTRATOR_UNDEPLOY.to_string()),
                        caveats: None,
                    },
                    Capability {
                        with: resource,
                        can: Ability(Ability::ORCHESTRATOR_STATUS.to_string()),
                        caveats: None,
                    },
                ],
                ..Default::default()
            },
            auth: AuthLevel::Delegated,
            proof: None,
        }
    }

    /// Builds a `ControlPlaneService` rooted at `temp_dir` with a caller-
    /// supplied node identity (so tests can compute the exact instance key
    /// the substrate will derive), returning the registry alongside it so
    /// tests can inspect what got stored.
    async fn service_with_node_identity(
        temp_dir: &std::path::Path,
        node_identity: Arc<syneroym_identity::Identity>,
    ) -> (ControlPlaneService, EndpointRegistry) {
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir, false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                registry.clone(),
            )
            .await
            .unwrap(),
        );
        let container_engine = Arc::new(ContainerEngine::new("podman".to_string(), temp_dir, None));

        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry.clone(),
            temp_dir.to_path_buf(),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            NativeDispatchRegistry::default(),
            Arc::new(DashMap::new()),
            node_identity,
        )
        .await
        .unwrap();

        (service, registry)
    }

    #[tokio::test]
    async fn the_derived_instance_identity_is_stable_across_calls() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, _registry) = service_with_node_identity(temp_dir.path(), node_identity).await;
        let caller = status_capable_caller("did:key:zOwner");

        let first = service.instance_identity("svc-a".to_string(), &caller).await.unwrap();
        let second = service.instance_identity("svc-a".to_string(), &caller).await.unwrap();

        assert_eq!(first.instance_did, second.instance_did);
        assert_eq!(first.pubkey_hex, second.pubkey_hex);
    }

    #[tokio::test]
    async fn two_owners_get_different_instance_identities_for_the_same_service_id() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, _registry) = service_with_node_identity(temp_dir.path(), node_identity).await;

        let alice = status_capable_caller("did:key:zAlice");
        let bob = status_capable_caller("did:key:zBob");

        let for_alice = service.instance_identity("shared-svc".to_string(), &alice).await.unwrap();
        let for_bob = service.instance_identity("shared-svc".to_string(), &bob).await.unwrap();

        assert_ne!(for_alice.instance_did, for_bob.instance_did);
    }

    #[tokio::test]
    async fn a_deploy_without_a_certificate_still_succeeds_and_stores_none() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry) = service_with_node_identity(temp_dir.path(), node_identity).await;
        let caller = node_wide_caller("did:key:zOwner");
        let service_id = "no-cert-svc".to_string();

        service.deploy(service_id.clone(), owner_test_manifest(), &caller).await.unwrap();

        assert_eq!(registry.instance_cert(&service_id), None);
    }

    #[tokio::test]
    async fn a_deploy_is_rejected_when_the_certificates_master_is_not_the_service_id() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry) =
            service_with_node_identity(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");
        let service_id = "member-master-svc".to_string();

        let wrong_master = syneroym_identity::Identity::generate().unwrap();
        let derived = node_identity.derive_service_identity(&caller.caller_did, &service_id);
        let cert = DelegationCertificate::issue(
            &wrong_master,
            derived.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let mut manifest = owner_test_manifest();
        manifest.instance_certificate = Some(cert.to_json().unwrap());

        let err = service.deploy(service_id.clone(), manifest, &caller).await.unwrap_err();
        assert!(err.contains("does not name this deploy's service_id"), "unexpected error: {err}");
        assert_eq!(registry.instance_cert(&service_id), None);
    }

    #[tokio::test]
    async fn a_deploy_is_rejected_when_the_certificate_certifies_a_different_key() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry) = service_with_node_identity(temp_dir.path(), node_identity).await;
        let caller = node_wide_caller("did:key:zOwner");

        let member_master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&member_master.public_key());

        // Certifies some *other* key, not the one this substrate would derive
        // for (caller, service_id).
        let wrong_instance = syneroym_identity::Identity::generate().unwrap();
        let cert = DelegationCertificate::issue(
            &member_master,
            wrong_instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let mut manifest = owner_test_manifest();
        manifest.instance_certificate = Some(cert.to_json().unwrap());

        let err = service.deploy(service_id.clone(), manifest, &caller).await.unwrap_err();
        assert!(err.contains("not the key this substrate would derive"), "unexpected error: {err}");
        assert_eq!(registry.instance_cert(&service_id), None);
    }

    #[tokio::test]
    async fn a_deploy_is_rejected_when_the_certificate_carries_the_routing_scope() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry) =
            service_with_node_identity(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let member_master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&member_master.public_key());
        let derived = node_identity.derive_service_identity(&caller.caller_did, &service_id);
        let cert = DelegationCertificate::issue(
            &member_master,
            derived.public_key(),
            3600,
            "routing".to_string(),
        )
        .unwrap();
        let mut manifest = owner_test_manifest();
        manifest.instance_certificate = Some(cert.to_json().unwrap());

        let err = service.deploy(service_id.clone(), manifest, &caller).await.unwrap_err();
        assert!(err.contains("scope"), "unexpected error: {err}");
        assert_eq!(registry.instance_cert(&service_id), None);
    }

    /// A0-02: `deploy-manifest.instance-certificate`'s WIT doc says "absent
    /// leaves the service its own master" -- that must hold on a redeploy
    /// that drops `--master`, not only on the first deploy of a service_id,
    /// or the stale certificate keeps being presented on outbound guest
    /// calls under a `temporary_did` the redeploy's new owner no longer
    /// derives to.
    #[tokio::test]
    async fn a_redeploy_without_a_certificate_clears_a_previously_installed_one() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry) =
            service_with_node_identity(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let member_master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&member_master.public_key());
        let derived = node_identity.derive_service_identity(&caller.caller_did, &service_id);
        let cert = DelegationCertificate::issue(
            &member_master,
            derived.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let mut manifest = owner_test_manifest();
        manifest.instance_certificate = Some(cert.to_json().unwrap());
        service.deploy(service_id.clone(), manifest, &caller).await.unwrap();
        assert!(registry.instance_cert(&service_id).is_some());

        service.deploy(service_id.clone(), owner_test_manifest(), &caller).await.unwrap();
        assert_eq!(
            registry.instance_cert(&service_id),
            None,
            "a redeploy without --master must clear the previously installed certificate"
        );
    }

    #[tokio::test]
    async fn undeploy_removes_the_instance_certificate_with_the_owner_row() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry) =
            service_with_node_identity(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let member_master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&member_master.public_key());
        let derived = node_identity.derive_service_identity(&caller.caller_did, &service_id);
        let cert = DelegationCertificate::issue(
            &member_master,
            derived.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let mut manifest = owner_test_manifest();
        manifest.instance_certificate = Some(cert.to_json().unwrap());

        service.deploy(service_id.clone(), manifest, &caller).await.unwrap();
        assert!(registry.instance_cert(&service_id).is_some());

        service.undeploy(service_id.clone(), &caller).await.unwrap();
        assert_eq!(registry.instance_cert(&service_id), None);
    }

    /// Builds a service rooted at `temp_dir`, for the inline-document tests
    /// below. They care about nothing in the wiring except that the working
    /// directory holds no schema or policy file.
    async fn service_for_inline_tests(temp_dir: &std::path::Path) -> ControlPlaneService {
        let config = SubstrateConfig::default();
        let key_store = Arc::new(KeyStore::new());
        let storage_provider = Arc::new(SqliteStorageProvider::new(temp_dir, false).unwrap());
        let blob_provider: Arc<dyn BlobProvider> =
            Arc::new(ObjectStoreBlobProvider::in_memory(u64::MAX, None));
        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig::default()).unwrap());
        let app_sandbox = Arc::new(
            AppSandboxEngine::init(
                &config,
                vec![],
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
                EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            )
            .await
            .unwrap(),
        );

        ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir, None)),
            EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            temp_dir.to_path_buf(),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            NativeDispatchRegistry::default(),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
        )
        .await
        .unwrap()
    }

    fn inline_manifest(
        custom_config: Option<&str>,
        schema: Option<DocumentSource>,
        fdae_policy: Option<DocumentSource>,
    ) -> DeployManifest {
        DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: custom_config.map(str::to_string),
                quota: None,
                schema,
                rotation_policy: None,
                fdae_policy,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        }
    }

    /// The whole point: nothing is staged on the substrate's filesystem, and
    /// the deploy still validates against a schema that arrived in the call.
    #[tokio::test]
    async fn test_deploy_inline_schema_validates_without_a_staged_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let schema = DocumentSource::Inline(
            r#"{"type":"object","properties":{"port":{"type":"integer"}}}"#.to_string(),
        );
        let result = service
            .deploy(
                "inline_schema_ok".to_string(),
                inline_manifest(Some(r#"{"port": 8080}"#), Some(schema), None),
                &node_wide_caller("test-caller"),
            )
            .await;

        assert!(result.is_ok(), "{:?}", result.unwrap_err());
    }

    #[tokio::test]
    async fn test_deploy_inline_schema_rejects_violating_config() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let schema = DocumentSource::Inline(
            r#"{"type":"object","properties":{"port":{"type":"integer"}}}"#.to_string(),
        );
        let err = service
            .deploy(
                "inline_schema_bad".to_string(),
                // A string where the schema demands an integer.
                inline_manifest(Some(r#"{"port": "8080"}"#), Some(schema), None),
                &node_wide_caller("test-caller"),
            )
            .await
            .unwrap_err();

        assert!(err.contains("Configuration validation failed"), "{err}");
    }

    #[tokio::test]
    async fn test_deploy_inline_fdae_policy_without_a_staged_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let policy =
            DocumentSource::Inline(r#"{"version":"fdae/v1","definitions":{}}"#.to_string());
        let result = service
            .deploy(
                "inline_policy_ok".to_string(),
                inline_manifest(None, None, Some(policy)),
                &node_wide_caller("test-caller"),
            )
            .await;

        assert!(result.is_ok(), "{:?}", result.unwrap_err());
    }

    /// An inline policy is caller-supplied, so the rule that a policy
    /// validation error never echoes the offending document back matters more
    /// here than it did for a host-side file.
    #[tokio::test]
    async fn test_deploy_inline_fdae_policy_error_does_not_echo_the_document() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let secret = "s3cret-marker-in-policy";
        let policy = DocumentSource::Inline(format!(
            r#"{{"version":"fdae/v1","definitions":{{"{secret}":"not-an-object"}}}}"#
        ));
        let err = service
            .deploy(
                "inline_policy_bad".to_string(),
                inline_manifest(None, None, Some(policy)),
                &node_wide_caller("test-caller"),
            )
            .await
            .unwrap_err();

        assert!(err.contains("FDAE policy validation failed"), "{err}");
        assert!(!err.contains(secret), "policy content leaked to the caller: {err}");
    }

    #[tokio::test]
    async fn test_deploy_rejects_oversize_inline_document() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let oversize = DocumentSource::Inline(
            "x".repeat(syneroym_core::deploy_docs::MAX_DEPLOY_DOCUMENT_BYTES as usize + 1),
        );
        let err = service
            .deploy(
                "oversize_schema".to_string(),
                inline_manifest(Some(r#"{"port": 8080}"#), Some(oversize), None),
                &node_wide_caller("test-caller"),
            )
            .await
            .unwrap_err();

        assert!(err.contains("exceeding the"), "{err}");
    }
}

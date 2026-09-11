//! Deploy/undeploy/list lifecycle for the orchestrator.
//!
//! Handles validating and applying a `DeployManifest` (wasm/container/tcp),
//! wiring up the native-capability endpoints and dispatch registration every
//! deployed service gets, and tearing all of that back down on undeploy.
//! Distinct from `service`'s own concern (`NativeService::dispatch`'s JSON-RPC
//! routing table and the KEK/secret management calls it handles directly).

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Component, PathBuf},
    result,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use serde_json::Value;
use syneroym_app_orchestration::{
    AppInstanceId, BindingWriteOutcome, HealthCheck, HttpProbe, InterfaceName, LogicalServiceName,
    RpcProbe, ServiceId as AppServiceId, ServiceType as AppServiceType, TcpProbe, TopologyEntry,
    TopologyEpoch, TopologyKey, TopologyMode as AppTopologyMode, Visibility as AppVisibility,
    classify_binding_write, compensated_operation,
};
use syneroym_core::{
    asset_manifest::ServiceAssets,
    deploy_docs,
    dht_registry::SignedEndpointInfo,
    http_routes::HttpRoute,
    local_registry::{NATIVE_CAPABILITY_INTERFACES, SubstrateEndpoint},
    storage::AppInstanceManagement,
    util,
};
use syneroym_fdae::Policy;
use syneroym_identity::{
    DelegationCertificate, delegation::SCOPE_SERVICE_INSTANCE, substrate::derive_did_key,
};
use syneroym_rpc::{
    Ability, CallOrigin, CallerContext, DeadLetterInfo, JsonRpcRequest, NativeService,
    ProxyProtocol, ProxyQueueInspector, ProxyRequest, QueuedCallInfo, ResourceUri, SagaInfo,
};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    AppContext, AppInstanceManagement as AppInstanceManagementWire, ArtifactSource, BindingWrite,
    BindingWriteOutcome as BindingWriteOutcomeWire, ContainerManifest, DependencyBinding,
    DeployManifest, DeployedService, DeploymentPlan, DocumentSource, HealthCheck as WitHealthCheck,
    InstanceIdentity, InstancePhase, NodeFacts, ProbeStatus, ServiceStatus,
    ServiceType as WitServiceType, SubstrateStatus, TcpManifest, TopologyMode as WitTopologyMode,
    Visibility as WitVisibility, WasmManifest,
};
use tokio::task;
use tracing::info;

use super::{AUTH_RESERVED_SERVICE_ID, ControlPlaneService, SUPERVISOR_RESERVED_SERVICE_ID};
use crate::{assets, config_utils, http_routes, synsvc_native::SynSvcNativeService};

pub(super) mod app_instance;
pub(super) mod backends;
pub(super) mod cert;
pub(super) mod deploy;
pub(super) mod lifecycle;
pub(super) mod proxy_queue;
pub(super) mod status;
pub(super) mod types;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use cert::*;
#[cfg(test)]
pub(crate) use status::*;
pub(crate) use types::*;

#[async_trait::async_trait]
pub trait OrchestratorInterface {
    async fn readyz(&self, service_id: String, caller: &CallerContext) -> Result<(), String>;
    /// Every call waiting in `service_id`'s durable proxy outbox.
    async fn proxy_outbox(
        &self,
        service_id: String,
        caller: &CallerContext,
    ) -> Result<Vec<QueuedCallInfo>, String>;
    /// Every dead letter `service_id`'s proxy outbox holds.
    async fn proxy_dead_letters(
        &self,
        service_id: String,
        caller: &CallerContext,
    ) -> Result<Vec<DeadLetterInfo>, String>;
    /// Re-enqueues one dead letter; it never executes inline.
    async fn proxy_replay(
        &self,
        service_id: String,
        dead_letter_id: u64,
        caller: &CallerContext,
    ) -> Result<(), String>;
    /// Every saga `service_id`'s own log holds, oldest first.
    async fn sagas(
        &self,
        service_id: String,
        caller: &CallerContext,
    ) -> Result<Vec<SagaInfo>, String>;
    /// Re-arms a `failed` saga back to `compensating`; it never walks
    /// inline.
    async fn saga_compensate(
        &self,
        service_id: String,
        saga_id: String,
        caller: &CallerContext,
    ) -> Result<(), String>;
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
    /// Epoch-guarded binding write (ADR-0021 §3). The only path
    /// that changes a dependent's resolution without redeploying it.
    async fn write_bindings(
        &self,
        write: BindingWrite,
        caller: &CallerContext,
    ) -> Result<Vec<BindingWriteOutcomeWire>, String>;
    /// `generation` is checked against the app instance's recorded
    /// management stamp when the service has one (ADR-0021 §4's
    /// "lifecycle actions"); a standalone service with no app context is
    /// ungated.
    async fn undeploy(
        &self,
        service_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String>;
    /// Restart a deployed service in place, without reinstalling it, as
    /// bounded remediation. `generation` follows `undeploy`'s rule.
    async fn restart(
        &self,
        service_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String>;
    /// Run one scheduled tick against a deployed service: dispatch `method`
    /// on `interface` locally, as the service itself (ADR-0023 §3/§6).
    /// Never queued -- a tick whose window has passed is
    /// not worth delivering late; the caller's next tick is the retry.
    /// `generation` follows `restart`'s rule.
    async fn run_scheduled(
        &self,
        service_id: String,
        generation: u64,
        interface: String,
        method: String,
        params_json: Option<String>,
        caller: &CallerContext,
    ) -> Result<(), String>;
    /// Install a freshly-issued instance certificate on an already-deployed
    /// service, without reinstalling it -- the certificate-only counterpart
    /// to `restart`, and the path an unattended renewal takes. `generation`
    /// follows `restart`'s rule.
    async fn renew_cert(
        &self,
        service_id: String,
        generation: u64,
        instance_certificate: String,
        caller: &CallerContext,
    ) -> Result<(), String>;
    /// `adopt`'s read half: the management stamp an app
    /// instance carries, or `None` if no deploy has ever named it here.
    /// `Ok(None)` (not an error) for a caller with no visibility into the
    /// instance, so a caller with no grant cannot use this to probe for its
    /// existence.
    async fn app_instance_management_of(
        &self,
        app_instance_id: String,
        caller: &CallerContext,
    ) -> Result<Option<AppInstanceManagementWire>, String>;
    /// Claim management of an app instance at `generation` -- the
    /// operator-minted adopt of ADR-0021 §4, made durable at the moment of
    /// the claim. Subject to the same four-case rule as every other write.
    async fn claim_app_instance(
        &self,
        app_instance_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String>;
    /// Clear an app instance's management stamp:
    /// `supervisor_did` back to `None` and `generation` back to 0, keeping
    /// `owner_did`. Without this, an adopted instance can never be
    /// hand-deployed again.
    async fn release_app_instance(
        &self,
        app_instance_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String>;
    async fn list(&self, caller: &CallerContext) -> Result<Vec<DeployedService>, String>;
    async fn deploy_plan(&self, plan: DeploymentPlan, caller: &CallerContext)
    -> Result<(), String>;
    /// Per-instance status for a supervisor's poll loop.
    async fn status(
        &self,
        service_ids: Vec<String>,
        caller: &CallerContext,
    ) -> Result<SubstrateStatus, String>;
    /// Node facts only -- what `status`'s `node` field alone would
    /// answer, with none of `status`'s per-service work. `None` for a caller
    /// without node-wide `orchestrator/status`, the same as
    /// `status`'s own `node` field.
    async fn node_facts(&self, caller: &CallerContext) -> Option<NodeFacts>;
}

#[async_trait::async_trait]
impl OrchestratorInterface for ControlPlaneService {
    /// `readyz` has two forms, and only one is a
    /// status-check in the ownership sense. Empty `service_id` is a
    /// substrate-liveness ping -- `SyneroymClient::wait_for_ready` calls it
    /// pre-capability during `connect()`, so gating it would break connect
    /// for every ordinary client; it stays open, as a health probe
    /// (liveness is not an authorization surface). A
    /// non-empty `service_id` is a per-service readiness check
    /// and is gated on `orchestrator/status`, exactly
    /// like `deploy`/`undeploy` gate on their own abilities below --
    /// node-wide authority (the owner, via a verified `ControllerAgreement`)
    /// passes for free; otherwise the caller needs a grant covering this
    /// app. An unowned substrate holds no node-wide authority, so this
    /// always falls through to the per-app grant check there.
    async fn proxy_outbox(
        &self,
        service_id: String,
        caller: &CallerContext,
    ) -> Result<Vec<QueuedCallInfo>, String> {
        self.authorize_proxy_queue_access(&service_id, caller)?;
        self.proxy_queue_inspector()?.queued_calls(&service_id).await
    }

    async fn proxy_dead_letters(
        &self,
        service_id: String,
        caller: &CallerContext,
    ) -> Result<Vec<DeadLetterInfo>, String> {
        self.authorize_proxy_queue_access(&service_id, caller)?;
        self.proxy_queue_inspector()?.dead_letters(&service_id).await
    }

    async fn proxy_replay(
        &self,
        service_id: String,
        dead_letter_id: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.authorize_proxy_queue_write(&service_id, caller)?;
        self.proxy_queue_inspector()?.replay_dead_letter(&service_id, dead_letter_id).await
    }

    async fn sagas(
        &self,
        service_id: String,
        caller: &CallerContext,
    ) -> Result<Vec<SagaInfo>, String> {
        self.authorize_proxy_queue_access(&service_id, caller)?;
        self.proxy_queue_inspector()?.sagas(&service_id).await
    }

    async fn saga_compensate(
        &self,
        service_id: String,
        saga_id: String,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.authorize_proxy_queue_write(&service_id, caller)?;
        self.proxy_queue_inspector()?.rearm_saga(&service_id, &saga_id).await
    }

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

            // This once used "any `TcpHostPort` endpoint means container",
            // which fires against real TCP services too (both register the
            // same endpoint variant) and reports the resulting failure as
            // unreadiness. Reads the recorded service type instead -- a
            // service with no recorded facts (deployed by an older binary)
            // is no longer podman-inspected, matching `status`'s `unknown`,
            // so the two surfaces cannot disagree.
            if let Some((t, ..)) = self.registry.deploy_facts(&service_id)
                && parse_service_type(&t) == Some(AppServiceType::Container)
            {
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
        // `instance_did` above is what *this caller* would
        // derive, prospective by design (the doc comment on the WIT
        // record explains why that must not change). `revoke-instance`
        // needs the DID actually in use, which is only the same thing
        // when the installed certificate happened to be minted for this
        // caller -- so it is reported separately, read straight from the
        // registry rather than derived.
        let installed_temporary_did =
            self.registry.instance_cert(&service_id).map(|c| c.temporary_did);
        Ok(InstanceIdentity {
            instance_did: derive_did_key(&instance.public_key()),
            pubkey_hex: hex::encode(instance.public_key().to_bytes()),
            installed_temporary_did,
        })
    }

    async fn deploy(
        &self,
        service_id: String,
        manifest: DeployManifest,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // A standalone deploy carries no app context, so it
        // resolves no declared dependency name -- it can still be called,
        // and can still call out by DID.
        self.deploy_with_context(service_id, manifest, None, caller).await
    }

    async fn write_bindings(
        &self,
        write: BindingWrite,
        caller: &CallerContext,
    ) -> Result<Vec<BindingWriteOutcomeWire>, String> {
        self.write_bindings_impl(write, caller).await
    }

    async fn undeploy(
        &self,
        service_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.undeploy_impl(service_id, generation, caller).await
    }

    async fn restart(
        &self,
        service_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.restart_impl(service_id, generation, caller).await
    }

    async fn run_scheduled(
        &self,
        service_id: String,
        generation: u64,
        interface: String,
        method: String,
        params_json: Option<String>,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.run_scheduled_impl(service_id, generation, interface, method, params_json, caller)
            .await
    }

    async fn renew_cert(
        &self,
        service_id: String,
        generation: u64,
        instance_certificate: String,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.renew_cert_impl(service_id, generation, instance_certificate, caller).await
    }

    async fn app_instance_management_of(
        &self,
        app_instance_id: String,
        caller: &CallerContext,
    ) -> Result<Option<AppInstanceManagementWire>, String> {
        self.app_instance_management_of_impl(app_instance_id, caller).await
    }

    async fn claim_app_instance(
        &self,
        app_instance_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.claim_app_instance_impl(app_instance_id, generation, caller).await
    }

    async fn release_app_instance(
        &self,
        app_instance_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.release_app_instance_impl(app_instance_id, generation, caller).await
    }

    async fn list(&self, caller: &CallerContext) -> Result<Vec<DeployedService>, String> {
        self.list_impl(caller).await
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
                                 are not allowed in deploy-plan: {path:?}"
                            ));
                        }

                        let bytes = util::read_local_artifact(&path)
                            .map_err(|e| format!("Failed to read WASM file at {path:?}: {e}"))?;
                        wasm_manifest.source = ArtifactSource::Binary(bytes);
                    }
                }
                WitServiceType::Tcp(_) | WitServiceType::Container(_) => {
                    // TCP and Container don't read host files directly in
                    // deploy_plan logic for sources
                }
            }

            self.deploy_with_context(service_id, deploy_manifest, service.app_context, caller)
                .await?;
        }

        Ok(())
    }

    async fn status(
        &self,
        service_ids: Vec<String>,
        caller: &CallerContext,
    ) -> Result<SubstrateStatus, String> {
        self.status_impl(service_ids, caller).await
    }

    async fn node_facts(&self, caller: &CallerContext) -> Option<NodeFacts> {
        self.node_facts_for(caller)
    }
}

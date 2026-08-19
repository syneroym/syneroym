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

use super::{ControlPlaneService, SUPERVISOR_RESERVED_SERVICE_ID};
use crate::{assets, config_utils, http_routes, synsvc_native::SynSvcNativeService};

/// The ceiling `verify_installed_instance_cert` enforces on an installed
/// instance certificate's lifetime (30 days). A backstop against an
/// unbounded mint, not a forcing function: ADR-0020 §3's attended posture
/// issues deliberately long-lived certificates on an operator's own
/// cadence, and a ceiling tuned for automated renewal would refuse them.
/// Same reasoning, and the same order of magnitude, as
/// `EndpointInfo.not_after`'s own generous bound.
const MAX_INSTANCE_CERT_LIFETIME_SECS: u64 = 30 * 24 * 3600;

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
    /// Epoch-guarded binding write (M05A A5, ADR-0021 §3). The only path
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
    /// Restart a deployed service in place, without reinstalling it (M05A
    /// A5's bounded remediation). `generation` follows `undeploy`'s rule.
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
    /// `adopt`'s read half (M05A A5a, §0.26): the management stamp an app
    /// instance carries, or `None` if no deploy has ever named it here.
    /// `Ok(None)` (not an error) for a caller with no visibility into the
    /// instance, so a caller with no grant cannot use this to probe for its
    /// existence (A4-10's rule, applied here too).
    async fn app_instance_management_of(
        &self,
        app_instance_id: String,
        caller: &CallerContext,
    ) -> Result<Option<AppInstanceManagementWire>, String>;
    /// Claim management of an app instance at `generation` (M05A A5a,
    /// §0.26) -- ADR-0021 §4's operator-minted adopt, made durable at the
    /// moment of the claim. Subject to the same four-case rule as every
    /// other write.
    async fn claim_app_instance(
        &self,
        app_instance_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String>;
    /// Clear an app instance's management stamp (M05A A5a, §0.24):
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
    /// Per-instance status for a supervisor's poll loop (M05A A4).
    async fn status(
        &self,
        service_ids: Vec<String>,
        caller: &CallerContext,
    ) -> Result<SubstrateStatus, String>;
    /// Node facts only (A4-06) -- what `status`'s `node` field alone would
    /// answer, with none of `status`'s per-service work. `None` for a caller
    /// without node-wide `orchestrator/status` (D-A4-18), the same as
    /// `status`'s own `node` field.
    async fn node_facts(&self, caller: &CallerContext) -> Option<NodeFacts>;
}

/// Maps the wire `topology-mode` variant to the app model's `TopologyMode`
/// (A2) -- the inverse of `syneroym_sdk::mapper`'s `map_mode`.
fn map_topology_mode(mode: WitTopologyMode) -> AppTopologyMode {
    match mode {
        WitTopologyMode::Singleton => AppTopologyMode::Singleton,
        WitTopologyMode::Redundant => AppTopologyMode::Redundant,
        WitTopologyMode::Sharded => AppTopologyMode::Sharded,
    }
}

/// Wire `service-type` variant -> the app model's `ServiceType` (M05A A4).
/// Only the discriminant matters here; the payload is what the deploy already
/// used. The wire variant has no `native-host` case -- only the three types a
/// deploy can actually produce reach here.
const fn app_service_type(t: &WitServiceType) -> AppServiceType {
    match t {
        WitServiceType::Wasm(_) => AppServiceType::Wasm,
        WitServiceType::Container(_) => AppServiceType::Container,
        WitServiceType::Tcp(_) => AppServiceType::Tcp,
    }
}

/// The content of a `registry_certificate` blob that actually describes the
/// deployed service, as distinct from the parts that change on every mint
/// regardless of whether anything else did (review finding E-1):
/// `SignedEndpointInfo.info.not_after` (`SystemTime::now()` plus a fixed
/// window) and `pkarr_packet_hex` (its own embedded signing timestamp) both
/// churn on every `certify_placed_members` call. Every other `EndpointInfo`
/// field belongs here, `generation` (ADR-0022 §2) included: every member
/// record today passes `0`, so its absence is latent, not yet a wrong
/// answer -- but a real generation change is a real content change, and
/// omitting it here would let this dedup treat two different generations
/// as identical the day one publisher's does. Falls back to hashing the raw
/// string on a parse failure -- that can only make the dedup *more*
/// conservative (a parse failure never equals anything, including itself
/// byte-for-byte reapplied), never less safe.
fn stable_registry_certificate_for_hash(json: &str) -> String {
    match serde_json::from_str::<SignedEndpointInfo>(json) {
        Ok(signed) => serde_json::to_string(&(
            &signed.info.service_id,
            &signed.info.substrate_id,
            &signed.info.endpoint_type,
            &signed.info.mechanisms,
            &signed.info.nickname,
            signed.info.is_private,
            &signed.info.ttl,
            signed.info.generation,
        ))
        .unwrap_or_else(|_| json.to_string()),
        Err(_) => json.to_string(),
    }
}

/// Whether `service_id` is safe to join, unescaped, into a filename under
/// `hosted_apps_dir` -- `deploy_with_context`'s certificate write/delete,
/// and `undeploy_impl`'s own delete of the same file. Real service ids are
/// DIDs, which use only `did:key:z...` characters -- none of the excluded
/// ones are ever legitimate here.
fn is_safe_service_id_for_path(service_id: &str) -> bool {
    !service_id.is_empty()
        && !service_id.contains('/')
        && !service_id.contains('\\')
        && !service_id.contains("..")
}

/// ADR-0018 §4: the substrate *validates* the declaration against the signed
/// artifact rather than deciding it -- `is_private` lives inside the
/// signature, so only the signer can set it. Returns the model `Visibility`
/// to record for this service.
fn validate_publication(
    service_id: &str,
    declared: Option<WitVisibility>,
    certificate: Option<&str>,
) -> Result<AppVisibility, String> {
    let v = match declared.unwrap_or(WitVisibility::Private) {
        WitVisibility::Public => AppVisibility::Public,
        WitVisibility::Internal => AppVisibility::Internal,
        WitVisibility::Private => AppVisibility::Private,
    };

    let v_str = v.as_str();

    match (v, certificate) {
        (AppVisibility::Private, None) => Ok(AppVisibility::Private),
        (AppVisibility::Private, Some(_)) => Err(format!(
            "service '{service_id}' declares visibility 'private' but a registry certificate was \
             supplied -- declare 'public' or 'internal', or deploy without the certificate"
        )),
        (AppVisibility::Public | AppVisibility::Internal, None) => Err(format!(
            "service '{service_id}' declares visibility '{v_str}' but no registry certificate was \
             supplied -- a record must be signed by the service's own key, which this substrate \
             does not hold"
        )),
        (AppVisibility::Public | AppVisibility::Internal, Some(json)) => {
            let signed = serde_json::from_str::<SignedEndpointInfo>(json).map_err(|e| {
                format!("registry certificate for '{service_id}' does not parse: {e}")
            })?;

            if signed.info.service_id != service_id {
                return Err(format!(
                    "registry certificate for '{service_id}' names service '{}' -- it would be \
                     rejected by the registry, which resolves the signing key from that field",
                    signed.info.service_id
                ));
            }

            let want_private = v == AppVisibility::Internal;
            if signed.info.is_private != want_private {
                return Err(format!(
                    "service '{service_id}' declares visibility '{v_str}', but its registry \
                     certificate carries is_private={}; the record is signed, so this can only be \
                     fixed by re-signing it",
                    signed.info.is_private
                ));
            }

            Ok(v)
        }
    }
}

/// `ServiceType` -> the string stored in `service_deploy_facts` and reported
/// on the wire. The inverse parse is `parse_service_type`, just below.
const fn service_type_str(t: AppServiceType) -> &'static str {
    match t {
        AppServiceType::Wasm => "wasm",
        AppServiceType::Container => "container",
        AppServiceType::Tcp => "tcp",
        AppServiceType::NativeHost => "nativehost",
    }
}

/// Validates one wire `dependency-binding` into `(LogicalServiceName,
/// TopologyEntry)`. Shared by the deploy path and `write_bindings` (M05A
/// A5a) so the two cannot validate differently -- every field is
/// caller-supplied (D-A2-15), and `LogicalServiceName::new` *panics* on an
/// empty name or one containing '/'.
fn prepare_binding(
    binding: &DependencyBinding,
    app_instance_id: &str,
) -> Result<(LogicalServiceName, TopologyEntry), String> {
    // D-A2-2 / ADR-0021 §2: A2 resolves intra-app dependencies only -- a
    // deploy (or a binding push) may bind dependencies for its own
    // declared app instance, never a different one. `DependencyBinding.
    // app_instance_id` is deliberately caller-supplied, ahead of the
    // cross-app `Bind` surface the WIT comment reserves it for ("equal to
    // the dependent's own app-instance-id today"); without this
    // comparison it goes unenforced, and one authorized writer could
    // silently overwrite the binding a *different* app instance's
    // services resolve.
    if binding.app_instance_id != app_instance_id {
        return Err(format!(
            "binding '{}' names app instance '{}', but this deploy's app context is '{}' -- a \
             deploy may only bind dependencies for its own app instance",
            binding.dependency_name, binding.app_instance_id, app_instance_id
        ));
    }
    let dependency_name = LogicalServiceName::try_new(&binding.dependency_name)
        .map_err(|e| format!("binding names an invalid dependency name: {e}"))?;
    let entry = TopologyEntry {
        mode: map_topology_mode(binding.mode),
        members: binding
            .members
            .iter()
            .map(AppServiceId::try_new)
            .collect::<result::Result<Vec<_>, _>>()
            .map_err(|e| {
                format!("binding '{}' names an invalid member DID: {e}", binding.dependency_name)
            })?,
        sharding_strategy: None, // D-A2-4
        epoch: TopologyEpoch(binding.epoch),
        cache_ttl: Duration::from_millis(binding.cache_ttl_ms),
        not_after: None,
    };
    Ok((dependency_name, entry))
}

/// `AppInstanceManagement` (the internal, storage-facing type) -> its wire
/// record. Kept as a free function rather than a `From` impl since the wire
/// type lives in a generated module neither type owns.
fn management_to_wire(m: &AppInstanceManagement) -> AppInstanceManagementWire {
    AppInstanceManagementWire {
        owner_did: m.owner_did.clone(),
        supervisor_did: m.supervisor_did.clone(),
        generation: m.generation,
    }
}

/// `BindingWriteOutcome` (the pure, `app_orchestration`-owned rule's
/// result) -> its wire variant. Same free-function shape as
/// `management_to_wire`, for the same reason.
const fn wire_binding_outcome(outcome: &BindingWriteOutcome) -> BindingWriteOutcomeWire {
    match outcome {
        BindingWriteOutcome::Applied => BindingWriteOutcomeWire::Applied,
        BindingWriteOutcome::NoOp => BindingWriteOutcomeWire::NoOp,
        BindingWriteOutcome::Stale(epoch) => BindingWriteOutcomeWire::Stale(epoch.0),
        BindingWriteOutcome::Conflict(epoch) => BindingWriteOutcomeWire::Conflict(epoch.0),
    }
}

fn parse_service_type(s: &str) -> Option<AppServiceType> {
    match s {
        "wasm" => Some(AppServiceType::Wasm),
        "container" => Some(AppServiceType::Container),
        "tcp" => Some(AppServiceType::Tcp),
        "nativehost" => Some(AppServiceType::NativeHost),
        _ => None,
    }
}

/// Wire `health-check` -> the app model's, so deploy-time validation can use
/// `HealthCheck::valid_for`/`kind_name` rather than restating the pairing
/// table on the wire type. The inverse of `syneroym_sdk::mapper`'s
/// `map_health_check`. Fallible, unlike that mapper direction: `interface`
/// is caller-supplied on this path (a wire deploy call, not a locally-parsed
/// manifest), so an empty name must be a deploy error, not a panic.
fn model_health_check(c: &WitHealthCheck) -> Result<HealthCheck, String> {
    let interface_name = |s: &str| {
        InterfaceName::try_new(s).map_err(|e| format!("invalid health check interface: {e}"))
    };
    Ok(match c {
        WitHealthCheck::TcpConnect(p) => HealthCheck::TcpConnect(TcpProbe {
            interface: interface_name(&p.interface_name)?,
            timeout_ms: p.timeout_ms,
        }),
        WitHealthCheck::HttpGet(p) => HealthCheck::HttpGet(HttpProbe {
            interface: interface_name(&p.interface_name)?,
            path: p.path.clone(),
            expect_status: p.expect_status,
            timeout_ms: p.timeout_ms,
        }),
        WitHealthCheck::Rpc(p) => HealthCheck::Rpc(RpcProbe {
            interface: interface_name(&p.interface_name)?,
            method: p.method.clone(),
            timeout_ms: p.timeout_ms,
        }),
    })
}

/// A deploy's `app_context`, validated but not yet written (A2, post-review
/// fix). Validation runs early -- a malformed or unauthorized binding is a
/// deploy failure, not a routing failure discovered later -- but the actual
/// registry/resolver write is deferred until every other fallible deploy
/// step has succeeded, so a deploy that goes on to fail never leaves a
/// binding installed. See `deploy_with_context` and `install_app_context`.
struct PreparedAppContext {
    instance_id: AppInstanceId,
    raw_instance_id: String,
    raw_service_name: String,
    /// (`dependency_name` as sent on the wire, the validated
    /// `LogicalServiceName`, the resolved `TopologyEntry`) per binding.
    bindings: Vec<(String, LogicalServiceName, TopologyEntry)>,
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

/// Resolves an `asset-bundle.archive` field (M06A A1) to raw bytes. `Binary`
/// is the only real case -- the SDK's mapper (`resolve_artifact_source`)
/// already turns a local path or an inlined hex artifact into `Binary` bytes
/// before the manifest ever reaches the wire. `Url` is a dead branch here,
/// exactly as it already is for the Wasm component's own `source` (nothing
/// fetches it); reviving it is out of A1's scope (deferred-backlog.md), so
/// it is rejected explicitly rather than silently accepted and ignored.
fn resolve_asset_archive(source: &ArtifactSource) -> Result<Vec<u8>, String> {
    match source {
        ArtifactSource::Binary(bytes) => Ok(bytes.clone()),
        ArtifactSource::Url(_) => Err("asset bundle archive via a URL artifact-source is not \
                                       supported; provide it as inline bytes"
            .to_string()),
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

    /// Writes `prepared`'s app-context and binding rows (A2, post-review
    /// fix). Called only once every earlier fallible step in `deploy_
    /// with_context` has already succeeded -- see the call site's own
    /// comment -- so a storage error here is the *only* way this can fail,
    /// never a validation problem (`prepared`'s fields already passed
    /// `try_new`). The app-instance management stamp is *not* written
    /// here (M05A A5a §0.27) -- `deploy_with_context` persists it right
    /// after `check_generation` succeeds, before this method ever runs,
    /// since it records who is writing rather than what was installed.
    async fn install_app_context(
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
            // Last-write-wins (D-A2-10). ADR-0021 §3's four-case epoch
            // guard -- lower rejects, equal+identical no-ops, equal+
            // different is a reported conflict, higher applies -- belongs
            // at exactly this call and is the supervisor slice's.
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
    /// what keeps A0-A4's operator-driven `app deploy` working unchanged
    /// after this lands, and what `release-app-instance` restores. The
    /// returned value is what the caller must persist immediately
    /// (`set_app_instance_management`), before anything else it does
    /// (M05A A5a §0.27) -- it records *who is writing*, not what was
    /// installed, so it is not behind A2's defer-until-everything-
    /// succeeds rule that governs bindings.
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
    fn check_generation(
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

    /// Rolls back an in-progress deploy's asset-bundle work (M06A D-A1-9,
    /// R3-B backward direction): deletes every blob this attempt itself
    /// wrote, keeping any hash the still-live previous generation (`old`)
    /// still references. A no-op when `written` is empty, so calling this
    /// unconditionally on every failure branch above the registry commit
    /// costs nothing for a deploy that declares no assets at all.
    /// Best-effort, same as `rollback_config_generation`.
    async fn rollback_asset_bundle(
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

    #[allow(clippy::too_many_arguments)]
    async fn deploy_wasm_service(
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

        // M06A D-A2-10b: a declared `guest` route whose compiled component
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

impl ControlPlaneService {
    /// The `proxy-*` verbs read and replay one service's queued work, so
    /// they are gated exactly as their per-service neighbours on this
    /// interface are -- `orchestrator/status`, node-wide or scoped to that
    /// one service. No new resource namespace: a second way to name the
    /// same authority is how an operator ends up holding a grant that does
    /// not mean what they think.
    fn authorize_proxy_queue_access(
        &self,
        service_id: &str,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.authorize_proxy_queue(service_id, caller, Ability::ORCHESTRATOR_STATUS)
    }

    /// `proxy-replay` re-enqueues a call the worker then *sends*, so it is
    /// a lifecycle write and takes the write gate -- the same one
    /// `restart` uses, for the same reason. Listing the queues is a read
    /// and keeps the read gate; a holder of read-only status must not be
    /// able to make a service emit calls.
    fn authorize_proxy_queue_write(
        &self,
        service_id: &str,
        caller: &CallerContext,
    ) -> Result<(), String> {
        self.authorize_proxy_queue(service_id, caller, Ability::ORCHESTRATOR_DEPLOY)
    }

    fn authorize_proxy_queue(
        &self,
        service_id: &str,
        caller: &CallerContext,
        ability: &'static str,
    ) -> Result<(), String> {
        if service_id.is_empty() {
            return Err("a service id is required".to_string());
        }
        if self.has_node_wide_ability(caller, ability) {
            return Ok(());
        }
        let resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if caller.has_capability(&resource, &Ability(ability.to_string())) {
            return Ok(());
        }
        Err(format!("caller {} holds no {ability} grant for '{service_id}'", caller.caller_did))
    }

    /// The router's queue view, once it has been wired in. Absent means
    /// this node has no durable proxy path at all (coordinator mode, or a
    /// test harness with no router), which is a different answer from "the
    /// queue is empty" and is reported as such.
    fn proxy_queue_inspector(&self) -> Result<Arc<dyn ProxyQueueInspector>, String> {
        self.proxy_queues
            .get()
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| "this node keeps no durable proxy queues".to_string())
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

            // D-A4-17: was "any `TcpHostPort` endpoint means container",
            // which fires against real TCP services too (both register the
            // same endpoint variant) and reports the resulting failure as
            // unreadiness. Reads the recorded service type instead -- a
            // service with no recorded facts (deployed by a pre-A4 binary)
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
        // A standalone deploy carries no app context (D-A2-2), so it
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

impl ControlPlaneService {
    /// The trait method's entire body (§3.1's `deploy` <->
    /// `deploy_with_context` split, D-A2-6): no app context, so no
    /// bindings, unchanged for every existing caller including the JSON-RPC
    /// `deploy` dispatch. `deploy_plan` calls this directly, passing
    /// `service.app_context`.
    async fn deploy_with_context(
        &self,
        service_id: String,
        manifest: DeployManifest,
        app_context: Option<AppContext>,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // `service_id` is joined verbatim into `hosted_apps_dir/<service_id>.json`
        // below (write, and now also delete on a private redeploy, D-B2-5) --
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
        if service_id == self.node_did || service_id == SUPERVISOR_RESERVED_SERVICE_ID {
            return Err(format!(
                "service_id '{service_id}' is reserved for this substrate's own dispatch and \
                 cannot be deployed to"
            ));
        }

        // M04A Slice B7a / F7: a service_id already owned by someone else may
        // not be re-deployed into. An unowned substrate holds no node-wide
        // orchestrator authority, so this always
        // enforces the takeover check there -- only an owned substrate's
        // owner can override it, and today's overwrite-on-redeploy behavior
        // is preserved exactly for that case. Checks ORCHESTRATOR_DEPLOY specifically
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

        // ADR-0021 §2 / D-A2-2: validated here, before the artifact work,
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
            // plain `String`s (D-A2-7), so this is the only place the shape
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

            // ADR-0021 §4's generation gate (M05A A5a §0.18): persisted
            // immediately, before binding validation or any artifact work,
            // so a manager is recorded even if a later step in this deploy
            // fails (§0.27) -- this write records *who is writing*, not
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

        // M05A A5a §4A / D-A5-18: deploy idempotency (failure-matrix row
        // 10), distinct from the epoch guard (dedups binding writes) and
        // the generation gate (picks between writers) -- ADR-0021 §3 says
        // explicitly that neither covers the other. Canonical hash over
        // (manifest, app_context-minus-generation); the generation is
        // excluded deliberately, since bumping it is a change of *writer*,
        // not a change to the deployed service, and hashing it would make
        // an `adopt` force a pointless reinstall of every service.
        let service_type = app_service_type(&manifest.service_type);
        // M05A A5a §0.23: every rollback below re-enters through
        // `self.undeploy`, now generation-gated -- send the same
        // generation this deploy itself presented, so a rollback of the
        // supervisor's own deploy is never rejected by its own gate.
        let generation = app_context.as_ref().map_or(0, |c| c.generation);
        // Review finding A-2: `epoch` is `generation`'s sibling, not its
        // opposite -- it too records who is writing (the supervisor
        // advances it before every apply, D-A5c-4), not what gets
        // installed. Hashing it raw meant every re-apply changed the
        // hash and forced a genuine reinstall of every dependent, which
        // is exactly the restart `write-bindings` exists to avoid. Each
        // binding is hashed minus its `epoch`, by the same reasoning
        // §4A already applies to `generation` above.
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
        // Review finding E-1 (A-2's fix was inert): `manifest.instance_
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
        // The owner check: row 10 is "a retry after a lost response" --
        // the *same* caller re-sending a request whose response never
        // arrived. A *different* caller presenting byte-identical content
        // is a takeover, not a retry, and `set_owner` below must still run
        // unconditionally for it (M04A B7a: "authorized or not") -- a
        // dedup that skipped straight to `Ok(())` here would silently
        // leave the service owned by whoever deployed it first.
        if self.registry.deploy_facts(&service_id).and_then(|(_, _, hash, _)| hash).as_deref()
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

        // D-A4-6: a probe kind that cannot address this service type is a
        // manifest error, checked before any engine work runs. Accepting it
        // would produce a permanently `failing` probe that is
        // indistinguishable, at the supervisor, from a real outage.
        // (`service_type` was already computed above, for the row-10 dedup
        // check.)
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

        // M06A A1: an asset bundle is only reachable through a `Wasm`
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
        // (`apps/roymctl/src/commands/svc.rs`) and this fact from
        // `status.md`: `Tcp`/`Container` services already run their own web
        // server outside the substrate, which is exactly what A1 exists to
        // stop being the only way to serve a web app -- they have no need
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

        // M06A A1: static asset bundle unpack, before the wasm/tcp/container
        // dispatch below so a bad archive fails deploy the same way a bad
        // FDAE policy does -- before anything guest-visible has started.
        //
        // `old_assets` is read now, before any mutation: it is the only
        // point that can see the still-live previous generation, which the
        // backward rollback below (any failure between here and the
        // registry commit further down) must keep, and which the forward
        // cleanup at the commit point must diff against (D-A1-9).
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
            // D-A1-1: a caller who forgets to declare `public` gets 404s
            // with no signal anywhere unless this is logged -- absence of
            // an explicit `visibility` defaults to `private` by
            // construction (the wire's `option<visibility>`), which is
            // deliberately silent at the *serving* layer (D-A1-8: a miss
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

        // M06A D-A2-7: a `public` guest route is reachable with no verified
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
                // their own rollback here too (D-A1-9, R3-B).
                self.rollback_asset_bundle(&service_id, &written_asset_hashes, old_assets.as_ref())
                    .await;
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
                    installed_instance_cert.clone(),
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
        match &new_assets {
            Some(sa) => {
                self.assets.insert(service_id.clone(), sa.clone());
            }
            None => {
                self.assets.remove(&service_id);
            }
        }
        // Forward cleanup (D-A1-9): remove whatever the *old* manifest held
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

        // M05A A4: what this deploy said the service is, and its declared
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

        Ok(())
    }

    /// Epoch-guarded binding write (M05A A5a, ADR-0021 §3): the only path
    /// that changes a dependent's resolution without redeploying it.
    /// Touches the binding tables and the resolver and nothing else -- no
    /// artifact work, no restart, no lifecycle hook.
    async fn write_bindings_impl(
        &self,
        write: BindingWrite,
        caller: &CallerContext,
    ) -> Result<Vec<BindingWriteOutcomeWire>, String> {
        // Same gate `deploy_with_context` applies, for the same reason: a
        // binding write changes what a service calls, which is a
        // deploy-class change to that service, not a read.
        let deploy_resource =
            ResourceUri(format!("substrate:{}/app/{}", self.node_did, write.service_id));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{}' on this substrate",
                caller.caller_did, write.service_id
            ));
        }

        // The service must be deployed here and its recorded app context
        // must match -- without this an authorized caller could write
        // bindings into an app instance its service does not belong to,
        // the same hole `deploy`'s `binding.app_instance_id != ctx.
        // app_instance_id` check closes at deploy time.
        match self.registry.app_context_of(&write.service_id) {
            None => {
                return Err(format!("'{}' has no app context on this substrate", write.service_id));
            }
            Some((instance, _)) if instance != write.app_instance_id => {
                return Err(format!(
                    "'{}' belongs to app instance '{instance}', not '{}'",
                    write.service_id, write.app_instance_id
                ));
            }
            Some(_) => {}
        }

        // The same app-instance-owner gate `deploy_with_context` applies
        // (orchestration.rs's deploy ownership check), for the same reason:
        // `write.service_id` genuinely belonging to `write.app_instance_id`
        // (just checked above) proves the write targets its own service's
        // app, not that the caller may manage that app instance as a
        // whole. Without this, an app-scoped `orchestrator/deploy` grant on
        // one service of an instance -- not its owner, not node-wide --
        // could push a binding change that, through the shared resolver
        // entry `write-bindings` writes into, affects every other service
        // of that instance too. `check_generation` below is not a
        // substitute: it is a tiebreaker among already-authorized writers,
        // not an authorization check, and an unmanaged instance's
        // generation-0 gate now correctly accepts any authorized writer --
        // "authorized" has to be decided here, same as `deploy`.
        if let Some(existing) =
            self.registry.app_instance_management_of(&write.app_instance_id).map(|m| m.owner_did)
            && existing != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "app instance '{}' is owned by {existing}; a binding write into it must come from \
                 its owner or a substrate owner",
                write.app_instance_id
            ));
        }

        // D-A5-23: persisted immediately, before any binding is examined
        // -- the same rule every other gate site follows (deploy, restart,
        // undeploy, claim). A mid-validation refusal below must not leave
        // the accepting generation unrecorded.
        let management = self.check_generation(&write.app_instance_id, caller, write.generation)?;
        self.registry
            .set_app_instance_management(write.app_instance_id.clone(), management)
            .await
            .map_err(|e| e.to_string())?;

        // Validate every binding before applying any of it: `prepare_binding`
        // and the `binding_of` existence check are both pure reads, so the
        // whole list can be checked up front. Without this, a refusal partway
        // through (a malformed member DID, an undeclared dependency) would
        // leave earlier bindings already applied with no way for the caller
        // to know which ones landed -- the WIT contract's "one outcome per
        // binding, in the order sent" reads as all-or-nothing.
        let mut prepared = Vec::with_capacity(write.bindings.len());
        for binding in &write.bindings {
            let (dependency_name, entry) = prepare_binding(binding, &write.app_instance_id)?;

            // Update-only: a push may not introduce a dependency the
            // guest never declared at deploy -- a new dependency changes
            // the guest's contract and needs a redeploy, not a push.
            let held_json = self
                .registry
                .binding_of(&write.service_id, &binding.dependency_name)
                .await
                .map_err(|e| e.to_string())?;
            let Some(held_json) = held_json else {
                return Err(format!(
                    "'{}' declares no dependency '{}'; a new dependency needs a redeploy, not a \
                     binding push",
                    write.service_id, binding.dependency_name
                ));
            };
            let held: TopologyEntry = serde_json::from_str(&held_json).map_err(|e| {
                format!(
                    "stored binding for '{}' dependency '{}' is corrupt: {e}",
                    write.service_id, binding.dependency_name
                )
            })?;

            let outcome = classify_binding_write(Some(&held), &entry);
            prepared.push((binding, dependency_name, entry, outcome));
        }

        let mut outcomes = Vec::with_capacity(prepared.len());
        let mut any_applied = false;
        for (binding, dependency_name, entry, outcome) in prepared {
            if outcome == BindingWriteOutcome::Applied {
                any_applied = true;
                let entry_json = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
                self.registry
                    .save_binding(
                        &write.service_id,
                        &write.app_instance_id,
                        &binding.dependency_name,
                        &entry_json,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                // `NoOp`/`Stale`/`Conflict` write nothing. `NoOp` in
                // particular must not re-register: re-registering evicts
                // the resolver cache for an unchanged entry, turning the
                // ordinary retry into cache churn on the hot path.
                //
                // `try_new`, not `new`: `write.app_instance_id` is
                // caller-supplied wire input. Not reachable today -- the
                // `app_context_of` equality check above already guarantees
                // it equals a stored id validated at deploy -- but that is
                // a non-local invariant to depend on for a panic, and
                // `prepare_binding`'s own doc calls out this exact hazard.
                let app_instance_id =
                    AppInstanceId::try_new(&write.app_instance_id).map_err(|e| e.to_string())?;
                self.logical_resolver
                    .register(TopologyKey::local(app_instance_id, dependency_name), entry);
            }
            outcomes.push(wire_binding_outcome(&outcome));
        }

        // §4A's dedup key hashes what a deploy *sends*, not what is
        // currently installed, so it cannot see a push that happened since
        // the last deploy. Without this, a repair redeploy of byte-identical
        // content after a push would match the stale hash and take the
        // no-op path, silently leaving the pushed (not the redeployed)
        // bindings in place -- exactly the case §4A's "restart is the cheap
        // path, deploy is the repair path" promises to handle. Clearing the
        // hash here forces that redeploy through the full reinstall instead.
        if any_applied
            && let Some((service_type, health_check_json, _, visibility)) =
                self.registry.deploy_facts(&write.service_id)
        {
            self.registry
                .set_deploy_facts(
                    write.service_id.clone(),
                    service_type,
                    health_check_json,
                    None,
                    visibility,
                )
                .await
                .map_err(|e| e.to_string())?;
        }

        Ok(outcomes)
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
    ///
    /// Renamed `undeploy_impl` (from `undeploy`) so the trait's own
    /// `undeploy` -- a thin wrapper -- can call it without recursing; the
    /// split mirrors `deploy`/`deploy_with_context` immediately above.
    async fn undeploy_impl(
        &self,
        service_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // Same reason and same placement as `deploy_with_context`'s
        // equivalent check: `service_id` is joined verbatim into
        // `hosted_apps_dir/<service_id>.json` below and then deleted,
        // before anything else runs against it.
        if !is_safe_service_id_for_path(&service_id) {
            return Err(format!(
                "service_id '{service_id}' is not a valid undeploy target: it must be non-empty \
                 and contain no '/', '\\\\', or '..' -- it is joined into a stored-record filename"
            ));
        }
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
        // `caller` on two failure paths. Abilities are deliberately flat
        // and independently grantable (§3.1 A2), so "deploy but not
        // undeploy" is a real, supported shape -- a deploy-only grantee
        // (`roymctl identity issue-grant --can orchestrator/deploy`, no
        // `orchestrator/undeploy`) whose deploy fails partway would be
        // rejected *again* by this check on the rollback attempt, on a
        // confusing second error. This was inert before anything could
        // create a `ControllerAgreement`, when every substrate was unowned
        // and every verified caller held all three abilities together for
        // free -- now that `ControllerAgreement`, and so real app-scoped
        // grants, are live: a grant meant to let its holder
        // deploy reliably should include `orchestrator/undeploy` alongside
        // `orchestrator/deploy` so a failed deploy can clean up after
        // itself. `deploy_grant.rs` documents the partial-grant shapes;
        // this comment records the specific rollback interaction so a
        // future grant-issuing tool does not reintroduce it silently.
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

        // M05A A5a §0.23: `undeploy` is a lifecycle action, gated the same
        // as `deploy`/`restart` -- a superseded supervisor must not be
        // able to tear down services it no longer manages. Ungated for a
        // standalone service with no app context, same as `restart`.
        if let Some((instance, _)) = self.registry.app_context_of(&service_id) {
            let management = self.check_generation(&instance, caller, generation)?;
            self.registry
                .set_app_instance_management(instance, management)
                .await
                .map_err(|e| e.to_string())?;
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
            self.app_sandbox_engine.forget_guest_http_permits(&service_id);
            self.app_sandbox_engine.forget_websocket_senders(&service_id);
        }
        self.sse_permits.remove(&service_id);

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

        // M06A A1: nothing survives an undeploy, so there is nothing to
        // keep -- unlike the deploy-time forward cleanup, which diffs
        // against a still-live new generation.
        if let Some((_, old)) = self.assets.remove(&service_id) {
            let remove = assets::hashes_of(&old.manifest, Some(&old.manifest_hash));
            if let Err(e) =
                assets::delete_hashes(&service_id, &remove, &BTreeSet::new(), &self.blob_provider)
                    .await
            {
                tracing::warn!(
                    "Failed to remove asset bundle blobs for service {}: {}",
                    service_id,
                    e
                );
            }
        }

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
        if let Err(e) = self.registry.remove_deploy_facts(&service_id).await {
            tracing::warn!("Failed to remove deploy facts for {}: {}", service_id, e);
        }
        self.probe_cache.remove(&service_id);
        // A2: persisted rows only -- the in-memory `StaticInventory` entry
        // stays (D-A2-9). A `TopologyEntry` is an app-scoped fact ("where
        // does `backend` live in instance X"), not a per-dependent one;
        // removing it when one of several dependents goes away would break
        // the others.
        //
        // Same call, same reasoning, on `install_app_context`'s redeploy
        // path: a redeploy that drops a dependency from its manifest calls
        // this exact method, and the entry it wrote into `StaticInventory`
        // stays too -- decided here explicitly, not inherited by accident,
        // because the "app-scoped, not per-dependent" argument above holds
        // just as much for a redeploy that stops declaring a dependency as
        // it does for an undeploy that removes the dependent entirely. The
        // two do diverge across a restart: `replay_persisted_bindings`
        // rebuilds `StaticInventory` from `service_bindings` alone, so an
        // entry no longer backed by any persisted row silently drops out on
        // restart even though it kept resolving right up to that point.
        // That is `StaticInventory`'s memory-vs-storage split working as
        // designed (deferred-backlog.md), not a new gap this call opens.
        let app_instance_id =
            self.registry.app_context_of(&service_id).map(|(instance, _)| instance);
        if let Err(e) = self.registry.remove_app_context(&service_id).await {
            tracing::warn!("Failed to remove app context for service {}: {}", service_id, e);
        }

        // M05A A5a §5.6: the standing backlog row `app_instance_owners`
        // rows never get forgotten. Once no service on this node names the
        // instance any more, its management row is dead weight and its id
        // can never be reclaimed by another caller without this.
        if let Some(instance_id) = app_instance_id
            && self.registry.app_context_of_any(&instance_id).is_none()
            && let Err(e) = self.registry.remove_app_instance_management(&instance_id).await
        {
            tracing::warn!("Failed to remove app instance management for {}: {}", instance_id, e);
        }

        Ok(())
    }

    /// Restart a deployed service in place (M05A A5a §4, ADR-0021 §4's
    /// "lifecycle actions"). Type-dispatched off `service_deploy_facts`
    /// (A4) recorded at deploy -- a `tcp` service's process runs outside
    /// this substrate and there is nothing here to restart, so it is
    /// refused rather than silently succeeding, which a supervisor's
    /// remediation budget would otherwise count as a real attempt.
    async fn restart_impl(
        &self,
        service_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // Same gate as `deploy`: a restart is a lifecycle write.
        let deploy_resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{service_id}' on this substrate",
                caller.caller_did
            ));
        }

        // M05A A5c §19.17: `deploy`/`undeploy`/`write-bindings` all refuse a
        // takeover of a service a different caller owns; `restart` was the
        // one lifecycle write missing this check. A node-wide grantee (the
        // same override `undeploy_impl` honours) skips it for free.
        if let Some(owner) = self.registry.owner_of(&service_id)
            && owner != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {owner}; only its owner or a substrate owner \
                 may restart it"
            ));
        }

        // Generation gate, only where an app instance exists (§0.23) --
        // ungated for a standalone service, same as `undeploy`.
        if let Some((instance, _)) = self.registry.app_context_of(&service_id) {
            let management = self.check_generation(&instance, caller, generation)?;
            self.registry
                .set_app_instance_management(instance, management)
                .await
                .map_err(|e| e.to_string())?;
        }

        let Some((recorded_type, ..)) = self.registry.deploy_facts(&service_id) else {
            return Err(format!(
                "no service type recorded for '{service_id}'; redeploy to record it"
            ));
        };
        match parse_service_type(&recorded_type) {
            Some(AppServiceType::Wasm) => {
                self.app_sandbox_engine.reload_wasm(&service_id).await.map_err(|e| e.to_string())
            }
            Some(AppServiceType::Container) => {
                self.podman_sandbox_engine.stop(&service_id).await.map_err(|e| e.to_string())?;
                self.podman_sandbox_engine.start(&service_id).await.map_err(|e| e.to_string())
            }
            Some(AppServiceType::Tcp) => Err(format!(
                "'{service_id}' is a tcp service; its process runs outside this substrate and \
                 cannot be restarted here"
            )),
            Some(AppServiceType::NativeHost) => {
                Err(format!("'{service_id}' is a native-host service and has no restart path"))
            }
            None => Err(format!(
                "'{service_id}' has a recorded service type ('{recorded_type}') this substrate \
                 does not recognize; redeploy to correct it"
            )),
        }
    }

    /// Run one scheduled tick (ADR-0023 §3/§6): dispatch
    /// `interface`/`method` on `service_id` through the local `ServiceProxy`,
    /// as `CallerContext::service_system(service_id)` -- the service acting
    /// as itself, not the supervisor calling it directly. Gated
    /// exactly as `restart_impl`: this is a lifecycle write, not a service
    /// call, so `orchestrator/deploy` decides it, not the target interface's
    /// own authorization.
    async fn run_scheduled_impl(
        &self,
        service_id: String,
        generation: u64,
        interface: String,
        method: String,
        params_json: Option<String>,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // Same gate as `restart`: a scheduled run is a lifecycle write.
        let deploy_resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{service_id}' on this substrate",
                caller.caller_did
            ));
        }

        // Same owner check `restart_impl` carries, and for the same reason.
        if let Some(owner) = self.registry.owner_of(&service_id)
            && owner != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {owner}; only its owner or a substrate owner \
                 may run a scheduled task on it"
            ));
        }

        // Generation gate, only where an app instance exists -- the same
        // rule `restart_impl` follows, so a superseded supervisor cannot
        // keep firing ticks at an instance another one now manages.
        if let Some((instance, _)) = self.registry.app_context_of(&service_id) {
            let management = self.check_generation(&instance, caller, generation)?;
            self.registry
                .set_app_instance_management(instance, management)
                .await
                .map_err(|e| e.to_string())?;
        }

        let params = match params_json {
            Some(text) => {
                serde_json::from_str(&text).map_err(|e| format!("params-json is not JSON: {e}"))?
            }
            // An empty positional array, not `Value::Null` -- the shape the
            // one existing in-tree caller of a no-argument guest method
            // sends (the `rpc` readiness probe).
            None => Value::Array(vec![]),
        };

        // The fourth gate, the counterpart of `restart_impl`'s "no service
        // type recorded" refusal, and for a sharper reason.
        // `ProxyRouter::invoke_inner` reads a miss in the local endpoint
        // registry as "the target lives somewhere else" and resolves it
        // through the community registry instead -- so without this, a
        // schedule naming a service this node does not host, or an
        // interface the deployed component does not export, turns into an
        // outbound call. That call carries this node's own key (the proxy
        // has no instance certificate to present for a service with no
        // local instance), and neither the owner check nor the generation
        // check above can see a service this node knows nothing about:
        // both are `if let Some`. The WIT contract for this verb says this
        // node only executes, so refusing here enforces what is already
        // written. The condition is exactly `invoke_inner`'s own
        // local-or-remote test, so this refuses when, and only when, the
        // call would otherwise leave the node.
        if self.registry.lookup(&service_id, &interface).is_none() {
            return Err(format!(
                "'{service_id}' has no local endpoint for interface '{interface}'; a scheduled \
                 run executes on the node that hosts the service and is never forwarded"
            ));
        }

        let proxy = self
            .current_service_proxy()
            .upgrade()
            .ok_or_else(|| "service proxy unavailable for a scheduled run".to_string())?;
        proxy
            .invoke(ProxyRequest {
                target_service: service_id.clone(),
                interface,
                method,
                params,
                caller: CallerContext::service_system(&service_id),
                origin: CallOrigin::Native { service_id: Some(service_id) },
                protocol: ProxyProtocol::JsonRpcV1,
                // A tick is not safe to repeat by default, and therefore
                // never fenced or replayed -- and never queued (ADR-0023
                // §3): the caller's next tick is the retry.
                idempotent: false,
                idempotency_key: None,
                // The proxy's own default; the guest's epoch budget is the
                // real ceiling on how long this can run.
                timeout: None,
            })
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// ADR-0020 §1's install-time verification of an instance certificate,
    /// shared by every path that installs one. Extracted so `deploy` and
    /// `renew-cert` cannot drift apart on it: two copies of DID and
    /// signature verification silently diverging is a security bug, not a
    /// style one.
    ///
    /// Four checks, in order: the certificate parses, it names
    /// `service_id` as its master, it certifies the key *this* node derives
    /// for *this* caller, and its signature/validity window/scope verify.
    /// Then the lifetime backstop below.
    fn verify_installed_instance_cert(
        &self,
        caller_did: &str,
        service_id: &str,
        cert_json: &str,
    ) -> Result<DelegationCertificate, String> {
        let cert = DelegationCertificate::from_json(cert_json)
            .map_err(|e| format!("Invalid instance certificate: {e}"))?;
        // (1) the certificate is for *this* member.
        if cert.master_did != service_id {
            return Err(format!(
                "instance certificate master_did '{}' does not name this deploy's service_id \
                 '{service_id}'",
                cert.master_did
            ));
        }
        // (2) it certifies *this node's* derived key, not some other key
        // the client chose.
        let derived_did = derive_did_key(
            &self.node_identity.derive_service_identity(caller_did, service_id).public_key(),
        );
        if cert.temporary_did != derived_did {
            return Err(format!(
                "instance certificate certifies '{}', not the key this substrate would derive \
                 ('{derived_did}') for this caller and service_id",
                cert.temporary_did
            ));
        }
        // (3) signature, validity window, and the narrow scope.
        cert.verify(service_id, &[SCOPE_SERVICE_INSTANCE])
            .map_err(|e| format!("Invalid instance certificate: {e}"))?;
        // (4) a deliberately generous ceiling on the lifetime, for the same
        // reason `EndpointInfo.not_after` has one: nothing else bounds
        // `expires_at_secs`, so a client-side mint error can produce a
        // certificate valid for years and the near-expiry warning that
        // would have caught it simply never fires. Generous rather than
        // tight on purpose -- the attended posture's certificates are
        // long-lived by design (ADR-0020 §3), and a ceiling tuned for an
        // automated renewal cadence would refuse those operators' own
        // deploys. This catches the unbounded mistake, not the deliberate
        // choice.
        let lifetime = cert.expires_at_secs.saturating_sub(cert.issued_at_secs);
        if lifetime > MAX_INSTANCE_CERT_LIFETIME_SECS {
            return Err(format!(
                "instance certificate lifetime is {lifetime}s, over this substrate's maximum of \
                 {MAX_INSTANCE_CERT_LIFETIME_SECS}s ({} days); reissue it with a shorter window",
                MAX_INSTANCE_CERT_LIFETIME_SECS / 86_400
            ));
        }
        Ok(cert)
    }

    /// Install a freshly-issued instance certificate on an already-deployed
    /// service without reinstalling it -- the certificate-only counterpart
    /// to `restart`, and the only path an unattended renewal has.
    ///
    /// Gated identically to `restart_impl`: the same `orchestrator/deploy`
    /// capability, the same owner-or-node-wide-grantee check, the same
    /// generation gate scoped to an app context, and the same "is this
    /// actually deployed" signal. Without that last one, a capability-
    /// holding caller could register a live native-dispatch entry for a
    /// `service_id` nothing ever deployed: `owner_of` passes vacuously for
    /// an unknown id, and an absent FDAE policy is not an error.
    ///
    /// Rebuilds the service's `SynSvcNativeService` around the new
    /// certificate, mirroring `deploy_with_context`'s own construction site
    /// in full. Without that rebuild the by-value copy the running service
    /// holds keeps the old certificate, and every `RelationshipProof` it
    /// signs afterwards fails verification -- so the rebuild is what makes
    /// renewal a fix rather than a new break.
    async fn renew_cert_impl(
        &self,
        service_id: String,
        generation: u64,
        instance_certificate: String,
        caller: &CallerContext,
    ) -> Result<(), String> {
        // Same gate as `deploy`/`restart`: installing a certificate changes
        // what a service speaks as, which is a lifecycle write.
        let deploy_resource = ResourceUri(format!("substrate:{}/app/{service_id}", self.node_did));
        if !caller
            .has_capability(&deploy_resource, &Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()))
        {
            return Err(format!(
                "caller {} holds no orchestrator/deploy grant for '{service_id}' on this substrate",
                caller.caller_did
            ));
        }

        if let Some(owner) = self.registry.owner_of(&service_id)
            && owner != caller.caller_did
            && !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY)
        {
            return Err(format!(
                "service '{service_id}' is owned by {owner}; only its owner or a substrate owner \
                 may renew its instance certificate"
            ));
        }

        if self.registry.deploy_facts(&service_id).is_none() {
            return Err(format!(
                "'{service_id}' is not deployed here; there is nothing to install a certificate on"
            ));
        }

        if let Some((instance, _)) = self.registry.app_context_of(&service_id) {
            let management = self.check_generation(&instance, caller, generation)?;
            self.registry
                .set_app_instance_management(instance, management)
                .await
                .map_err(|e| e.to_string())?;
        }

        let cert = self.verify_installed_instance_cert(
            &caller.caller_did,
            &service_id,
            &instance_certificate,
        )?;

        // Re-read rather than re-parse from a manifest this call does not
        // have. A stored document that no longer parses aborts here, before
        // anything is installed: falling back to `None` would silently drop
        // row/column filtering for the renewed instance, a materially worse
        // failure than the one `deploy_with_context` already has for the
        // same bad document (it fails the whole call).
        let stored_fdae = self
            .storage_provider
            .load_fdae_policy(&service_id)
            .await
            .map_err(|e| format!("Failed to read the stored FDAE policy: {e}"))?;
        let fdae_policy = match stored_fdae {
            Some(doc) => Some(Arc::new(syneroym_fdae::parse_and_validate(&doc).map_err(|e| {
                tracing::warn!("stored FDAE policy for service {service_id} no longer parses: {e}");
                "the stored FDAE policy no longer validates; renewal aborted rather than \
                 installing a certificate with row filtering silently dropped"
                    .to_string()
            })?)),
            None => None,
        };

        self.registry
            .set_instance_cert(service_id.clone(), cert.clone())
            .await
            .map_err(|e| format!("Instance certificate installation failed: {e}"))?;

        // Mirrors `deploy_with_context`'s own construction site in full --
        // every parameter, not an enumerated subset of it, or the renewed
        // service comes back with dead proxy/authorizer hooks.
        if let Some(native_dispatch) = self.native_dispatch.upgrade() {
            native_dispatch.insert(
                service_id.clone(),
                Arc::new(SynSvcNativeService::new(
                    service_id.clone(),
                    self.key_store.clone(),
                    self.storage_provider.clone(),
                    self.blob_provider.clone(),
                    self.messaging_broker.clone(),
                    fdae_policy,
                    self.node_identity.clone(),
                    &caller.caller_did,
                    self.current_service_proxy(),
                    self.current_row_authorizer(),
                    Some(cert),
                )) as Arc<dyn NativeService>,
            );
            Ok(())
        } else {
            Err(format!(
                "the certificate for '{service_id}' was installed, but the native dispatch \
                 registry is unavailable, so the running service still holds the old one"
            ))
        }
    }

    /// M05A A5a §0.26/§5.7: `adopt`'s read half. `Ok(None)` (not an error)
    /// for a caller with no visibility into the instance -- indistinguish-
    /// able from "no deploy has ever named this instance here", so a
    /// caller with no grant cannot use this to probe for an instance's
    /// existence (the same rule `status`'s `not-found` already follows,
    /// A4-10).
    async fn app_instance_management_of_impl(
        &self,
        app_instance_id: String,
        caller: &CallerContext,
    ) -> Result<Option<AppInstanceManagementWire>, String> {
        let held = self.registry.app_instance_management_of(&app_instance_id);
        if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_STATUS)
            && held.as_ref().is_none_or(|m| {
                m.owner_did != caller.caller_did
                    && m.supervisor_did.as_deref() != Some(caller.caller_did.as_str())
            })
        {
            return Ok(None);
        }
        Ok(held.as_ref().map(management_to_wire))
    }

    /// M05A A5a §0.26/§5.7: `adopt`'s write half. Subject to the same
    /// four-case rule as every other write, so a racing adopt loses here
    /// rather than at whichever supervisor issues a deploy first. A claim
    /// against an instance with no row at all creates one with
    /// `owner_did = caller`, the same first-write-wins rule `deploy`
    /// uses -- letting a supervisor adopt an instance before its first
    /// deploy lands.
    async fn claim_app_instance_impl(
        &self,
        app_instance_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY) {
            return Err(format!(
                "caller {} holds no node-wide orchestrator/deploy on this substrate; claiming an \
                 app instance is node-scoped because the instance spans services",
                caller.caller_did
            ));
        }
        // Generation 0 means unmanaged (the WIT `app-context.generation`
        // doc, and `check_generation`'s own rule): a claim presenting it
        // would persist a row with no supervisor recorded, reporting
        // success while claiming nothing. Refused outright rather than
        // silently accepted -- a real `adopt` always presents `held + 1`
        // (D-A5-10, at least 1), so this only rejects a caller invoking the
        // raw verb with a generation that cannot mean what `claim` means.
        if generation == 0 {
            return Err(format!(
                "app instance '{app_instance_id}' cannot be claimed at generation 0; 0 means \
                 unmanaged, so a claim must present a generation of 1 or higher"
            ));
        }
        let management = self.check_generation(&app_instance_id, caller, generation)?;
        self.registry
            .set_app_instance_management(app_instance_id, management)
            .await
            .map_err(|e| e.to_string())
    }

    /// M05A A5a §0.24/§5.6: clears an app instance's management stamp --
    /// `supervisor_did` back to `None`, `generation` back to 0, keeping
    /// `owner_did`. Gated node-wide (§0.28), not on an invented
    /// `app-instance/<id>` selector: `covers_resource` matches over a
    /// documented selector set with no such segment, and reusing
    /// `app/<app_instance_id>` would put app-instance ids and service ids
    /// in one namespace. The releasing writer must be the current manager
    /// (or ahead of it), so a superseded supervisor cannot release the
    /// instance out from under the live one.
    async fn release_app_instance_impl(
        &self,
        app_instance_id: String,
        generation: u64,
        caller: &CallerContext,
    ) -> Result<(), String> {
        if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_DEPLOY) {
            return Err(format!(
                "caller {} holds no node-wide orchestrator/deploy on this substrate; releasing an \
                 app instance is node-scoped because the instance spans services",
                caller.caller_did
            ));
        }
        // A release against an app instance with no row at all must be a
        // no-op: `check_generation`'s `None` arm exists to let `deploy` and
        // `claim` create a row on first touch, which is right for them but
        // wrong here -- it would let a release mint an ownership row for an
        // instance nobody has ever deployed, blocking a later legitimate
        // deploy from a different caller and leaving an unreachable row
        // behind (no service ever names an instance nobody deployed, so
        // `undeploy_impl`'s cleanup can never find it).
        if self.registry.app_instance_management_of(&app_instance_id).is_none() {
            return Ok(());
        }
        let mut management = self.check_generation(&app_instance_id, caller, generation)?;
        management.supervisor_did = None;
        management.generation = 0;
        self.registry
            .set_app_instance_management(app_instance_id, management)
            .await
            .map_err(|e| e.to_string())
    }

    async fn list_impl(&self, caller: &CallerContext) -> Result<Vec<DeployedService>, String> {
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
                visibility: registry.deploy_facts(&service_id).and_then(|f| f.3).map(|v| {
                    match v.as_str() {
                        "public" => WitVisibility::Public,
                        "internal" => WitVisibility::Internal,
                        _ => WitVisibility::Private,
                    }
                }),
            });
            entry.interfaces.push(interface);
        }

        let mut result: Vec<DeployedService> = services.into_values().collect();
        result.sort_by(|a, b| a.service_id.cmp(&b.service_id));

        // M04A Slice B7a: node-wide orchestrator authority sees everything --
        // the substrate owner (a verified `ControllerAgreement` controller;
        // an unowned substrate holds no node-wide authority
        // and so sees nothing here). Checks
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

    /// M05A A4: per-instance status for a supervisor's poll loop. An empty
    /// `service_ids` means "every service this caller may see", using
    /// `list_impl`'s own visibility filter -- reused verbatim rather than
    /// re-derived, since two independently-maintained visibility rules is
    /// how a disclosure bug gets introduced.
    /// Node facts (D-A4-18): gated on node-wide authority, not on seeing any
    /// one service -- what this node can run and where it publishes is a
    /// property of the node, not of a service grant. Split out of
    /// `status_impl` (A4-06) so a caller that only wants these four fields
    /// (`app deploy`'s preflight, D-A4-15) never pays `status_impl`'s
    /// per-service phase-check-and-probe cost, which for the node-wide owner
    /// credential means every deployed service on the node.
    fn node_facts_for(&self, caller: &CallerContext) -> Option<NodeFacts> {
        if !self.has_node_wide_ability(caller, Ability::ORCHESTRATOR_STATUS) {
            return None;
        }
        let (registry_url, dht_enabled) = match self.endpoint_publisher.get() {
            Some(publisher) => {
                let client = publisher.registry_client();
                (client.registry_url().map(str::to_string), client.dht_enabled())
            }
            // A substrate with no publisher wired (a test harness, or a node
            // with no registry role) reports "unknown", not "none" -- a
            // caller must not read an unwired publisher as a split-registry
            // fleet.
            None => (None, false),
        };
        Some(NodeFacts {
            node_did: self.node_did.clone(),
            service_types: compiled_service_types(),
            registry_url,
            dht_enabled,
        })
    }

    async fn status_impl(
        &self,
        service_ids: Vec<String>,
        caller: &CallerContext,
    ) -> Result<SubstrateStatus, String> {
        // A4-11: `parse_status_params` deliberately accepts anything on the
        // way in (an empty list already means "everything visible", so there
        // is no separate "give me nothing" case to protect), which leaves no
        // size gate on a caller-supplied id list otherwise. Checked before
        // any work happens on the list.
        if service_ids.len() > MAX_STATUS_SERVICE_IDS {
            return Err(format!(
                "status request names {} service ids, over the {MAX_STATUS_SERVICE_IDS} limit",
                service_ids.len()
            ));
        }

        let now = unix_seconds();
        let node = self.node_facts_for(caller);

        let visible = self.list_impl(caller).await?;
        let visible_by_id: HashMap<&str, &DeployedService> =
            visible.iter().map(|d| (d.service_id.as_str(), d)).collect();

        // A4-09 (post-review): a duplicate id in a caller-supplied list is
        // otherwise a target twice over -- A4-05's `join_all` runs both
        // concurrently, so they race the same cache-miss window `probe_cached`
        // has no single-flight for, bypassing the cache entirely rather than
        // the second occurrence reading the first's fresh entry. Deduped
        // here, before the partition, so the raw (pre-dedup) length is still
        // what `MAX_STATUS_SERVICE_IDS` bounds above.
        let mut seen_ids = BTreeSet::new();
        let service_ids: Vec<String> =
            service_ids.into_iter().filter(|id| seen_ids.insert(id.clone())).collect();

        let (targets, named_missing): (Vec<String>, Vec<String>) = if service_ids.is_empty() {
            (visible.iter().map(|d| d.service_id.clone()).collect(), Vec::new())
        } else {
            service_ids.into_iter().partition(|id| visible_by_id.contains_key(id.as_str()))
        };

        // A4-05: every target's phase check and probe run concurrently, not
        // one after another -- a node with several probed services would
        // otherwise serialize their timeouts inside this single RPC, and
        // enough of them could exceed the caller's own deadline, which reads
        // as `SubstrateUnreachable` for every service on an otherwise-healthy
        // node.
        let mut services: Vec<ServiceStatus> =
            futures::future::join_all(targets.iter().map(|service_id| {
                self.service_status_for(service_id, visible_by_id[service_id.as_str()], now)
            }))
            .await;

        // A4-10: a named id that is not visible is always reported
        // `not-found`, never `unauthorized` -- a caller without node-wide
        // `orchestrator/status` must not be able to tell "exists, but I
        // can't see it" from "never deployed" for an id it holds no grant
        // on at all. `readyz`'s rejection text was cited as already leaking
        // this, but it does not: `readyz` returns the identical "holds no
        // orchestrator/status grant" message whether or not the service
        // exists, checked before any existence lookup at all. A caller that
        // *does* hold node-wide status never reaches this branch for an id
        // that actually exists, since `list_impl` already returned it to
        // them above -- so no legitimate caller loses information here.
        for service_id in named_missing {
            services.push(ServiceStatus {
                service_id,
                service_type: None,
                endpoint_type: String::new(),
                app_instance_id: None,
                service_name: None,
                phase: InstancePhase::NotFound,
                probe: ProbeStatus::NotDeclared,
                instance_certificate_issued_at: None,
                instance_certificate_expires_at: None,
                probe_checked_at: None,
                binding_epochs: Vec::new(),
            });
        }

        services.sort_by(|a, b| a.service_id.cmp(&b.service_id));
        Ok(SubstrateStatus { node, checked_at: now, services })
    }

    /// Builds one service's status entry -- phase, probe (D-A4-7's
    /// probe-not-gated-by-phase rule), and certificate metadata. Split out of
    /// `status_impl` so every target can be computed concurrently (A4-05)
    /// via `join_all` instead of one after another.
    async fn service_status_for(
        &self,
        service_id: &str,
        dep: &DeployedService,
        now: u64,
    ) -> ServiceStatus {
        let facts = self.registry.deploy_facts(service_id);
        let service_type = facts.as_ref().map(|(t, ..)| t.clone());
        let phase = self.instance_phase(service_id, service_type.as_deref()).await;

        // D-A4-7: phase does NOT gate the probe. A `tcp` service is always
        // `Unknown` -- probing only `Running` would mean a declared probe
        // never runs for exactly the type that has no other signal. It is
        // skipped only where the instance is already known to be down,
        // where it would report a second symptom of one fault.
        let (probe, probe_checked_at) = match &phase {
            InstancePhase::Running | InstancePhase::Unknown(_) => {
                self.probe_cached(service_id, now).await
            }
            _ => (ProbeStatus::NotDeclared, None),
        };

        let cert = self.registry.instance_cert(service_id);
        let app_ctx = self.registry.app_context_of(service_id);

        // M05A A5a §6: read from the per-dependent persisted row, not the
        // shared resolver entry -- the resolver is keyed
        // `(app-instance-id, service-name)` and is one value per node, so
        // reading it would give every dependent the same answer.
        let binding_epochs = match self.registry.bindings_of(service_id).await {
            Ok(bindings) => bindings
                .into_iter()
                .filter_map(|(name, entry_json)| {
                    match serde_json::from_str::<TopologyEntry>(&entry_json) {
                        Ok(entry) => Some((name, entry.epoch.0)),
                        Err(e) => {
                            tracing::warn!(
                                "stored binding for '{service_id}' dependency '{name}' is \
                                 corrupt: {e}"
                            );
                            None
                        }
                    }
                })
                .collect(),
            Err(e) => {
                tracing::warn!("failed to load bindings for '{service_id}': {e}");
                Vec::new()
            }
        };

        ServiceStatus {
            service_id: service_id.to_string(),
            service_type,
            endpoint_type: dep.endpoint_type.clone(),
            app_instance_id: app_ctx.as_ref().map(|(id, _)| id.clone()),
            service_name: app_ctx.as_ref().map(|(_, name)| name.clone()),
            phase,
            probe,
            instance_certificate_issued_at: cert.as_ref().map(|c| c.issued_at_secs),
            instance_certificate_expires_at: cert.as_ref().map(|c| c.expires_at_secs),
            probe_checked_at,
            binding_epochs,
        }
    }

    /// Derives an [`InstancePhase`] for `service_id` from its recorded
    /// service type (M05A A4, D-A4-7). `readyz`'s `is_container` guess
    /// (D-A4-17) is repaired to read this same fact, so the two surfaces
    /// cannot disagree.
    async fn instance_phase(&self, service_id: &str, service_type: Option<&str>) -> InstancePhase {
        let Some(t) = service_type.and_then(parse_service_type) else {
            // Two cases land here, both correctly "the substrate cannot
            // say": (a) deployed by a pre-A4 binary -- pre-release, there is
            // no migration, the row appears on the next deploy; (b) the
            // node's own `orchestrator`/`security` endpoints, which
            // `list_impl` includes (it filters `NATIVE_CAPABILITY_INTERFACES`,
            // not the node-level ones) and which no deploy ever created.
            return InstancePhase::Unknown(
                "no service type recorded for this service; redeploy to record it".to_string(),
            );
        };

        // Only the three types a deploy can produce reach here: the wire
        // `service-type` variant has no `native-host` case.
        match t {
            AppServiceType::Wasm => {
                if self.app_sandbox_engine.is_deployed(service_id) {
                    InstancePhase::Running
                } else {
                    InstancePhase::NotRunning(
                        "no compiled component is loaded for this id".to_string(),
                    )
                }
            }
            AppServiceType::Container => {
                match self.podman_sandbox_engine.readyz(service_id).await {
                    Ok(()) => InstancePhase::Running,
                    Err(e) => InstancePhase::NotRunning(e.to_string()),
                }
            }
            // The process runs outside this substrate. A registration is
            // not liveness, and reporting it as `running` would be a lie the
            // supervisor then acts on. A declared probe still runs.
            AppServiceType::Tcp => InstancePhase::Unknown(
                "tcp services run outside this substrate; a declared health check is their only \
                 liveness signal"
                    .to_string(),
            ),
            AppServiceType::NativeHost => InstancePhase::Unknown(
                "native-host services have no deploy-time liveness signal".to_string(),
            ),
        }
    }

    /// Serves a cached probe result within `PROBE_MIN_INTERVAL_SECS`, or runs
    /// a fresh one (D-A4-8): a supervisor polling every few seconds must not
    /// turn into probe load on the target, and a wasm `rpc` probe costs a
    /// component instantiation.
    async fn probe_cached(&self, service_id: &str, now: u64) -> (ProbeStatus, Option<u64>) {
        if let Some(entry) = self.probe_cache.get(service_id)
            && now.saturating_sub(entry.0) < PROBE_MIN_INTERVAL_SECS
        {
            return (entry.1.clone(), Some(entry.0));
        }
        let status = self.run_probe(service_id).await;
        self.probe_cache.insert(service_id.to_string(), (now, status.clone()));
        (status, Some(now))
    }

    /// Runs the declared probe, if any, against the endpoint it names.
    async fn run_probe(&self, service_id: &str) -> ProbeStatus {
        let Some((_, Some(check_json), ..)) = self.registry.deploy_facts(service_id) else {
            return ProbeStatus::NotDeclared;
        };
        let check: WitHealthCheck = match serde_json::from_str(&check_json) {
            Ok(c) => c,
            Err(e) => {
                return ProbeStatus::Failing(format!("stored health check is unreadable: {e}"));
            }
        };

        let interface_name = match &check {
            WitHealthCheck::TcpConnect(p) => p.interface_name.clone(),
            WitHealthCheck::HttpGet(p) => p.interface_name.clone(),
            WitHealthCheck::Rpc(p) => p.interface_name.clone(),
        };
        let Some((endpoint, _)) = self.registry.lookup(service_id, &interface_name) else {
            return ProbeStatus::Failing(format!(
                "no endpoint registered for interface '{interface_name}'"
            ));
        };

        match check {
            WitHealthCheck::TcpConnect(p) => {
                let SubstrateEndpoint::TcpHostPort { host, port } = endpoint else {
                    return ProbeStatus::Failing(format!(
                        "interface '{interface_name}' is not a TCP endpoint"
                    ));
                };
                match tokio::time::timeout(
                    Duration::from_millis(u64::from(p.timeout_ms)),
                    tokio::net::TcpStream::connect((host.as_str(), port)),
                )
                .await
                {
                    Ok(Ok(_)) => ProbeStatus::Passing,
                    Ok(Err(e)) => ProbeStatus::Failing(format!("connect failed: {e}")),
                    Err(_) => {
                        ProbeStatus::Failing(format!("connect timed out after {}ms", p.timeout_ms))
                    }
                }
            }
            WitHealthCheck::HttpGet(p) => {
                let SubstrateEndpoint::TcpHostPort { host, port } = endpoint else {
                    return ProbeStatus::Failing(format!(
                        "interface '{interface_name}' is not a TCP endpoint"
                    ));
                };
                let url = format!("http://{host}:{port}{}", p.path);
                match tokio::time::timeout(
                    Duration::from_millis(u64::from(p.timeout_ms)),
                    self.http_probe_client.get(&url).send(),
                )
                .await
                {
                    Ok(Ok(resp)) if resp.status().as_u16() == p.expect_status => {
                        ProbeStatus::Passing
                    }
                    Ok(Ok(resp)) => ProbeStatus::Failing(format!(
                        "expected status {}, got {}",
                        p.expect_status,
                        resp.status().as_u16()
                    )),
                    Ok(Err(e)) => ProbeStatus::Failing(format!("http probe failed: {e}")),
                    Err(_) => ProbeStatus::Failing(format!(
                        "http probe timed out after {}ms",
                        p.timeout_ms
                    )),
                }
            }
            WitHealthCheck::Rpc(p) => {
                let request = JsonRpcRequest {
                    jsonrpc: "2.0".to_string(),
                    method: p.method.clone(),
                    params: Value::Array(vec![]),
                    id: Some(Value::from(1)),
                    idempotency_key: None,
                };
                // M05A A5c §19.13/D-A5c-12: `execute_probe_json`, not
                // `execute_wasm_json` directly -- bounded by the engine's
                // own `probe_instance_permits`, so a sweep with many
                // `rpc`-probed wasm services cannot request more
                // concurrent component instantiations than the pool can
                // serve (`caller: None` is still a substrate-originated
                // probe, the same choice `ProxyRouter::invoke_local` makes
                // for a guest-to-guest call).
                match tokio::time::timeout(
                    Duration::from_millis(u64::from(p.timeout_ms)),
                    self.app_sandbox_engine.execute_probe_json(
                        service_id,
                        &p.interface_name,
                        &request,
                    ),
                )
                .await
                {
                    Ok(Ok(_)) => ProbeStatus::Passing,
                    Ok(Err(e)) => ProbeStatus::Failing(format!("rpc probe failed: {e}")),
                    Err(_) => ProbeStatus::Failing(format!(
                        "rpc probe timed out after {}ms",
                        p.timeout_ms
                    )),
                }
            }
        }
    }
}

/// Service types this build can actually run (M05A A4). Container support is
/// a compile-time Cargo feature and invisible on the wire, which is why the
/// A3 substrate inventory had to trust an operator-typed `capabilities` list
/// (deferred-backlog.md). `tcp` needs no engine and is always available.
fn compiled_service_types() -> Vec<String> {
    let mut types = vec!["tcp".to_string()];
    if cfg!(feature = "app_sandbox") {
        types.push("wasm".to_string());
    }
    if cfg!(feature = "podman_sandbox") {
        types.push("container".to_string());
    }
    types.sort();
    types
}

fn unix_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A supervisor polling every few seconds must not turn into probe load on
/// the target substrate (the milestone's "health poll cost" budget), and a
/// wasm `rpc` probe costs a component instantiation (D-A4-8).
const PROBE_MIN_INTERVAL_SECS: u64 = 5;

/// The most `service_ids` a single `status` call answers (A4-11). Well above
/// any real fleet a `HealthTarget`/inventory names today; exists only to cap
/// an unbounded, caller-supplied list from any verified caller, not to
/// constrain normal use.
const MAX_STATUS_SERVICE_IDS: usize = 500;

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Mutex, Weak},
    };

    use dashmap::DashMap;
    use sha2::{Digest, Sha256};
    use syneroym_core::{
        asset_manifest::AssetRegistry,
        config::SubstrateConfig,
        http_routes::HttpRouteRegistry,
        local_registry::EndpointRegistry,
        storage::{EndpointStorage, MockStorage},
        test_constants::{
            GREETER_INTERFACE_NAME, STREAM_TEST_DRIVER_INTERFACE, greeter_wasm_path,
            stream_test_wasm_path,
        },
    };
    use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
    use syneroym_data_db::{SqliteStorageProvider, traits::StorageProvider};
    use syneroym_data_keystore::KeyStore;
    use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
    use syneroym_rpc::{NativeDispatchRegistry, ProxyError, ServiceProxy};
    use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
        AssetBundle as WitAssetBundle, HttpProbe as WitHttpProbe, NetworkEndpoint, PlannedService,
        RpcProbe as WitRpcProbe, ServiceConfig, TcpProbe as WitTcpProbe,
    };

    use super::*;
    use crate::dummy_sandbox::{AppSandboxEngine, ContainerEngine};

    /// A fake `ServiceProxy` that records the last request it
    /// received and answers with whatever `response` was primed with, so
    /// `run_scheduled`'s dispatch is assertable without a real deployed
    /// target on the other end.
    #[derive(Debug, Default)]
    struct RecordingProxy {
        last_request: Mutex<Option<ProxyRequest>>,
        response: Mutex<Option<Result<Value, ProxyError>>>,
    }

    #[async_trait::async_trait]
    impl ServiceProxy for RecordingProxy {
        async fn invoke(&self, request: ProxyRequest) -> Result<Value, ProxyError> {
            *self.last_request.lock().unwrap() = Some(request);
            self.response.lock().unwrap().take().expect("response not set for test")
        }
    }

    /// Wires `proxy` into `service.service_proxy` the way `RouteHandler::init`
    /// does post-construction -- `service_proxy` is a `Weak`,
    /// so the caller must keep `proxy` alive for as long as the service is
    /// used.
    fn wire_service_proxy(service: &ControlPlaneService, proxy: &Arc<RecordingProxy>) {
        let dynamic: Arc<dyn ServiceProxy> = proxy.clone();
        let weak: Weak<dyn ServiceProxy> = Arc::downgrade(&dynamic);
        service.service_proxy.set(weak).expect("service_proxy already set");
    }

    /// M04A Slice B7b: a caller holding node-wide orchestrator authority on
    /// `"did:key:zTestNode"` (every test in this module inits
    /// `ControlPlaneService` with that node DID) -- the shape `build_caller`
    /// issues for a verified `ControllerAgreement` controller (before that
    /// tool existed, this was also the unowned-substrate bootstrap grant,
    /// now removed). Deploy/undeploy
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

    /// M04A Slice B7b: a caller holding an app-scoped `orchestrator/deploy`
    /// grant for exactly `service_id` (`substrate:<node>/app/<service_id>`
    /// selector) rather than `node_wide_caller`'s bare, node-wide form.
    /// `has_node_wide_ability` returns `false` for this caller -- needed for
    /// tests that must reach *past* the admission gate to exercise a
    /// takeover/ownership rejection, which a node-wide caller always
    /// bypasses.
    fn scoped_deploy_caller(caller_did: &str, service_id: &str) -> CallerContext {
        use syneroym_rpc::{AuthLevel, Capability, SessionContext};

        let resource = ResourceUri(format!("substrate:did:key:zTestNode/app/{service_id}"));
        CallerContext {
            caller_did: caller_did.to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: caller_did.to_string(),
                capabilities: vec![Capability {
                    with: resource,
                    can: Ability(Ability::ORCHESTRATOR_DEPLOY.to_string()),
                    caveats: None,
                }],
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                        health_check: None,
                        assets: None,
                        visibility: None,
                    },
                    service_type: WitServiceType::Wasm(WasmManifest {
                        source: ArtifactSource::Url("../../../../../etc/passwd".to_string()),
                        hash: None,
                        interfaces: vec![],
                    }),
                    registry_certificate: None,
                    instance_certificate: None,
                },
                app_context: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                        health_check: None,
                        assets: None,
                        visibility: None,
                    },
                    service_type: WitServiceType::Wasm(WasmManifest {
                        source: ArtifactSource::Url("/etc/passwd".to_string()),
                        hash: None,
                        interfaces: vec![],
                    }),
                    registry_certificate: None,
                    instance_certificate: None,
                },
                app_context: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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

    // ── A2: app context and dependency bindings ─────────────────────────

    fn app_context(
        app_instance_id: &str,
        service_name: &str,
        bindings: Vec<DependencyBinding>,
    ) -> AppContext {
        AppContext {
            app_instance_id: app_instance_id.to_string(),
            service_name: service_name.to_string(),
            bindings,
            // Unmanaged (M05A A5a): every existing test here is an
            // ordinary operator-style deploy, unaffected by the
            // generation gate. Tests that need a specific generation
            // override it with `AppContext { generation: N, ..app_context(...) }`.
            generation: 0,
        }
    }

    fn dependency_binding(name: &str, members: Vec<&str>) -> DependencyBinding {
        DependencyBinding {
            dependency_name: name.to_string(),
            app_instance_id: "app-1".to_string(),
            mode: WitTopologyMode::Singleton,
            members: members.into_iter().map(str::to_string).collect(),
            epoch: 0,
            cache_ttl_ms: 60_000,
        }
    }

    #[tokio::test]
    async fn a_deploy_carrying_an_app_context_registers_a_resolvable_binding() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let ctx = app_context(
            "app-1",
            "frontend",
            vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
        );
        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(ctx),
                &node_wide_caller("test-caller"),
            )
            .await
            .unwrap();

        assert_eq!(
            service.registry.app_context_of("frontend-svc"),
            Some(("app-1".to_string(), "frontend".to_string()))
        );
        let resolved = service
            .logical_resolver
            .resolve(
                &TopologyKey::local(
                    AppInstanceId::new("app-1"),
                    LogicalServiceName::new("backend"),
                ),
                None,
            )
            .unwrap();
        assert_eq!(resolved.to_string(), "did:key:zBackendMember");
    }

    #[tokio::test]
    async fn a_redeploy_that_drops_a_dependency_leaves_no_stale_persisted_row() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("test-caller");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();
        assert_eq!(
            service.registry.all_bindings().await.unwrap().len(),
            1,
            "the first deploy must have written exactly one binding row"
        );

        // A redeploy whose manifest no longer declares any dependency.
        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![])),
                &caller,
            )
            .await
            .unwrap();

        assert!(
            service.registry.all_bindings().await.unwrap().is_empty(),
            "a redeploy that drops a dependency must not leave its persisted row behind"
        );
    }

    /// D-A2-9, extended explicitly to redeploy (post-review): a `Topology
    /// Entry` is an app-scoped fact, not a per-dependent one, so dropping
    /// the only dependent that declared it must not evict the in-memory
    /// `StaticInventory` entry other dependents in the same app instance
    /// might still rely on. Pins the decision either way, as the review
    /// asked for -- this asserts "keep it", matching `undeploy`'s existing
    /// behavior and the same reasoning restated at its call site.
    #[tokio::test]
    async fn a_redeploy_that_drops_a_dependency_still_resolves_it_in_memory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("test-caller");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();

        // A redeploy whose manifest no longer declares any dependency.
        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![])),
                &caller,
            )
            .await
            .unwrap();

        let resolved = service
            .logical_resolver
            .resolve(
                &TopologyKey::local(
                    AppInstanceId::new("app-1"),
                    LogicalServiceName::new("backend"),
                ),
                None,
            )
            .unwrap();
        assert_eq!(
            resolved.to_string(),
            "did:key:zBackendMember",
            "the persisted row is gone (asserted above), but the in-memory StaticInventory entry \
             a different dependent in the same app instance might still rely on must survive \
             until restart"
        );
    }

    /// D-A2-2 / ADR-0021 §2 (post-review fix): a deploy may only bind
    /// dependencies for its own declared app instance. Without this check,
    /// a `DependencyBinding.app_instance_id` that disagrees with its own
    /// `AppContext.app_instance_id` would silently write into a different
    /// app instance's resolution table.
    #[tokio::test]
    async fn a_binding_naming_a_different_app_instance_than_its_own_context_fails_the_deploy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let mismatched_binding = DependencyBinding {
            dependency_name: "backend".to_string(),
            app_instance_id: "app-2".to_string(),
            mode: WitTopologyMode::Singleton,
            members: vec!["did:key:zBackendMember".to_string()],
            epoch: 0,
            cache_ttl_ms: 60_000,
        };
        let err = service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![mismatched_binding])),
                &node_wide_caller("test-caller"),
            )
            .await
            .unwrap_err();
        assert!(err.contains("app-1") && err.contains("app-2"), "{err}");

        // Nothing must have been written -- the rejection is at validation
        // time, before any registry write.
        assert!(service.registry.all_bindings().await.unwrap().is_empty());
    }

    /// A2 post-review fix: an app instance's first successful deploy
    /// becomes its owner (first-write-wins, the same shape `service_id`
    /// ownership already uses). Without it, any caller authorized to
    /// deploy *some* service could name a different, already-claimed app
    /// instance in its own `app_context` and overwrite the binding that
    /// instance's other, unrelated services resolve -- reachable even
    /// though every `binding.app_instance_id` here correctly matches its
    /// own `app_context.app_instance_id` (finding 02's check alone does
    /// not close this: it only forces the attacker to also lie about which
    /// app instance its own service belongs to).
    /// B1 (Slice A5b review): `open_service_db` and `native_dispatch` both
    /// key on a bare `service_id` with no reservation of their own, so
    /// before this check a deploy under the node's own DID overwrote
    /// `ControlPlaneService`'s own dispatch entry (full node takeover), and
    /// a deploy under `"supervisor"` opened the supervisor's vault and
    /// overwrote its dispatch entry. Each caller below holds a capability
    /// scoped exactly to the `service_id` it targets, so the rejection is
    /// the reserved-name check firing, not the ordinary authorization gate.
    #[tokio::test]
    async fn a_deploy_cannot_claim_the_nodes_own_did_or_the_supervisor_dispatch_id() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let err = service
            .deploy(
                "did:key:zTestNode".to_string(),
                inline_manifest(None, None, None),
                &scoped_deploy_caller("did:key:zMallory", "did:key:zTestNode"),
            )
            .await
            .unwrap_err();
        assert!(err.contains("reserved"), "{err}");

        let err = service
            .deploy(
                SUPERVISOR_RESERVED_SERVICE_ID.to_string(),
                inline_manifest(None, None, None),
                &scoped_deploy_caller("did:key:zMallory", SUPERVISOR_RESERVED_SERVICE_ID),
            )
            .await
            .unwrap_err();
        assert!(err.contains("reserved"), "{err}");
    }

    #[tokio::test]
    async fn a_deploy_cannot_claim_an_app_instance_owned_by_a_different_caller() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &scoped_deploy_caller("did:key:zAlice", "frontend-svc"),
            )
            .await
            .unwrap();

        let attacker_binding = DependencyBinding {
            dependency_name: "backend".to_string(),
            app_instance_id: "app-1".to_string(),
            mode: WitTopologyMode::Singleton,
            members: vec!["did:key:zAttackerMember".to_string()],
            epoch: 0,
            cache_ttl_ms: 60_000,
        };
        let err = service
            .deploy_with_context(
                "evil-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "evil", vec![attacker_binding])),
                &scoped_deploy_caller("did:key:zBob", "evil-svc"),
            )
            .await
            .unwrap_err();
        assert!(err.contains("app-1") && err.contains("owned by"), "{err}");

        let resolved = service
            .logical_resolver
            .resolve(
                &TopologyKey::local(
                    AppInstanceId::new("app-1"),
                    LogicalServiceName::new("backend"),
                ),
                None,
            )
            .unwrap();
        assert_eq!(
            resolved.to_string(),
            "did:key:zBackendMember",
            "the rejected deploy must not have overwritten alice's binding"
        );
    }

    /// Positive-path counterpart: the app instance's own owner may go on
    /// deploying further services into it, and a second service sharing an
    /// app instance with the first is the ordinary multi-service-per-app
    /// shape A2 exists for -- the ownership check above must not lock out
    /// the caller who legitimately owns the app instance.
    #[tokio::test]
    async fn an_app_instance_owner_may_deploy_a_second_service_into_it() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice_frontend = scoped_deploy_caller("did:key:zAlice", "frontend-svc");
        let alice_worker = scoped_deploy_caller("did:key:zAlice", "worker-svc");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &alice_frontend,
            )
            .await
            .unwrap();

        let result = service
            .deploy_with_context(
                "worker-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "worker",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &alice_worker,
            )
            .await;
        assert!(result.is_ok(), "the app instance's own owner must be able to join it: {result:?}");
    }

    // ── M05A A5a: the generation stamp ───────────────────────────────────

    /// Matrix row 9's substrate half: a write presenting a generation
    /// below the held one is rejected, and the error names the held
    /// generation -- the text A5b's supervisor parses to know it has been
    /// superseded (ADR-0021 §4).
    #[tokio::test]
    async fn a_lower_generation_write_is_rejected_and_the_error_names_the_held_generation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = scoped_deploy_caller("did:key:zAlice", "frontend-svc");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(AppContext { generation: 5, ..app_context("app-1", "frontend", vec![]) }),
                &alice,
            )
            .await
            .unwrap();

        let err = service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![])),
                &alice,
            )
            .await
            .unwrap_err();
        assert!(err.contains("at generation 5"), "{err}");
        assert!(err.contains("ADR-0021"), "{err}");
    }

    /// Matrix row 8: two writers both authorized on this substrate (both
    /// node-wide here, so the pre-existing app-instance ownership check
    /// does not itself reject the second one) presenting the *same*
    /// generation is a two-writer conflict, not a tie.
    #[tokio::test]
    async fn a_second_writer_at_the_same_generation_is_rejected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = node_wide_caller("did:key:zAlice");
        let bob = node_wide_caller("did:key:zBob");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(AppContext { generation: 3, ..app_context("app-1", "frontend", vec![]) }),
                &alice,
            )
            .await
            .unwrap();

        let err = service
            .deploy_with_context(
                "worker-svc".to_string(),
                inline_manifest(None, None, None),
                Some(AppContext { generation: 3, ..app_context("app-1", "worker", vec![]) }),
                &bob,
            )
            .await
            .unwrap_err();
        assert!(err.contains("second writer"), "{err}");
    }

    /// §0.18's regression guard: the bug that would have locked a
    /// supervisor out of its own app on its first post-adopt reconcile.
    /// The same caller, presenting the *same* generation it already holds
    /// (not 0), must keep succeeding -- this is the supervisor's steady
    /// state.
    #[tokio::test]
    async fn the_recorded_supervisor_may_write_repeatedly_at_its_own_generation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let supervisor = node_wide_caller("did:key:zSupervisor");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(AppContext { generation: 7, ..app_context("app-1", "frontend", vec![]) }),
                &supervisor,
            )
            .await
            .unwrap();

        let result = service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(AppContext { generation: 7, ..app_context("app-1", "frontend", vec![]) }),
                &supervisor,
            )
            .await;
        assert!(
            result.is_ok(),
            "the recorded supervisor must be able to redeploy at its own generation: {result:?}"
        );
    }

    /// The A0-A4 compatibility property: an app instance nobody has ever
    /// `adopt`ed keeps accepting its authorized writer's ordinary,
    /// unmanaged (`generation: 0`) deploys, unaffected by the new gate.
    #[tokio::test]
    async fn an_unadopted_app_instance_accepts_any_authorized_writer() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = scoped_deploy_caller("did:key:zAlice", "frontend-svc");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![])),
                &alice,
            )
            .await
            .unwrap();

        // No `adopt`/`claim` ever ran -- every deploy still presents
        // generation 0, the A0-A4 convention, and must keep succeeding.
        let result = service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![])),
                &alice,
            )
            .await;
        assert!(
            result.is_ok(),
            "an unadopted instance must keep accepting its authorized writer: {result:?}"
        );
    }

    /// The same property, but with a genuinely *different* authorized
    /// writer than the instance's first deploy -- the case
    /// `an_unadopted_app_instance_accepts_any_authorized_writer` names but
    /// does not actually exercise, since it reuses the same caller twice.
    /// A node-wide caller is what `deploy_with_context`'s own
    /// app-instance-owner check requires to deploy over a different
    /// owner's instance; without generation 0 staying unmanaged on the
    /// instance's *first* write, this second, node-wide-authorized deploy
    /// would be rejected as "a second writer at the same generation",
    /// defeating that owner-check bypass entirely.
    #[tokio::test]
    async fn a_second_different_authorized_writer_may_also_deploy_into_an_unadopted_instance() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = scoped_deploy_caller("did:key:zAlice", "frontend-svc");
        let bob = node_wide_caller("did:key:zBob");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![])),
                &alice,
            )
            .await
            .unwrap();

        let result = service
            .deploy_with_context(
                "backend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "backend", vec![])),
                &bob,
            )
            .await;
        assert!(
            result.is_ok(),
            "a different node-wide-authorized caller must also be able to deploy into an \
             unadopted instance: {result:?}"
        );
    }

    /// §0.24: releasing an app instance clears its management stamp
    /// (`supervisor_did`/`generation`, not `owner_did` -- release restores
    /// manual operation, it does not transfer ownership), so a plain
    /// operator deploy (presenting generation 0, since nothing manages the
    /// instance any more) can touch it again. Without this an
    /// adopted-then-released instance would be locked out forever. Uses a
    /// node-wide caller for the post-release deploy since `owner_did`
    /// still names the supervisor, not this operator -- the same bypass
    /// the pre-existing ownership check already grants a substrate owner.
    #[tokio::test]
    async fn releasing_an_app_instance_lets_a_plain_deploy_touch_it_again() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let supervisor = node_wide_caller("did:key:zSupervisor");

        service.claim_app_instance("app-1".to_string(), 1, &supervisor).await.unwrap();
        service.release_app_instance("app-1".to_string(), 1, &supervisor).await.unwrap();

        let operator = node_wide_caller("did:key:zOperator");
        let result = service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![])),
                &operator,
            )
            .await;
        assert!(result.is_ok(), "a released instance must accept a plain deploy again: {result:?}");
    }

    /// `check_generation`'s `None` arm exists so `deploy`/`claim` can
    /// create a row on an app instance's first touch -- right for them,
    /// wrong for `release`. Releasing an instance nobody has ever deployed
    /// must be a no-op, not mint an `owner_did` row that blocks a later
    /// legitimate deploy from a different caller and can never be reclaimed
    /// (no service ever names an instance nobody deployed, so `undeploy`'s
    /// cleanup can never reach it).
    #[tokio::test]
    async fn releasing_an_unknown_app_instance_is_a_no_op() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let attacker = node_wide_caller("did:key:zAttacker");

        service.release_app_instance("app-never-deployed".to_string(), 0, &attacker).await.unwrap();

        assert!(
            service.registry.app_instance_management_of("app-never-deployed").is_none(),
            "releasing an app instance with no row must not create one"
        );
    }

    /// The other half of the backlog row `release-app-instance` was built
    /// to close: undeploying the last service naming an app instance must
    /// forget its management row, or the instance id can never be
    /// reclaimed by another caller.
    #[tokio::test]
    async fn undeploying_the_last_service_of_an_instance_forgets_its_management_row() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![])),
                &caller,
            )
            .await
            .unwrap();
        assert!(service.registry.app_instance_management_of("app-1").is_some());

        service.undeploy("frontend-svc".to_string(), 0, &caller).await.unwrap();

        assert!(
            service.registry.app_instance_management_of("app-1").is_none(),
            "the last service's undeploy must forget the app instance's management row"
        );
    }

    /// §0.26: `adopt`'s read half must report the held generation to the
    /// instance's own owner -- otherwise a supervisor cannot compute
    /// `held + 1`.
    #[tokio::test]
    async fn app_instance_management_of_reports_the_held_generation_to_the_owner() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = scoped_deploy_caller("did:key:zAlice", "frontend-svc");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(AppContext { generation: 4, ..app_context("app-1", "frontend", vec![]) }),
                &alice,
            )
            .await
            .unwrap();

        let management = service
            .app_instance_management_of("app-1".to_string(), &alice)
            .await
            .unwrap()
            .expect("the owner must see its own instance's management stamp");
        assert_eq!(management.generation, 4);
        assert_eq!(management.owner_did, "did:key:zAlice");
        assert_eq!(management.supervisor_did.as_deref(), Some("did:key:zAlice"));
    }

    /// A4-10's rule, applied here too (§0.26): a caller with no visibility
    /// into the instance gets `Ok(None)`, indistinguishable from "never
    /// deployed here", not an error -- so it cannot be used to probe for
    /// the instance's existence.
    #[tokio::test]
    async fn app_instance_management_of_returns_none_to_a_caller_with_no_grant() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = scoped_deploy_caller("did:key:zAlice", "frontend-svc");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![])),
                &alice,
            )
            .await
            .unwrap();

        let mallory = scoped_deploy_caller("did:key:zMallory", "some-other-svc");
        let result =
            service.app_instance_management_of("app-1".to_string(), &mallory).await.unwrap();
        assert!(
            result.is_none(),
            "a caller with no visibility into the instance must not learn it exists, not even as \
             an error"
        );
    }

    /// §0.26: the property that makes `adopt` durable at the moment of the
    /// claim, not on whatever write happens next -- a bare claim, with no
    /// deploy at all, must be readable back and must not have installed
    /// anything else.
    #[tokio::test]
    async fn claim_app_instance_records_the_generation_without_any_other_write() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let supervisor = node_wide_caller("did:key:zSupervisor");

        service.claim_app_instance("app-1".to_string(), 1, &supervisor).await.unwrap();

        let management = service
            .app_instance_management_of("app-1".to_string(), &supervisor)
            .await
            .unwrap()
            .expect("the claim must have created a management row");
        assert_eq!(management.generation, 1);
        assert_eq!(management.supervisor_did.as_deref(), Some("did:key:zSupervisor"));
        assert!(
            service.registry.app_context_of_any("app-1").is_none(),
            "a bare claim must not install a service or an app context"
        );
    }

    /// Two supervisors racing an `adopt` must lose deterministically at
    /// the substrate, at the moment of the claim -- not discover it only
    /// once one of them happens to issue a deploy.
    #[tokio::test]
    async fn a_second_claim_at_the_same_generation_is_rejected() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let supervisor_a = node_wide_caller("did:key:zSupervisorA");
        let supervisor_b = node_wide_caller("did:key:zSupervisorB");

        service.claim_app_instance("app-1".to_string(), 1, &supervisor_a).await.unwrap();

        let err =
            service.claim_app_instance("app-1".to_string(), 1, &supervisor_b).await.unwrap_err();
        assert!(err.contains("second writer"), "{err}");
    }

    /// §0.28: `claim`/`release` are node-scoped acts (an app instance
    /// spans services), so an app-scoped `orchestrator/deploy` grant --
    /// enough to deploy one service -- must not be enough for either.
    #[tokio::test]
    async fn claim_and_release_are_rejected_without_node_wide_orchestrator_deploy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let scoped = scoped_deploy_caller("did:key:zAlice", "frontend-svc");

        let claim_err =
            service.claim_app_instance("app-1".to_string(), 1, &scoped).await.unwrap_err();
        assert!(claim_err.contains("node-wide"), "{claim_err}");

        let release_err =
            service.release_app_instance("app-1".to_string(), 1, &scoped).await.unwrap_err();
        assert!(release_err.contains("node-wide"), "{release_err}");
    }

    /// Generation 0 means unmanaged, so a claim presenting it cannot mean
    /// "claim supervision" -- without this refusal it would silently
    /// record no supervisor at all and still report success, and on a
    /// fresh instance still create the owner row, making the no-op harder
    /// to notice.
    #[tokio::test]
    async fn claiming_an_app_instance_at_generation_0_is_refused() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let supervisor = node_wide_caller("did:key:zSupervisor");

        let err =
            service.claim_app_instance("app-1".to_string(), 0, &supervisor).await.unwrap_err();
        assert!(err.contains("generation 0"), "{err}");
        assert!(
            service.registry.app_instance_management_of("app-1").is_none(),
            "a refused claim must not create a management row"
        );
    }

    // ── M05A A5a: write-bindings ─────────────────────────────────────────

    #[tokio::test]
    async fn write_bindings_is_rejected_without_an_orchestrator_deploy_grant() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();

        let no_grant = CallerContext::service_system("nobody");
        let err = service
            .write_bindings(
                BindingWrite {
                    service_id: "frontend-svc".to_string(),
                    app_instance_id: "app-1".to_string(),
                    bindings: vec![dependency_binding("backend", vec!["did:key:zNewMember"])],
                    generation: 0,
                },
                &no_grant,
            )
            .await
            .unwrap_err();
        assert!(err.contains("orchestrator/deploy"), "{err}");
    }

    /// A grant scoped to one service of an app instance is not the same as
    /// authority over the instance as a whole: `bob` holds
    /// `orchestrator/deploy` on `worker-svc` specifically -- enough to pass
    /// the capability check and the app-context match, since `worker-svc`
    /// genuinely belongs to `app-1` -- but `app-1` is `alice`'s, and `bob`
    /// holds no node-wide authority either. `deploy_with_context` refuses
    /// exactly this shape of caller for the same app instance; `write-
    /// bindings` must too, since a push lands in the shared resolver entry
    /// every service of the instance resolves through, not only `bob`'s
    /// own.
    #[tokio::test]
    async fn write_bindings_is_rejected_for_a_non_owner_with_only_a_service_scoped_grant() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "frontend", vec![])),
                &alice,
            )
            .await
            .unwrap();
        service
            .deploy_with_context(
                "worker-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "worker",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &alice,
            )
            .await
            .unwrap();

        let bob = scoped_deploy_caller("did:key:zBob", "worker-svc");
        let err = service
            .write_bindings(
                BindingWrite {
                    service_id: "worker-svc".to_string(),
                    app_instance_id: "app-1".to_string(),
                    bindings: vec![dependency_binding("backend", vec!["did:key:zNewMember"])],
                    generation: 0,
                },
                &bob,
            )
            .await
            .unwrap_err();
        assert!(err.contains("app-1") && err.contains("zAlice"), "{err}");
    }

    #[tokio::test]
    async fn write_bindings_refuses_a_service_whose_app_context_names_another_instance() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();

        let err = service
            .write_bindings(
                BindingWrite {
                    service_id: "frontend-svc".to_string(),
                    app_instance_id: "app-2".to_string(),
                    bindings: vec![dependency_binding("backend", vec!["did:key:zNewMember"])],
                    generation: 0,
                },
                &caller,
            )
            .await
            .unwrap_err();
        assert!(err.contains("app-1") && err.contains("app-2"), "{err}");
    }

    /// A push may only update a dependency the service already declared
    /// at deploy -- a new logical name changes the guest's contract and
    /// needs a redeploy, not a push.
    #[tokio::test]
    async fn write_bindings_refuses_a_dependency_the_service_never_declared() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();

        let err = service
            .write_bindings(
                BindingWrite {
                    service_id: "frontend-svc".to_string(),
                    app_instance_id: "app-1".to_string(),
                    bindings: vec![dependency_binding("cache", vec!["did:key:zCacheMember"])],
                    generation: 0,
                },
                &caller,
            )
            .await
            .unwrap_err();
        assert!(err.contains("cache") && err.contains("redeploy"), "{err}");
    }

    /// D-A5-23: the accepting generation is persisted before any binding is
    /// examined, not after the whole call succeeds -- so a write that is
    /// later refused (here, an undeclared dependency) still leaves the
    /// substrate remembering who was authorized to write at that
    /// generation, the same property §0.27 proves on the deploy path.
    #[tokio::test]
    async fn a_refused_write_still_persists_the_accepting_generation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let supervisor = node_wide_caller("did:key:zSupervisor");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(AppContext {
                    generation: 1,
                    ..app_context(
                        "app-1",
                        "frontend",
                        vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                    )
                }),
                &supervisor,
            )
            .await
            .unwrap();

        let result = service
            .write_bindings(
                BindingWrite {
                    service_id: "frontend-svc".to_string(),
                    app_instance_id: "app-1".to_string(),
                    bindings: vec![dependency_binding("cache", vec!["did:key:zCacheMember"])],
                    generation: 2,
                },
                &supervisor,
            )
            .await;
        assert!(result.is_err(), "the undeclared dependency must still be refused");

        let management = service.registry.app_instance_management_of("app-1").unwrap();
        assert_eq!(
            management.generation, 2,
            "the accepting generation must be persisted even though the write itself failed"
        );
    }

    /// The whole binding list is validated before any of it is applied: a
    /// refusal partway through (here, the second binding names an
    /// undeclared dependency) must leave every earlier binding in the same
    /// call untouched, not partially applied with no way for the caller to
    /// know which ones landed.
    #[tokio::test]
    async fn a_refused_binding_leaves_no_earlier_binding_in_the_same_call_applied() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();

        let err = service
            .write_bindings(
                BindingWrite {
                    service_id: "frontend-svc".to_string(),
                    app_instance_id: "app-1".to_string(),
                    bindings: vec![
                        DependencyBinding {
                            epoch: 1,
                            members: vec!["did:key:zNewMember".to_string()],
                            ..dependency_binding("backend", vec!["did:key:zNewMember"])
                        },
                        dependency_binding("cache", vec!["did:key:zCacheMember"]),
                    ],
                    generation: 0,
                },
                &caller,
            )
            .await
            .unwrap_err();
        assert!(err.contains("cache") && err.contains("redeploy"), "{err}");

        let backend =
            service.registry.binding_of("frontend-svc", "backend").await.unwrap().unwrap();
        assert!(
            backend.contains("zBackendMember") && !backend.contains("zNewMember"),
            "the earlier, individually-valid binding must not have been applied: {backend}"
        );
    }

    /// Matrix row 6: an ordinary retry -- the same epoch, the same content
    /// -- is a success that writes nothing.
    #[tokio::test]
    async fn a_binding_write_at_the_current_epoch_with_identical_content_writes_nothing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();

        let outcomes = service
            .write_bindings(
                BindingWrite {
                    service_id: "frontend-svc".to_string(),
                    app_instance_id: "app-1".to_string(),
                    bindings: vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                    generation: 0,
                },
                &caller,
            )
            .await
            .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert!(
            matches!(outcomes[0], BindingWriteOutcomeWire::NoOp),
            "expected NoOp, got a differently-shaped outcome"
        );
    }

    /// The property reference-scenario step 5 turns on: a binding push
    /// must never go through the deploy path. Uses the config-generation
    /// counter `deploy_with_context` always bumps as the proxy, since
    /// nothing else in this test harness tracks sandbox-engine calls.
    #[tokio::test]
    async fn a_binding_write_does_not_restart_the_service() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();
        let generation_before =
            service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap();

        service
            .write_bindings(
                BindingWrite {
                    service_id: "frontend-svc".to_string(),
                    app_instance_id: "app-1".to_string(),
                    bindings: vec![dependency_binding(
                        "backend",
                        vec!["did:key:zNewBackendMember"],
                    )],
                    generation: 0,
                },
                &caller,
            )
            .await
            .unwrap();

        let generation_after =
            service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap();
        assert_eq!(
            generation_before, generation_after,
            "a binding push must not go through the deploy path"
        );
    }

    /// §0.20: the epoch guard and the convergence read both classify
    /// against the **persisted per-dependent row**, not the shared
    /// resolver entry -- a push targeted at one dependent must not affect
    /// what a different dependent of the same instance has recorded.
    #[tokio::test]
    async fn two_dependents_of_one_instance_report_their_own_binding_epochs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();
        service
            .deploy_with_context(
                "worker-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "worker",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();

        service
            .write_bindings(
                BindingWrite {
                    service_id: "frontend-svc".to_string(),
                    app_instance_id: "app-1".to_string(),
                    bindings: vec![DependencyBinding {
                        dependency_name: "backend".to_string(),
                        app_instance_id: "app-1".to_string(),
                        mode: WitTopologyMode::Singleton,
                        members: vec!["did:key:zNewBackendMember".to_string()],
                        epoch: 1,
                        cache_ttl_ms: 60_000,
                    }],
                    generation: 0,
                },
                &caller,
            )
            .await
            .unwrap();

        let frontend_entry: TopologyEntry = serde_json::from_str(
            &service.registry.binding_of("frontend-svc", "backend").await.unwrap().unwrap(),
        )
        .unwrap();
        let worker_entry: TopologyEntry = serde_json::from_str(
            &service.registry.binding_of("worker-svc", "backend").await.unwrap().unwrap(),
        )
        .unwrap();
        assert_eq!(
            frontend_entry.epoch,
            TopologyEpoch(1),
            "frontend must report the epoch pushed to it"
        );
        assert_eq!(
            worker_entry.epoch,
            TopologyEpoch(0),
            "worker's own persisted row must be unaffected by a push targeted at frontend -- the \
             epoch guard classifies against the per-dependent row, not the shared resolver entry"
        );
    }

    // ── M05A A5a: deploy idempotency (matrix row 10) ─────────────────────

    /// Matrix row 10: a retry after a lost response -- the same manifest,
    /// the same app context minus generation, against a still-running
    /// service -- is a no-op. §0.27's regression guard: the management
    /// stamp must still advance to the new generation even though the
    /// deploy itself is deduplicated, because it is persisted at the
    /// generation gate, before the dedup check ever runs.
    #[tokio::test]
    async fn an_identical_redeploy_of_a_running_service_is_a_no_op() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");
        let manifest = inline_manifest(None, None, None);
        let ctx = app_context(
            "app-1",
            "frontend",
            vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
        );

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                manifest.clone(),
                Some(AppContext { generation: 1, ..ctx.clone() }),
                &caller,
            )
            .await
            .unwrap();
        let gen_before =
            service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap();

        // A later write at a higher generation (the supervisor's own
        // reconcile after an `adopt`) with an identical manifest and
        // context.
        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                manifest,
                Some(AppContext { generation: 2, ..ctx }),
                &caller,
            )
            .await
            .unwrap();
        let gen_after =
            service.storage_provider.get_latest_config_generation("frontend-svc").await.unwrap();
        assert_eq!(
            gen_before, gen_after,
            "an identical redeploy of a running service must be a no-op"
        );

        let management = service
            .registry
            .app_instance_management_of("app-1")
            .expect("the generation gate's persist must not be skipped by the dedup no-op");
        assert_eq!(
            management.generation, 2,
            "the redeploy's generation must be recorded even though the deploy itself was a no-op \
             -- without this assertion this test passes against the bug §0.27 fixes"
        );
    }

    /// Review finding E-1: `instance_certificate`/`registry_certificate`
    /// are minted fresh by `certify_placed_members` on every real apply
    /// (a new signature, a `SystemTime::now()`-derived expiry), so the
    /// test above -- which leaves both `None` on every call, like every
    /// other row-10 test -- never exercised the actual supervisor/
    /// `roymctl app deploy` path: hashing the whole manifest made those
    /// two fields alone change the hash every time, epoch or no epoch,
    /// so the no-op branch was unreachable from either real deploy path.
    /// Two independently-issued, genuinely different-in-bytes certificates
    /// for the *same* member -- like two real applies of the same desired
    /// state -- must still dedup as a no-op: without this assertion, this
    /// test passes against the bug E-1 found.
    #[tokio::test]
    async fn an_identical_redeploy_with_freshly_minted_certificates_is_still_a_no_op() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, _registry) =
            service_with_node_identity(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let member_master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&member_master.public_key());
        let derived = node_identity.derive_service_identity(&caller.caller_did, &service_id);

        // Different `expires_in_secs` guarantees different `expires_at_secs`
        // (and therefore a different signature) even if both calls land in
        // the same wall-clock second -- the churn E-1 is about, reproduced
        // deterministically rather than raced against real time.
        let cert_a = DelegationCertificate::issue(
            &member_master,
            derived.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        let cert_b = DelegationCertificate::issue(
            &member_master,
            derived.public_key(),
            7200,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        assert_ne!(
            cert_a.to_json().unwrap(),
            cert_b.to_json().unwrap(),
            "the two certificates must actually differ in bytes for this test to mean anything"
        );

        let ctx = app_context("app-1", "frontend", vec![]);
        let mut first = owner_test_manifest();
        first.instance_certificate = Some(cert_a.to_json().unwrap());
        let mut second = owner_test_manifest();
        second.instance_certificate = Some(cert_b.to_json().unwrap());

        service
            .deploy_with_context(
                service_id.clone(),
                first,
                Some(AppContext { generation: 1, ..ctx.clone() }),
                &caller,
            )
            .await
            .unwrap();
        let gen_before =
            service.storage_provider.get_latest_config_generation(&service_id).await.unwrap();

        service
            .deploy_with_context(
                service_id.clone(),
                second,
                Some(AppContext { generation: 2, ..ctx }),
                &caller,
            )
            .await
            .unwrap();
        let gen_after =
            service.storage_provider.get_latest_config_generation(&service_id).await.unwrap();
        assert_eq!(
            gen_before, gen_after,
            "a redeploy with identical content but a freshly re-issued certificate for the same \
             member must still be a no-op -- certificate freshness is not a content change"
        );
    }

    /// Row 10's boundary: an identical redeploy of a service the substrate
    /// no longer considers running must still reinstall it -- `restart` is
    /// the cheap path, `deploy` is the repair path.
    #[tokio::test]
    async fn an_identical_redeploy_of_a_stopped_service_still_reinstalls_it() {
        let wasm_bytes = fs::read(greeter_wasm_path())
            .expect("greeter fixture must be built (see test-components/greeter's own build step)");
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let owner = node_wide_caller("owner");

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wasm_bytes),
                hash: None,
                interfaces: vec![GREETER_INTERFACE_NAME.to_string()],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("greeter-svc".to_string(), manifest.clone(), &owner).await.unwrap();
        let gen_after_first =
            service.storage_provider.get_latest_config_generation("greeter-svc").await.unwrap();

        // Simulate the instance stopping without an `undeploy` -- the
        // substrate still holds the deploy facts (and their
        // manifest_hash), but nothing is loaded any more.
        service.app_sandbox_engine.stop_wasm("greeter-svc").await.unwrap();

        service.deploy("greeter-svc".to_string(), manifest, &owner).await.unwrap();
        let gen_after_second =
            service.storage_provider.get_latest_config_generation("greeter-svc").await.unwrap();
        assert_ne!(
            gen_after_first, gen_after_second,
            "a redeploy of a stopped service must reinstall, not no-op"
        );
    }

    /// The dedup key hashes what a deploy *sends*, not what a later
    /// `write-bindings` push installs, so a repair redeploy of
    /// byte-identical content after a push must not match the stale hash
    /// and take the no-op path -- that would leave the pushed bindings in
    /// place under a deploy that reports success, defeating "restart is
    /// the cheap path, deploy is the repair path" (§4A).
    #[tokio::test]
    async fn a_redeploy_after_a_binding_push_reinstalls_the_manifests_own_bindings() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");
        let manifest = inline_manifest(None, None, None);
        let ctx = app_context(
            "app-1",
            "frontend",
            vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
        );

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                manifest.clone(),
                Some(ctx.clone()),
                &caller,
            )
            .await
            .unwrap();

        // A push moves the installed binding to a different target at a
        // higher epoch, as a supervisor's `write-bindings` would.
        service
            .write_bindings(
                BindingWrite {
                    service_id: "frontend-svc".to_string(),
                    app_instance_id: "app-1".to_string(),
                    bindings: vec![DependencyBinding {
                        epoch: 1,
                        members: vec!["did:key:zPushedMember".to_string()],
                        ..dependency_binding("backend", vec!["did:key:zPushedMember"])
                    }],
                    generation: 0,
                },
                &caller,
            )
            .await
            .unwrap();
        let pushed = service.registry.binding_of("frontend-svc", "backend").await.unwrap().unwrap();
        assert!(pushed.contains("zPushedMember"), "{pushed}");

        // An operator, unaware of the push, redeploys the identical
        // manifest and context to repair the app -- this must reinstall
        // the manifest's own bindings, not no-op against the pushed state.
        service
            .deploy_with_context("frontend-svc".to_string(), manifest, Some(ctx), &caller)
            .await
            .unwrap();
        let repaired =
            service.registry.binding_of("frontend-svc", "backend").await.unwrap().unwrap();
        assert!(
            repaired.contains("zBackendMember") && !repaired.contains("zPushedMember"),
            "a repair redeploy after a push must reinstall the manifest's own bindings: {repaired}"
        );
    }

    /// The dedup check's own regression guard: row 10 is "the same caller
    /// retrying a lost response", not "any caller sending identical
    /// bytes". A *different*, authorized caller presenting byte-identical
    /// content must still take ownership -- `set_owner` runs
    /// unconditionally on every successful deploy (M04A B7a) -- rather
    /// than being silently skipped by the dedup no-op.
    #[tokio::test]
    async fn an_identical_redeploy_by_a_different_caller_still_transfers_ownership() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = node_wide_caller("did:key:zAlice");
        let bob = node_wide_caller("did:key:zBob");

        service
            .deploy("shared-svc".to_string(), inline_manifest(None, None, None), &alice)
            .await
            .unwrap();
        assert_eq!(service.registry.owner_of("shared-svc"), Some("did:key:zAlice".to_string()));

        service
            .deploy("shared-svc".to_string(), inline_manifest(None, None, None), &bob)
            .await
            .unwrap();
        assert_eq!(
            service.registry.owner_of("shared-svc"),
            Some("did:key:zBob".to_string()),
            "a different caller's byte-identical redeploy must still transfer ownership, not be \
             deduplicated as a no-op retry"
        );
    }

    /// The hash is written only on full deploy success -- a half-failed
    /// deploy must not be deduplicated on the next attempt.
    #[tokio::test]
    async fn a_half_failed_deploy_does_not_record_a_manifest_hash() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let owner = node_wide_caller("owner");

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Url("/does_not_exist.wasm".to_string()),
                hash: None,
                interfaces: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let result = service.deploy("half-failed-svc".to_string(), manifest, &owner).await;
        assert!(result.is_err());

        assert!(
            service.registry.deploy_facts("half-failed-svc").is_none(),
            "a deploy that fails before set_deploy_facts must not have recorded any deploy facts, \
             manifest_hash included"
        );
    }

    /// §0.27's other half: the management stamp records *who is writing*,
    /// not what was installed, so it must survive a deploy that fails
    /// after the generation gate -- unlike the bindings, it is not behind
    /// A2's defer-until-everything-succeeds rule.
    #[tokio::test]
    async fn a_deploy_that_fails_after_the_gate_still_recorded_its_writer() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let owner = node_wide_caller("owner");

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Url("/does_not_exist.wasm".to_string()),
                hash: None,
                interfaces: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let ctx = app_context("app-1", "frontend", vec![]);
        let result = service
            .deploy_with_context(
                "half-failed-svc".to_string(),
                manifest,
                Some(AppContext { generation: 3, ..ctx }),
                &owner,
            )
            .await;
        assert!(result.is_err());

        let management = service
            .registry
            .app_instance_management_of("app-1")
            .expect("the generation gate's persist must survive a later deploy failure");
        assert_eq!(management.generation, 3);
    }

    // ── M05A A5a: restart ─────────────────────────────────────────────────

    /// A5's remediation half of restart-in-place: evicting and recompiling
    /// a wasm component from the artifact the substrate already holds,
    /// with no redeploy and no identity work.
    #[tokio::test]
    async fn restart_reloads_a_wasm_component_from_disk() {
        let wasm_bytes = fs::read(greeter_wasm_path())
            .expect("greeter fixture must be built (see test-components/greeter's own build step)");
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let owner = node_wide_caller("owner");

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wasm_bytes),
                hash: None,
                interfaces: vec![GREETER_INTERFACE_NAME.to_string()],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("greeter-restart-svc".to_string(), manifest, &owner).await.unwrap();
        assert!(service.app_sandbox_engine.is_deployed("greeter-restart-svc"));

        service.app_sandbox_engine.stop_wasm("greeter-restart-svc").await.unwrap();
        assert!(!service.app_sandbox_engine.is_deployed("greeter-restart-svc"));

        service.restart("greeter-restart-svc".to_string(), 0, &owner).await.unwrap();
        assert!(
            service.app_sandbox_engine.is_deployed("greeter-restart-svc"),
            "restart must recompile the component from the artifact on disk"
        );
    }

    /// A `tcp` service's process runs outside this substrate -- restart
    /// must refuse it and say why, rather than silently succeeding, which
    /// a supervisor's remediation budget would count as a real attempt.
    #[tokio::test]
    async fn restart_refuses_a_tcp_service_naming_why() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let owner = node_wide_caller("owner");

        service
            .deploy("tcp-restart-svc".to_string(), inline_manifest(None, None, None), &owner)
            .await
            .unwrap();

        let err = service.restart("tcp-restart-svc".to_string(), 0, &owner).await.unwrap_err();
        assert!(err.contains("tcp") && err.contains("outside this substrate"), "{err}");
    }

    /// §0.23: `restart` is a lifecycle action and must be generation-gated
    /// exactly like `deploy`/`write-bindings` -- a superseded supervisor
    /// must not be able to restart a service it no longer manages.
    #[tokio::test]
    async fn restart_is_rejected_at_a_lower_generation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(AppContext { generation: 5, ..app_context("app-1", "frontend", vec![]) }),
                &caller,
            )
            .await
            .unwrap();

        let err = service.restart("frontend-svc".to_string(), 3, &caller).await.unwrap_err();
        assert!(err.contains("at generation 5"), "{err}");
    }

    /// M05A A5c §19.17: `restart` was the one lifecycle write with no
    /// service-owner check -- a scoped grantee for `service_id` could
    /// restart a service a *different* caller owns, which `deploy`/
    /// `undeploy`/`write-bindings` all already refuse as a takeover.
    #[tokio::test]
    async fn restart_is_refused_for_a_service_owned_by_another_caller() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = node_wide_caller("did:key:zAlice");

        service
            .deploy("owned-svc".to_string(), inline_manifest(None, None, None), &alice)
            .await
            .unwrap();
        assert_eq!(service.registry.owner_of("owned-svc"), Some("did:key:zAlice".to_string()));

        let bob = scoped_deploy_caller("did:key:zBob", "owned-svc");
        let err = service.restart("owned-svc".to_string(), 0, &bob).await.unwrap_err();
        assert!(err.contains("owned-svc") && err.contains("owned by"), "{err}");
    }

    /// The boundary of the check above: a node-wide `orchestrator/deploy`
    /// grantee -- the shape a supervisor holds (§0.28) -- restarts a
    /// service it does not own without being blocked by the new check.
    #[tokio::test]
    async fn restart_by_a_node_wide_deploy_grantee_ignores_the_service_owner() {
        let wasm_bytes = fs::read(greeter_wasm_path())
            .expect("greeter fixture must be built (see test-components/greeter's own build step)");
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = node_wide_caller("did:key:zAlice");

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wasm_bytes),
                hash: None,
                interfaces: vec![GREETER_INTERFACE_NAME.to_string()],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("owned-wasm-svc".to_string(), manifest, &alice).await.unwrap();
        assert_eq!(service.registry.owner_of("owned-wasm-svc"), Some("did:key:zAlice".to_string()));

        let bob = node_wide_caller("did:key:zBob");
        service.restart("owned-wasm-svc".to_string(), 0, &bob).await.unwrap();
    }

    // ── run-scheduled ─────────────────────────────────────────────────────

    /// Registers a local endpoint for `service_id`/`interface` -- the fact
    /// `run_scheduled` requires before it dispatches, since a target the
    /// endpoint registry does not know would be resolved through the
    /// community registry and called on another node.
    async fn register_local_endpoint(
        service: &ControlPlaneService,
        service_id: &str,
        interface: &str,
    ) {
        service
            .registry
            .register(
                service_id.to_string(),
                interface.to_string(),
                SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
            )
            .await
            .unwrap();
    }

    /// A schedule naming a service this node does not host must be refused
    /// outright, never handed to the proxy: `invoke_inner` would read the
    /// empty local lookup as "remote", resolve the name through the
    /// community registry, and dispatch under this node's own key -- with
    /// the owner and generation checks both skipped, since neither can see
    /// a service the node knows nothing about.
    #[tokio::test]
    async fn run_scheduled_refuses_a_target_with_no_local_endpoint() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");
        let proxy = Arc::new(RecordingProxy::default());
        wire_service_proxy(&service, &proxy);

        let err = service
            .run_scheduled(
                "elsewhere-svc".to_string(),
                0,
                "scheduled-driver".to_string(),
                "tick".to_string(),
                None,
                &caller,
            )
            .await
            .unwrap_err();
        assert!(err.contains("no local endpoint"), "{err}");
        assert!(
            proxy.last_request.lock().unwrap().is_none(),
            "the proxy must not be reached for a target this node does not host"
        );
    }

    /// The same refusal for the everyday mistake: the service is deployed
    /// here, but the schedule names an interface it does not export.
    #[tokio::test]
    async fn run_scheduled_refuses_an_interface_the_local_service_does_not_export() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");
        let proxy = Arc::new(RecordingProxy::default());
        wire_service_proxy(&service, &proxy);
        register_local_endpoint(&service, "worker-svc", "some-other-interface").await;

        let err = service
            .run_scheduled(
                "worker-svc".to_string(),
                0,
                "scheduled-driver".to_string(),
                "tick".to_string(),
                None,
                &caller,
            )
            .await
            .unwrap_err();
        assert!(err.contains("scheduled-driver"), "{err}");
        assert!(proxy.last_request.lock().unwrap().is_none());
    }

    /// `run-scheduled` takes exactly `restart`'s gate -- a caller
    /// with no `orchestrator/deploy` grant is refused before the proxy is
    /// ever touched.
    #[tokio::test]
    async fn run_scheduled_is_refused_without_an_orchestrator_deploy_grant() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let no_grant = CallerContext::service_system("no-grant-caller");

        let err = service
            .run_scheduled(
                "unscheduled-svc".to_string(),
                0,
                "scheduled-driver".to_string(),
                "tick".to_string(),
                None,
                &no_grant,
            )
            .await
            .unwrap_err();
        assert!(err.contains("orchestrator/deploy"), "{err}");
    }

    /// The owner check `restart_impl` carries, applied identically here:
    /// a scoped grantee for `service_id`
    /// must not run a scheduled task on a service a *different* caller
    /// owns.
    #[tokio::test]
    async fn run_scheduled_is_refused_for_a_service_another_caller_owns() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let alice = node_wide_caller("did:key:zAlice");

        service
            .deploy("owned-svc".to_string(), inline_manifest(None, None, None), &alice)
            .await
            .unwrap();

        let bob = scoped_deploy_caller("did:key:zBob", "owned-svc");
        let err = service
            .run_scheduled(
                "owned-svc".to_string(),
                0,
                "scheduled-driver".to_string(),
                "tick".to_string(),
                None,
                &bob,
            )
            .await
            .unwrap_err();
        assert!(err.contains("owned-svc") && err.contains("owned by"), "{err}");
    }

    /// `generation` follows `restart`'s rule: gated only where an
    /// app instance exists, so a superseded supervisor cannot keep firing
    /// ticks at an instance another one now manages.
    #[tokio::test]
    async fn run_scheduled_is_refused_at_a_stale_generation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(AppContext { generation: 5, ..app_context("app-1", "frontend", vec![]) }),
                &caller,
            )
            .await
            .unwrap();

        let err = service
            .run_scheduled(
                "frontend-svc".to_string(),
                3,
                "scheduled-driver".to_string(),
                "tick".to_string(),
                None,
                &caller,
            )
            .await
            .unwrap_err();
        assert!(err.contains("at generation 5"), "{err}");
    }

    /// The whole of §0.1's authorization argument: the target observes
    /// `CallerContext::service_system(service_id)` -- the service acting as
    /// itself -- not the supervisor's own identity, and the call travels as
    /// `CallOrigin::Native` with the dispatching service named.
    #[tokio::test]
    async fn run_scheduled_dispatches_the_named_method_as_the_service_itself() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");
        let proxy = Arc::new(RecordingProxy::default());
        wire_service_proxy(&service, &proxy);
        register_local_endpoint(&service, "worker-svc", "scheduled-driver").await;
        *proxy.response.lock().unwrap() = Some(Ok(Value::Null));

        service
            .run_scheduled(
                "worker-svc".to_string(),
                0,
                "scheduled-driver".to_string(),
                "tick".to_string(),
                Some(r#"["arg"]"#.to_string()),
                &caller,
            )
            .await
            .unwrap();

        let req = proxy.last_request.lock().unwrap().take().expect("proxy was not invoked");
        assert_eq!(req.target_service, "worker-svc");
        assert_eq!(req.interface, "scheduled-driver");
        assert_eq!(req.method, "tick");
        assert_eq!(req.params, serde_json::json!(["arg"]));
        assert_eq!(req.caller.caller_did, "system:worker-svc");
        assert_eq!(req.origin, CallOrigin::Native { service_id: Some("worker-svc".to_string()) });
        assert!(!req.idempotent);
        assert_eq!(req.idempotency_key, None);
    }

    /// §0.10 bullet 2: absent `params-json` sends an empty positional array,
    /// not `Value::Null` -- the shape the one existing in-tree caller of a
    /// no-argument guest method (the `rpc` readiness probe) sends.
    #[tokio::test]
    async fn run_scheduled_passes_absent_params_as_an_empty_positional_array() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");
        let proxy = Arc::new(RecordingProxy::default());
        wire_service_proxy(&service, &proxy);
        register_local_endpoint(&service, "worker-svc", "scheduled-driver").await;
        *proxy.response.lock().unwrap() = Some(Ok(Value::Null));

        service
            .run_scheduled(
                "worker-svc".to_string(),
                0,
                "scheduled-driver".to_string(),
                "tick".to_string(),
                None,
                &caller,
            )
            .await
            .unwrap();

        let req = proxy.last_request.lock().unwrap().take().expect("proxy was not invoked");
        assert_eq!(req.params, Value::Array(vec![]));
    }

    /// A hand-edited or malformed `params-json` is refused before ever
    /// reaching the proxy, with a message naming the field.
    #[tokio::test]
    async fn run_scheduled_refuses_params_json_that_is_not_json() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");
        let proxy = Arc::new(RecordingProxy::default());
        wire_service_proxy(&service, &proxy);

        let err = service
            .run_scheduled(
                "worker-svc".to_string(),
                0,
                "scheduled-driver".to_string(),
                "tick".to_string(),
                Some("not json".to_string()),
                &caller,
            )
            .await
            .unwrap_err();
        assert!(err.contains("params-json is not JSON"), "{err}");
        assert!(proxy.last_request.lock().unwrap().is_none(), "the proxy must not be reached");
    }

    /// The callee's own error surfaces to the caller rather than being
    /// swallowed -- the direct statement that a scheduled run's failure is
    /// visible, which the alert this slice raises depends on.
    #[tokio::test]
    async fn run_scheduled_reports_a_callee_error_rather_than_swallowing_it() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");
        let proxy = Arc::new(RecordingProxy::default());
        wire_service_proxy(&service, &proxy);
        register_local_endpoint(&service, "worker-svc", "scheduled-driver").await;
        *proxy.response.lock().unwrap() = Some(Err(ProxyError::Callee {
            code: -32010,
            message: "guest refused".to_string(),
            data: None,
        }));

        let err = service
            .run_scheduled(
                "worker-svc".to_string(),
                0,
                "scheduled-driver".to_string(),
                "tick".to_string(),
                None,
                &caller,
            )
            .await
            .unwrap_err();
        assert!(err.contains("guest refused"), "{err}");
    }

    /// §0.23 / matrix row 14's blast-radius half at the substrate level: a
    /// superseded supervisor must not be able to undeploy -- the most
    /// destructive lifecycle action there is -- a service it no longer
    /// manages.
    #[tokio::test]
    async fn undeploy_is_rejected_at_a_lower_generation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(AppContext { generation: 5, ..app_context("app-1", "frontend", vec![]) }),
                &caller,
            )
            .await
            .unwrap();

        let err = service.undeploy("frontend-svc".to_string(), 3, &caller).await.unwrap_err();
        assert!(err.contains("at generation 5"), "{err}");
    }

    /// Finding 04 (post-review fix): the app-context/binding write is
    /// deferred until every fallible step earlier in the deploy has
    /// succeeded (`install_app_context`, called near owner attribution),
    /// so a redeploy whose `app_context.service_name` fails validation must
    /// leave the previous deploy's bindings completely untouched -- not
    /// removed-then-never-replaced, which is what happened when the same
    /// removal ran before validation.
    #[tokio::test]
    async fn a_redeploy_with_an_invalid_service_name_preserves_the_previous_bindings() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("test-caller");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();

        // `LogicalServiceName::try_new` rejects a name containing '/'.
        let err = service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context("app-1", "bad/name", vec![])),
                &caller,
            )
            .await
            .unwrap_err();
        assert!(err.contains("invalid service name"), "{err}");

        assert_eq!(
            service.registry.app_context_of("frontend-svc"),
            Some(("app-1".to_string(), "frontend".to_string())),
            "the failed redeploy must not have removed the previous app context"
        );
        assert_eq!(
            service.registry.all_bindings().await.unwrap().len(),
            1,
            "the failed redeploy must not have removed the previous binding row"
        );
    }

    #[tokio::test]
    async fn a_binding_naming_a_non_did_key_member_fails_the_deploy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let ctx = app_context(
            "app-1",
            "frontend",
            vec![dependency_binding("backend", vec!["not-a-did"])],
        );
        let err = service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(ctx),
                &node_wide_caller("test-caller"),
            )
            .await
            .unwrap_err();
        assert!(err.contains("invalid member DID"), "{err}");
    }

    #[tokio::test]
    async fn undeploy_clears_the_persisted_binding_rows() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("test-caller");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();

        service.undeploy("frontend-svc".to_string(), 0, &caller).await.unwrap();

        assert_eq!(service.registry.app_context_of("frontend-svc"), None);
        assert!(service.registry.all_bindings().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_dependency_name_containing_a_slash_fails_the_deploy_rather_than_panicking() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let ctx = app_context(
            "app-1",
            "frontend",
            vec![dependency_binding("bad/name", vec!["did:key:zBackendMember"])],
        );
        let err = service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(ctx),
                &node_wide_caller("test-caller"),
            )
            .await
            .unwrap_err();
        assert!(err.contains("invalid dependency name"), "{err}");
    }

    #[tokio::test]
    async fn an_empty_dependency_name_fails_the_deploy_rather_than_panicking() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let ctx = app_context(
            "app-1",
            "frontend",
            vec![dependency_binding("", vec!["did:key:zBackendMember"])],
        );
        let err = service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(ctx),
                &node_wide_caller("test-caller"),
            )
            .await
            .unwrap_err();
        assert!(err.contains("invalid dependency name"), "{err}");
    }

    #[tokio::test]
    async fn an_empty_app_instance_id_in_the_app_context_fails_the_deploy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let ctx = app_context("", "frontend", vec![]);
        let err = service
            .deploy_with_context(
                "frontend-svc".to_string(),
                inline_manifest(None, None, None),
                Some(ctx),
                &node_wide_caller("test-caller"),
            )
            .await
            .unwrap_err();
        assert!(err.contains("invalid app instance id"), "{err}");
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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

    /// Shared harness for the saga compensation deploy-gate tests below --
    /// the same construction every other test in this module repeats
    /// inline, factored here only because this group needs it six times in
    /// a row.
    async fn saga_gate_test_service(
        temp_dir: &std::path::Path,
    ) -> (ControlPlaneService, Arc<SqliteStorageProvider>) {
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
                syneroym_app_orchestration::empty_resolver(),
            )
            .await
            .unwrap(),
        );
        let container_engine = Arc::new(ContainerEngine::new("podman".to_string(), temp_dir, None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));
        let native_dispatch = NativeDispatchRegistry::default();
        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            container_engine,
            registry,
            temp_dir.to_path_buf(),
            key_store,
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();
        (service, storage_provider)
    }

    fn saga_gate_manifest(wat: &str, interfaces: Vec<String>) -> DeployManifest {
        DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wat.as_bytes().to_vec()),
                hash: None,
                interfaces,
            }),
            registry_certificate: None,
            instance_certificate: None,
        }
    }

    /// Exports `saga-undo-reserve` with no `reserve` beside it -- the
    /// defect the deploy gate exists to catch.
    const WASM_UNDO_WITH_NO_FORWARD: &str = r#"
(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface (export "saga-undo-reserve" (func $noop)))
  (export "test-interface" (instance $interface))
)
"#;

    /// Exports both `reserve` and its compensation `saga-undo-reserve`.
    const WASM_UNDO_WITH_FORWARD: &str = r#"
(component
  (core module $m
    (func (export "noop_a"))
    (func (export "noop_b")))
  (core instance $i (instantiate $m))
  (func $a (canon lift (core func $i "noop_a")))
  (func $b (canon lift (core func $i "noop_b")))
  (instance $interface
    (export "reserve" (func $a))
    (export "saga-undo-reserve" (func $b)))
  (export "test-interface" (instance $interface))
)
"#;

    /// Exports a plain `undo-last-update` -- an ordinary business verb, not
    /// a saga compensation (`undo-` is not the reserved prefix).
    const WASM_PLAIN_UNDO_PREFIX: &str = r#"
(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface (export "undo-last-update" (func $noop)))
  (export "test-interface" (instance $interface))
)
"#;

    #[tokio::test]
    async fn deploy_refuses_a_component_whose_compensation_has_no_forward_operation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let (service, _storage) = saga_gate_test_service(temp_dir.path()).await;
        let manifest =
            saga_gate_manifest(WASM_UNDO_WITH_NO_FORWARD, vec!["test-interface".to_string()]);
        let result = service
            .deploy("saga_missing_forward_svc".to_string(), manifest, &node_wide_caller("test"))
            .await;
        assert!(result.is_err(), "a saga-undo- export with no forward operation must fail deploy");
        let err = result.unwrap_err();
        assert!(
            err.contains("saga-undo-reserve") && err.contains("no 'reserve' beside it"),
            "expected the saga compensation gate error, got: {err}"
        );
    }

    #[tokio::test]
    async fn deploy_accepts_a_component_exporting_both_halves() {
        let temp_dir = tempfile::tempdir().unwrap();
        let (service, _storage) = saga_gate_test_service(temp_dir.path()).await;
        let manifest =
            saga_gate_manifest(WASM_UNDO_WITH_FORWARD, vec!["test-interface".to_string()]);
        let result = service
            .deploy("saga_both_halves_svc".to_string(), manifest, &node_wide_caller("test"))
            .await;
        assert!(result.is_ok(), "both halves present must deploy cleanly: {result:?}");
    }

    /// The false-refusal the reserved `saga-undo-` prefix exists to
    /// prevent: `undo-last-update` is a legal business verb with no
    /// `last-update` beside it, and must not be refused.
    #[tokio::test]
    async fn deploy_accepts_a_component_exporting_a_plain_undo_prefixed_function() {
        let temp_dir = tempfile::tempdir().unwrap();
        let (service, _storage) = saga_gate_test_service(temp_dir.path()).await;
        let manifest =
            saga_gate_manifest(WASM_PLAIN_UNDO_PREFIX, vec!["test-interface".to_string()]);
        let result = service
            .deploy("saga_plain_undo_svc".to_string(), manifest, &node_wide_caller("test"))
            .await;
        assert!(result.is_ok(), "a plain undo- business verb must not be refused: {result:?}");
    }

    /// The common case: a service with no compensations at all deploys
    /// cleanly, with no declaration required anywhere.
    #[tokio::test]
    async fn deploy_accepts_a_component_with_no_compensations_at_all() {
        let temp_dir = tempfile::tempdir().unwrap();
        let (service, _storage) = saga_gate_test_service(temp_dir.path()).await;
        let manifest = saga_gate_manifest(
            WASM_WITHOUT_AUTHORIZE_ROWS_EXPORT,
            vec!["test-interface".to_string()],
        );
        let result = service
            .deploy("saga_no_compensations_svc".to_string(), manifest, &node_wide_caller("test"))
            .await;
        assert!(
            result.is_ok(),
            "a component with no compensations must deploy cleanly: {result:?}"
        );
    }

    #[tokio::test]
    async fn a_refused_compensation_pairing_rolls_back_the_config_generation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let (service, storage) = saga_gate_test_service(temp_dir.path()).await;
        let manifest =
            saga_gate_manifest(WASM_UNDO_WITH_NO_FORWARD, vec!["test-interface".to_string()]);
        let result = service
            .deploy("saga_rollback_svc".to_string(), manifest, &node_wide_caller("test"))
            .await;
        assert!(result.is_err());
        assert!(
            storage.get_latest_config_generation("saga_rollback_svc").await.unwrap().is_none(),
            "a refused saga compensation pairing must roll back the config generation it wrote \
             before validating exports"
        );
    }

    /// A known limit, pinned so the behavior is a choice and not an
    /// accident: `exported_functions` returns `None` for an interface the
    /// manifest never declared, which turns a declared-but-absent
    /// compensation pairing into a silent pass rather than a refusal.
    /// Backlog: "A declared interface that is not a component export is not
    /// refused at deploy".
    #[tokio::test]
    async fn a_compensation_on_an_interface_the_manifest_does_not_declare_is_not_examined() {
        let temp_dir = tempfile::tempdir().unwrap();
        let (service, _storage) = saga_gate_test_service(temp_dir.path()).await;
        // The component exports `saga-undo-reserve` with no `reserve`, but
        // the manifest's own `interfaces` list never names `test-interface`
        // -- so the gate's loop (which walks only declared interfaces)
        // never looks at it.
        let manifest = saga_gate_manifest(WASM_UNDO_WITH_NO_FORWARD, vec![]);
        let result = service
            .deploy("saga_undeclared_iface_svc".to_string(), manifest, &node_wide_caller("test"))
            .await;
        assert!(
            result.is_ok(),
            "a compensation on an undeclared interface is not examined by this gate: {result:?}"
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let caller = node_wide_caller("test-caller");
        service.deploy("undeploy_fdae_svc".to_string(), manifest, &caller).await.unwrap();
        let _ = fs::remove_file(&policy_filename);
        assert!(storage_provider.load_fdae_policy("undeploy_fdae_svc").await.unwrap().is_some());

        service.undeploy("undeploy_fdae_svc".to_string(), 0, &caller).await.unwrap();
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                health_check: None,
                assets: None,
                visibility: None,
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
        async fn load_all_deploy_facts(
            &self,
        ) -> Result<Vec<(String, String, Option<String>, Option<String>, Option<String>)>> {
            self.inner.load_all_deploy_facts().await
        }
        async fn save_deploy_facts(
            &self,
            service_id: &str,
            service_type: &str,
            health_check_json: Option<&str>,
            manifest_hash: Option<&str>,
            visibility: Option<&str>,
        ) -> Result<()> {
            self.inner
                .save_deploy_facts(
                    service_id,
                    service_type,
                    health_check_json,
                    manifest_hash,
                    visibility,
                )
                .await
        }
        async fn remove_deploy_facts(&self, service_id: &str) -> Result<()> {
            self.inner.remove_deploy_facts(service_id).await
        }
        async fn load_all_app_contexts(&self) -> Result<Vec<(String, String, String)>> {
            self.inner.load_all_app_contexts().await
        }
        async fn save_app_context(
            &self,
            service_id: &str,
            app_instance_id: &str,
            service_name: &str,
        ) -> Result<()> {
            self.inner.save_app_context(service_id, app_instance_id, service_name).await
        }
        async fn remove_app_context(&self, service_id: &str) -> Result<()> {
            self.inner.remove_app_context(service_id).await
        }
        async fn load_all_bindings(&self) -> Result<Vec<(String, String, String, String)>> {
            self.inner.load_all_bindings().await
        }
        async fn save_binding(
            &self,
            service_id: &str,
            app_instance_id: &str,
            dependency_name: &str,
            topology_entry_json: &str,
        ) -> Result<()> {
            self.inner
                .save_binding(service_id, app_instance_id, dependency_name, topology_entry_json)
                .await
        }
        async fn load_binding(
            &self,
            service_id: &str,
            dependency_name: &str,
        ) -> Result<Option<String>> {
            self.inner.load_binding(service_id, dependency_name).await
        }
        async fn load_bindings_for(&self, service_id: &str) -> Result<Vec<(String, String)>> {
            self.inner.load_bindings_for(service_id).await
        }
        async fn load_all_app_instance_management(
            &self,
        ) -> Result<Vec<(String, AppInstanceManagement)>> {
            self.inner.load_all_app_instance_management().await
        }
        async fn save_app_instance_management(
            &self,
            app_instance_id: &str,
            management: &AppInstanceManagement,
        ) -> Result<()> {
            self.inner.save_app_instance_management(app_instance_id, management).await
        }
        async fn remove_app_instance_management(&self, app_instance_id: &str) -> Result<()> {
            self.inner.remove_app_instance_management(app_instance_id).await
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                    health_check: None,
                    assets: None,
                    visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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

        service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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

    /// A real, trivial WASM component's WAT text, encoded as bytes -- the
    /// same one `test_deploy_failure_after_successful_wasm_compile_rolls_back_gen_and_policy`
    /// uses, so `deploy_wasm_service` itself succeeds without needing the
    /// `greeter`/`proxy-test` test-fixture artifacts (which may not be
    /// built) for tests that only need *a* valid component, not any
    /// particular one -- e.g. an asset-bundle test, where the component
    /// itself is incidental to what's under test.
    fn minimal_wasm_component() -> Vec<u8> {
        r#"
(component
  (core module $m (func (export "noop")))
  (core instance $i (instantiate $m))
  (func $noop (canon lift (core func $i "noop")))
  (instance $interface (export "greet" (func $noop)))
  (export "test-interface" (instance $interface))
)
"#
        .as_bytes()
        .to_vec()
    }

    /// A minimal gzip-compressed tar archive, one entry per `(path, bytes)`
    /// pair -- the same shape `syneroym_control_plane::assets`' own unit
    /// tests build, duplicated here rather than exposed as a `pub`
    /// test-only helper across the module boundary.
    fn make_asset_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut builder = tar::Builder::new(Vec::new());
        for (path, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, *data).unwrap();
        }
        let tar_bytes = builder.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// M06A A1: a deploy declaring `assets` unpacks them into blobs,
    /// registers a `ServiceAssets` entry the router can serve from, and
    /// undeploy removes both the registry entry and the underlying blobs.
    #[tokio::test]
    async fn test_asset_bundle_populated_on_deploy_and_cleared_on_undeploy() {
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
                syneroym_app_orchestration::empty_resolver(),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let asset_registry: AssetRegistry = Arc::new(DashMap::new());
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
            messaging_broker,
            native_dispatch,
            http_routes,
            asset_registry.clone(),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        let service_id = "asset-svc".to_string();
        let archive = make_asset_archive(&[("index.html", b"<html>hi</html>")]);
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: Some(WitAssetBundle {
                    archive: ArtifactSource::Binary(archive),
                    hash: None,
                    visibility: Some(WitVisibility::Public),
                }),
                visibility: None,
            },
            // An asset bundle is only servable for a `Wasm` service (a
            // `Tcp`/`Container` endpoint is raw passthrough and never
            // reaches the asset-serving HTTP path) -- `minimal_wasm_component`
            // is a real, trivial component so `deploy_wasm_service` itself
            // succeeds.
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(minimal_wasm_component()),
                hash: None,
                interfaces: vec!["asset-svc-interface".to_string()],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let caller = node_wide_caller("test-caller");
        service.deploy(service_id.clone(), manifest, &caller).await.unwrap();

        let entry = asset_registry.get(&service_id).expect("asset bundle populated on deploy");
        assert!(entry.public);
        assert_eq!(entry.manifest.entries.len(), 1);
        let asset = entry.manifest.entries.get("/index.html").unwrap();
        let stored = blob_provider.get_blob(&service_id, &asset.hash, None).await.unwrap();
        assert_eq!(stored, b"<html>hi</html>");
        let manifest_hash = entry.manifest_hash.clone();
        drop(entry);

        service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
        assert!(
            asset_registry.get(&service_id).is_none(),
            "asset registry entry must be removed on undeploy"
        );
        assert!(
            blob_provider.get_blob(&service_id, &manifest_hash, None).await.is_err(),
            "the manifest blob itself must be deleted on undeploy"
        );
    }

    /// M06A A1/D-A1-9: a redeploy that changes only some files keeps every
    /// blob the new manifest still shares with the old one, and deletes
    /// only what genuinely dropped out.
    #[tokio::test]
    async fn test_asset_bundle_redeploy_keeps_shared_blobs_and_drops_removed_ones() {
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
                syneroym_app_orchestration::empty_resolver(),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let asset_registry: AssetRegistry = Arc::new(DashMap::new());
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
            messaging_broker,
            native_dispatch,
            http_routes,
            asset_registry.clone(),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        let service_id = "asset-redeploy-svc".to_string();
        let caller = node_wide_caller("test-caller");

        let bundle = |archive: Vec<u8>| DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: Some(WitAssetBundle {
                    archive: ArtifactSource::Binary(archive),
                    hash: None,
                    visibility: Some(WitVisibility::Public),
                }),
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(minimal_wasm_component()),
                hash: None,
                interfaces: vec!["asset-redeploy-svc-interface".to_string()],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };

        let first_archive =
            make_asset_archive(&[("shared.txt", b"unchanged"), ("old_only.txt", b"gone soon")]);
        service.deploy(service_id.clone(), bundle(first_archive), &caller).await.unwrap();
        let first_hashes = {
            let entry = asset_registry.get(&service_id).unwrap();
            (
                entry.manifest.entries.get("/shared.txt").unwrap().hash.clone(),
                entry.manifest.entries.get("/old_only.txt").unwrap().hash.clone(),
            )
        };
        let (shared_hash, old_only_hash) = first_hashes;

        let second_archive = make_asset_archive(&[("shared.txt", b"unchanged")]);
        service.deploy(service_id.clone(), bundle(second_archive), &caller).await.unwrap();

        assert!(
            blob_provider.get_blob(&service_id, &shared_hash, None).await.is_ok(),
            "unchanged file's blob must survive the redeploy"
        );
        assert!(
            blob_provider.get_blob(&service_id, &old_only_hash, None).await.is_err(),
            "the dropped file's blob must be garbage-collected"
        );

        service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
    }

    /// M06A A1 (D-A1-9): the backward asset rollback, driven through a real
    /// deploy failure rather than `delete_hashes` called directly as pure
    /// set arithmetic. `rollback_asset_bundle` is reached from five
    /// separate failure branches in `deploy_with_context`; this exercises
    /// the one already covered for FDAE-policy/config-generation rollback
    /// by `test_deploy_failure_after_successful_wasm_compile_rolls_back_gen_and_policy`
    /// (same `FailingEndpointStorage` fixture and `minimal_wasm_component`,
    /// same failure point -- `register_wasm_endpoints`, which runs after
    /// the asset block has already written the new generation's blobs, and
    /// after the component itself compiled successfully), but proves the
    /// asset half: the failed redeploy's own writes are gone, *and* every
    /// blob the still-live previous generation references survives.
    /// `Wasm`, not `Tcp`, since an asset bundle is only accepted for a
    /// `Wasm` service as of the same review pass that added this test.
    #[tokio::test]
    async fn test_asset_bundle_rollback_on_a_real_deploy_failure_keeps_the_old_generation() {
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
                syneroym_app_orchestration::empty_resolver(),
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
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let asset_registry: AssetRegistry = Arc::new(DashMap::new());
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
            messaging_broker,
            native_dispatch,
            http_routes,
            asset_registry.clone(),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        let service_id = "asset-wasm-rollback-svc".to_string();
        let caller = node_wide_caller("test-caller");

        let manifest = |archive: Vec<u8>, interface_name: &str| DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: Some(WitAssetBundle {
                    archive: ArtifactSource::Binary(archive),
                    hash: None,
                    visibility: Some(WitVisibility::Public),
                }),
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(minimal_wasm_component()),
                hash: None,
                interfaces: vec![interface_name.to_string()],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };

        // First deploy succeeds: generation 0's asset bundle is live.
        let first_archive = make_asset_archive(&[("index.html", b"gen0")]);
        service
            .deploy(service_id.clone(), manifest(first_archive, "safe-interface"), &caller)
            .await
            .unwrap();
        let (gen0_manifest_hash, gen0_asset_hash) = {
            let entry = asset_registry.get(&service_id).unwrap();
            (
                entry.manifest_hash.clone(),
                entry.manifest.entries.get("/index.html").unwrap().hash.clone(),
            )
        };

        // Redeploy with a *different* asset bundle (different content, so a
        // different blob hash) plus an interface name the registry is
        // rigged to reject -- the asset block runs and writes gen 1's blob
        // successfully, the component itself compiles fine, and then
        // `register_wasm_endpoints` fails, well after the asset write.
        let second_archive = make_asset_archive(&[("index.html", b"gen1 -- must not survive")]);
        let result = service
            .deploy(service_id.clone(), manifest(second_archive, "fails-to-register"), &caller)
            .await;
        assert!(result.is_err(), "endpoint registration must fail: {result:?}");
        assert!(result.unwrap_err().contains("Endpoint registration failed"));

        let entry = asset_registry.get(&service_id).expect(
            "a failed redeploy must leave the still-live generation 0 asset registry entry in \
             place",
        );
        assert_eq!(
            entry.manifest_hash, gen0_manifest_hash,
            "the registry must still point at generation 0's manifest, not a half-applied \
             generation 1"
        );
        let gen0_entry = entry.manifest.entries.get("/index.html").unwrap();
        assert_eq!(gen0_entry.hash, gen0_asset_hash);
        drop(entry);

        assert_eq!(
            blob_provider.get_blob(&service_id, &gen0_asset_hash, None).await.unwrap(),
            b"gen0",
            "generation 0's blob, still referenced by the live manifest, must survive the failed \
             redeploy's rollback"
        );
        assert!(
            blob_provider.get_blob(&service_id, &gen0_manifest_hash, None).await.is_ok(),
            "generation 0's manifest blob must survive too"
        );

        // The failed generation 1's own write must be gone -- find it by
        // hashing the content directly, since nothing in the live manifest
        // references it to look it up by.
        let gen1_hash = hex::encode(Sha256::digest(b"gen1 -- must not survive"));
        assert!(
            blob_provider.get_blob(&service_id, &gen1_hash, None).await.is_err(),
            "the failed redeploy's own blob write must have been rolled back, not orphaned \
             alongside generation 0"
        );

        service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
    }

    /// M06A A1: an asset bundle is only reachable through a `Wasm`
    /// service's HTTP path -- a `Tcp`/`Container` endpoint is registered as
    /// `SubstrateEndpoint::TcpHostPort`, which the router's `dispatch.rs`
    /// unconditionally routes to raw passthrough regardless of what the
    /// client actually sends, so an asset bundle attached to one is
    /// silently unreachable dead data. Rejected at deploy instead, before
    /// the asset block (or anything else fallible) has run -- asserted by
    /// checking nothing was written, not just that the call errored.
    #[tokio::test]
    async fn test_asset_bundle_is_rejected_for_a_tcp_service() {
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
                syneroym_app_orchestration::empty_resolver(),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let asset_registry: AssetRegistry = Arc::new(DashMap::new());
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
            messaging_broker,
            native_dispatch,
            http_routes,
            asset_registry.clone(),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        let service_id = "tcp-with-assets-svc".to_string();
        let archive = make_asset_archive(&[("index.html", b"unreachable")]);
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: Some(WitAssetBundle {
                    archive: ArtifactSource::Binary(archive),
                    hash: None,
                    visibility: Some(WitVisibility::Public),
                }),
                visibility: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let caller = node_wide_caller("test-caller");
        let err = service
            .deploy(service_id.clone(), manifest, &caller)
            .await
            .expect_err("a Tcp service must not accept an asset bundle");
        assert!(err.contains("only servable for a 'Wasm' service"), "{err}");
        assert!(
            asset_registry.get(&service_id).is_none(),
            "rejected at validation, before the asset registry is ever touched"
        );
    }

    /// Same as `test_asset_bundle_is_rejected_for_a_tcp_service`, for a
    /// `Container` service -- `deploy_container_service` also registers a
    /// `SubstrateEndpoint::TcpHostPort` (`crates/control_plane/src/service/
    /// orchestration.rs`'s own `deploy_container_service`), so it is raw
    /// passthrough for the identical reason.
    #[tokio::test]
    async fn test_asset_bundle_is_rejected_for_a_container_service() {
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
                syneroym_app_orchestration::empty_resolver(),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let asset_registry: AssetRegistry = Arc::new(DashMap::new());
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
            messaging_broker,
            native_dispatch,
            http_routes,
            asset_registry.clone(),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        let service_id = "container-with-assets-svc".to_string();
        let archive = make_asset_archive(&[("index.html", b"unreachable")]);
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: Some(WitAssetBundle {
                    archive: ArtifactSource::Binary(archive),
                    hash: None,
                    visibility: Some(WitVisibility::Public),
                }),
                visibility: None,
            },
            service_type: WitServiceType::Container(ContainerManifest {
                source: ArtifactSource::Url("docker.io/library/nginx:1.27".to_string()),
                hash: None,
                image: "docker.io/library/nginx:1.27".to_string(),
                ports: vec![],
                volumes: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let caller = node_wide_caller("test-caller");
        let err = service
            .deploy(service_id.clone(), manifest, &caller)
            .await
            .expect_err("a Container service must not accept an asset bundle");
        assert!(err.contains("only servable for a 'Wasm' service"), "{err}");
        assert!(asset_registry.get(&service_id).is_none());
    }

    fn guest_route_custom_config() -> String {
        serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/echo", "target": "guest", "operation": "handle-request"}
            ]
        })
        .to_string()
    }

    /// M06A D-A2-10a: same reasoning as
    /// `test_asset_bundle_is_rejected_for_a_tcp_service` -- a `Tcp`
    /// service's endpoint is raw passthrough, so a declared `guest` route
    /// would be silent dead configuration.
    #[tokio::test]
    async fn test_guest_route_is_rejected_for_a_tcp_service() {
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
                syneroym_app_orchestration::empty_resolver(),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let asset_registry: AssetRegistry = Arc::new(DashMap::new());
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
            messaging_broker,
            native_dispatch,
            http_routes.clone(),
            asset_registry,
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        let service_id = "tcp-with-guest-route-svc".to_string();
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: Some(guest_route_custom_config()),
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest { endpoints: vec![] }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let caller = node_wide_caller("test-caller");
        let err = service
            .deploy(service_id.clone(), manifest, &caller)
            .await
            .expect_err("a Tcp service must not accept a guest route");
        assert!(err.contains("is only servable for a 'Wasm' service"), "{err}");
        assert!(
            storage_provider.get_latest_config_generation(&service_id).await.unwrap().is_none(),
            "rejected before anything fallible runs -- no config generation saved"
        );
        assert!(http_routes.get(&service_id).is_none());
    }

    /// Same as `test_guest_route_is_rejected_for_a_tcp_service`, for a
    /// `Container` service.
    #[tokio::test]
    async fn test_guest_route_is_rejected_for_a_container_service() {
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
                syneroym_app_orchestration::empty_resolver(),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let asset_registry: AssetRegistry = Arc::new(DashMap::new());
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
            messaging_broker,
            native_dispatch,
            http_routes.clone(),
            asset_registry,
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        let service_id = "container-with-guest-route-svc".to_string();
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: Some(guest_route_custom_config()),
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Container(ContainerManifest {
                source: ArtifactSource::Url("docker.io/library/nginx:1.27".to_string()),
                hash: None,
                image: "docker.io/library/nginx:1.27".to_string(),
                ports: vec![],
                volumes: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        let caller = node_wide_caller("test-caller");
        let err = service
            .deploy(service_id.clone(), manifest, &caller)
            .await
            .expect_err("a Container service must not accept a guest route");
        assert!(err.contains("is only servable for a 'Wasm' service"), "{err}");
        assert!(http_routes.get(&service_id).is_none());
    }

    /// M06A D-A2-10b: a declared `guest` route whose compiled component
    /// does not export `handle-request` must fail the deploy -- rolling
    /// back the config generation, the FDAE policy, and any asset bundle
    /// already written, exactly as `test_stage4_policy_without_the_export_
    /// fails_deploy` does for the stage-4 export gate.
    #[tokio::test]
    async fn test_guest_route_without_the_export_fails_deploy_and_rolls_back() {
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
                syneroym_app_orchestration::empty_resolver(),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let asset_registry: AssetRegistry = Arc::new(DashMap::new());
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
            messaging_broker,
            native_dispatch,
            http_routes.clone(),
            asset_registry.clone(),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        let service_id = "guest-route-missing-export-svc".to_string();
        let archive = make_asset_archive(&[("index.html", b"hi")]);
        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: Some(guest_route_custom_config()),
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: None,
                assets: Some(WitAssetBundle {
                    archive: ArtifactSource::Binary(archive),
                    hash: None,
                    visibility: Some(WitVisibility::Public),
                }),
                visibility: None,
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
        let caller = node_wide_caller("test-caller");
        let err = service.deploy(service_id.clone(), manifest, &caller).await.expect_err(
            "a component without the handler export must not deploy with a guest route",
        );
        assert!(
            err.contains("target=guest") && err.contains("does not export"),
            "expected the D-A2-10b error, got: {err}"
        );
        assert!(
            storage_provider.get_latest_config_generation(&service_id).await.unwrap().is_none(),
            "config generation must be rolled back"
        );
        assert!(
            storage_provider.load_fdae_policy(&service_id).await.unwrap().is_none(),
            "fdae policy must be rolled back"
        );
        assert!(asset_registry.get(&service_id).is_none(), "asset bundle must be rolled back");
    }

    #[tokio::test]
    async fn test_websocket_route_without_the_export_fails_deploy_and_rolls_back() {
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
                syneroym_app_orchestration::empty_resolver(),
            )
            .await
            .unwrap(),
        );
        let container_engine =
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir.path(), None));
        let registry = EndpointRegistry::new_mock(Arc::new(MockStorage::new()));

        let native_dispatch = NativeDispatchRegistry::default();
        let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());
        let asset_registry: AssetRegistry = Arc::new(DashMap::new());
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
            messaging_broker,
            native_dispatch,
            http_routes.clone(),
            asset_registry.clone(),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        let service_id = "ws-route-missing-export-svc".to_string();
        let custom_config = serde_json::json!({
            "http_routes": [
                {"method": "GET", "path": "/ws", "target": "websocket", "operation": "handle-upgrade"}
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
                health_check: None,
                assets: None,
                visibility: None,
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
        let caller = node_wide_caller("test-caller");
        let err = service.deploy(service_id.clone(), manifest, &caller).await.expect_err(
            "a component without the websocket export must not deploy with a websocket route",
        );
        assert!(
            err.contains("target=websocket") && err.contains("does not export"),
            "expected websocket export error, got: {err}"
        );
        assert!(
            storage_provider.get_latest_config_generation(&service_id).await.unwrap().is_none(),
            "config generation must be rolled back"
        );
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
                health_check: None,
                assets: None,
                visibility: None,
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        let caller = node_wide_caller("did:key:zOwnerDid");
        let service_id = "owner-attribution-svc".to_string();
        service.deploy(service_id.clone(), owner_test_manifest(), &caller).await.unwrap();

        assert_eq!(registry.owner_of(&service_id), Some(caller.caller_did.clone()));

        service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            node_identity,
            syneroym_app_orchestration::empty_resolver(),
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

    /// Before anything is installed, there is no ground
    /// truth to report -- `installed_temporary_did` is `None`, and
    /// `instance_did` alone is what a caller about to certify a service
    /// for the first time reads.
    #[tokio::test]
    async fn instance_identity_reports_no_installed_did_before_anything_is_deployed() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, _registry) = service_with_node_identity(temp_dir.path(), node_identity).await;
        let caller = status_capable_caller("did:key:zOwner");

        let identity = service.instance_identity("svc-a".to_string(), &caller).await.unwrap();

        assert_eq!(identity.installed_temporary_did, None);
    }

    /// The whole reason `installed_temporary_did` exists.
    /// Once a certificate is installed under one caller, a *different*
    /// caller's `instance_identity` still derives its own (different)
    /// prospective DID in `instance_did` -- unchanged, since `deploy`'s
    /// certify flow depends on that -- but `installed_temporary_did` now
    /// reports the certificate actually in force, which is alice's, not
    /// bob's, regardless of who is asking.
    #[tokio::test]
    async fn instance_identity_reports_the_installed_did_even_for_a_different_caller() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry, _dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let alice = node_wide_caller("did:key:zAlice");
        // `instance_identity` gates on `orchestrator/status`, not `deploy`
        // (§0.28's own flat-abilities split) -- bob needs the former here.
        let bob = status_capable_caller("did:key:zBob");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        service.deploy(service_id.clone(), owner_test_manifest(), &alice).await.unwrap();
        let cert = instance_cert_for(&node_identity, &master, &alice.caller_did, &service_id, 3600);
        service.renew_cert(service_id.clone(), 0, cert.to_json().unwrap(), &alice).await.unwrap();
        let installed = registry.instance_cert(&service_id).unwrap().temporary_did;

        let for_bob = service.instance_identity(service_id.clone(), &bob).await.unwrap();

        assert_ne!(
            for_bob.instance_did, installed,
            "bob's own derived DID must not equal what alice actually installed"
        );
        assert_eq!(
            for_bob.installed_temporary_did,
            Some(installed),
            "installed_temporary_did must report the real key regardless of who is asking"
        );
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

    /// ADR-0022 §2's `generation` is a real content field, not freshness
    /// churn like `not_after`/`pkarr_packet_hex` -- omitting it from the
    /// dedup hash would let two records that differ only in generation
    /// hash identically, silently defeating redeploy dedup the day a
    /// publisher's generation is ever nonzero (every one is `0` today).
    #[test]
    fn stable_registry_certificate_for_hash_distinguishes_by_generation() {
        fn record_json(generation: u64) -> String {
            format!(
                r#"{{"info":{{"service_id":"did:key:zA","substrate_id":"did:key:zNode",
                "endpoint_type":"service","mechanisms":[],"is_private":false,
                "not_after":4102444800,"generation":{generation}}},
                "pkarr_packet_hex":"aa"}}"#
            )
        }
        let at_zero = stable_registry_certificate_for_hash(&record_json(0));
        let at_one = stable_registry_certificate_for_hash(&record_json(1));
        assert_ne!(
            at_zero, at_one,
            "two records differing only in generation must hash differently"
        );
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

    // ── M05A A5d: renew-cert ──────────────────────────────────────────────

    /// A minimal, stage-4-free policy for the renewal tests: enough for
    /// `resolve-relation` on `members` to reach a real query rather than
    /// short-circuiting, which is what makes "did the policy survive the
    /// rebuild" observable from outside. Deliberately not `STAGE4_POLICY`
    /// -- `authorize_rows` needs a guest component to export the after-step
    /// and is refused on the `tcp` services these tests deploy.
    const RENEWAL_POLICY: &str = r#"{
        "version": "fdae/v1",
        "definitions": {
            "members": {
                "table": "members",
                "principal_column": "owner_id",
                "permissions": {
                    "view": {
                        "allows": ["data-layer/read"],
                        "paths": [["caller"]]
                    }
                }
            }
        }
    }"#;

    /// Like `service_with_node_identity`, but hands the caller the
    /// `NativeDispatchRegistry` back as well. `ControlPlaneService` holds it
    /// as a `Weak` (see the cycle its own field doc explains), so a test
    /// that lets the `Arc` drop at the end of construction gets a registry
    /// that never upgrades -- and `renew_cert` fails closed on exactly that,
    /// since the whole point of the verb is the rebuild it does through it.
    async fn service_with_dispatch(
        temp_dir: &std::path::Path,
        node_identity: Arc<syneroym_identity::Identity>,
    ) -> (ControlPlaneService, EndpointRegistry, NativeDispatchRegistry) {
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
                syneroym_app_orchestration::empty_resolver(),
            )
            .await
            .unwrap(),
        );
        let native_dispatch = NativeDispatchRegistry::default();

        let service = ControlPlaneService::init(
            "orchestrator".to_string(),
            "did:key:zTestNode".to_string(),
            app_sandbox,
            Arc::new(ContainerEngine::new("podman".to_string(), temp_dir, None)),
            registry.clone(),
            temp_dir.to_path_buf(),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            native_dispatch.clone(),
            Arc::new(DashMap::new()),
            Arc::new(DashMap::new()),
            node_identity,
            syneroym_app_orchestration::empty_resolver(),
        )
        .await
        .unwrap();

        (service, registry, native_dispatch)
    }

    /// A `service-instance`-scoped certificate over exactly the key this
    /// substrate derives for `(caller, service_id)` -- what every real mint
    /// produces and what every install-time check below expects.
    fn instance_cert_for(
        node_identity: &syneroym_identity::Identity,
        master: &syneroym_identity::Identity,
        caller_did: &str,
        service_id: &str,
        expires_in_secs: u64,
    ) -> DelegationCertificate {
        let derived = node_identity.derive_service_identity(caller_did, service_id);
        DelegationCertificate::issue(
            master,
            derived.public_key(),
            expires_in_secs,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap()
    }

    /// Asks the service's own native-dispatch entry to sign a relationship
    /// proof, which carries whichever certificate that entry currently
    /// holds -- the only way to observe the by-value copy a renewal has to
    /// refresh, from outside.
    async fn resolve_relation_through_dispatch(
        native_dispatch: &NativeDispatchRegistry,
        service_id: &str,
        relation: &str,
        caller: &CallerContext,
    ) -> Result<syneroym_rpc::RelationshipProof, syneroym_rpc::RpcError> {
        let entry = native_dispatch
            .get(service_id)
            .unwrap_or_else(|| panic!("no native dispatch entry for '{service_id}'"))
            .clone();
        let response = entry
            .dispatch(syneroym_rpc::NativeInvocation {
                interface: "data-layer".to_string(),
                method: "resolve-relation".to_string(),
                params: serde_json::json!({
                    "relation": relation,
                    "principal": caller.session.subject_did,
                }),
                caller: caller.clone(),
            })
            .await?;
        Ok(serde_json::from_value(response.payload).expect("payload must be a RelationshipProof"))
    }

    /// The proof a service with no matching relation definition still
    /// signs -- empty `ids`, but carrying whichever certificate the
    /// dispatch entry currently holds, which is the only way to observe
    /// the by-value copy a renewal has to refresh.
    async fn relationship_proof_from_dispatch(
        native_dispatch: &NativeDispatchRegistry,
        service_id: &str,
        caller: &CallerContext,
    ) -> syneroym_rpc::RelationshipProof {
        resolve_relation_through_dispatch(native_dispatch, service_id, "unmatched", caller)
            .await
            .expect("resolve-relation must succeed")
    }

    /// The certificate-only install path: the new certificate lands and the
    /// service's *config* generation is untouched, which is the whole
    /// reason `renew-cert` exists rather than a redeploy.
    #[tokio::test]
    async fn renew_cert_installs_a_new_certificate_without_touching_the_config_generation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry, _dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        let first =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
        let mut manifest = owner_test_manifest();
        manifest.instance_certificate = Some(first.to_json().unwrap());
        service.deploy(service_id.clone(), manifest, &caller).await.unwrap();

        let gen_before =
            service.storage_provider.get_latest_config_generation(&service_id).await.unwrap();

        let renewed =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 7200);
        service
            .renew_cert(service_id.clone(), 0, renewed.to_json().unwrap(), &caller)
            .await
            .unwrap();

        assert_eq!(
            registry.instance_cert(&service_id).map(|c| c.expires_at_secs),
            Some(renewed.expires_at_secs),
            "the renewed certificate must be the one installed"
        );
        assert_eq!(
            service.storage_provider.get_latest_config_generation(&service_id).await.unwrap(),
            gen_before,
            "a renewal changes no configuration, so it must not bump the config generation"
        );
    }

    /// The correctness prerequisite: the running service's by-value copy of
    /// the certificate must move with the installed one, or every
    /// `RelationshipProof` it signs afterwards carries a certificate the
    /// verifier will reject.
    #[tokio::test]
    async fn renew_cert_rebuilds_syn_svc_native_service_with_the_new_certificate() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, _registry, dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        let first =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
        let mut manifest = owner_test_manifest();
        manifest.instance_certificate = Some(first.to_json().unwrap());
        service.deploy(service_id.clone(), manifest, &caller).await.unwrap();

        let before = relationship_proof_from_dispatch(&dispatch, &service_id, &caller).await;
        assert_eq!(before.delegation.as_deref(), Some(first.to_json().unwrap().as_str()));

        let renewed =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 7200);
        assert_ne!(renewed.to_json().unwrap(), first.to_json().unwrap());
        service
            .renew_cert(service_id.clone(), 0, renewed.to_json().unwrap(), &caller)
            .await
            .unwrap();

        let after = relationship_proof_from_dispatch(&dispatch, &service_id, &caller).await;
        assert_eq!(
            after.delegation.as_deref(),
            Some(renewed.to_json().unwrap().as_str()),
            "a proof signed after the renewal must carry the *new* certificate, not the one the \
             native service was constructed with"
        );
        after.verify(&service_id).expect("the proof must verify against the renewed certificate");
    }

    /// `renew-cert` is a lifecycle write, so it inherits `restart`'s own
    /// service-owner check rather than being reachable by any holder of a
    /// service-scoped grant.
    #[tokio::test]
    async fn renew_cert_is_refused_for_a_service_owned_by_another_caller() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, _registry, _dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let alice = node_wide_caller("did:key:zAlice");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        service.deploy(service_id.clone(), owner_test_manifest(), &alice).await.unwrap();

        let bob = scoped_deploy_caller("did:key:zBob", &service_id);
        let cert = instance_cert_for(&node_identity, &master, &bob.caller_did, &service_id, 3600);
        let err = service
            .renew_cert(service_id.clone(), 0, cert.to_json().unwrap(), &bob)
            .await
            .unwrap_err();
        assert!(err.contains("owned by"), "{err}");
    }

    /// The boundary of the check above: a node-wide `orchestrator/deploy`
    /// grantee -- the shape a supervisor holds -- renews a service it does
    /// not own. The certificate must be minted for *that* caller, since the
    /// derived instance key depends on the calling DID.
    #[tokio::test]
    async fn renew_cert_by_a_node_wide_deploy_grantee_ignores_the_service_owner() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry, _dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let alice = node_wide_caller("did:key:zAlice");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        service.deploy(service_id.clone(), owner_test_manifest(), &alice).await.unwrap();

        let bob = node_wide_caller("did:key:zBob");
        let cert = instance_cert_for(&node_identity, &master, &bob.caller_did, &service_id, 3600);
        service.renew_cert(service_id.clone(), 0, cert.to_json().unwrap(), &bob).await.unwrap();
        assert_eq!(
            registry.instance_cert(&service_id).map(|c| c.temporary_did),
            Some(cert.temporary_did)
        );
    }

    /// A superseded supervisor must not be able to install a certificate on
    /// a service it no longer manages -- the same generation gate `restart`
    /// and `undeploy` already apply.
    #[tokio::test]
    async fn renew_cert_respects_the_same_generation_gate_as_restart() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, _registry, _dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zAlice");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        service
            .deploy_with_context(
                service_id.clone(),
                owner_test_manifest(),
                Some(AppContext { generation: 5, ..app_context("app-1", "frontend", vec![]) }),
                &caller,
            )
            .await
            .unwrap();

        let cert =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
        let err = service
            .renew_cert(service_id.clone(), 3, cert.to_json().unwrap(), &caller)
            .await
            .unwrap_err();
        assert!(err.contains("at generation 5"), "{err}");
    }

    /// The backstop against an unbounded mint: nothing else caps
    /// `expires_at_secs`, so a certificate valid for years would sit there
    /// unnoticed -- the near-expiry warning that would catch it never fires.
    #[tokio::test]
    async fn verify_installed_instance_cert_rejects_a_certificate_over_the_thirty_day_cap() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry, _dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        let too_long = instance_cert_for(
            &node_identity,
            &master,
            &caller.caller_did,
            &service_id,
            MAX_INSTANCE_CERT_LIFETIME_SECS + 3600,
        );
        let mut manifest = owner_test_manifest();
        manifest.instance_certificate = Some(too_long.to_json().unwrap());

        let err = service.deploy(service_id.clone(), manifest, &caller).await.unwrap_err();
        assert!(err.contains("maximum"), "{err}");
        assert_eq!(registry.instance_cert(&service_id), None);
    }

    /// The regression guard for the cap above: the attended posture's own
    /// CLI default (24 hours) is nowhere near it, and so is any reasonable
    /// manual cadence. The cap catches an unbounded mistake, not a
    /// deliberate long-lived certificate.
    #[tokio::test]
    async fn verify_installed_instance_cert_accepts_the_cli_default_twenty_four_hour_certificate() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry, _dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        // `roymctl svc deploy --expires-hours`'s own default, restated here
        // rather than imported: `syneroym-sdk` depends on this crate, so
        // the constant cannot travel the other way.
        let cli_default_expires_hours = 24;
        let cli_default = instance_cert_for(
            &node_identity,
            &master,
            &caller.caller_did,
            &service_id,
            cli_default_expires_hours * 3600,
        );
        let mut manifest = owner_test_manifest();
        manifest.instance_certificate = Some(cli_default.to_json().unwrap());

        service.deploy(service_id.clone(), manifest, &caller).await.unwrap();
        assert!(registry.instance_cert(&service_id).is_some());
    }

    /// The `load_fdae_policy` `None` arm: a service that never declared a
    /// policy renews cleanly and still has none afterwards.
    #[tokio::test]
    async fn renew_cert_leaves_fdae_policy_untouched_when_none_was_ever_saved() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, _registry, _dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        service.deploy(service_id.clone(), owner_test_manifest(), &caller).await.unwrap();
        assert_eq!(service.storage_provider.load_fdae_policy(&service_id).await.unwrap(), None);

        let cert =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
        service.renew_cert(service_id.clone(), 0, cert.to_json().unwrap(), &caller).await.unwrap();
        assert_eq!(service.storage_provider.load_fdae_policy(&service_id).await.unwrap(), None);
    }

    /// Without this gate a capability-holding caller could hand
    /// `renew-cert` a `service_id` nothing ever deployed and have it
    /// register a live native-dispatch entry for it: `owner_of` passes
    /// vacuously for an unknown id, and an absent FDAE policy is not an
    /// error. `restart` already refuses on the same signal.
    #[tokio::test]
    async fn renew_cert_is_refused_for_a_service_id_with_no_recorded_deploy_facts() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, _registry, dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        let cert =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);

        let err = service
            .renew_cert(service_id.clone(), 0, cert.to_json().unwrap(), &caller)
            .await
            .unwrap_err();
        assert!(err.contains("not deployed here"), "{err}");
        assert!(
            dispatch.get(&service_id).is_none(),
            "a refused renewal must never have registered a dispatch entry"
        );
    }

    /// The renewed native service must mirror the *whole* of
    /// `deploy_with_context`'s construction site, not an enumerated subset
    /// of it: an implementation copying only the obvious inputs produces a
    /// service whose FDAE policy, proxy, and row-authorizer hooks are dead.
    ///
    /// Observed through `RENEWAL_POLICY`: a service holding it routes
    /// `resolve-relation` on `members` into a real query against the
    /// `members` table (which this fixture never creates, so the query
    /// reports collection-not-found), while a service whose policy was
    /// silently dropped short-circuits and answers with a signed, empty
    /// proof. Err-versus-Ok is therefore exactly "does the rebuilt service
    /// still hold its policy", and the certificate on the proof is the
    /// renewal itself.
    #[tokio::test]
    async fn renew_cert_mirrors_the_deploy_call_sites_service_proxy_and_row_authorizer() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, _registry, dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        let first =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
        let mut manifest =
            inline_manifest(None, None, Some(DocumentSource::Inline(RENEWAL_POLICY.to_string())));
        manifest.instance_certificate = Some(first.to_json().unwrap());
        service.deploy(service_id.clone(), manifest, &caller).await.unwrap();
        assert!(service.storage_provider.load_fdae_policy(&service_id).await.unwrap().is_some());

        let after_deploy =
            resolve_relation_through_dispatch(&dispatch, &service_id, "members", &caller).await;
        assert!(
            after_deploy.is_err(),
            "the deploy-built service must route this relation through its policy"
        );
        let deploy_asserter =
            relationship_proof_from_dispatch(&dispatch, &service_id, &caller).await.asserter_did;

        let renewed =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 7200);
        service
            .renew_cert(service_id.clone(), 0, renewed.to_json().unwrap(), &caller)
            .await
            .unwrap();

        let after_renewal =
            resolve_relation_through_dispatch(&dispatch, &service_id, "members", &caller).await;
        assert!(
            after_renewal.is_err(),
            "the rebuilt service must still route this relation through its policy -- an \
             implementation that mirrored only part of the call site would answer it with a \
             signed, empty proof instead"
        );
        let renewed_proof = relationship_proof_from_dispatch(&dispatch, &service_id, &caller).await;
        assert_eq!(
            renewed_proof.asserter_did, deploy_asserter,
            "the rebuilt service must speak as the same member master"
        );
        assert_eq!(renewed_proof.delegation.as_deref(), Some(renewed.to_json().unwrap().as_str()));
    }

    /// A stored FDAE document that no longer parses must abort the renewal
    /// before anything is installed. Falling back to `fdae_policy: None`
    /// would silently drop row/column filtering for the renewed instance --
    /// a materially worse failure than `deploy`'s, which fails the whole
    /// call on the same bad document.
    #[tokio::test]
    async fn renew_cert_aborts_on_a_stored_fdae_policy_that_fails_to_reparse() {
        let temp_dir = tempfile::tempdir().unwrap();
        let node_identity = Arc::new(syneroym_identity::Identity::generate().unwrap());
        let (service, registry, dispatch) =
            service_with_dispatch(temp_dir.path(), node_identity.clone()).await;
        let caller = node_wide_caller("did:key:zOwner");

        let master = syneroym_identity::Identity::generate().unwrap();
        let service_id = derive_did_key(&master.public_key());
        let first =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 3600);
        let mut manifest =
            inline_manifest(None, None, Some(DocumentSource::Inline(RENEWAL_POLICY.to_string())));
        manifest.instance_certificate = Some(first.to_json().unwrap());
        service.deploy(service_id.clone(), manifest, &caller).await.unwrap();

        // Stands in for a schema or parser change landing between the
        // deploy that saved this document and the renewal that re-reads it.
        service
            .storage_provider
            .save_fdae_policy(&service_id, "{ this is not a policy document }")
            .await
            .unwrap();

        let renewed =
            instance_cert_for(&node_identity, &master, &caller.caller_did, &service_id, 7200);
        let err = service
            .renew_cert(service_id.clone(), 0, renewed.to_json().unwrap(), &caller)
            .await
            .unwrap_err();
        assert!(err.contains("no longer validates"), "{err}");
        assert_eq!(
            registry.instance_cert(&service_id).map(|c| c.expires_at_secs),
            Some(first.expires_at_secs),
            "an aborted renewal must leave the previously installed certificate in place"
        );
        let still = relationship_proof_from_dispatch(&dispatch, &service_id, &caller).await;
        assert_eq!(still.delegation.as_deref(), Some(first.to_json().unwrap().as_str()));
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

        service.undeploy(service_id.clone(), 0, &caller).await.unwrap();
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
                syneroym_app_orchestration::empty_resolver(),
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
            Arc::new(DashMap::new()),
            Arc::new(syneroym_identity::Identity::generate().unwrap()),
            syneroym_app_orchestration::empty_resolver(),
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
                health_check: None,
                assets: None,
                visibility: None,
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

    // -----------------------------------------------------------------
    // M05A A4: health-check declaration, deploy-facts recording, status
    // -----------------------------------------------------------------

    fn tcp_manifest_with(port: u16, health_check: Option<WitHealthCheck>) -> DeployManifest {
        DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Tcp(TcpManifest {
                endpoints: vec![NetworkEndpoint {
                    interface_name: "main".to_string(),
                    host: "127.0.0.1".to_string(),
                    port,
                }],
            }),
            registry_certificate: None,
            instance_certificate: None,
        }
    }

    fn container_manifest_with(health_check: Option<WitHealthCheck>) -> DeployManifest {
        DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Container(ContainerManifest {
                source: ArtifactSource::Url("docker.io/library/nginx:1.27".to_string()),
                hash: None,
                image: "docker.io/library/nginx:1.27".to_string(),
                ports: vec![],
                volumes: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        }
    }

    fn wasm_manifest_with(health_check: Option<WitHealthCheck>) -> DeployManifest {
        DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check,
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(vec![]),
                hash: None,
                interfaces: vec![],
            }),
            registry_certificate: None,
            instance_certificate: None,
        }
    }

    #[tokio::test]
    async fn a_probe_kind_that_cannot_address_the_service_type_is_rejected_at_deploy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        // `rpc` on a container.
        let rpc_on_container = container_manifest_with(Some(WitHealthCheck::Rpc(WitRpcProbe {
            interface_name: "main".to_string(),
            method: "ping".to_string(),
            timeout_ms: 1000,
        })));
        let err = service
            .deploy("rpc-on-container".to_string(), rpc_on_container, &node_wide_caller("owner"))
            .await
            .unwrap_err();
        assert!(err.contains("cannot address"), "{err}");

        // `http-get` on wasm.
        let http_on_wasm = wasm_manifest_with(Some(WitHealthCheck::HttpGet(WitHttpProbe {
            interface_name: "main".to_string(),
            path: "/healthz".to_string(),
            expect_status: 200,
            timeout_ms: 1000,
        })));
        let err = service
            .deploy("http-on-wasm".to_string(), http_on_wasm, &node_wide_caller("owner"))
            .await
            .unwrap_err();
        assert!(err.contains("cannot address"), "{err}");
    }

    #[tokio::test]
    async fn an_http_probe_path_that_does_not_start_with_a_slash_is_rejected_at_deploy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let manifest = tcp_manifest_with(
            9,
            Some(WitHealthCheck::HttpGet(WitHttpProbe {
                interface_name: "main".to_string(),
                path: "healthz".to_string(),
                expect_status: 200,
                timeout_ms: 1000,
            })),
        );
        let err = service
            .deploy("bad-path".to_string(), manifest, &node_wide_caller("owner"))
            .await
            .unwrap_err();
        assert!(err.contains("must start with '/'"), "{err}");
    }

    #[tokio::test]
    async fn a_deploy_records_its_service_type_and_health_check_and_undeploy_removes_them() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let manifest = tcp_manifest_with(
            9,
            Some(WitHealthCheck::TcpConnect(WitTcpProbe {
                interface_name: "main".to_string(),
                timeout_ms: 1000,
            })),
        );
        service
            .deploy("facts-svc".to_string(), manifest, &node_wide_caller("owner"))
            .await
            .unwrap();

        let (service_type, check_json, ..) = service.registry.deploy_facts("facts-svc").unwrap();
        assert_eq!(service_type, "tcp");
        // Stored as the wire variant's own JSON, not the app model's
        // kebab-case one -- `run_probe` deserializes back into the same
        // wire type it reads here, so the two must agree on shape.
        let stored: WitHealthCheck = serde_json::from_str(&check_json.unwrap()).unwrap();
        assert!(matches!(stored, WitHealthCheck::TcpConnect(_)), "{stored:?}");

        service.undeploy("facts-svc".to_string(), 0, &node_wide_caller("owner")).await.unwrap();
        assert!(service.registry.deploy_facts("facts-svc").is_none());
    }

    #[tokio::test]
    async fn a_redeploy_without_a_health_check_clears_the_stored_one() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let with_check = tcp_manifest_with(
            9,
            Some(WitHealthCheck::TcpConnect(WitTcpProbe {
                interface_name: "main".to_string(),
                timeout_ms: 1000,
            })),
        );
        service
            .deploy("redeploy-svc".to_string(), with_check, &node_wide_caller("owner"))
            .await
            .unwrap();
        assert!(service.registry.deploy_facts("redeploy-svc").unwrap().1.is_some());

        let without_check = tcp_manifest_with(9, None);
        service
            .deploy("redeploy-svc".to_string(), without_check, &node_wide_caller("owner"))
            .await
            .unwrap();
        let (service_type, check_json, ..) = service.registry.deploy_facts("redeploy-svc").unwrap();
        assert_eq!(service_type, "tcp");
        assert!(check_json.is_none());
    }

    #[tokio::test]
    async fn a_container_and_a_tcp_service_are_distinguished_by_the_recorded_type() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        // Both register the identical `TcpHostPort` endpoint variant --
        // the §0.5 finding -- so the distinction must come from the
        // recorded fact, not the endpoint.
        service
            .registry
            .register(
                "container-svc".to_string(),
                "main".to_string(),
                SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
            )
            .await
            .unwrap();
        service
            .registry
            .set_deploy_facts(
                "container-svc".to_string(),
                "container".to_string(),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        service
            .registry
            .register(
                "tcp-svc".to_string(),
                "main".to_string(),
                SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
            )
            .await
            .unwrap();
        service
            .registry
            .set_deploy_facts("tcp-svc".to_string(), "tcp".to_string(), None, None, None)
            .await
            .unwrap();

        let container_phase = service.instance_phase("container-svc", Some("container")).await;
        let tcp_phase = service.instance_phase("tcp-svc", Some("tcp")).await;

        assert!(
            matches!(container_phase, InstancePhase::NotRunning(_)),
            "expected NotRunning, got {container_phase:?}"
        );
        assert!(
            matches!(tcp_phase, InstancePhase::Unknown(_)),
            "expected Unknown, got {tcp_phase:?}"
        );
    }

    // -- the durable proxy queue's operator surface ------------------------

    /// Stands in for the router's outbox, which this crate cannot depend
    /// on. Records what was asked of it so `replay` can be shown not to
    /// execute anything inline.
    #[derive(Debug, Default)]
    struct FakeProxyQueues {
        queued: std::sync::Mutex<Vec<QueuedCallInfo>>,
        dead: std::sync::Mutex<Vec<DeadLetterInfo>>,
        replayed: std::sync::Mutex<Vec<(String, u64)>>,
        delivered: std::sync::atomic::AtomicUsize,
        sagas: std::sync::Mutex<Vec<SagaInfo>>,
        rearmed: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl ProxyQueueInspector for FakeProxyQueues {
        async fn queued_calls(&self, _service_id: &str) -> Result<Vec<QueuedCallInfo>, String> {
            Ok(self.queued.lock().unwrap().clone())
        }

        async fn dead_letters(&self, _service_id: &str) -> Result<Vec<DeadLetterInfo>, String> {
            Ok(self.dead.lock().unwrap().clone())
        }

        async fn replay_dead_letter(&self, service_id: &str, id: u64) -> Result<(), String> {
            // Re-enqueue only: a replay that executed here would be doing
            // the delivery itself, which is exactly what must not happen.
            self.replayed.lock().unwrap().push((service_id.to_string(), id));
            let mut dead = self.dead.lock().unwrap();
            let Some(pos) = dead.iter().position(|d| d.id == id) else {
                return Err(format!("no dead letter with id {id}"));
            };
            let letter = dead.remove(pos);
            self.queued.lock().unwrap().push(QueuedCallInfo {
                id: letter.id,
                idempotency_key: letter.idempotency_key,
                attempts: letter.attempts,
            });
            Ok(())
        }

        async fn sagas(&self, _service_id: &str) -> Result<Vec<SagaInfo>, String> {
            Ok(self.sagas.lock().unwrap().clone())
        }

        async fn rearm_saga(&self, service_id: &str, saga_id: &str) -> Result<(), String> {
            // Re-arm only: walking the saga here would be doing the
            // delivery itself, which is exactly what must not happen.
            self.rearmed.lock().unwrap().push((service_id.to_string(), saga_id.to_string()));
            Ok(())
        }
    }

    async fn service_with_proxy_queues(
        dir: &std::path::Path,
        queues: &Arc<FakeProxyQueues>,
    ) -> ControlPlaneService {
        let service = service_for_inline_tests(dir).await;
        service
            .proxy_queues
            .set(Arc::downgrade(queues) as std::sync::Weak<dyn ProxyQueueInspector>)
            .expect("proxy queues set once");
        service
    }

    fn a_dead_letter(id: u64, key: &str) -> DeadLetterInfo {
        DeadLetterInfo {
            id,
            idempotency_key: key.to_string(),
            attempts: 54,
            last_error: "target unreachable".to_string(),
            created_at: 1_700_000_000_000,
        }
    }

    #[tokio::test]
    async fn proxy_dead_letters_lists_what_the_services_queue_holds() {
        let temp_dir = tempfile::tempdir().unwrap();
        let queues = Arc::new(FakeProxyQueues::default());
        queues.dead.lock().unwrap().push(a_dead_letter(1, "msg-7"));
        let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

        let listed = service
            .proxy_dead_letters("svc-a".to_string(), &status_capable_caller("owner"))
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].idempotency_key, "msg-7");
        assert_eq!(listed[0].last_error, "target unreachable");
    }

    /// The verb B1 shipped without and had to add afterwards, because its
    /// e2e could not otherwise assert that an item was queued, survived a
    /// restart, and then left.
    #[tokio::test]
    async fn proxy_outbox_lists_an_item_before_it_lands() {
        let temp_dir = tempfile::tempdir().unwrap();
        let queues = Arc::new(FakeProxyQueues::default());
        queues.queued.lock().unwrap().push(QueuedCallInfo {
            id: 1,
            idempotency_key: "msg-7".to_string(),
            attempts: 2,
        });
        let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

        let listed = service
            .proxy_outbox("svc-a".to_string(), &status_capable_caller("owner"))
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].idempotency_key, "msg-7");
        assert_eq!(listed[0].attempts, 2);
    }

    #[tokio::test]
    async fn replay_re_enqueues_and_does_not_execute_inline() {
        let temp_dir = tempfile::tempdir().unwrap();
        let queues = Arc::new(FakeProxyQueues::default());
        queues.dead.lock().unwrap().push(a_dead_letter(1, "msg-7"));
        let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

        service
            .proxy_replay("svc-a".to_string(), 1, &status_capable_caller("owner"))
            .await
            .unwrap();

        assert!(queues.dead.lock().unwrap().is_empty(), "the dead letter must be consumed");
        assert_eq!(
            queues.queued.lock().unwrap().len(),
            1,
            "and reappear in the outbox for the worker to pick up"
        );
        assert_eq!(
            queues.delivered.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "replay must not deliver anything itself"
        );
    }

    /// Failure-matrix row 14, extending the gate their neighbours already
    /// use rather than inventing a second authority to hold.
    #[tokio::test]
    async fn the_new_verbs_are_refused_without_the_gate_their_neighbours_use() {
        use syneroym_rpc::{AuthLevel, SessionContext};

        let temp_dir = tempfile::tempdir().unwrap();
        let queues = Arc::new(FakeProxyQueues::default());
        queues.dead.lock().unwrap().push(a_dead_letter(1, "msg-7"));
        let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

        let ungranted = CallerContext {
            caller_did: "did:key:zStranger".to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: "did:key:zStranger".to_string(),
                ..Default::default()
            },
            auth: AuthLevel::Delegated,
            proof: None,
        };

        assert!(service.proxy_outbox("svc-a".to_string(), &ungranted).await.is_err());
        assert!(service.proxy_dead_letters("svc-a".to_string(), &ungranted).await.is_err());
        assert!(service.proxy_replay("svc-a".to_string(), 1, &ungranted).await.is_err());
        assert_eq!(
            queues.dead.lock().unwrap().len(),
            1,
            "a refused replay must not have consumed the dead letter"
        );
    }

    /// Replay re-enqueues a call the worker then sends, so it is a
    /// lifecycle write and must not be reachable with the read grant the
    /// listing verbs use. `status_capable_caller` holds both, so this
    /// drives a caller holding *only* the read one.
    #[tokio::test]
    async fn proxy_replay_is_not_reachable_with_only_the_read_grant() {
        use syneroym_rpc::{AuthLevel, Capability, SessionContext};

        let temp_dir = tempfile::tempdir().unwrap();
        let queues = Arc::new(FakeProxyQueues::default());
        queues.dead.lock().unwrap().push(a_dead_letter(1, "msg-7"));
        let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

        let read_only = CallerContext {
            caller_did: "did:key:zReader".to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: "did:key:zReader".to_string(),
                capabilities: vec![Capability {
                    with: ResourceUri::substrate("did:key:zTestNode"),
                    can: Ability(Ability::ORCHESTRATOR_STATUS.to_string()),
                    caveats: None,
                }],
                ..Default::default()
            },
            auth: AuthLevel::Delegated,
            proof: None,
        };

        // The listings are reads and stay reachable.
        assert!(service.proxy_outbox("svc-a".to_string(), &read_only).await.is_ok());
        assert!(service.proxy_dead_letters("svc-a".to_string(), &read_only).await.is_ok());

        // The write is not.
        assert!(
            service.proxy_replay("svc-a".to_string(), 1, &read_only).await.is_err(),
            "a read grant must not let a caller make a service emit calls"
        );
        assert_eq!(queues.dead.lock().unwrap().len(), 1);
    }

    fn a_saga(saga_id: &str, state: &str) -> SagaInfo {
        SagaInfo {
            saga_id: saga_id.to_string(),
            name: "checkout".to_string(),
            state: syneroym_rpc::SagaState::Compensating,
            steps: 2,
            compensated_steps: 1,
            created_at: 1_700_000_000_000,
            deadline_at: 1_700_003_600_000,
            last_error: Some(state.to_string()),
        }
    }

    #[tokio::test]
    async fn sagas_lists_what_the_services_log_holds() {
        let temp_dir = tempfile::tempdir().unwrap();
        let queues = Arc::new(FakeProxyQueues::default());
        queues.sagas.lock().unwrap().push(a_saga("saga-1", "unreachable"));
        let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

        let listed =
            service.sagas("svc-a".to_string(), &status_capable_caller("owner")).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].saga_id, "saga-1");
        assert_eq!(listed[0].name, "checkout");
    }

    #[tokio::test]
    async fn sagas_is_refused_without_a_status_grant() {
        use syneroym_rpc::{AuthLevel, SessionContext};

        let temp_dir = tempfile::tempdir().unwrap();
        let queues = Arc::new(FakeProxyQueues::default());
        queues.sagas.lock().unwrap().push(a_saga("saga-1", "unreachable"));
        let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

        let ungranted = CallerContext {
            caller_did: "did:key:zStranger".to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: "did:key:zStranger".to_string(),
                ..Default::default()
            },
            auth: AuthLevel::Delegated,
            proof: None,
        };

        assert!(service.sagas("svc-a".to_string(), &ungranted).await.is_err());
    }

    /// `saga-compensate` causes calls to leave the node, so it takes the
    /// write gate, not the listing's read gate -- the same rule
    /// `proxy-replay` follows (B2's F7).
    #[tokio::test]
    async fn saga_compensate_is_not_reachable_with_only_the_read_grant() {
        use syneroym_rpc::{AuthLevel, Capability, SessionContext};

        let temp_dir = tempfile::tempdir().unwrap();
        let queues = Arc::new(FakeProxyQueues::default());
        queues.sagas.lock().unwrap().push(a_saga("saga-1", "unreachable"));
        let service = service_with_proxy_queues(temp_dir.path(), &queues).await;

        let read_only = CallerContext {
            caller_did: "did:key:zReader".to_string(),
            app_instance: None,
            session: SessionContext {
                subject_did: "did:key:zReader".to_string(),
                capabilities: vec![Capability {
                    with: ResourceUri::substrate("did:key:zTestNode"),
                    can: Ability(Ability::ORCHESTRATOR_STATUS.to_string()),
                    caveats: None,
                }],
                ..Default::default()
            },
            auth: AuthLevel::Delegated,
            proof: None,
        };

        assert!(service.sagas("svc-a".to_string(), &read_only).await.is_ok());
        assert!(
            service
                .saga_compensate("svc-a".to_string(), "saga-1".to_string(), &read_only)
                .await
                .is_err(),
            "a read grant must not let a caller make a service emit undos"
        );
        assert!(queues.rearmed.lock().unwrap().is_empty());

        service
            .saga_compensate(
                "svc-a".to_string(),
                "saga-1".to_string(),
                &status_capable_caller("owner"),
            )
            .await
            .unwrap();
        assert_eq!(queues.rearmed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn readyz_does_not_podman_inspect_a_tcp_service() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        service
            .registry
            .register(
                "tcp-readyz-svc".to_string(),
                "main".to_string(),
                SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
            )
            .await
            .unwrap();
        service
            .registry
            .set_deploy_facts("tcp-readyz-svc".to_string(), "tcp".to_string(), None, None, None)
            .await
            .unwrap();

        // Before D-A4-17 this called `podman inspect` against a real TCP
        // service and reported the resulting failure as unreadiness.
        let result =
            service.readyz("tcp-readyz-svc".to_string(), &status_capable_caller("owner")).await;
        assert!(result.is_ok(), "{:?}", result);
    }

    #[tokio::test]
    async fn status_reports_unknown_for_a_service_with_no_recorded_type() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        service
            .registry
            .register(
                "pre-a4-svc".to_string(),
                "main".to_string(),
                SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
            )
            .await
            .unwrap();
        // Deliberately no `set_deploy_facts` call -- simulates a service
        // deployed by a pre-A4 binary.

        let phase = service.instance_phase("pre-a4-svc", None).await;
        assert!(
            matches!(phase, InstancePhase::Unknown(ref r) if r.contains("no service type recorded"))
        );
    }

    #[tokio::test]
    async fn status_reports_not_found_for_an_id_this_substrate_has_no_endpoints_for() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let status = service
            .status(vec!["never-deployed".to_string()], &node_wide_caller("owner"))
            .await
            .unwrap();
        assert_eq!(status.services.len(), 1);
        assert!(matches!(status.services[0].phase, InstancePhase::NotFound));
    }

    #[tokio::test]
    async fn status_omits_a_service_the_caller_may_not_see_and_reports_not_found_when_named() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        service
            .deploy(
                "owned-by-alice".to_string(),
                tcp_manifest_with(9, None),
                &node_wide_caller("alice"),
            )
            .await
            .unwrap();

        let bob = scoped_deploy_caller("bob", "some-other-service");
        let swept = service.status(vec![], &bob).await.unwrap();
        assert!(
            swept.services.iter().all(|s| s.service_id != "owned-by-alice"),
            "bob must not see alice's service in an unnamed sweep"
        );

        // A4-10: named explicitly, it must read identically to an id that
        // was never deployed at all -- `not-found`, not `unauthorized`. Bob
        // holds no grant on "owned-by-alice" whatsoever; distinguishing the
        // two would let any verified caller probe for the existence of an
        // arbitrary DID on this node with no grant at all.
        let named = service.status(vec!["owned-by-alice".to_string()], &bob).await.unwrap();
        assert_eq!(named.services.len(), 1);
        assert!(matches!(named.services[0].phase, InstancePhase::NotFound));
    }

    /// M05A A5a §6: `status` reports the epoch this substrate currently
    /// serves for each of a service's own declared dependencies, read
    /// from the per-dependent persisted binding row -- the exit
    /// criterion's per-dependent binding convergence data.
    #[tokio::test]
    async fn status_reports_the_epoch_it_currently_serves_per_dependency() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let caller = node_wide_caller("did:key:zAlice");

        service
            .deploy_with_context(
                "frontend-svc".to_string(),
                tcp_manifest_with(9, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &caller,
            )
            .await
            .unwrap();
        service
            .write_bindings(
                BindingWrite {
                    service_id: "frontend-svc".to_string(),
                    app_instance_id: "app-1".to_string(),
                    bindings: vec![DependencyBinding {
                        dependency_name: "backend".to_string(),
                        app_instance_id: "app-1".to_string(),
                        mode: WitTopologyMode::Singleton,
                        members: vec!["did:key:zNewBackendMember".to_string()],
                        epoch: 3,
                        cache_ttl_ms: 60_000,
                    }],
                    generation: 0,
                },
                &caller,
            )
            .await
            .unwrap();

        let status = service.status(vec!["frontend-svc".to_string()], &caller).await.unwrap();
        assert_eq!(status.services.len(), 1);
        assert_eq!(
            status.services[0].binding_epochs,
            vec![("backend".to_string(), 3)],
            "{:?}",
            status.services[0].binding_epochs
        );
    }

    /// A4-10's rule, re-pinned (M05A A5a §6 adds `binding-epochs` to the
    /// same record): a caller with no grant on a named id must not learn
    /// anything about it, including what it depends on -- `not-found`
    /// carries an empty `binding_epochs`, same as every other field.
    #[tokio::test]
    async fn status_reports_not_found_for_a_named_id_the_caller_may_not_see() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        service
            .deploy_with_context(
                "owned-by-alice".to_string(),
                tcp_manifest_with(9, None),
                Some(app_context(
                    "app-1",
                    "frontend",
                    vec![dependency_binding("backend", vec!["did:key:zBackendMember"])],
                )),
                &node_wide_caller("alice"),
            )
            .await
            .unwrap();

        let bob = scoped_deploy_caller("bob", "some-other-service");
        let named = service.status(vec!["owned-by-alice".to_string()], &bob).await.unwrap();
        assert_eq!(named.services.len(), 1);
        assert!(matches!(named.services[0].phase, InstancePhase::NotFound));
        assert!(
            named.services[0].binding_epochs.is_empty(),
            "a caller with no grant must not learn what a service it cannot see depends on"
        );
    }

    /// A4-11: an unbounded `service_ids` list from any verified caller must
    /// not be free to accept -- each named id that turns out not to exist
    /// used to cost a full scan of every registered endpoint.
    #[tokio::test]
    async fn status_rejects_a_service_ids_list_over_the_cap() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let too_many: Vec<String> =
            (0..=MAX_STATUS_SERVICE_IDS).map(|i| format!("svc-{i}")).collect();
        let err = service.status(too_many, &status_capable_caller("owner")).await.unwrap_err();
        assert!(err.contains("over the"), "{err}");
    }

    /// A4-09 (post-review): a duplicate id in one `service_ids` list used to
    /// be a target twice over, racing itself inside the same `join_all`
    /// (A4-05) and bypassing `probe_cached`'s cache entirely -- confirmed
    /// empirically before this fix (8 copies of one id produced 8 concurrent
    /// probes). One entry per distinct id now, regardless of how many times
    /// the caller names it.
    #[tokio::test]
    async fn status_deduplicates_repeated_service_ids_before_probing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let owner = status_capable_caller("owner");

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let manifest = tcp_manifest_with(
            port,
            Some(WitHealthCheck::TcpConnect(WitTcpProbe {
                interface_name: "main".to_string(),
                timeout_ms: 2000,
            })),
        );
        service.deploy("dup-svc".to_string(), manifest, &owner).await.unwrap();

        let repeated = vec!["dup-svc".to_string(); 8];
        let status = service.status(repeated, &owner).await.unwrap();
        assert_eq!(status.services.len(), 1, "{:?}", status.services);
        assert!(matches!(status.services[0].probe, ProbeStatus::Passing));
    }

    #[tokio::test]
    async fn node_facts_are_absent_for_a_caller_without_node_wide_status() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let bob = scoped_deploy_caller("bob", "bobs-service");
        service.deploy("bobs-service".to_string(), tcp_manifest_with(9, None), &bob).await.unwrap();

        let status = service.status(vec![], &bob).await.unwrap();
        assert!(status.node.is_none());
        assert_eq!(status.services.len(), 1);
        assert_eq!(status.services[0].service_id, "bobs-service");
    }

    #[tokio::test]
    async fn node_facts_are_returned_for_the_substrate_owner() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let status = service.status(vec![], &status_capable_caller("owner")).await.unwrap();
        assert!(status.node.is_some());
    }

    #[tokio::test]
    async fn status_reports_the_compiled_in_service_types() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let status = service.status(vec![], &status_capable_caller("owner")).await.unwrap();
        let node = status.node.unwrap();
        // Default features enable both sandboxes; `tcp` needs no engine.
        assert_eq!(node.service_types, vec!["container", "tcp", "wasm"]);
    }

    /// A4-06: the same gate `status`'s own `node` field applies, on the
    /// standalone path -- a caller without node-wide `orchestrator/status`
    /// must not read node facts through this narrower method either.
    #[tokio::test]
    async fn node_facts_is_none_without_node_wide_status() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let bob = scoped_deploy_caller("bob", "bobs-service");
        assert!(service.node_facts(&bob).await.is_none());
    }

    /// A4-06: `node_facts` must answer identically to `status(vec![])`'s
    /// `node` field -- the whole point of splitting it out is a cheaper path
    /// to the *same* facts, not a different set of them.
    #[tokio::test]
    async fn node_facts_answers_the_same_as_status_with_no_service_ids() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let owner = status_capable_caller("owner");

        let registry_client = Arc::new(syneroym_core::dht_registry::RegistryClient::new(
            true,
            Some("http://registry.example".to_string()),
        ));
        service.set_endpoint_publisher(Arc::new(
            syneroym_core::endpoint_publisher::EndpointPublisher::new(
                registry_client,
                temp_dir.path().to_path_buf(),
            ),
        ));

        let via_status = service.status(vec![], &owner).await.unwrap().node.unwrap();
        let via_node_facts = service.node_facts(&owner).await.unwrap();
        assert_eq!(via_status.registry_url, via_node_facts.registry_url);
        assert_eq!(via_status.dht_enabled, via_node_facts.dht_enabled);
        assert_eq!(via_status.node_did, via_node_facts.node_did);
        assert_eq!(via_status.service_types, via_node_facts.service_types);
    }

    #[tokio::test]
    async fn status_reports_the_registry_this_node_publishes_into() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let registry_client = Arc::new(syneroym_core::dht_registry::RegistryClient::new(
            true,
            Some("http://registry.example".to_string()),
        ));
        service.set_endpoint_publisher(Arc::new(
            syneroym_core::endpoint_publisher::EndpointPublisher::new(
                registry_client,
                temp_dir.path().to_path_buf(),
            ),
        ));

        let status = service.status(vec![], &status_capable_caller("owner")).await.unwrap();
        let node = status.node.unwrap();
        assert_eq!(node.registry_url.as_deref(), Some("http://registry.example"));
        assert!(node.dht_enabled);
    }

    #[tokio::test]
    async fn a_probe_runs_for_a_tcp_service_whose_phase_is_unknown() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        // A `connect` completes once the OS accepts the SYN into its own
        // backlog queue -- the listener need not call `accept()` itself, so
        // just keeping it bound (and alive for the test's duration) is
        // enough for the probe to see a `Passing` connect.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let manifest = tcp_manifest_with(
            port,
            Some(WitHealthCheck::TcpConnect(WitTcpProbe {
                interface_name: "main".to_string(),
                timeout_ms: 2000,
            })),
        );
        service
            .deploy("probed-tcp-svc".to_string(), manifest, &node_wide_caller("owner"))
            .await
            .unwrap();

        let status = service
            .status(vec!["probed-tcp-svc".to_string()], &node_wide_caller("owner"))
            .await
            .unwrap();
        assert_eq!(status.services.len(), 1);
        assert!(matches!(status.services[0].phase, InstancePhase::Unknown(_)));
        assert!(
            matches!(status.services[0].probe, ProbeStatus::Passing),
            "expected the probe to have run and passed, got {:?}",
            status.services[0].probe
        );
    }

    /// A bare TCP listener that answers every connection with a fixed,
    /// hand-written HTTP response -- avoids pulling in a full HTTP server
    /// framework as a test-only dependency just to drive an `http-get`
    /// probe (A4-13).
    async fn serve_http_responses(response: &'static str) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { break };
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 1024];
                    let _ = stream.read(&mut buf).await;
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        port
    }

    /// A4-13: nothing before this pinned that `run_probe`'s `HttpGet` arm
    /// ever actually reaches a listener -- the only other `HttpGet` tests in
    /// this file check deploy-time rejection.
    #[tokio::test]
    async fn an_http_probe_passes_on_the_expected_status() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let port = serve_http_responses(
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;

        let manifest = tcp_manifest_with(
            port,
            Some(WitHealthCheck::HttpGet(WitHttpProbe {
                interface_name: "main".to_string(),
                path: "/healthz".to_string(),
                expect_status: 200,
                timeout_ms: 2000,
            })),
        );
        service
            .deploy("http-ok-svc".to_string(), manifest, &node_wide_caller("owner"))
            .await
            .unwrap();
        let status = service
            .status(vec!["http-ok-svc".to_string()], &node_wide_caller("owner"))
            .await
            .unwrap();
        assert!(
            matches!(status.services[0].probe, ProbeStatus::Passing),
            "{:?}",
            status.services[0].probe
        );
    }

    #[tokio::test]
    async fn an_http_probe_fails_on_an_unexpected_status() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let port = serve_http_responses(
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;

        let manifest = tcp_manifest_with(
            port,
            Some(WitHealthCheck::HttpGet(WitHttpProbe {
                interface_name: "main".to_string(),
                path: "/healthz".to_string(),
                expect_status: 200,
                timeout_ms: 2000,
            })),
        );
        service
            .deploy("http-503-svc".to_string(), manifest, &node_wide_caller("owner"))
            .await
            .unwrap();
        let status = service
            .status(vec!["http-503-svc".to_string()], &node_wide_caller("owner"))
            .await
            .unwrap();
        assert!(
            matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("got 503")),
            "{:?}",
            status.services[0].probe
        );
    }

    /// A4-12: a hostile or compromised container answering a probe with a
    /// redirect must not make the substrate follow it -- a readiness check
    /// has no reason to, and `reqwest`'s default policy (up to ten hops)
    /// would otherwise let the probed service steer this substrate's own
    /// requests (SSRF).
    #[tokio::test]
    async fn an_http_probe_does_not_follow_a_redirect() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let port = serve_http_responses(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/elsewhere\r\nContent-Length: \
             0\r\nConnection: close\r\n\r\n",
        )
        .await;

        let manifest = tcp_manifest_with(
            port,
            Some(WitHealthCheck::HttpGet(WitHttpProbe {
                interface_name: "main".to_string(),
                path: "/healthz".to_string(),
                expect_status: 200,
                timeout_ms: 2000,
            })),
        );
        service
            .deploy("http-redirect-svc".to_string(), manifest, &node_wide_caller("owner"))
            .await
            .unwrap();
        let status = service
            .status(vec!["http-redirect-svc".to_string()], &node_wide_caller("owner"))
            .await
            .unwrap();
        assert!(
            matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("got 302")),
            "the probe must report the redirect itself, not follow it: {:?}",
            status.services[0].probe
        );
    }

    /// A4-14: `run_probe`'s four error branches, each worded distinctly and
    /// read straight off `app health` by an operator -- none were reachable
    /// from the suite before this.
    #[tokio::test]
    async fn run_probe_reports_an_unreadable_stored_health_check() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        service
            .registry
            .set_deploy_facts(
                "bad-check-svc".to_string(),
                "tcp".to_string(),
                Some("not valid json".to_string()),
                None,
                None,
            )
            .await
            .unwrap();
        service
            .registry
            .register(
                "bad-check-svc".to_string(),
                "main".to_string(),
                SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
            )
            .await
            .unwrap();

        let status = service
            .status(vec!["bad-check-svc".to_string()], &status_capable_caller("owner"))
            .await
            .unwrap();
        assert!(
            matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("stored health check is unreadable")),
            "{:?}",
            status.services[0].probe
        );
    }

    #[tokio::test]
    async fn run_probe_reports_no_endpoint_for_the_declared_interface() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        service
            .registry
            .set_deploy_facts(
                "no-endpoint-svc".to_string(),
                "tcp".to_string(),
                Some(
                    serde_json::to_string(&WitHealthCheck::TcpConnect(WitTcpProbe {
                        interface_name: "main".to_string(),
                        timeout_ms: 1000,
                    }))
                    .unwrap(),
                ),
                None,
                None,
            )
            .await
            .unwrap();
        // Registered under a different interface, so the service is visible
        // at all -- but deliberately no `registry.register(...)` call for
        // "main", the interface the health check actually names.
        service
            .registry
            .register(
                "no-endpoint-svc".to_string(),
                "other".to_string(),
                SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port: 9 },
            )
            .await
            .unwrap();

        let status = service
            .status(vec!["no-endpoint-svc".to_string()], &status_capable_caller("owner"))
            .await
            .unwrap();
        assert!(
            matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("no endpoint registered for interface 'main'")),
            "{:?}",
            status.services[0].probe
        );
    }

    #[tokio::test]
    async fn run_probe_reports_a_non_tcp_endpoint_under_tcp_connect() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        service
            .registry
            .set_deploy_facts(
                "non-tcp-endpoint-svc".to_string(),
                // "nativehost" so `instance_phase` reports `Unknown` (the
                // probe runs) without this test also having to fake a real
                // running instance -- "wasm" would report `NotRunning` and
                // skip the probe entirely, before ever reaching `run_probe`.
                "nativehost".to_string(),
                Some(
                    serde_json::to_string(&WitHealthCheck::TcpConnect(WitTcpProbe {
                        interface_name: "main".to_string(),
                        timeout_ms: 1000,
                    }))
                    .unwrap(),
                ),
                None,
                None,
            )
            .await
            .unwrap();
        service
            .registry
            .register(
                "non-tcp-endpoint-svc".to_string(),
                "main".to_string(),
                SubstrateEndpoint::WasmChannel { service_id: "non-tcp-endpoint-svc".to_string() },
            )
            .await
            .unwrap();

        let status = service
            .status(vec!["non-tcp-endpoint-svc".to_string()], &status_capable_caller("owner"))
            .await
            .unwrap();
        assert!(
            matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("is not a TCP endpoint")),
            "{:?}",
            status.services[0].probe
        );
    }

    #[tokio::test]
    async fn run_probe_reports_a_non_tcp_endpoint_under_http_get() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        service
            .registry
            .set_deploy_facts(
                "non-tcp-http-svc".to_string(),
                // See the identical note in the tcp-connect version of this
                // test just above.
                "nativehost".to_string(),
                Some(
                    serde_json::to_string(&WitHealthCheck::HttpGet(WitHttpProbe {
                        interface_name: "main".to_string(),
                        path: "/healthz".to_string(),
                        expect_status: 200,
                        timeout_ms: 1000,
                    }))
                    .unwrap(),
                ),
                None,
                None,
            )
            .await
            .unwrap();
        service
            .registry
            .register(
                "non-tcp-http-svc".to_string(),
                "main".to_string(),
                SubstrateEndpoint::WasmChannel { service_id: "non-tcp-http-svc".to_string() },
            )
            .await
            .unwrap();

        let status = service
            .status(vec!["non-tcp-http-svc".to_string()], &status_capable_caller("owner"))
            .await
            .unwrap();
        assert!(
            matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("is not a TCP endpoint")),
            "{:?}",
            status.services[0].probe
        );
    }

    #[tokio::test]
    async fn a_probe_is_not_run_for_an_instance_that_is_not_running() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        // A `wasm` service with a recorded type but nothing loaded --
        // `instance_phase` reports `NotRunning`, and the probe must not run
        // for a fault the substrate already knows about.
        service
            .registry
            .set_deploy_facts(
                "not-running-svc".to_string(),
                "wasm".to_string(),
                Some(
                    serde_json::to_string(&WitHealthCheck::Rpc(WitRpcProbe {
                        interface_name: "main".to_string(),
                        method: "ping".to_string(),
                        timeout_ms: 1000,
                    }))
                    .unwrap(),
                ),
                None,
                None,
            )
            .await
            .unwrap();
        service
            .registry
            .register(
                "not-running-svc".to_string(),
                "main".to_string(),
                SubstrateEndpoint::WasmChannel { service_id: "not-running-svc".to_string() },
            )
            .await
            .unwrap();

        let status = service
            .status(vec!["not-running-svc".to_string()], &status_capable_caller("owner"))
            .await
            .unwrap();
        assert_eq!(status.services.len(), 1);
        assert!(matches!(status.services[0].phase, InstancePhase::NotRunning(_)));
        assert!(matches!(status.services[0].probe, ProbeStatus::NotDeclared));
    }

    /// A4-13: nothing before this pinned that `run_probe`'s `Rpc` arm ever
    /// actually reaches a running guest -- every other `Rpc` test in this
    /// file deploys `Binary(vec![])` (deliberately fake, for deploy-time
    /// rejection only) or skips deploy entirely via `set_deploy_facts`. This
    /// one deploys the real `greeter` fixture and lets the probe genuinely
    /// invoke it.
    ///
    /// Post-review (N-1): `run_probe`'s `Rpc` arm always sends
    /// `params: Value::Array(vec![])`, so a probe method that takes a
    /// required argument -- `greeter`'s own `greet(name: string)` -- can
    /// never pass; `json_to_wasm_params`'s `default_for_missing`
    /// (`sandbox_wasm/src/conversions.rs`) errors for any non-`Option`
    /// parameter the array doesn't supply. That is a real, permanent
    /// `ProbeFailing` an operator would read as a live outage, not a
    /// declaration mistake -- recorded in `deferred-backlog.md` rather than
    /// fixed here, since the real fix (a `params` field on `rpc-probe`, or
    /// deploy-time introspection of the guest's exported signature) is a
    /// WIT/schema decision of its own, not a drive-by change. Pinned
    /// explicitly here rather than left as an accidentally-green test.
    #[tokio::test]
    async fn an_rpc_probe_permanently_fails_for_a_method_that_takes_a_required_argument() {
        let wasm_bytes = fs::read(greeter_wasm_path())
            .expect("greeter fixture must be built (see test-components/greeter's own build step)");
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let owner = node_wide_caller("owner");

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: Some(WitHealthCheck::Rpc(WitRpcProbe {
                    interface_name: GREETER_INTERFACE_NAME.to_string(),
                    method: "greet".to_string(),
                    timeout_ms: 2000,
                })),
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wasm_bytes),
                hash: None,
                interfaces: vec![GREETER_INTERFACE_NAME.to_string()],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("greeter-svc".to_string(), manifest, &owner).await.unwrap();

        let status = service.status(vec!["greeter-svc".to_string()], &owner).await.unwrap();
        assert_eq!(status.services.len(), 1);
        assert!(
            matches!(&status.services[0].probe, ProbeStatus::Failing(d) if d.contains("missing required parameter")),
            "{:?}",
            status.services[0].probe
        );
    }

    /// The genuine happy path N-1 found missing: a real `rpc` probe against
    /// a method that takes no arguments must report `Passing`.
    /// `stream-test`'s `get-uploaded-content` (its own `test-driver`
    /// interface) takes none and is side-effect-free against an empty
    /// store.
    #[tokio::test]
    async fn an_rpc_probe_passes_for_a_method_that_takes_no_arguments() {
        let wasm_bytes = fs::read(stream_test_wasm_path()).expect(
            "stream-test fixture must be built (see test-components/stream-test's own build step)",
        );
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let owner = node_wide_caller("owner");

        let manifest = DeployManifest {
            config: ServiceConfig {
                env: vec![],
                args: vec![],
                custom_config: None,
                quota: None,
                schema: None,
                rotation_policy: None,
                fdae_policy: None,
                health_check: Some(WitHealthCheck::Rpc(WitRpcProbe {
                    interface_name: STREAM_TEST_DRIVER_INTERFACE.to_string(),
                    method: "get-uploaded-content".to_string(),
                    timeout_ms: 2000,
                })),
                assets: None,
                visibility: None,
            },
            service_type: WitServiceType::Wasm(WasmManifest {
                source: ArtifactSource::Binary(wasm_bytes),
                hash: None,
                interfaces: vec![STREAM_TEST_DRIVER_INTERFACE.to_string()],
            }),
            registry_certificate: None,
            instance_certificate: None,
        };
        service.deploy("stream-test-svc".to_string(), manifest, &owner).await.unwrap();

        let status = service.status(vec!["stream-test-svc".to_string()], &owner).await.unwrap();
        assert_eq!(status.services.len(), 1);
        assert!(
            matches!(status.services[0].probe, ProbeStatus::Passing),
            "{:?}",
            status.services[0].probe
        );
    }

    /// M05A A5c §19.13 / D-A5c-12: the health-poll-cost budget, measured
    /// **before** the resident loop exists so `poll_interval_secs`'s
    /// default is chosen from this number rather than defended after it.
    /// "One in-process node" (one real `ControlPlaneService`, dispatched
    /// directly -- no client, no network) with 20 real `rpc`-probed wasm
    /// services, all cache-missing on this, their first sweep --
    /// `probe_cached`'s 5s minimum interval means every sweep at the
    /// default 30s `poll_interval_secs` pays this cost, which is the
    /// number this finding says is at risk. A4-05 already runs every
    /// target's probe concurrently, so this also pins that the batching
    /// holds at 20 rather than degrading linearly.
    ///
    /// Budget, set a priori: **under 2s** for the whole pass. The other
    /// two numbers in the finding (**at most 2 RPCs per substrate**, and
    /// **under 5% of one core**) are not asserted here: the RPC count is
    /// this test's own shape by construction -- one `status` call for all
    /// 20 ids, exactly what a supervisor's sweep issues, with the second
    /// RPC (`app-instance-management-of`) being an O(1) generation read
    /// unrelated to service count. That second RPC's own "exactly one
    /// call per substrate, not per service" half is a separate,
    /// dedicated regression test at the call site (review finding C-3:
    /// this doc used to claim that without one existing --
    /// `max_held_generation_from_clients_calls_held_generation_once_per_alias`,
    /// `crates/app_supervisor/src/service.rs`, drives
    /// `SupervisorService::max_held_generation_from_clients` against a
    /// counting fake and pins the call count directly). CPU-percent is
    /// not portably measurable from a `#[tokio::test]` -- a wall-clock
    /// budget comfortably under 2s on a shared thread pool is the proxy
    /// for it here, with `mise run bench:poll-cost` re-running this same
    /// test with its duration printed for a repeatable number outside
    /// the pass/fail assertion.
    #[tokio::test]
    async fn a_steady_state_sweep_of_twenty_services_stays_within_the_poll_budget() {
        let wasm_bytes = fs::read(stream_test_wasm_path()).expect(
            "stream-test fixture must be built (see test-components/stream-test's own build step)",
        );
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;
        let owner = node_wide_caller("owner");

        let mut service_ids = Vec::new();
        for i in 0..20 {
            let id = format!("budget-svc-{i}");
            let manifest = DeployManifest {
                config: ServiceConfig {
                    env: vec![],
                    args: vec![],
                    custom_config: None,
                    quota: None,
                    schema: None,
                    rotation_policy: None,
                    fdae_policy: None,
                    health_check: Some(WitHealthCheck::Rpc(WitRpcProbe {
                        interface_name: STREAM_TEST_DRIVER_INTERFACE.to_string(),
                        method: "get-uploaded-content".to_string(),
                        timeout_ms: 2000,
                    })),
                    assets: None,
                    visibility: None,
                },
                service_type: WitServiceType::Wasm(WasmManifest {
                    source: ArtifactSource::Binary(wasm_bytes.clone()),
                    hash: None,
                    interfaces: vec![STREAM_TEST_DRIVER_INTERFACE.to_string()],
                }),
                registry_certificate: None,
                instance_certificate: None,
            };
            service.deploy(id.clone(), manifest, &owner).await.unwrap();
            service_ids.push(id);
        }

        let start = std::time::Instant::now();
        let status = service.status(service_ids, &owner).await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(status.services.len(), 20);
        for s in &status.services {
            assert!(matches!(s.probe, ProbeStatus::Passing), "{:?}", s.probe);
        }
        eprintln!("D-A5c-12 poll-cost budget: 20-service sweep took {elapsed:?} (budget 2s)");
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "a 20-service sweep took {elapsed:?}, over the 2s budget (D-A5c-12)"
        );
    }

    #[tokio::test]
    async fn a_probe_result_is_cached_within_the_minimum_interval() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        // See the comment in `a_probe_runs_for_a_tcp_service_whose_phase_is_unknown`:
        // no accept loop needed, and a blocking `accept()` inside
        // `tokio::spawn` on this single-threaded test runtime would starve
        // the runtime instead of servicing connections.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        service
            .registry
            .register(
                "cached-probe-svc".to_string(),
                "main".to_string(),
                SubstrateEndpoint::TcpHostPort { host: "127.0.0.1".to_string(), port },
            )
            .await
            .unwrap();
        service
            .registry
            .set_deploy_facts(
                "cached-probe-svc".to_string(),
                "tcp".to_string(),
                Some(
                    serde_json::to_string(&WitHealthCheck::TcpConnect(WitTcpProbe {
                        interface_name: "main".to_string(),
                        timeout_ms: 2000,
                    }))
                    .unwrap(),
                ),
                None,
                None,
            )
            .await
            .unwrap();

        let now = 1_000_000;
        let (first, first_at) = service.probe_cached("cached-probe-svc", now).await;
        assert!(matches!(first, ProbeStatus::Passing));
        // Within the interval: served from cache, same `checked_at`.
        let (second, second_at) = service.probe_cached("cached-probe-svc", now + 1).await;
        assert!(matches!(second, ProbeStatus::Passing));
        assert_eq!(first_at, second_at);
    }

    fn test_signed_record(service_id: &str, is_private: bool) -> String {
        serde_json::to_string(&SignedEndpointInfo {
            info: syneroym_core::dht_registry::EndpointInfo {
                service_id: service_id.to_string(),
                substrate_id: "test-node".to_string(),
                endpoint_type: syneroym_core::dht_registry::EndpointType::Service,
                mechanisms: vec![],
                nickname: None,
                is_private,
                ttl: None,
                not_after: 0,
                generation: 0,
            },
            pkarr_packet_hex: "abcd".to_string(),
        })
        .unwrap()
    }

    #[test]
    fn validate_publication_public_with_matching_record_succeeds() {
        let record = test_signed_record("test-svc", false);
        let res = validate_publication("test-svc", Some(WitVisibility::Public), Some(&record));
        assert_eq!(res, Ok(AppVisibility::Public));
    }

    #[test]
    fn validate_publication_public_without_certificate_fails() {
        let res = validate_publication("test-svc", Some(WitVisibility::Public), None);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("no registry certificate was supplied"));
    }

    #[test]
    fn validate_publication_public_with_mismatched_service_id_fails() {
        let record = test_signed_record("other-svc", false);
        let res = validate_publication("test-svc", Some(WitVisibility::Public), Some(&record));
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("names service 'other-svc'"));
    }

    #[test]
    fn validate_publication_public_with_is_private_true_fails() {
        let record = test_signed_record("test-svc", true);
        let res = validate_publication("test-svc", Some(WitVisibility::Public), Some(&record));
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("is_private=true"));
    }

    #[test]
    fn validate_publication_internal_with_matching_record_succeeds() {
        let record = test_signed_record("test-svc", true);
        let res = validate_publication("test-svc", Some(WitVisibility::Internal), Some(&record));
        assert_eq!(res, Ok(AppVisibility::Internal));
    }

    #[test]
    fn validate_publication_internal_without_certificate_fails() {
        let res = validate_publication("test-svc", Some(WitVisibility::Internal), None);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("no registry certificate was supplied"));
    }

    #[test]
    fn validate_publication_internal_with_is_private_false_fails() {
        let record = test_signed_record("test-svc", false);
        let res = validate_publication("test-svc", Some(WitVisibility::Internal), Some(&record));
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("is_private=false"));
    }

    #[test]
    fn validate_publication_private_without_certificate_succeeds() {
        let res1 = validate_publication("test-svc", Some(WitVisibility::Private), None);
        assert_eq!(res1, Ok(AppVisibility::Private));
        let res2 = validate_publication("test-svc", None, None);
        assert_eq!(res2, Ok(AppVisibility::Private));
    }

    #[test]
    fn validate_publication_private_with_certificate_fails() {
        let record = test_signed_record("test-svc", false);
        let res = validate_publication("test-svc", Some(WitVisibility::Private), Some(&record));
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .contains("declares visibility 'private' but a registry certificate was supplied")
        );
    }

    #[test]
    fn validate_publication_malformed_certificate_fails() {
        let res =
            validate_publication("test-svc", Some(WitVisibility::Public), Some("not valid json"));
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("does not parse"));
    }

    /// The guard both `deploy_with_context` and `undeploy_impl` call before
    /// joining `service_id` into a stored-record filename. A real DID never
    /// trips any of the rejected shapes.
    #[test]
    fn is_safe_service_id_for_path_rejects_traversal_and_admits_a_real_did() {
        assert!(!is_safe_service_id_for_path(""));
        assert!(!is_safe_service_id_for_path("../escaped"));
        assert!(!is_safe_service_id_for_path("a/b"));
        assert!(!is_safe_service_id_for_path("a\\b"));
        assert!(is_safe_service_id_for_path("did:key:z6MkExample"));
    }

    /// Test 36 / `D-B2-5`: a public service redeployed as `private` clears
    /// its stored record file -- otherwise the substrate keeps republishing
    /// the old record on every heartbeat sweep for up to `not_after` (F2).
    #[tokio::test]
    async fn a_private_redeploy_removes_the_stored_endpoint_record_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let service = service_for_inline_tests(temp_dir.path()).await;

        let mut public_manifest = tcp_manifest_with(9, None);
        public_manifest.config.visibility = Some(WitVisibility::Public);
        public_manifest.registry_certificate = Some(test_signed_record("stale-record-svc", false));
        service
            .deploy("stale-record-svc".to_string(), public_manifest, &node_wide_caller("owner"))
            .await
            .unwrap();

        let cert_path = temp_dir.path().join("stale-record-svc.json");
        assert!(cert_path.exists(), "the record file must exist after a public deploy");

        let mut private_manifest = tcp_manifest_with(9, None);
        private_manifest.config.visibility = Some(WitVisibility::Private);
        service
            .deploy("stale-record-svc".to_string(), private_manifest, &node_wide_caller("owner"))
            .await
            .unwrap();

        assert!(
            !cert_path.exists(),
            "redeploying as private must remove the stale record file, or the heartbeat sweep \
             keeps republishing it"
        );
    }
}

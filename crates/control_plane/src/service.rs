//! Orchestrator control service implementation
//!
//! Handles requests for registering, deploying, listing, and destroying
//! sandbox instances or services running on the node.

use std::{
    fmt,
    fmt::{Debug, Formatter},
    fs,
    path::PathBuf,
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};

use anyhow::{Context, Result};
use dashmap::DashMap;
use reqwest::{Client, redirect::Policy};
use serde_json::Value;
use syneroym_core::{
    asset_manifest::AssetRegistry,
    endpoint_publisher::EndpointPublisher,
    http_routes::{HttpRouteRegistry, SsePermitRegistry},
    local_registry::EndpointRegistry,
    record_signer::NodeRecordSigner,
};
use syneroym_data_blob::BlobProvider;
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_identity::Identity;
use syneroym_mqtt_broker::MqttBroker;
use syneroym_rpc::{
    Ability, CallerContext, NativeDispatchRegistry, NativeInvocation, NativeResponse,
    NativeService, PERMISSION_DENIED_CODE, ProxyQueueInspector, ResourceUri, RowAuthorizer,
    RpcError, RpcResult, ServiceProxy, WeakNativeDispatchRegistry, empty_row_authorizer,
};
use syneroym_wit_interfaces::control_plane::exports::syneroym::control_plane::orchestrator::{
    BindingWrite, DeployManifest, DeploymentPlan, ProbeStatus,
};
use tokio::sync::{mpsc, oneshot};
use tracing::info;

use crate::{
    dummy_sandbox::{AppSandboxEngine, ContainerEngine},
    synsvc_native::empty_conversation_host,
};

mod dispatch;
pub(crate) mod orchestration;

#[cfg(test)]
mod tests;

pub use orchestration::OrchestratorInterface;

pub(super) const ORCHESTRATOR_INTERFACE: &str = "orchestrator";
pub(super) const SECURITY_INTERFACE: &str = "security";

/// The `native_dispatch` key the supervisor role's own `NativeService`
/// registers under (`syneroym_substrate::runtime::init_supervisor`), fixed
/// and independent of any node's own DID. `deploy_with_context` refuses a
/// caller-supplied `service_id` equal to this: `open_service_db` and
/// `native_dispatch` both key on a bare `service_id` with no reservation of
/// their own, so an unreserved deploy under this name would open the
/// supervisor's vault (leaking every member master key it holds) and
/// overwrite the dispatch entry `init_supervisor` registered, taking over
/// every later `supervisor://` call.
pub const SUPERVISOR_RESERVED_SERVICE_ID: &str = "supervisor";
pub const AUTH_RESERVED_SERVICE_ID: &str = syneroym_core::protocol_utils::AUTH_SERVICE_ALIAS;

/// The Substrate Service (The Control Plane Orchestrator)
/// This service handles the deployment and lifecycle of applications
/// (`SynApps`) within the substrate. It interacts with sandbox environments
/// like Podman or Wasmtime.
pub struct ControlPlaneService {
    pub(crate) service_id: String,
    /// This node's own DID; `substrate:<node_did>` resources name it.
    /// Distinct field from `service_id` above (which happens to
    /// hold the same value in production) because it is used as an
    /// *identity*, not a routing key -- mirrors `RouteHandlerInner::node_did`.
    pub(crate) node_did: String,
    pub(crate) registry: EndpointRegistry,
    pub(crate) app_sandbox_engine: Arc<AppSandboxEngine>,
    pub(crate) podman_sandbox_engine: Arc<ContainerEngine>,
    pub(crate) hosted_apps_dir: PathBuf,
    pub(crate) key_store: Arc<KeyStore>,
    pub(crate) storage_provider: Arc<dyn StorageProvider>,
    pub(crate) blob_provider: Arc<dyn BlobProvider>,
    pub(crate) messaging_broker: Arc<MqttBroker>,
    /// This node's own signing identity -- threaded to each deployed
    /// service's `SynSvcNativeService` so it can sign relationship-proof
    /// records as this node's asserter DID
    /// (`node_identity.to_doc(..).id`). Distinct from `node_did` above,
    /// which is only the DID string; this carries the actual key material.
    pub(crate) node_identity: Arc<Identity>,
    /// The write side of intra-app dependency resolution (ADR-0021 §2).
    /// `deploy` writes a binding's persisted row and its in-memory topology
    /// through this in one step (`LogicalResolver::register`), so a
    /// scale-out is never left serving a stale cached topology for up to
    /// `cache_ttl`.
    /// Shared with `AppSandboxEngine`'s read side over the same
    /// `StaticInventory` -- one registry, one resolver, two holders.
    pub(crate) logical_resolver: Arc<syneroym_app_orchestration::LogicalResolver>,
    /// The Universal Proxy, for the cross-service relationship-proof fetch,
    /// threaded on into each deployed service's `SynSvcNativeService`. `pub`
    /// and a post-construction `OnceLock`, mirroring
    /// `AppSandboxEngine.service_proxy`
    /// exactly (`crates/sandbox_wasm/src/engine.rs`): `ProxyRouter` (the
    /// only implementation) is constructed in `RouteHandler::init`, which
    /// runs *after* this service (`RouteHandlerDeps` already holds it), so
    /// there is no construction-time value to inject -- `RouteHandler::init`
    /// calls `.set(...)` on this field the same way it already does for
    /// `AppSandboxEngine`'s.
    pub service_proxy: OnceLock<Weak<dyn ServiceProxy>>,
    /// Read-and-replay access to the durable proxy queues, for the
    /// `proxy-*` operator verbs. Wired the same post-construction way, and
    /// for the same ordering reason, as `service_proxy` above: the router
    /// owns the queues and is built after this service.
    pub proxy_queues: OnceLock<Weak<dyn ProxyQueueInspector>>,
    /// The stage-4 ABAC after-step invoker (ADR-0017 §7), threaded on into
    /// each deployed service's `SynSvcNativeService` exactly like
    /// `service_proxy` above -- same reason (`AppSandboxEngine`, the sole
    /// implementation, is constructed in `RouteHandler::init`, which runs
    /// after this service) and same two-phase `OnceLock` wiring.
    pub row_authorizer: OnceLock<Weak<dyn RowAuthorizer>>,
    /// The Conversation service, threaded on to each
    /// deployed service's `SynSvcNativeService` at construction time
    /// (`orchestration.rs`'s two `SynSvcNativeService::new` call sites).
    /// Unlike `service_proxy`/`row_authorizer`, `ConversationService`
    /// exists *before* `ControlPlaneService` does (`build_route_handler_deps`
    /// builds it first), so this could have been a constructor parameter --
    /// kept as an `OnceLock` anyway, to avoid a
    /// parameter on `SynSvcNativeService::new` itself: consistency with the
    /// other two fields this struct already threads the same way.
    pub conversation: OnceLock<Weak<dyn syneroym_rpc::ConversationHost>>,
    /// The node's record signer (`syneroym:signing`).
    pub record_signer: OnceLock<Arc<NodeRecordSigner>>,
    /// Set after construction by the substrate's composition root. A setter
    /// rather than an `init` parameter because `init` has many call sites,
    /// almost all of them tests with nothing to publish. Same two-phase
    /// wiring `service_proxy` already uses, for the same ordering reason --
    /// unlike that field, this one is a strong `Arc`: `EndpointPublisher`
    /// holds no reference back to `ControlPlaneService`, so there is no
    /// cycle to guard against with a `Weak`.
    pub(crate) endpoint_publisher: OnceLock<Arc<EndpointPublisher>>,
    /// Signals the substrate's background registry-heartbeat loop
    /// (`publish_to_community_registry` in `crates/substrate/src/runtime.rs`)
    /// to publish this node's own endpoint record and every hosted
    /// service's record right now instead of waiting up to
    /// `HEARTBEAT_INTERVAL_SECS` (1h). Wired the same two-phase,
    /// post-construction way as `endpoint_publisher` and for the same
    /// reason (`init` has many test call sites with no registry to
    /// publish to). A request carries its own reply channel because the
    /// loop, not this service, holds the node's own secret key and
    /// endpoint address -- `endpoint_publisher` alone can only replay
    /// *hosted* records, not re-sign this substrate's own.
    pub(crate) republish_trigger: OnceLock<mpsc::Sender<oneshot::Sender<Result<()>>>>,
    // `Weak`, not `NativeDispatchRegistry` -- see the cycle explained in
    // `syneroym_rpc::dispatch_registry`'s module docs. `RouteHandlerInner`
    // owns the strong clone for as long as the router itself is alive.
    pub(crate) native_dispatch: WeakNativeDispatchRegistry,
    // Strong, unlike `native_dispatch`: `ControlPlaneService` is never
    // itself keyed into this map, so there is no reference-cycle hazard
    // (contrast `syneroym_rpc::dispatch_registry`'s module docs, which
    // explain why `native_dispatch` above can't use the same plain-`Arc`
    // approach). `RouteHandlerInner` holds the same `Arc` (the type lives in
    // `syneroym_core::http_routes`) for lookup from
    // `crates/router/src/route_handler/http.rs`.
    pub(crate) http_routes: HttpRouteRegistry,
    /// Static asset manifests, per service. Same `Arc`, same
    /// producer/consumer split, and same cache-not-persistence lifecycle as
    /// `http_routes` above -- `RouteHandlerInner` holds the identical `Arc`
    /// for lookup from `crates/router/src/route_handler/http.rs`.
    pub(crate) assets: AssetRegistry,
    /// Service ids this *process* has run a full `deploy_with_context`
    /// for. The three route tables (`native_dispatch`, `http_routes`,
    /// `assets`) are process-local and start empty on every boot; nothing
    /// rehydrates them from the persisted catalog, and the sandbox warm-up
    /// restores only the WASM instance. So a redeploy right after a
    /// substrate restart -- matching persisted `manifest_hash`, warmed
    /// `Running` instance -- must not dedup into a no-op, or guest
    /// `POST /rpc` and every native-capability call stay unrouted. An
    /// entry here is the witness that the routing for this service was
    /// registered *by this process*; a fresh boot has none, so its
    /// redeploy falls through and re-registers. Populated at the end of a
    /// successful deploy, cleared by `undeploy`.
    pub(crate) full_deploy_completed: DashMap<String, ()>,
    /// Bounded concurrent SSE subscriptions per service.
    pub(crate) sse_permits: SsePermitRegistry,
    /// Last probe result per service, `(checked_at_secs, ProbeStatus)`. A
    /// supervisor polling every few seconds must not turn into probe
    /// load on the target (the health-poll-cost budget), and a
    /// wasm `rpc` probe costs a component instantiation. Entries are dropped
    /// on undeploy.
    pub(crate) probe_cache: DashMap<String, (u64, ProbeStatus)>,
    /// The declared readiness probe's own `http-get` client (A4-08/A4-12):
    /// built once here rather than per probe, and with redirects disabled --
    /// a readiness check has no reason to follow one, and a hostile or
    /// compromised container answering with a 3xx must not make this
    /// substrate issue a request to a target of its own choosing. The
    /// per-probe deadline is applied per call with `tokio::time::timeout`,
    /// matching the other two probe kinds, rather than on the client itself.
    pub(crate) http_probe_client: Client,
}

impl Debug for ControlPlaneService {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlPlaneService")
            .field("service_id", &self.service_id)
            .finish_non_exhaustive()
    }
}

impl ControlPlaneService {
    #[allow(clippy::too_many_arguments)]
    pub async fn init(
        service_id: String,
        node_did: String,
        app_sandbox_engine: Arc<AppSandboxEngine>,
        podman_sandbox_engine: Arc<ContainerEngine>,
        registry: EndpointRegistry,
        hosted_apps_dir: PathBuf,
        key_store: Arc<KeyStore>,
        storage_provider: Arc<dyn StorageProvider>,
        blob_provider: Arc<dyn BlobProvider>,
        messaging_broker: Arc<MqttBroker>,
        native_dispatch: NativeDispatchRegistry,
        http_routes: HttpRouteRegistry,
        assets: AssetRegistry,
        node_identity: Arc<Identity>,
        logical_resolver: Arc<syneroym_app_orchestration::LogicalResolver>,
    ) -> Result<Self> {
        info!("Initializing ControlPlaneService (Orchestrator)...");

        if !hosted_apps_dir.exists() {
            fs::create_dir_all(&hosted_apps_dir)?;
        }

        Ok(Self {
            service_id,
            node_did,
            registry,
            app_sandbox_engine,
            podman_sandbox_engine,
            hosted_apps_dir,
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            node_identity,
            logical_resolver,
            service_proxy: OnceLock::new(),
            proxy_queues: OnceLock::new(),
            row_authorizer: OnceLock::new(),
            conversation: OnceLock::new(),
            record_signer: OnceLock::new(),
            endpoint_publisher: OnceLock::new(),
            republish_trigger: OnceLock::new(),
            native_dispatch: Arc::downgrade(&native_dispatch),
            http_routes,
            assets,
            full_deploy_completed: DashMap::new(),
            sse_permits: Arc::new(DashMap::new()),
            probe_cache: DashMap::new(),
            http_probe_client: Client::builder()
                .redirect(Policy::none())
                .build()
                .context("failed to build the HTTP probe client")?,
        })
    }

    /// Access to the per-service SSE subscription permit registry.
    #[must_use]
    pub fn sse_permits(&self) -> SsePermitRegistry {
        self.sse_permits.clone()
    }

    /// Wires in the substrate's `EndpointPublisher` so `deploy` can publish
    /// an endpoint record immediately instead of waiting for the
    /// next heartbeat. A no-op past the first call -- `OnceLock::set` simply
    /// returns `Err`, which is discarded, mirroring `service_proxy`'s and
    /// `row_authorizer`'s two-phase wiring.
    pub fn set_endpoint_publisher(&self, publisher: Arc<EndpointPublisher>) {
        let _ = self.endpoint_publisher.set(publisher);
    }

    /// Wires in the channel that reaches the substrate's registry-heartbeat
    /// loop, so `republish_now` has somewhere to send a request. Same
    /// two-phase, no-op-past-first-call wiring as `set_endpoint_publisher`.
    pub fn set_republish_trigger(&self, tx: mpsc::Sender<oneshot::Sender<Result<()>>>) {
        let _ = self.republish_trigger.set(tx);
    }

    pub fn set_record_signer(&self, signer: Arc<NodeRecordSigner>) {
        let _ = self.record_signer.set(signer);
    }

    /// Forces the registry-heartbeat loop to publish this node's own
    /// endpoint record and every hosted service's record right now, rather
    /// than waiting up to `HEARTBEAT_INTERVAL_SECS` (1h) for the next
    /// scheduled pass -- e.g. after a registry this node's records were
    /// wiped from (an in-memory community registry that itself restarted)
    /// comes back up. Errs if no community registry is configured for this
    /// node (`set_republish_trigger` never called), the loop is not
    /// running, or it does not reply within 30s -- generous relative to the
    /// loop's own worst case (30 registration attempts at 500ms apart).
    pub async fn republish_now(&self) -> Result<()> {
        let tx = self
            .republish_trigger
            .get()
            .ok_or_else(|| anyhow::anyhow!("no community registry is configured for this node"))?;
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(reply_tx)
            .await
            .map_err(|_| anyhow::anyhow!("the registry-heartbeat loop is not running"))?;
        tokio::time::timeout(Duration::from_secs(30), reply_rx)
            .await
            .map_err(|_| anyhow::anyhow!("republish timed out waiting for the heartbeat loop"))?
            .map_err(|_| anyhow::anyhow!("the registry-heartbeat loop dropped the reply"))?
    }

    /// Never-constructed marker type coerced to an unsized `Weak<dyn
    /// ServiceProxy>` -- mirrors `sandbox_wasm::host_capabilities::
    /// empty_service_proxy` exactly, duplicated here (not shared) since
    /// `control_plane` cannot depend on `sandbox_wasm` unconditionally (the
    /// `app_sandbox` feature is optional). Used only before
    /// `RouteHandler::init` has called `service_proxy.set(...)` -- a real
    /// fetch attempted against it fails closed via `.upgrade()` returning
    /// `None`, never a panic.
    fn empty_service_proxy() -> Weak<dyn ServiceProxy> {
        #[derive(Debug)]
        struct NeverConstructed;
        #[async_trait::async_trait]
        impl ServiceProxy for NeverConstructed {
            async fn invoke(
                &self,
                _request: syneroym_rpc::ProxyRequest,
            ) -> Result<Value, syneroym_rpc::ProxyError> {
                unreachable!("NeverConstructed is only used to type an empty Weak; never upgraded")
            }
        }
        Weak::<NeverConstructed>::new()
    }

    /// The current `Weak<dyn ServiceProxy>`, or an always-empty one if
    /// `RouteHandler::init` hasn't populated `service_proxy` yet (a
    /// substrate startup ordering that never lasts past the first deploy in
    /// practice, but is a real possibility for a test harness that
    /// constructs `ControlPlaneService` without a full `RouteHandler`).
    pub(crate) fn current_service_proxy(&self) -> Weak<dyn ServiceProxy> {
        self.service_proxy.get().cloned().unwrap_or_else(Self::empty_service_proxy)
    }

    /// The current `Weak<dyn RowAuthorizer>`, or an always-empty one if
    /// `RouteHandler::init` hasn't populated `row_authorizer` yet -- mirrors
    /// `current_service_proxy` exactly. Unlike `empty_service_proxy` above,
    /// reuses `syneroym_rpc::empty_row_authorizer` directly rather than a
    /// locally duplicated marker type: `RowAuthorizer` lives in
    /// `syneroym-rpc`, a dependency `control_plane` already carries
    /// unconditionally (unlike `sandbox_wasm`, which is behind the optional
    /// `app_sandbox` feature).
    pub(crate) fn current_row_authorizer(&self) -> Weak<dyn RowAuthorizer> {
        self.row_authorizer.get().cloned().unwrap_or_else(empty_row_authorizer)
    }

    /// The current `Weak<dyn ConversationHost>`, or an always-empty one if
    /// unset -- mirrors `current_row_authorizer` exactly.
    pub(crate) fn current_conversation(&self) -> Weak<dyn syneroym_rpc::ConversationHost> {
        self.conversation.get().cloned().unwrap_or_else(empty_conversation_host)
    }

    /// Whether `caller` holds a specific **node-wide** ability -- the
    /// substrate owner, whose `substrate/admin` entails every ability. Used
    /// both for the `orchestrator/*` abilities below and, with
    /// `Ability::SUBSTRATE_ADMIN` itself, to gate the `security` interface.
    /// There is deliberately no "is the substrate owned?"
    /// branch anywhere else, because ownership is expressed as an issued
    /// capability, not as a skipped check. An unowned
    /// substrate holds no node-wide capability at all: it
    /// fails closed rather than granting every verified caller
    /// `orchestrator/*` the way the old bootstrap posture did.
    ///
    /// **Parameterized by `ability`, not hardcoded to one.** The three
    /// `orchestrator/*` abilities are flat and independently grantable
    /// ("deploy but not undeploy" must stay expressible), so a future
    /// grantee could hold `orchestrator/status` alone. Checking a single
    /// hardcoded ability here for every caller-side use -- deploy's takeover
    /// override, undeploy's gate, and list's visibility -- would let a
    /// *read-only, status-only* grantee also override another owner's
    /// deploy/undeploy, a privilege escalation once such a partial grant can
    /// be minted. Each call site below must pass the ability it actually
    /// needs to exercise -- `ORCHESTRATOR_DEPLOY` to override a takeover,
    /// `ORCHESTRATOR_UNDEPLOY` to override an undeploy gate,
    /// `ORCHESTRATOR_STATUS` for list's broader visibility bar (a
    /// monitoring-only grantee is meant to see the list; it is not thereby
    /// meant to deploy/undeploy over someone else's app).
    ///
    /// The resource is the **bare** `substrate:<node_did>` -- node-wide.
    /// That excludes an app-scoped grantee (`substrate:<node>/app/foo`):
    /// their capability carries a selector, so it is not `is_substrate_scope`
    /// (`ResourceUri::is_substrate_scope` excludes selector-bearing
    /// resources). They are prefix-covered by `covers_resource` instead, at
    /// each gate's own selectored resource check
    /// (deploy/undeploy/per-service readyz).
    pub(super) fn has_node_wide_ability(
        &self,
        caller: &CallerContext,
        ability: &'static str,
    ) -> bool {
        caller
            .has_capability(&ResourceUri::substrate(&self.node_did), &Ability(ability.to_string()))
    }
}

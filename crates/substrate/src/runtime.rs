//! Substrate execution runtime
//!
//! Manages the lifecycle of all substrate components including the App Sandbox,
//! Observability engine, Router, Client Gateway, and Coordinators.

use std::{
    collections::HashMap,
    fmt::{self, Debug, Formatter},
    future,
    future::Future,
    pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{Json, Router, routing};
use dashmap::DashMap;
use iroh::EndpointAddr;
use syneroym_app_orchestration::{
    AppInstanceId, AppRegistry, LogicalResolver, LogicalServiceName, StaticInventory, TopologyEntry,
};
use syneroym_client_gateway::ClientGateway;
use syneroym_community_registry::EcosystemRegistry;
use syneroym_control_plane::ControlPlaneService;
use syneroym_coordinator::EcosystemCoordinator;
use syneroym_core::{
    config::{BlobBackend, SubstrateConfig},
    dht_registry::{
        DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointMechanism, EndpointType,
        HEARTBEAT_INTERVAL_SECS, RegistryClient, SignedEndpointInfo,
    },
    endpoint_publisher::EndpointPublisher,
    http_routes::HttpRouteRegistry,
    local_registry::{EndpointRegistry, SubstrateEndpoint},
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, registry_store, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{Identity, substrate::SubstrateIdentityStatus};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_observability::{MemoryRecorder, MetricsSnapshot, ObservabilityEngine};
use syneroym_router::{ConnectionRouter, RouteHandlerDeps};
use syneroym_rpc::{NativeDispatchRegistry, NativeService};
use syneroym_sandbox_podman::ContainerEngine;
use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::{net::TcpListener, signal, task::JoinHandle, time};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::identity;

/// Runs the substrate given the consolidated configuration, using the default
/// ctrl-c shutdown signal.
pub async fn run(config: SubstrateConfig) -> anyhow::Result<()> {
    init_and_run_with_signal(config, async {
        let _ = signal::ctrl_c().await;
    })
    .await
}

pub struct InitializedRuntime {
    pub observability: ObservabilityEngine,
    pub services: RuntimeServices,
    pub connection_router: ConnectionRouter,
    /// The same registry `connection_router` routes through -- kept here too
    /// so `RuntimeServices::run_until_shutdown` can run the instance-
    /// certificate expiry sweep without `ConnectionRouter` growing a getter
    /// for something external to routing.
    pub endpoint_registry: EndpointRegistry,
}

impl Debug for InitializedRuntime {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("InitializedRuntime")
            .field("observability", &"ObservabilityEngine")
            .field("services", &self.services)
            .field("connection_router", &"ConnectionRouter")
            .field("endpoint_registry", &"EndpointRegistry")
            .finish()
    }
}

/// Runs the substrate given the consolidated configuration and a custom
/// shutdown signal.
pub async fn init_and_run_with_signal<F>(
    config: SubstrateConfig,
    shutdown_signal: F,
) -> anyhow::Result<()>
where
    F: Future<Output = ()>,
{
    let runtime = init(config.clone()).await?;
    run_with_signal(config, runtime, shutdown_signal).await
}

/// Runs the substrate given the consolidated configuration and a custom
/// shutdown signal.
pub async fn run_with_signal<F>(
    config: SubstrateConfig,
    mut runtime: InitializedRuntime,
    shutdown_signal: F,
) -> anyhow::Result<()>
where
    F: Future<Output = ()>,
{
    runtime
        .services
        .run_until_shutdown(
            &config,
            &runtime.connection_router,
            &runtime.endpoint_registry,
            shutdown_signal,
        )
        .await;

    info!("shutting down substrate components");
    runtime.services.shutdown().await;

    if let Err(error) = runtime.observability.shutdown().await {
        error!(error = %error, "error flushing observability data");
    }

    if let Err(error) = runtime.connection_router.shutdown().await {
        error!(error = %error, "error shutting down connection router");
    }

    info!("shutdown complete");
    Ok(())
}

/// Runs the substrate given the consolidated configuration and a custom
/// shutdown signal.
pub async fn init(config: SubstrateConfig) -> anyhow::Result<InitializedRuntime> {
    info!(profile = %config.profile, "initializing substrate");

    let observability = ObservabilityEngine::init(&config)?;
    // `community_registry`/`coordinator`/`client_gateway` construct first,
    // exactly as before this role existed -- swapping this order (tried
    // first, reverted) measurably slowed every substrate's startup, ~6s
    // to 15-30s against the same config, even with `[roles.supervisor]`
    // absent. `RuntimeServices::init` and `setup_connection_router` both
    // do real network setup (iroh endpoint bring-up, DHT bootstrap), and
    // apparently benefit from *this* relative order in ways not worth
    // taking on faith a second time. The supervisor role, constructed
    // inside `setup_connection_router` because it needs handles
    // (`KeyStore`, `StorageProvider`, `native_dispatch`, the node's own
    // identity) that only exist once the connection router's own
    // composition root has built them, is injected into `RuntimeServices`
    // afterward instead of changing when either call runs.
    let mut services = RuntimeServices::init(&config).await?;
    let (connection_router, endpoint_registry, supervisor) =
        setup_connection_router(&config).await?;
    services.set_supervisor(supervisor);

    Ok(InitializedRuntime { observability, services, connection_router, endpoint_registry })
}

pub struct RuntimeServices {
    #[cfg(feature = "community_registry")]
    community_registry: Option<EcosystemRegistry>,
    #[cfg(feature = "coordinator")]
    coordinator: Option<EcosystemCoordinator>,
    #[cfg(feature = "client_gateway")]
    client_gateway: Option<ClientGateway>,
    supervisor: Option<Arc<SupervisorHandle>>,
    /// M05A A5c §19.8/D-A5c-8: the supervisor's resident loop is spawned
    /// (not pinned in `run_until_shutdown`'s own `select!`), so it
    /// outlives that function's stack frame -- `shutdown` cancels the
    /// loop's token and awaits this handle, rather than relying on a
    /// token nothing would ever join in time. Populated by
    /// `run_until_shutdown`, not `init`, so A5b's startup-ordering note
    /// (the loop starts only after both composition calls have already
    /// run) stays true.
    supervisor_join: Option<JoinHandle<anyhow::Result<()>>>,
    /// The durable outbox worker's own task (M05B B1, D-B1-1), spawned and
    /// raced the same way `supervisor_join` is. **Not** awaited in
    /// `shutdown` (D-B1-8): the resident loop's join proves an in-flight
    /// pass finished closing its clients, but the worker's own in-flight
    /// deliveries are deliberately abandoned -- their visibility timeout
    /// returns them to `Pending` on the next start, and waiting for a
    /// delivery against a substrate that is offline (the exact case this
    /// queue exists for) would make shutdown itself hang on the very
    /// condition it is meant to survive.
    queue_worker_join: Option<JoinHandle<anyhow::Result<()>>>,
    /// The guest proxy outbox worker's task. Constructed only when this
    /// node has a Universal Proxy with a durable outbox behind it -- the
    /// same condition that makes a guest, and therefore the queue's only
    /// producer, possible at all.
    ///
    /// Raced beside the others and, like `queue_worker_join`, **not**
    /// awaited in `shutdown`: a delivery in flight against an unreachable
    /// peer is the exact case this queue exists for, and waiting for it
    /// would make shutdown hang on the condition it is meant to survive.
    /// The abandoned item stays on disk and returns after its visibility
    /// timeout.
    proxy_outbox_join: Option<JoinHandle<()>>,
    /// Cancels `proxy_outbox_join`'s loop. Held separately because the
    /// worker takes the token rather than a handle with a `shutdown`
    /// method of its own.
    proxy_outbox_cancel: CancellationToken,
}

impl Debug for RuntimeServices {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut debug_struct = f.debug_struct("RuntimeServices");

        #[cfg(feature = "community_registry")]
        debug_struct.field(
            "community_registry",
            &self.community_registry.as_ref().map(|_| "EcosystemRegistry"),
        );

        #[cfg(feature = "coordinator")]
        debug_struct.field("coordinator", &self.coordinator);

        #[cfg(feature = "client_gateway")]
        debug_struct.field("client_gateway", &self.client_gateway);

        debug_struct.field("supervisor", &self.supervisor.as_ref().map(|_| "SupervisorService"));

        debug_struct.finish()
    }
}

impl RuntimeServices {
    async fn init(config: &SubstrateConfig) -> anyhow::Result<Self> {
        Ok(Self {
            #[cfg(feature = "community_registry")]
            community_registry: if config.roles.community_registry.is_some() {
                Some(EcosystemRegistry::init(config).await?)
            } else {
                None
            },
            #[cfg(feature = "coordinator")]
            coordinator: if config.roles.coordinator.is_some() {
                Some(EcosystemCoordinator::init(config).await?)
            } else {
                None
            },
            #[cfg(feature = "client_gateway")]
            client_gateway: if config.roles.client_gateway.is_some() {
                Some(ClientGateway::init(config).await?)
            } else {
                None
            },
            supervisor: None,
            supervisor_join: None,
            queue_worker_join: None,
            proxy_outbox_join: None,
            proxy_outbox_cancel: CancellationToken::new(),
        })
    }

    /// Injects the supervisor role, already constructed by
    /// `setup_connection_router` (see `init`'s own doc comment for why
    /// this is a setter rather than an `init` parameter).
    fn set_supervisor(&mut self, supervisor: Option<Arc<SupervisorHandle>>) {
        self.supervisor = supervisor;
    }

    async fn run_until_shutdown<F>(
        &mut self,
        config: &SubstrateConfig,
        connection_router: &ConnectionRouter,
        endpoint_registry: &EndpointRegistry,
        shutdown_signal: F,
    ) where
        F: Future<Output = ()>,
    {
        // M05A A5c §19.8/D-A5c-8: spawned here, at the top of this
        // function rather than in `init`, so the loop's start still comes
        // after both composition calls -- and so it is a real
        // `tokio::spawn`ed task by the time the `select!` below races its
        // `JoinHandle`, not the pinned-and-dropped-on-exit future this
        // used to be.
        self.supervisor_join = spawn_supervisor_role(&self.supervisor);
        self.queue_worker_join = spawn_queue_worker_role(&self.supervisor);
        // Spawned here rather than in `init` for the same reason the two
        // above are: the loop must start only after both composition calls
        // have already run.
        self.proxy_outbox_join = connection_router.proxy().map(|proxy| {
            let tick = Duration::from_secs(
                config.roles.app_sandbox.as_ref().map_or(5, |role| role.queue_tick_secs).max(1),
            );
            let cancel = self.proxy_outbox_cancel.clone();
            tokio::spawn(async move { proxy.run_async_worker(tick, cancel).await })
        });

        #[cfg(feature = "community_registry")]
        let mut registry_fut = pin::pin!(async {
            match self.community_registry.as_mut() {
                Some(service) => service.run().await,
                None => pending_component().await,
            }
        });
        #[cfg(not(feature = "community_registry"))]
        let mut registry_fut = pin::pin!(pending_component());

        #[cfg(feature = "coordinator")]
        let mut coordinator_fut = pin::pin!(async {
            match self.coordinator.as_mut() {
                Some(service) => service.run().await,
                None => pending_component().await,
            }
        });
        #[cfg(not(feature = "coordinator"))]
        let mut coordinator_fut = pin::pin!(pending_component());

        #[cfg(feature = "client_gateway")]
        let mut client_gateway_fut = pin::pin!(async {
            match self.client_gateway.as_mut() {
                Some(service) => service.run().await,
                None => pending_component().await,
            }
        });
        #[cfg(not(feature = "client_gateway"))]
        let mut client_gateway_fut = pin::pin!(pending_component());

        let mut health_fut = pin::pin!(async {
            if let Some(obs) = &config.roles.observability
                && let Some(health) = &obs.health
                && health.enabled
            {
                let app = Router::new().route(&health.endpoint, routing::get(|| async { "OK" }));
                match TcpListener::bind(&health.bind_address).await {
                    Ok(listener) => {
                        if let Ok(addr) = listener.local_addr() {
                            info!("observability health endpoint listening on http://{}", addr);
                        }
                        let _ = axum::serve(listener, app).await;
                    }
                    Err(e) => {
                        error!(
                            "failed to bind health endpoint on {}: {:?}",
                            health.bind_address, e
                        );
                    }
                }
            }
            pending_component().await
        });

        let mut metrics_fut = pin::pin!(async {
            if let Some(obs) = &config.roles.observability
                && let Some(metrics_cfg) = &obs.metrics
                && metrics_cfg.enabled
            {
                let app = Router::new().route(
                    &metrics_cfg.endpoint,
                    routing::get(|| async {
                        if let Some(recorder) = MemoryRecorder::global() {
                            let snapshot = recorder.snapshot();
                            Json(snapshot)
                        } else {
                            Json(MetricsSnapshot {
                                counters: HashMap::new(),
                                gauges: HashMap::new(),
                                histograms: HashMap::new(),
                            })
                        }
                    }),
                );
                match TcpListener::bind(&metrics_cfg.bind_address).await {
                    Ok(listener) => {
                        if let Ok(addr) = listener.local_addr() {
                            info!("observability metrics endpoint listening on http://{}", addr);
                        }
                        let _ = axum::serve(listener, app).await;
                    }
                    Err(e) => {
                        error!(
                            "failed to bind metrics endpoint on {}: {:?}",
                            metrics_cfg.bind_address, e
                        );
                    }
                }
            }
            pending_component().await
        });

        let mut connection_router_fut = pin::pin!(connection_router.run());
        let mut expiry_sweep_fut = pin::pin!(instance_cert_expiry_sweep_loop(endpoint_registry));
        // M05A A5c §19.8/D-A5c-8: races the spawned loop's `JoinHandle`
        // rather than pinning the loop itself -- the supervisor exiting
        // (a task panic, or the join failing) still brings the substrate
        // down, unchanged from before, but the loop itself now survives
        // past this `select!` returning instead of being dropped mid-pass.
        let mut supervisor_fut = pin::pin!(async {
            match self.supervisor_join.as_mut() {
                Some(handle) => match handle.await {
                    Ok(res) => res,
                    Err(join_err) => {
                        Err(anyhow::anyhow!("supervisor loop task panicked: {join_err}"))
                    }
                },
                None => pending_component().await,
            }
        });
        // M05B B1: raced the same way `supervisor_fut` is -- a panic in the
        // queue worker still brings the substrate down, but the task
        // itself outlives this `select!` returning. Its ordinary exit path
        // (cancellation) only fires from `shutdown`, at which point this
        // arm racing is moot; see `queue_worker_join`'s own doc for why
        // `shutdown` does not also await it.
        let mut queue_worker_fut = pin::pin!(async {
            match self.queue_worker_join.as_mut() {
                Some(handle) => match handle.await {
                    Ok(res) => res,
                    Err(join_err) => Err(anyhow::anyhow!("queue worker task panicked: {join_err}")),
                },
                None => pending_component().await,
            }
        });
        // Raced the same way the others are, so a panic in the outbox
        // worker still brings the substrate down rather than silently
        // stopping delivery.
        let mut proxy_outbox_fut = pin::pin!(async {
            match self.proxy_outbox_join.as_mut() {
                Some(handle) => match handle.await {
                    Ok(()) => Ok(()),
                    Err(join_err) => {
                        Err(anyhow::anyhow!("proxy outbox worker task panicked: {join_err}"))
                    }
                },
                None => pending_component().await,
            }
        });
        let mut shutdown_signal = pin::pin!(shutdown_signal);

        info!(profile = %config.profile, "starting substrate components");
        tokio::select! {
            res = &mut connection_router_fut => log_component_exit("connection router", res),
            res = &mut registry_fut => log_component_exit("service registry", res),
            res = &mut coordinator_fut => log_component_exit("coordinator", res),
            res = &mut client_gateway_fut => log_component_exit("http proxy", res),
            res = &mut health_fut => log_component_exit("health server", res),
            res = &mut metrics_fut => log_component_exit("metrics server", res),
            res = &mut supervisor_fut => log_component_exit("supervisor", res),
            res = &mut queue_worker_fut => log_component_exit("queue worker", res),
            res = &mut proxy_outbox_fut => log_component_exit("proxy outbox worker", res),
            () = &mut expiry_sweep_fut => {},
            () = &mut shutdown_signal => warn!("received shutdown signal"),
        }
    }

    async fn shutdown(&mut self) {
        shutdown_supervisor_role(&self.supervisor).await;
        // M05A A5c D-A5c-8: cancelling the token above unblocks `run`'s
        // `select!`, but only awaiting this handle proves the pass in
        // flight, if any, actually finished closing the clients it had
        // open -- a token alone does not wait for anything.
        if let Some(handle) = self.supervisor_join.take() {
            match handle.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => error!(error = %error, "supervisor loop exited with an error"),
                Err(join_err) => {
                    error!(error = %join_err, "supervisor loop task panicked during shutdown")
                }
            }
        }
        // M05B B1, D-B1-8: `shutdown_supervisor_role` above already
        // cancelled the token the queue worker watches too -- both loops
        // are methods on the same `SupervisorService`, sharing one token --
        // so the worker has already stopped ticking. Deliberately **not**
        // awaited here, unlike `supervisor_join`: a delivery in flight
        // against an offline substrate is exactly the case this queue
        // exists for, and waiting for it here would make shutdown hang on
        // it. Dropping the handle detaches the task; the process exiting
        // shortly after this function returns is what actually ends it.
        drop(self.queue_worker_join.take());

        // Same rule, same reason: cancel so the loop stops ticking, but do
        // not await a delivery that may be waiting on an offline peer.
        self.proxy_outbox_cancel.cancel();
        drop(self.proxy_outbox_join.take());

        #[cfg(feature = "client_gateway")]
        if let Some(service) = self.client_gateway.as_mut()
            && let Err(error) = service.shutdown().await
        {
            error!(error = %error, "error shutting down http proxy");
        }

        #[cfg(feature = "coordinator")]
        if let Some(service) = self.coordinator.as_mut()
            && let Err(error) = service.shutdown().await
        {
            error!(error = %error, "error shutting down coordinator");
        }

        #[cfg(feature = "community_registry")]
        if let Some(service) = self.community_registry.as_mut()
            && let Err(error) = service.shutdown().await
        {
            error!(error = %error, "error shutting down service registry");
        }
    }
}

async fn pending_component() -> anyhow::Result<()> {
    future::pending().await
}

/// Spawns the supervisor's resident loop (M05A A5c §19.8/D-A5c-8) so it
/// outlives `run_until_shutdown`'s own stack frame instead of being
/// dropped mid-pass when some other component's future resolves first.
/// Not `#[cfg(feature = "supervisor")]` itself -- `supervisor` is always
/// `None` when the cargo feature is off (`init_supervisor` refuses to
/// build one), so this only needs its *body* gated, keeping
/// `RuntimeServices` free of per-call `cfg` splits the way
/// `community_registry`/`coordinator`/`client_gateway` already are on
/// their own fields.
fn spawn_supervisor_role(
    supervisor: &Option<Arc<SupervisorHandle>>,
) -> Option<JoinHandle<anyhow::Result<()>>> {
    #[cfg(feature = "supervisor")]
    {
        supervisor.clone().map(|service| tokio::spawn(async move { service.run().await }))
    }
    #[cfg(not(feature = "supervisor"))]
    {
        let _ = supervisor;
        None
    }
}

/// Spawns the durable outbox worker (M05B B1, D-B1-1) beside the resident
/// loop -- the same shape `spawn_supervisor_role` uses, for the same
/// reason: it must outlive `run_until_shutdown`'s own stack frame.
fn spawn_queue_worker_role(
    supervisor: &Option<Arc<SupervisorHandle>>,
) -> Option<JoinHandle<anyhow::Result<()>>> {
    #[cfg(feature = "supervisor")]
    {
        supervisor
            .clone()
            .map(|service| tokio::spawn(async move { service.run_queue_worker().await }))
    }
    #[cfg(not(feature = "supervisor"))]
    {
        let _ = supervisor;
        None
    }
}

/// Cancels the loop's token (`SupervisorService::shutdown`) -- the caller
/// is responsible for then awaiting the `JoinHandle`
/// `spawn_supervisor_role` returned, which is what actually waits for the
/// pass in flight to finish closing its clients (M05A A5c D-A5c-8;
/// cancelling alone does not wait for anything).
async fn shutdown_supervisor_role(supervisor: &Option<Arc<SupervisorHandle>>) {
    #[cfg(feature = "supervisor")]
    if let Some(service) = supervisor
        && let Err(error) = service.shutdown().await
    {
        error!(error = %error, "error shutting down supervisor");
    }
    #[cfg(not(feature = "supervisor"))]
    {
        let _ = supervisor;
    }
}

fn log_component_exit(component: &str, result: anyhow::Result<()>) {
    match result {
        Ok(()) => info!(component = component, "component finished"),
        Err(error) => {
            error!(component = component, error = %error, "component finished with error");
        }
    }
}

/// Sets up the connection router and its tightly coupled dependencies,
/// including the substrate identity, data store, endpoint registry, and the
/// native service.
async fn setup_connection_router(
    config: &SubstrateConfig,
) -> anyhow::Result<(ConnectionRouter, EndpointRegistry, Option<Arc<SupervisorHandle>>)> {
    let (service_id, secret_key, verified_controller) = setup_identity_and_storage(config).await?;

    // A verified `ControllerAgreement` (mutually signed by the substrate and
    // its controller, see `identity::setup_substrate_identity`) is the
    // authoritative substrate owner and takes precedence over the plain
    // `[iam].admin_ucan_root` config string -- a controller cannot claim
    // ownership unilaterally, only a two-way-signed agreement counts. The
    // config value remains a fallback for deployments with no agreement
    // configured at all.
    let mut effective_config = config.clone();
    if let Some(controller) = verified_controller {
        effective_config.iam.admin_ucan_root = Some(controller);
    }
    let config = &effective_config;

    // No verified ControllerAgreement controller and no
    // [iam].admin_ucan_root means the substrate is unowned and fails
    // closed -- no caller holds any node-wide capability at all
    // (`build_caller`, `crates/router/src/route_handler/io.rs`). Logged
    // loudly because the operator-facing fix is a local, offline step.
    if config.iam.admin_ucan_root.is_none() {
        warn!(
            "substrate has no verified ControllerAgreement controller and no \
             [iam].admin_ucan_root: running UNOWNED and FAIL-CLOSED -- no caller can deploy, \
             undeploy, status-check, or reach the security interface (KEK/secrets) on this node. \
             Establish ownership on this host with: roymctl substrate claim --controller <name>  \
             (then restart)"
        );
    }

    let (router, endpoint_registry, publisher, supervisor) =
        setup_router(config, &service_id, secret_key).await?;

    if let Some(publisher) = publisher
        && let Some(endpoint_addr) = router.endpoint_addr()
    {
        let relay_url = config.parent_coordinator.iroh.as_ref().map(|c| c.url.clone());
        publish_to_community_registry(
            service_id,
            endpoint_addr,
            relay_url,
            secret_key,
            config.identity.nickname.clone(),
            publisher,
        );
    }

    Ok((router, endpoint_registry, supervisor))
}

async fn setup_identity_and_storage(
    config: &SubstrateConfig,
) -> anyhow::Result<(String, [u8; 32], Option<String>)> {
    let substrate_identity_state =
        identity::setup_substrate_identity(&config.identity, &config.app_data_dir)?;
    let substrate_secret_key = identity::get_secret(&config.identity, &config.app_data_dir)?;
    // Only a *verified* (mutually signed) controller agreement establishes
    // substrate ownership -- `Unverified`/`None` never grant `substrate/admin`.
    let verified_controller = (substrate_identity_state.status
        == SubstrateIdentityStatus::Verified)
        .then_some(substrate_identity_state.controller)
        .flatten();
    Ok((substrate_identity_state.did, substrate_secret_key, verified_controller))
}

async fn setup_router(
    config: &SubstrateConfig,
    service_id: &str,
    secret_key: [u8; 32],
) -> anyhow::Result<(
    ConnectionRouter,
    EndpointRegistry,
    Option<Arc<EndpointPublisher>>,
    Option<Arc<SupervisorHandle>>,
)> {
    let data_store = registry_store::init_store(config).await?;
    let endpoint_registry = EndpointRegistry::new(data_store).await?;

    debug!("Registering native SubstrateService at {}", service_id);
    let endpoint = SubstrateEndpoint::NativeHostChannel { service_id: service_id.to_string() };
    endpoint_registry
        .register(service_id.to_string(), "orchestrator".to_string(), endpoint)
        .await?;
    let security_endpoint =
        SubstrateEndpoint::NativeHostChannel { service_id: service_id.to_string() };
    endpoint_registry
        .register(service_id.to_string(), "security".to_string(), security_endpoint)
        .await?;

    let (route_handler_deps, shared) =
        build_route_handler_deps(config, service_id, &endpoint_registry, secret_key).await?;
    let control_plane = route_handler_deps.control_plane.clone();

    let supervisor = if config.roles.supervisor.is_some() {
        let supervisor_endpoint =
            SubstrateEndpoint::NativeHostChannel { service_id: SUPERVISOR_DISPATCH_ID.to_string() };
        endpoint_registry
            .register(service_id.to_string(), "supervisor".to_string(), supervisor_endpoint)
            .await?;
        // M05A A5c §19.5a/D-A5c-6: gives the supervisor's own alert
        // publication a `messaging` endpoint to publish under -- a
        // supervisor role is not a deployed service, so without this
        // registration nothing resolves `SUPERVISOR_DISPATCH_ID` for the
        // `messaging` interface at all. **Deliberately** registered under
        // the same reserved id every other supervisor verb uses: the
        // router's own subscribe path (`dispatch.rs::handle_messaging_
        // subscribe`) namespaces this one service id with the
        // publish-side (unconditional-prefix) rule instead of the
        // ordinary subscribe-side rule every deployed service's
        // `messaging` endpoint gets -- see that function's own comment
        // for why. Do not "correct" that divergence back to the ordinary
        // rule: it is what keeps a caller's subscribe confined to
        // `svc/supervisor/...` on a node that hosts no deployed services
        // of its own to share the reach with (test 26 fails if reverted).
        let messaging_endpoint =
            SubstrateEndpoint::NativeHostChannel { service_id: SUPERVISOR_DISPATCH_ID.to_string() };
        endpoint_registry
            .register(service_id.to_string(), "messaging".to_string(), messaging_endpoint)
            .await?;
        Some(init_supervisor(config, service_id, &shared).await?)
    } else {
        None
    };

    let router = ConnectionRouter::init(
        endpoint_registry.clone(),
        config.clone(),
        secret_key,
        service_id.to_string(),
        route_handler_deps,
    )
    .await?;

    // Built here rather than in `build_route_handler_deps` because it needs
    // the finished `EndpointRegistry`, and handed to the control plane so a
    // deploy can publish immediately instead of waiting for the heartbeat.
    let publisher = (config.substrate.registry_url.is_some()
        || config.substrate.enable_bep0044_dht)
        .then(|| {
            Arc::new(EndpointPublisher::new(
                Arc::new(RegistryClient::new(
                    config.substrate.enable_bep0044_dht,
                    config.substrate.registry_url.clone(),
                )),
                config.hosted_apps_dir(),
            ))
        });

    if let Some(publisher) = &publisher {
        // A registry is configured, so a deploy must be able to publish. A
        // type-erased control plane cannot, and silently skipping the wiring
        // would leave deploy-time publishing off with nothing to notice it.
        let control_plane = control_plane.ok_or_else(|| {
            anyhow::anyhow!(
                "a community registry is configured but no concrete ControlPlaneService was \
                 built, so a deploy could not publish its endpoint record"
            )
        })?;
        control_plane.set_endpoint_publisher(publisher.clone());
    }

    Ok((router, endpoint_registry, publisher, supervisor))
}

/// Handles the supervisor role (and any future post-router role that must
/// act as a first-class dispatch target on this same node) needs, but
/// which are otherwise fully consumed by `build_route_handler_deps`'s
/// return value before this function's caller gets to see them again.
/// Built once, in `build_route_handler_deps`, since only it has all of
/// these in scope before they move into `RouteHandlerDeps`/
/// `ControlPlaneService`.
struct SharedNodeHandles {
    key_store: Arc<KeyStore>,
    storage_provider: Arc<dyn StorageProvider>,
    native_dispatch: NativeDispatchRegistry,
    /// The identity a post-router role presents when it connects, as a
    /// client, to other substrates (ADR-0021 §8) -- a second handle to the
    /// node's own key material, not a distinct identity.
    client_identity: Arc<Identity>,
    /// M05A A5c §19.5a/D-A5c-6: the same broker `AppSandboxEngine` and
    /// `ControlPlaneService` publish/subscribe through, so the supervisor's
    /// alert publication shares one broker with the rest of the node
    /// instead of standing up a second one.
    messaging_broker: Arc<MqttBroker>,
}

/// The literal `native_dispatch` key the supervisor's `NativeService`
/// registers under, independent of this node's own DID: unlike
/// `orchestrator`/`security`, which are the node addressing *itself*,
/// `supervisor` is dispatched to for the *same* connection preamble
/// (`<scheme>://supervisor.<node-did>`) but must not share `native_dispatch`'s
/// entry with `ControlPlaneService`, which is already registered under the
/// node's own DID by `RouteHandler::init`. Sourced from
/// `syneroym_control_plane`, which is also the crate that refuses a deploy
/// under this name, so the reserved word cannot drift between the two.
const SUPERVISOR_DISPATCH_ID: &str = syneroym_control_plane::SUPERVISOR_RESERVED_SERVICE_ID;

#[cfg(feature = "supervisor")]
type SupervisorHandle = syneroym_app_supervisor::SupervisorService;
#[cfg(not(feature = "supervisor"))]
type SupervisorHandle = ();

#[cfg(feature = "supervisor")]
async fn init_supervisor(
    config: &SubstrateConfig,
    service_id: &str,
    shared: &SharedNodeHandles,
) -> anyhow::Result<Arc<SupervisorHandle>> {
    use syneroym_app_supervisor::{
        MasterVault, RegistryAnchorWriter, SupervisorService, store::SupervisorStore,
    };

    let role = config
        .roles
        .supervisor
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("init_supervisor called with no [roles.supervisor]"))?;

    std::fs::create_dir_all(&config.app_data_dir)?;
    let store = SupervisorStore::open_with_role(&config.app_data_dir, &role.db_name, role)?;
    let backup_dir = config.app_data_dir.join(&role.master_backup_dir);
    let vault = MasterVault::new(
        shared.storage_provider.clone(),
        shared.key_store.clone(),
        SUPERVISOR_DISPATCH_ID.to_string(),
        backup_dir,
    );
    let supervisor = Arc::new(SupervisorService::new(
        service_id.to_string(),
        store,
        vault,
        &shared.client_identity,
        shared.messaging_broker.clone(),
        role.alert_topic.clone(),
        role.poll_interval_secs,
        role.max_restart_attempts,
        role.restart_backoff_secs,
        role.renewed_cert_expires_hours,
        role.max_renewals_per_pass,
        role.master_anchor_refresh_interval_secs,
        // The node's own registry, the same one every other publisher on
        // this host uses. Absent when none is configured -- see the field's
        // own doc for why the supervisor then holds no writer at all rather
        // than one that silently does nothing.
        RegistryAnchorWriter::from_registry_url(
            config.substrate.enable_bep0044_dht,
            config.substrate.registry_url.as_deref(),
        ),
        role.queue_tick_secs,
    ));
    shared
        .native_dispatch
        .insert(SUPERVISOR_DISPATCH_ID.to_string(), supervisor.clone() as Arc<dyn NativeService>);

    if !shared.key_store.kek_is_loaded() {
        warn!(
            "supervisor role is enabled but its vault is LOCKED: no KEK has been injected, so it \
             cannot mint, certify, or renew member masters. Inject one with: roymctl --substrate \
             {service_id} security inject-kek --kek-hex <...>"
        );
    }

    Ok(supervisor)
}

#[cfg(not(feature = "supervisor"))]
async fn init_supervisor(
    _config: &SubstrateConfig,
    _service_id: &str,
    _shared: &SharedNodeHandles,
) -> anyhow::Result<Arc<SupervisorHandle>> {
    Err(anyhow::anyhow!(
        "[roles.supervisor] is configured but this binary was built without the `supervisor` \
         feature"
    ))
}

/// Rebuilds the in-memory `StaticInventory` from every dependency binding
/// `EndpointRegistry` has persisted (A2, ADR-0021 §5) -- a restarted
/// substrate must answer a guest's first call, and nothing re-pushes on
/// restart. A row that fails to parse is warned and skipped, exactly like
/// the unparseable-`TopologyEntry`-JSON case beside it: every one of the
/// three stored strings is caller-supplied at some point in its history
/// (D-A2-15), so `LogicalServiceName::new` would *panic* substrate startup
/// on a row containing a `/`, which is a strictly worse outcome than
/// skipping that one row.
async fn replay_persisted_bindings(
    registry: &EndpointRegistry,
) -> anyhow::Result<Arc<StaticInventory>> {
    let app_registry = Arc::new(StaticInventory::new());
    for (_service_id, instance, dep_name, entry_json) in registry.all_bindings().await? {
        let parsed = (|| -> anyhow::Result<_> {
            Ok((
                AppInstanceId::try_new(&instance)?,
                LogicalServiceName::try_new(&dep_name)?,
                serde_json::from_str::<TopologyEntry>(&entry_json)?,
            ))
        })();
        match parsed {
            Ok((instance_id, service_name, entry)) => {
                app_registry.register(instance_id, service_name, entry);
            }
            Err(e) => {
                warn!(%instance, %dep_name, error = %e, "skipping an unreadable persisted binding");
            }
        }
    }
    Ok(app_registry)
}

/// Constructs every capability the connection router holds and dispatches
/// through but does not itself build: storage, blob, and messaging
/// backends, the WASM and container sandboxes, and the control-plane
/// native service. This is the substrate's composition root -- `router`
/// only needs the finished handles, not the knowledge of how to build them.
async fn build_route_handler_deps(
    config: &SubstrateConfig,
    service_id: &str,
    registry: &EndpointRegistry,
    secret_key: [u8; 32],
) -> anyhow::Result<(RouteHandlerDeps, SharedNodeHandles)> {
    // Shared with `ControlPlaneService`'s native `data-layer` dispatch
    // (`SynSvcNativeService`), which signs Slice B3's relationship-proof
    // records as this node's own asserter identity -- the same key material
    // `ConnectionRouter::init` (below, in the caller) separately constructs
    // its own `Identity` from for `ProxyRouter`'s `node_identity`.
    let node_identity = Arc::new(Identity::from_bytes(&secret_key));
    let key_store = Arc::new(KeyStore::new());
    let storage_provider: Arc<dyn StorageProvider> =
        Arc::new(SqliteStorageProvider::new(&config.storage.db_dir, config.storage.encryption)?);
    let blob_provider = build_blob_provider(config)?;

    let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig {
        channel_capacity: config.mqtt.channel_capacity as usize,
    })?);

    // A2 (ADR-0021 §2): replay persisted bindings before anything can
    // resolve one -- a restarted substrate must answer a guest's first
    // call, and nothing re-pushes on restart (ADR-0021 §5 -- push failure
    // is sticky, and so is push absence).
    let app_registry = replay_persisted_bindings(registry).await?;
    let logical_resolver = Arc::new(LogicalResolver::new(app_registry));

    let app_sandbox_engine = Arc::new(
        AppSandboxEngine::init(
            config,
            registry.get_all_endpoints(),
            key_store.clone(),
            storage_provider.clone(),
            blob_provider.clone(),
            messaging_broker.clone(),
            registry.clone(),
            logical_resolver.clone(),
        )
        .await?,
    );
    app_sandbox_engine
        .self_weak
        .set(Arc::downgrade(&app_sandbox_engine))
        .map_err(|_| anyhow::anyhow!("AppSandboxEngine::self_weak set more than once"))?;

    replay_persisted_subscriptions(&storage_provider, &app_sandbox_engine).await?;

    let podman_path = config
        .roles
        .podman_sandbox
        .as_ref()
        .map(|cfg| cfg.podman_path.clone())
        .unwrap_or_else(|| "podman".to_string());
    let podman_sandbox_engine = Arc::new(ContainerEngine::new(
        podman_path,
        &config.app_local_data_dir,
        Some(storage_provider.clone()),
    ));

    // Shared with `ControlPlaneService`, which registers/deregisters
    // per-deployment native services (data-layer/vault/app-config/
    // blob-store) and HTTP routes into these same tables on deploy/undeploy
    // -- `RouteHandler`'s own dispatch path reads through the identical
    // handles.
    let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());
    let http_routes: HttpRouteRegistry = Arc::new(DashMap::new());

    // Cloning the `Arc`, not `node_identity` itself -- `Identity`
    // deliberately does not implement `Clone`, but a second handle to the
    // same key material is exactly what's needed here. The supervisor role
    // (when configured) uses this as the identity it presents when it
    // connects, as a client, to the substrates it manages (ADR-0021 §8).
    let supervisor_client_identity = node_identity.clone();

    let control_plane_service = ControlPlaneService::init(
        service_id.to_string(),
        service_id.to_string(),
        app_sandbox_engine.clone(),
        podman_sandbox_engine,
        registry.clone(),
        config.hosted_apps_dir(),
        key_store.clone(),
        storage_provider.clone(),
        blob_provider,
        messaging_broker.clone(),
        native_dispatch.clone(),
        http_routes.clone(),
        node_identity,
        logical_resolver.clone(),
    )
    .await?;
    let control_plane_service = Arc::new(control_plane_service);

    let shared = SharedNodeHandles {
        key_store: key_store.clone(),
        storage_provider: storage_provider.clone(),
        native_dispatch: native_dispatch.clone(),
        client_identity: supervisor_client_identity,
        messaging_broker: messaging_broker.clone(),
    };

    Ok((
        RouteHandlerDeps {
            logical_resolver: logical_resolver.clone(),
            key_store,
            storage_provider,
            app_sandbox_engine,
            messaging_broker,
            native_dispatch,
            http_routes,
            control_plane_service: control_plane_service.clone(),
            control_plane: Some(control_plane_service),
        },
        shared,
    ))
}

/// Guest subscriptions survive a restart (ADR-0010 Finding A1): replay
/// every persisted row into the broker before the router starts accepting
/// connections. Best-effort per row -- one bad topic shouldn't block
/// substrate startup. Replayed concurrently (independent rows, no shared
/// state) to keep this bounded by the slowest single subscribe rather than
/// their sum.
async fn replay_persisted_subscriptions(
    storage_provider: &Arc<dyn StorageProvider>,
    app_sandbox_engine: &AppSandboxEngine,
) -> anyhow::Result<()> {
    let persisted_subscriptions = storage_provider.list_all_messaging_subscriptions().await?;
    let replay_results = futures::future::join_all(persisted_subscriptions.iter().map(
        |(subscribed_service_id, topic)| {
            app_sandbox_engine.register_internal_subscription(subscribed_service_id, topic)
        },
    ))
    .await;
    for ((subscribed_service_id, topic), result) in
        persisted_subscriptions.iter().zip(replay_results)
    {
        if let Err(e) = result {
            warn!(
                service_id = %subscribed_service_id,
                topic = %topic,
                error = %e,
                "Failed to replay messaging subscription on startup"
            );
        }
    }
    Ok(())
}

/// Constructs the configured blob backend (`Local` or `S3`). `S3` requires
/// building with the `aws` cargo feature (off by default -- see the
/// `object_store`/`digest` version-pin comment in the root `Cargo.toml`);
/// selecting it otherwise fails fast here with an actionable message rather
/// than silently falling back to `Local`.
fn build_blob_provider(config: &SubstrateConfig) -> anyhow::Result<Arc<dyn BlobProvider>> {
    let bs = &config.storage.blob_store;
    match bs.backend {
        BlobBackend::Local => Ok(Arc::new(ObjectStoreBlobProvider::new_local(
            bs.local_root.clone(),
            bs.max_blob_bytes,
            bs.max_service_total_bytes,
        )?)),
        BlobBackend::S3 => {
            #[cfg(feature = "aws")]
            {
                let s3 = bs.s3.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "storage.blob_store.backend = \"s3\" requires [storage.blob_store.s3] to \
                         be configured"
                    )
                })?;
                Ok(Arc::new(ObjectStoreBlobProvider::new_s3(
                    &s3.endpoint,
                    &s3.bucket,
                    &s3.region,
                    bs.max_blob_bytes,
                    bs.max_service_total_bytes,
                )?))
            }
            #[cfg(not(feature = "aws"))]
            {
                Err(anyhow::anyhow!(
                    "storage.blob_store.backend = \"s3\" requires building syneroym-substrate \
                     with the `aws` feature (off by default -- see the object_store/digest \
                     version-pin comment in the root Cargo.toml)"
                ))
            }
        }
    }
}

fn publish_to_community_registry(
    service_id: String,
    endpoint_addr: EndpointAddr,
    relay_url: Option<String>,
    secret_key: [u8; 32],
    nickname: Option<String>,
    publisher: Arc<EndpointPublisher>,
) {
    tokio::spawn(async move {
        // Reuses the publisher's own client rather than building a second
        // one from the same config: each opens its own pkarr DHT client
        // when the DHT is enabled, so a duplicate is a real (if small) cost,
        // not just noise.
        let registry_client = publisher.registry_client();

        loop {
            // Register native substrate endpoint
            let signed_info = match build_signed_endpoint_info(
                &service_id,
                &endpoint_addr,
                relay_url.clone(),
                &secret_key,
                nickname.clone(),
            ) {
                Ok(info) => info,
                Err(e) => {
                    warn!("Failed to build signed endpoint info: {}", e);
                    time::sleep(Duration::from_secs(60)).await;
                    continue;
                }
            };

            let mut attempts = 0;
            let mut success = false;
            while attempts < 30 {
                if let Err(e) = registry_client.register(&signed_info, false).await {
                    warn!("Failed to register endpoint (attempt {}): {}", attempts + 1, e);
                    time::sleep(Duration::from_millis(500)).await;
                    attempts += 1;
                } else {
                    success = true;
                    break;
                }
            }

            if success {
                info!(
                    service_id = %service_id,
                    "Successfully registered substrate endpoint"
                );
            } else {
                warn!(
                    service_id = %service_id,
                    "Exhausted registration retries. Substrate may be unreachable."
                );
            }

            // Hosted services: replay every stored, still-verifying record
            // verbatim. The substrate holds no key that could ever sign one
            // itself (ADR-0020 §3), so this is pure replay, never a rebuild.
            publisher.publish_all_services().await;
            publisher.warn_on_near_expiry_records().await;

            // Sleep until the next heartbeat interval
            time::sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
        }
    });
}

/// The attended posture's visibility half (ADR-0020 §3): nothing here
/// renews a certificate, only warns before a missed renewal becomes an
/// outage. Runs on the same cadence as the community-registry heartbeat
/// above but as its own sibling loop in `RuntimeServices`'s `select!`,
/// rather than growing `publish_to_community_registry`'s argument list with
/// a registry it has no other reason to hold.
async fn instance_cert_expiry_sweep_loop(registry: &EndpointRegistry) -> ! {
    loop {
        warn_on_near_expiry_instance_certs(registry);
        time::sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
    }
}

/// Warns for any installed instance certificate within 25% of its lifetime
/// of expiring, and returns their `service_id`s. Split out from the sleep
/// loop above -- and returning the warned set rather than only logging it --
/// so it's testable without waiting on a real timer or scraping log output.
fn warn_on_near_expiry_instance_certs(registry: &EndpointRegistry) -> Vec<String> {
    let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);

    let mut near_expiry = Vec::new();
    for (service_id, cert) in registry.all_instance_certs() {
        if cert.is_near_expiry(now_secs) {
            let remaining_secs = cert.expires_at_secs.saturating_sub(now_secs);
            warn!(
                service_id = %service_id,
                expires_at_secs = cert.expires_at_secs,
                remaining_secs,
                "instance certificate is within 25% of its lifetime of expiring -- renew with \
                 `roymctl identity certify-instance` before it lapses, which fails the \
                 handshake closed"
            );
            near_expiry.push(service_id);
        }
    }
    near_expiry
}

fn build_signed_endpoint_info(
    service_id: &str,
    endpoint_addr: &EndpointAddr,
    relay_url: Option<String>,
    secret_key: &[u8; 32],
    nickname: Option<String>,
) -> anyhow::Result<SignedEndpointInfo> {
    // Prune direct addresses to keep the serialized PKARR record under the
    // 1000-byte DNS limit
    let pruned_addr = EndpointAddr::new(endpoint_addr.id);
    let endpoint_addr_bytes = serde_json::to_vec(&pruned_addr)
        .map_err(|e| anyhow::anyhow!("Failed to serialize endpoint addr: {e}"))?;

    let not_after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);

    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: service_id.to_string(),
        endpoint_type: EndpointType::Substrate,
        nickname,
        mechanisms: vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url }],
        is_private: false,
        ttl: None,
        not_after,
    };

    let identity = Identity::from_bytes(secret_key);
    info.sign(&identity)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use syneroym_app_orchestration::{ServiceId, TopologyEpoch, TopologyMode};
    use syneroym_core::storage::MockStorage;
    use syneroym_identity::{DelegationCertificate, delegation::SCOPE_SERVICE_INSTANCE};

    use super::*;

    /// Matrix row 3's observability half: a certificate within 25% of its
    /// lifetime of expiring is flagged; one nowhere near expiry is not.
    #[tokio::test]
    async fn a_certificate_near_expiry_is_warned_about_on_the_heartbeat_sweep() {
        let registry = EndpointRegistry::new(Arc::new(MockStorage::new())).await.unwrap();
        let master = Identity::generate().unwrap();
        let instance = Identity::generate().unwrap();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

        // 1000s lifetime, 100s (10%) remaining -- inside the 25% window.
        let mut near_expiry = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            1000,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        near_expiry.issued_at_secs = now - 900;
        near_expiry.expires_at_secs = now + 100;
        registry.set_instance_cert("near-expiry-svc".to_string(), near_expiry).await.unwrap();

        // Freshly issued, nowhere near its 3600s expiry.
        let mut fresh = DelegationCertificate::issue(
            &master,
            instance.public_key(),
            3600,
            SCOPE_SERVICE_INSTANCE.to_string(),
        )
        .unwrap();
        fresh.issued_at_secs = now;
        fresh.expires_at_secs = now + 3600;
        registry.set_instance_cert("fresh-svc".to_string(), fresh).await.unwrap();

        let warned = warn_on_near_expiry_instance_certs(&registry);
        assert_eq!(warned, vec!["near-expiry-svc".to_string()]);
    }

    /// D-A2-15: a persisted binding row that would panic `LogicalServiceName::
    /// new` (a `/` in the dependency name) or fails to parse as JSON must be
    /// warned and skipped, not crash substrate startup -- and a good row
    /// alongside it must still replay.
    #[tokio::test]
    async fn an_unreadable_persisted_binding_is_skipped_not_fatal() {
        let storage = Arc::new(MockStorage::new());
        let registry = EndpointRegistry::new(storage.clone()).await.unwrap();

        registry
            .save_binding("svc-slash", "app-1", "bad/name", r#"{"fake":"entry"}"#)
            .await
            .unwrap();
        registry.save_binding("svc-badjson", "app-1", "backend", "not json").await.unwrap();
        let good_entry = TopologyEntry {
            mode: TopologyMode::Singleton,
            members: vec![ServiceId::new("did:key:zGoodMember")],
            sharding_strategy: None,
            epoch: TopologyEpoch::default(),
            cache_ttl: Duration::from_secs(60),
        };
        registry
            .save_binding(
                "svc-good",
                "app-1",
                "good-dep",
                &serde_json::to_string(&good_entry).unwrap(),
            )
            .await
            .unwrap();

        let app_registry = replay_persisted_bindings(&registry).await.unwrap();

        assert!(
            app_registry
                .get(&AppInstanceId::new("app-1"), &LogicalServiceName::new("good-dep"))
                .is_some(),
            "the well-formed row alongside the corrupt ones must still replay"
        );
        assert_eq!(app_registry.list(&AppInstanceId::new("app-1")).len(), 1);
    }
}

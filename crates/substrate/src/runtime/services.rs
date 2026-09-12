//! Substrate runtime services lifecycle and execution.

use std::{
    collections::HashMap,
    fmt::{self, Debug, Formatter},
    future::{self, Future},
    pin,
    sync::Arc,
    time::Duration,
};

use axum::{Json, Router, routing};
#[cfg(feature = "client_gateway")]
use syneroym_client_gateway::ClientGateway;
#[cfg(feature = "community_registry")]
use syneroym_community_registry::EcosystemRegistry;
use syneroym_conversation::ConversationService;
#[cfg(feature = "coordinator")]
use syneroym_coordinator::EcosystemCoordinator;
use syneroym_core::{config::SubstrateConfig, local_registry::EndpointRegistry};
use syneroym_observability::{MemoryRecorder, MetricsSnapshot};
use syneroym_router::ConnectionRouter;
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::{publish::instance_cert_expiry_sweep_loop, supervisor::SupervisorHandle};

pub struct RuntimeServices {
    #[cfg(feature = "community_registry")]
    community_registry: Option<EcosystemRegistry>,
    #[cfg(feature = "coordinator")]
    coordinator: Option<EcosystemCoordinator>,
    #[cfg(feature = "client_gateway")]
    client_gateway: Option<ClientGateway>,
    supervisor: Option<Arc<SupervisorHandle>>,
    /// The supervisor's resident loop is spawned (not pinned in
    /// `run_until_shutdown`'s own `select!`), so it outlives that
    /// function's stack frame -- `shutdown` cancels the loop's token and
    /// awaits this handle, rather than relying on a token nothing would
    /// ever join in time. Populated by `run_until_shutdown`, not `init`,
    /// so the startup-ordering rule (the loop starts only after both
    /// composition calls have already run) stays true.
    supervisor_join: Option<JoinHandle<anyhow::Result<()>>>,
    /// The durable outbox worker's own task, spawned and raced the same
    /// way `supervisor_join` is. **Not** awaited in `shutdown`: the
    /// resident loop's join proves an in-flight pass finished closing its
    /// clients, but the worker's own in-flight deliveries are deliberately
    /// abandoned -- their visibility timeout
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
    /// `None` only in a test harness that never calls `set_conversation`;
    /// the real composition root always sets it.
    conversation: Option<Arc<ConversationService>>,
    /// The conversation delivery worker's task — raced and shut down
    /// exactly like `proxy_outbox_join`, for the identical reason:
    /// a delivery in flight against an offline peer is the case this
    /// queue exists to survive, so shutdown must not wait for it.
    conversation_worker_join: Option<JoinHandle<()>>,
    conversation_worker_cancel: CancellationToken,
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
        debug_struct
            .field("conversation", &self.conversation.as_ref().map(|_| "ConversationService"));

        debug_struct.finish()
    }
}

impl RuntimeServices {
    pub(super) async fn init(config: &SubstrateConfig) -> anyhow::Result<Self> {
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
            conversation: None,
            conversation_worker_join: None,
            conversation_worker_cancel: CancellationToken::new(),
        })
    }

    /// Injects the supervisor role, already constructed by
    /// `setup_connection_router` (see `init`'s own doc comment for why
    /// this is a setter rather than an `init` parameter).
    pub(super) fn set_supervisor(&mut self, supervisor: Option<Arc<SupervisorHandle>>) {
        self.supervisor = supervisor;
    }

    /// Injects the Conversation service, same reasoning as
    /// `set_supervisor`.
    pub(super) fn set_conversation(&mut self, conversation: Arc<ConversationService>) {
        self.conversation = Some(conversation);
    }

    /// Passes the auth service DID to the client gateway so it can route
    /// `/_syneroym/session/*` requests to the auth service. Matches the
    /// pattern of `set_supervisor`/`set_conversation` — the gateway is
    /// already constructed in `init`, so injection after
    /// `setup_connection_router` keeps the two composition calls in the
    /// same relative order they have always been in.
    #[cfg(feature = "client_gateway")]
    pub(super) fn set_gateway_auth_did(&self, auth_did: Option<String>) {
        if let Some(gateway) = self.client_gateway.as_ref() {
            gateway.set_auth_did(auth_did);
        }
    }

    pub(super) async fn run_until_shutdown<F>(
        &mut self,
        config: &SubstrateConfig,
        connection_router: &ConnectionRouter,
        endpoint_registry: &EndpointRegistry,
        shutdown_signal: F,
    ) where
        F: Future<Output = ()>,
    {
        // Spawned here, at the top of this function rather than in `init`,
        // so the loop's start still comes after both composition calls --
        // and so it is a real `tokio::spawn`ed task by the time the
        // `select!` below races its `JoinHandle`, not the
        // pinned-and-dropped-on-exit future this used to be.
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
        self.conversation_worker_join = self.conversation.clone().map(|svc| {
            let tick = Duration::from_secs(
                config
                    .roles
                    .app_sandbox
                    .as_ref()
                    .map_or(5, |role| role.conversation_tick_secs)
                    .max(1),
            );
            let cancel = self.conversation_worker_cancel.clone();
            tokio::spawn(async move { svc.run_worker(tick, cancel).await })
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
        // Races the spawned loop's `JoinHandle` rather than pinning the
        // loop itself -- the supervisor exiting (a task panic, or the join
        // failing) still brings the substrate down, unchanged from before,
        // but the loop itself now survives past this `select!` returning
        // instead of being dropped mid-pass.
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
        // Raced the same way `supervisor_fut` is -- a panic in the queue
        // worker still brings the substrate down, but the task itself
        // outlives this `select!` returning. Its ordinary exit path
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
        let mut conversation_outbox_fut = pin::pin!(async {
            match self.conversation_worker_join.as_mut() {
                Some(handle) => match handle.await {
                    Ok(()) => Ok(()),
                    Err(join_err) => {
                        Err(anyhow::anyhow!("conversation outbox worker task panicked: {join_err}"))
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
            res = &mut conversation_outbox_fut => log_component_exit("conversation outbox worker", res),
            () = &mut expiry_sweep_fut => {},
            () = &mut shutdown_signal => warn!("received shutdown signal"),
        }
    }

    pub(super) async fn shutdown(&mut self) {
        shutdown_supervisor_role(&self.supervisor).await;
        // Cancelling the token above unblocks `run`'s `select!`, but only
        // awaiting this handle proves the pass in flight, if any, actually
        // finished closing the clients it had open -- a token alone does
        // not wait for anything.
        if let Some(handle) = self.supervisor_join.take() {
            match handle.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => error!(error = %error, "supervisor loop exited with an error"),
                Err(join_err) => {
                    error!(error = %join_err, "supervisor loop task panicked during shutdown")
                }
            }
        }
        // `shutdown_supervisor_role` above already cancelled the token the
        // queue worker watches too -- both loops are methods on the same
        // `SupervisorService`, sharing one token -- so the worker has
        // already stopped ticking. Deliberately **not**
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

        // Same rule, same reason, as `proxy_outbox_join` above: a delivery
        // in flight against an offline peer is exactly the case this
        // queue exists for.
        self.conversation_worker_cancel.cancel();
        drop(self.conversation_worker_join.take());

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

/// Spawns the supervisor's resident loop so it outlives
/// `run_until_shutdown`'s own stack frame instead of being dropped
/// mid-pass when some other component's future resolves first.
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

/// Spawns the durable outbox worker beside the resident loop -- the same
/// shape `spawn_supervisor_role` uses, for the same reason: it must outlive
/// `run_until_shutdown`'s own stack frame.
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
/// pass in flight to finish closing its clients (cancelling alone does
/// not wait for anything).
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

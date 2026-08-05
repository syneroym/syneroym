//! Protocol-specific routing handlers
//!
//! Defines dispatch pipelines for HTTP, JSON-RPC, and raw TCP traffic (wRPC —
//! TODO: not yet implemented).

use std::{
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::Result;
use dashmap::DashMap;
use fmt::{Debug, Formatter};
use iroh::{
    Endpoint,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler as IrohProtocolHandler},
};
use syneroym_app_orchestration::LogicalResolver;
use syneroym_async_queue::{DedupConfig, QueueConfig};
use syneroym_control_plane::ControlPlaneService;
use syneroym_core::{
    config::{RetryPolicy, SubstrateConfig},
    dht_registry::RegistryClient,
    http_routes::HttpRouteRegistry,
    local_registry::EndpointRegistry,
    storage::MockStorage,
};
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_identity::{Identity, substrate};
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{
    DEFAULT_PROXY_CALL_TIMEOUT, NativeDispatchRegistry, NativeService, ProxyQueueInspector,
    RowAuthorizer, ServiceProxy,
};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error};

use crate::{
    call_dedup::CallDedupGuard,
    net_iroh::IrohStream,
    proxy::{IrohHop, ProxyRouter},
    proxy_outbox::ProxyOutbox,
};

pub mod dispatch;
pub mod encryption;
pub mod http;
pub mod io;

#[derive(Clone)]
pub struct RouteHandler {
    pub(crate) inner: Arc<RouteHandlerInner>,
}

/// Returns true if the error represents an expected client disconnect.
pub fn is_expected_disconnect<E: fmt::Display>(e: E) -> bool {
    let err_msg = e.to_string();
    err_msg.contains("connection lost")
        || err_msg.contains("closed stream")
        || err_msg.contains("Broken pipe")
        || err_msg.contains("Connection reset by peer")
}

#[derive(Debug)]
pub struct ConnectionSlot {
    counter: Arc<AtomicUsize>,
}

impl ConnectionSlot {
    pub fn new_pre_incremented(counter: Arc<AtomicUsize>) -> Self {
        Self { counter }
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct RouteHandlerInner {
    pub registry: EndpointRegistry,
    pub native_dispatch: NativeDispatchRegistry,
    pub app_sandbox_engine: Option<Arc<AppSandboxEngine>>,
    /// `Arc`-wrapped (M04A Slice A1) so the `ProxyRouter`'s outbound remote
    /// hop can share this exact node identity rather than constructing (and
    /// signing with) a second one.
    pub identity: Arc<Identity>,
    pub iroh_endpoint: Option<Endpoint>,
    /// `Arc`-wrapped (M04A Slice A1) so the `ProxyRouter` can share this
    /// exact client -- re-constructing a second `RegistryClient` would spin
    /// up a second DHT client (background bootstrap/routing-table tasks and
    /// sockets) when DHT is enabled.
    pub registry_client: Arc<RegistryClient>,
    pub _parent_relay_url: Option<String>,
    pub retry_policy: RetryPolicy,
    pub active_connections: Arc<AtomicUsize>,
    pub max_connections: Option<usize>,
    /// Core, always-on per-node capability (ADR-0010) -- kept alive for as
    /// long as any `RouteHandler` clone is; its `Drop` cancels the
    /// `CancellationToken` governing its own subscription-forwarding
    /// tasks, mirroring `AppSandboxEngine`'s epoch-timer task lifecycle.
    pub messaging_broker: Arc<MqttBroker>,
    /// Per-service HTTP route table (M3B Slice 7); the `HttpRoute`/
    /// `HttpRouteRegistry` types live in `syneroym_core::http_routes`,
    /// populated by `ControlPlaneService::deploy`/`undeploy` -- see
    /// `syneroym_control_plane::http_routes`. Read by
    /// `route_handler::http` to resolve `(service_id, method, path)` to a
    /// bridged `data-layer`/`messaging`/stream-protocol route.
    pub http_routes: HttpRouteRegistry,
    /// `None` in coordinator mode (`new_coordinator`), `Some` for a real
    /// substrate node (`init`) -- mirrors `app_sandbox_engine`'s own
    /// coordinator-mode-is-absent pattern. Used only by the signed-URL blob
    /// `GET` route (M3B Slice 7) to resolve a service's DEK the same way
    /// `SynSvcNativeService::resolve_blob_dek` does, so
    /// `crypto::verify_signed_url` can be checked before any bytes are
    /// streamed -- the streaming itself still goes through the existing
    /// `blob-store/open-download`+`read-chunk` native-dispatch methods.
    pub key_store: Option<Arc<KeyStore>>,
    pub storage_provider: Option<Arc<dyn StorageProvider>>,
    /// Interim Admin-capability allowlist root (M04A Slice B0, ADR-0015/0016
    /// `[iam].admin_ucan_root`): a caller whose verified master DID equals
    /// this is granted `substrate/admin`. `None` in coordinator mode
    /// (coordinators don't host native capabilities).
    pub admin_ucan_root: Option<String>,
    /// This node's own DID -- `RouteHandler::init`'s `service_id` (M04A
    /// Slice B7a). Named `node_did` where it is used as an identity rather
    /// than a routing key: `substrate:<node_did>` resources name it.
    pub node_did: String,
    /// The Universal Proxy (M04A Slice A1). `RouteHandlerInner` is its
    /// strong owner -- `AppSandboxEngine::service_proxy` only ever holds the
    /// `Weak` published from here, to avoid the `RouteHandlerInner ->
    /// ProxyRouter -> AppSandboxEngine -> ProxyRouter` reference cycle that
    /// hung graceful shutdown in Slice 6B. `None` in coordinator mode
    /// (coordinators have no native capabilities or sandbox to proxy to).
    /// Not read anywhere yet -- A1 only wires the *outbound* call surface
    /// (reachable via the guest WIT import and `AppSandboxEngine`'s `Weak`
    /// handle); this field's job is solely to keep the `ProxyRouter` alive
    /// for as long as this `RouteHandler` is (mirrors `_parent_relay_url`'s
    /// established underscore convention for a stored-but-unread field).
    pub _proxy: Option<Arc<ProxyRouter>>,
    /// The receiver-side idempotency fence, shared with `ProxyRouter` so a
    /// call arriving over the wire and one dispatched on this node meet the
    /// same guard and the same records. `None` in coordinator mode, where
    /// there is no per-service storage to remember a key in.
    pub dedup_guard: Option<Arc<CallDedupGuard>>,
}

impl Debug for RouteHandler {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteHandler")
            .field("registry", &self.inner.registry)
            .field("native_dispatch_len", &self.inner.native_dispatch.len())
            .finish_non_exhaustive()
    }
}

/// Capabilities `RouteHandler` holds and dispatches through, but does not
/// itself construct. Building these (storage, blob, messaging, the WASM/
/// container sandboxes, and the control-plane native service) is the
/// substrate's composition-root responsibility (see
/// `syneroym_substrate::runtime`) -- `router`'s job is routing, not wiring
/// up the node's capabilities.
pub struct RouteHandlerDeps {
    pub key_store: Arc<KeyStore>,
    pub storage_provider: Arc<dyn StorageProvider>,
    /// Resolves a declared dependency name to the member currently bound
    /// to it. Held here (not only inside the sandbox engine) because the
    /// durable outbox re-resolves a queued call's dependency at every
    /// delivery attempt, long after the component instance that wrote it
    /// is gone.
    pub logical_resolver: Arc<LogicalResolver>,
    pub app_sandbox_engine: Arc<AppSandboxEngine>,
    pub messaging_broker: Arc<MqttBroker>,
    pub native_dispatch: NativeDispatchRegistry,
    pub http_routes: HttpRouteRegistry,
    /// The node's control-plane service (deploy/undeploy/list, security
    /// ops), already registered against `native_dispatch`/`http_routes` by
    /// the caller during construction -- `RouteHandler::init` only needs to
    /// register it into its own dispatch table under `service_id`. Type-
    /// erased so test doubles can substitute a fake here without being a
    /// real `ControlPlaneService`.
    pub control_plane_service: Arc<dyn NativeService>,
    /// The concrete `ControlPlaneService`, when the caller has a real one --
    /// `None` for a test double that only needs `control_plane_service`
    /// above for dispatch-registration behavior. Slice B3 Phase 4 uses this
    /// solely to set `ControlPlaneService.service_proxy`'s `OnceLock` once
    /// `ProxyRouter` exists (same two-phase wiring `AppSandboxEngine.
    /// service_proxy` already needs, for the identical ordering reason);
    /// nothing else reads it.
    pub control_plane: Option<Arc<ControlPlaneService>>,
}

impl Debug for RouteHandlerDeps {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteHandlerDeps").finish_non_exhaustive()
    }
}

impl RouteHandler {
    pub async fn init(
        service_id: String,
        config: &SubstrateConfig,
        registry: EndpointRegistry,
        secret_key: [u8; 32],
        iroh_endpoint: Option<Endpoint>,
        deps: RouteHandlerDeps,
    ) -> Result<Self> {
        let identity = Arc::new(Identity::from_bytes(&secret_key));

        let parent_coordinator_url =
            config.parent_coordinator.iroh.as_ref().map(|cfg| cfg.url.clone());

        let registry_client = Arc::new(RegistryClient::new(
            config.substrate.enable_bep0044_dht,
            config.substrate.registry_url.clone(),
        ));

        let max_connections = config
            .roles
            .coordinator
            .as_ref()
            .and_then(|c| c.iroh.as_ref())
            .and_then(|i| i.max_connections);

        // The Universal Proxy (M04A Slice A1). Built here, before `inner`,
        // so its `Weak` handles can be downgraded from the still-owned
        // `deps.native_dispatch`/`deps.app_sandbox_engine` Arcs -- `Weak`,
        // never a second strong ref, matching the `RouteHandlerInner ->
        // ProxyRouter -> AppSandboxEngine -> ProxyRouter` cycle avoidance
        // documented on `RouteHandlerInner::proxy`.
        // The fence a keyed call meets, whichever entry point it arrives
        // through. Its windows are derived from the outbox's own retry
        // budget rather than separately configured: a TTL shorter than the
        // sender's retry window silently converts dedup into no dedup.
        let queue_config = QueueConfig::from(&config.roles.app_sandbox.clone().unwrap_or_default());
        let dedup_guard = Arc::new(CallDedupGuard::new(
            deps.storage_provider.clone(),
            deps.key_store.clone(),
            DedupConfig::derive(
                queue_config.total_retry_window_ms(),
                queue_config.visibility_timeout_ms,
                DEFAULT_PROXY_CALL_TIMEOUT.as_millis() as u64,
            ),
        ));

        // One queue per calling service, in that service's own encrypted
        // database. The resolver is handed over so a queued dependency is
        // re-resolved at every delivery attempt rather than snapshotted
        // when the item was written (ADR-0021 §2).
        let outbox = Arc::new(ProxyOutbox::new(
            deps.storage_provider.clone(),
            deps.key_store.clone(),
            deps.logical_resolver.clone(),
            queue_config.clone(),
        ));

        let proxy = Arc::new(
            ProxyRouter::new(
                registry.clone(),
                registry_client.clone(),
                Arc::downgrade(&deps.native_dispatch),
                Arc::downgrade(&deps.app_sandbox_engine),
                Arc::new(IrohHop::new(iroh_endpoint.clone(), config.retry.clone())),
                identity.clone(),
                config.retry.clone(),
            )
            .with_dedup_guard(dedup_guard.clone())
            .with_outbox(outbox.clone()),
        );
        deps.app_sandbox_engine
            .service_proxy
            .set(Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>)
            .map_err(|_| anyhow::anyhow!("AppSandboxEngine::service_proxy set more than once"))?;
        // Slice B4-fdae: `SynSvcNativeService`'s stage-4 ABAC after-step
        // (ADR-0017 §7) needs a handle to the same engine -- `AppSandboxEngine`
        // is the sole `RowAuthorizer` implementation. Threaded through
        // `ControlPlaneService.row_authorizer` below, same two-phase
        // `OnceLock` wiring as `service_proxy` (this engine already exists
        // by construction time; only the *native-dispatch* side needs the
        // handle threaded in post-construction).
        if let Some(control_plane) = &deps.control_plane {
            control_plane
                .row_authorizer
                .set(Arc::downgrade(&deps.app_sandbox_engine) as Weak<dyn RowAuthorizer>)
                .map_err(|_| {
                    anyhow::anyhow!("ControlPlaneService::row_authorizer set more than once")
                })?;
        }
        // Slice B3 Phase 4: `SynSvcNativeService`'s cross-service
        // relationship-proof fetch needs the same proxy handle, threaded
        // through `ControlPlaneService` at deploy time -- same two-phase
        // wiring as `AppSandboxEngine`'s above, for the identical ordering
        // reason (`ProxyRouter` doesn't exist yet when either service is
        // constructed). `None` only for a test harness that substitutes a
        // fake `control_plane_service` and never deploys a real FDAE-policy
        // service through it.
        if let Some(control_plane) = &deps.control_plane {
            control_plane
                .service_proxy
                .set(Arc::downgrade(&proxy) as Weak<dyn ServiceProxy>)
                .map_err(|_| {
                    anyhow::anyhow!("ControlPlaneService::service_proxy set more than once")
                })?;
            // The `proxy-*` operator verbs read the queues the router owns,
            // so they need the outbox itself rather than the proxy.
            control_plane
                .proxy_queues
                .set(Arc::downgrade(&outbox) as Weak<dyn ProxyQueueInspector>)
                .map_err(|_| {
                    anyhow::anyhow!("ControlPlaneService::proxy_queues set more than once")
                })?;
        }

        let inner = Arc::new(RouteHandlerInner {
            registry,
            native_dispatch: deps.native_dispatch,
            app_sandbox_engine: Some(deps.app_sandbox_engine),
            identity,
            iroh_endpoint,
            registry_client,
            _parent_relay_url: parent_coordinator_url,
            retry_policy: config.retry.clone(),
            active_connections: Arc::new(AtomicUsize::new(0)),
            max_connections,
            messaging_broker: deps.messaging_broker,
            http_routes: deps.http_routes,
            key_store: Some(deps.key_store),
            storage_provider: Some(deps.storage_provider),
            admin_ucan_root: config.iam.admin_ucan_root.clone(),
            node_did: service_id.clone(),
            _proxy: Some(proxy),
            dedup_guard: Some(dedup_guard),
        });

        let s = Self { inner };
        s.register_native_service(service_id, deps.control_plane_service);
        Ok(s)
    }

    #[allow(clippy::expect_used)]
    #[must_use]
    pub fn new_coordinator(
        iroh_endpoint: Endpoint,
        registry_client: RegistryClient,
        parent_relay_url: Option<String>,
        retry_policy: RetryPolicy,
        max_connections: Option<usize>,
    ) -> Self {
        let identity = Identity::generate().expect("coordinator identity");
        let node_did = substrate::derive_did_key(&identity.public_key());
        let inner = Arc::new(RouteHandlerInner {
            registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            native_dispatch: Arc::new(DashMap::new()),
            app_sandbox_engine: None,
            identity: Arc::new(identity),
            iroh_endpoint: Some(iroh_endpoint),
            registry_client: Arc::new(registry_client),
            _parent_relay_url: parent_relay_url,
            retry_policy,
            active_connections: Arc::new(AtomicUsize::new(0)),
            max_connections,
            messaging_broker: Arc::new(
                MqttBroker::new(MqttBrokerConfig::default()).expect("coordinator mqtt broker"),
            ),
            http_routes: Arc::new(DashMap::new()),
            key_store: None,
            storage_provider: None,
            admin_ucan_root: None,
            node_did,
            // Coordinators have no native capabilities or sandbox to proxy
            // to, and no per-service storage to fence a keyed call with.
            _proxy: None,
            dedup_guard: None,
        });
        Self { inner }
    }

    /// The Universal Proxy this handler dispatches through, when it has
    /// one. Published for the substrate's composition root, which drives
    /// the durable outbox worker off the same router live calls use.
    #[must_use]
    pub fn proxy(&self) -> Option<Arc<ProxyRouter>> {
        self.inner._proxy.clone()
    }

    pub fn register_native_service(&self, service_id: String, service: Arc<dyn NativeService>) {
        self.inner.native_dispatch.insert(service_id, service);
    }

    /// Dynamically acquire a connection slot. Returns Some(ConnectionSlot) if
    /// the slot is available, or None if the max_connections limit is
    /// reached.
    pub fn acquire_connection_slot(&self) -> Option<ConnectionSlot> {
        if let Some(max_conns) = self.inner.max_connections {
            let res = self.inner.active_connections.fetch_update(
                Ordering::SeqCst,
                Ordering::SeqCst,
                |curr| {
                    if curr >= max_conns { None } else { Some(curr + 1) }
                },
            );
            match res {
                Ok(_) => {
                    Some(ConnectionSlot::new_pre_incremented(self.inner.active_connections.clone()))
                }
                Err(_) => None,
            }
        } else {
            self.inner.active_connections.fetch_add(1, Ordering::SeqCst);
            Some(ConnectionSlot::new_pre_incremented(self.inner.active_connections.clone()))
        }
    }

    pub fn active_connections(&self) -> Arc<AtomicUsize> {
        self.inner.active_connections.clone()
    }
}

impl IrohProtocolHandler for RouteHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let endpoint_id = connection.remote_id();
        debug!("[Router] Accepted Iroh connection from {endpoint_id}");

        let _slot = if let Some(slot) = self.acquire_connection_slot() {
            slot
        } else {
            debug!(
                "[Router] Rejecting connection from {endpoint_id} due to connection cap ({}/{:?})",
                self.inner.active_connections.load(Ordering::SeqCst),
                self.inner.max_connections
            );
            if let Ok((mut send, _recv)) = connection.accept_bi().await {
                let _ = send.write_all(b"ServiceUnavailable\n").await;
                let _ = send.flush().await;
            }
            connection.close(503u32.into(), b"ServiceUnavailable");
            return Ok(());
        };
        metrics::gauge!("substrate.connections.active").increment(1.0);

        loop {
            match connection.accept_bi().await {
                Ok((send, recv)) => {
                    debug!(
                        "[Router] New bi-directional stream from {endpoint_id}; spawning handler"
                    );
                    let iroh_stream = IrohStream::new(send, recv);
                    let handler = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handler.handle_stream(iroh_stream).await {
                            if is_expected_disconnect(&e) {
                                debug!("[Router] Stream from {endpoint_id} closed by peer ({e})");
                            } else {
                                error!(
                                    "[Router] Error handling Iroh stream from {}: {}",
                                    endpoint_id, e
                                );
                            }
                        }
                        debug!("[Router] Stream from {} completed", endpoint_id);
                    });
                }
                Err(e) => {
                    debug!("[Router] Connection {endpoint_id} closed or error: {e}");
                    break;
                }
            }
        }

        connection.closed().await;
        debug!("[Router] Iroh connection from {endpoint_id} fully closed");
        metrics::gauge!("substrate.connections.active").decrement(1.0);

        Ok(())
    }
}

//! Protocol-specific routing handlers
//!
//! Defines dispatch pipelines for HTTP, JSON-RPC, and raw TCP traffic (wRPC —
//! TODO: not yet implemented).

use std::{
    fmt,
    sync::{
        Arc,
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
use syneroym_core::{
    config::{RetryPolicy, SubstrateConfig},
    dht_registry::RegistryClient,
    http_routes::HttpRouteRegistry,
    local_registry::EndpointRegistry,
    storage::MockStorage,
};
use syneroym_data_db::traits::StorageProvider;
use syneroym_data_keystore::KeyStore;
use syneroym_identity::Identity;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{NativeDispatchRegistry, NativeService};
use syneroym_sandbox_wasm::AppSandboxEngine;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error};

use crate::net_iroh::IrohStream;

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
    pub identity: Identity,
    pub iroh_endpoint: Option<Endpoint>,
    pub registry_client: RegistryClient,
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
    pub app_sandbox_engine: Arc<AppSandboxEngine>,
    pub messaging_broker: Arc<MqttBroker>,
    pub native_dispatch: NativeDispatchRegistry,
    pub http_routes: HttpRouteRegistry,
    /// The node's control-plane service (deploy/undeploy/list, security
    /// ops), already registered against `native_dispatch`/`http_routes` by
    /// the caller during construction -- `RouteHandler::init` only needs to
    /// register it into its own dispatch table under `service_id`.
    pub control_plane_service: Arc<dyn NativeService>,
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
        deps: RouteHandlerDeps,
    ) -> Result<Self> {
        let identity = Identity::from_bytes(&secret_key);

        let parent_coordinator_url =
            config.parent_coordinator.iroh.as_ref().map(|cfg| cfg.url.clone());

        let registry_client = RegistryClient::new(
            config.substrate.enable_bep0044_dht,
            config.substrate.registry_url.clone(),
        );

        let max_connections = config
            .roles
            .coordinator
            .as_ref()
            .and_then(|c| c.iroh.as_ref())
            .and_then(|i| i.max_connections);

        let inner = Arc::new(RouteHandlerInner {
            registry,
            native_dispatch: deps.native_dispatch,
            app_sandbox_engine: Some(deps.app_sandbox_engine),
            identity,
            iroh_endpoint: None,
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
        let inner = Arc::new(RouteHandlerInner {
            registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            native_dispatch: Arc::new(DashMap::new()),
            app_sandbox_engine: None,
            identity: Identity::generate().expect("coordinator identity"),
            iroh_endpoint: Some(iroh_endpoint),
            registry_client,
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
        });
        Self { inner }
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

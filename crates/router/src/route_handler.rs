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
use syneroym_control_plane::ControlPlaneService;
use syneroym_core::{
    config::{BlobBackend, RetryPolicy, SubstrateConfig},
    dht_registry::RegistryClient,
    local_registry::EndpointRegistry,
    storage::MockStorage,
};
use syneroym_data_blob::{BlobProvider, ObjectStoreBlobProvider};
use syneroym_data_db::{SqliteStorageProvider, traits::StorageProvider};
use syneroym_data_keystore::KeyStore;
use syneroym_identity::Identity;
use syneroym_mqtt_broker::{MqttBroker, MqttBrokerConfig};
use syneroym_rpc::{NativeDispatchRegistry, NativeService};
use syneroym_sandbox_podman::ContainerEngine;
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
}

impl Debug for RouteHandler {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteHandler")
            .field("registry", &self.inner.registry)
            .field("native_dispatch_len", &self.inner.native_dispatch.len())
            .finish_non_exhaustive()
    }
}

/// Constructs the configured blob backend (`Local` or `S3`). `S3` requires
/// building with the `aws` cargo feature (off by default -- see the
/// `object_store`/`digest` version-pin comment in the root `Cargo.toml`);
/// selecting it otherwise fails fast here with an actionable message rather
/// than silently falling back to `Local`.
fn build_blob_provider(config: &SubstrateConfig) -> Result<Arc<dyn BlobProvider>> {
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
                    "storage.blob_store.backend = \"s3\" requires building syneroym-router with \
                     the `aws` feature (off by default -- see the object_store/digest version-pin \
                     comment in the root Cargo.toml)"
                ))
            }
        }
    }
}

impl RouteHandler {
    pub async fn init(
        service_id: String,
        config: &SubstrateConfig,
        registry: EndpointRegistry,
        secret_key: [u8; 32],
    ) -> Result<Self> {
        let key_store = Arc::new(KeyStore::new());
        let storage_provider: Arc<dyn StorageProvider> = Arc::new(SqliteStorageProvider::new(
            &config.storage.db_dir,
            config.storage.encryption,
        )?);
        let blob_provider = build_blob_provider(config)?;

        let messaging_broker = Arc::new(MqttBroker::new(MqttBrokerConfig {
            channel_capacity: config.mqtt.channel_capacity as usize,
        })?);

        let app_sandbox_engine = Arc::new(
            AppSandboxEngine::init(
                config,
                registry.get_all_endpoints(),
                key_store.clone(),
                storage_provider.clone(),
                blob_provider.clone(),
                messaging_broker.clone(),
            )
            .await?,
        );
        app_sandbox_engine
            .self_weak
            .set(Arc::downgrade(&app_sandbox_engine))
            .map_err(|_| anyhow::anyhow!("AppSandboxEngine::self_weak set more than once"))?;

        // Guest subscriptions survive a restart (ADR-0010 Finding A1):
        // replay every persisted row into the broker before the router
        // starts accepting connections. Best-effort per row -- one bad
        // topic shouldn't block substrate startup.
        for (subscribed_service_id, topic) in
            storage_provider.list_all_messaging_subscriptions().await?
        {
            if let Err(e) = app_sandbox_engine
                .register_internal_subscription(&subscribed_service_id, &topic)
                .await
            {
                tracing::warn!(
                    service_id = %subscribed_service_id,
                    topic = %topic,
                    error = %e,
                    "Failed to replay messaging subscription on startup"
                );
            }
        }

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

        // Constructed before `RouteHandlerInner` so the identical shared
        // handle can also be passed to `ControlPlaneService::init` below --
        // `ControlPlaneService` needs to register/deregister per-deployment
        // native services (data-layer/vault/app-config/blob-store) into the
        // same registry `RouteHandler`'s own dispatch path reads from.
        let native_dispatch: NativeDispatchRegistry = Arc::new(DashMap::new());

        let inner = Arc::new(RouteHandlerInner {
            registry: registry.clone(),
            native_dispatch: native_dispatch.clone(),
            app_sandbox_engine: Some(app_sandbox_engine.clone()),
            identity,
            iroh_endpoint: None,
            registry_client,
            _parent_relay_url: parent_coordinator_url,
            retry_policy: config.retry.clone(),
            active_connections: Arc::new(AtomicUsize::new(0)),
            max_connections,
            messaging_broker: messaging_broker.clone(),
        });

        let s = Self { inner };

        let substrate_service = ControlPlaneService::init(
            service_id.clone(),
            app_sandbox_engine,
            podman_sandbox_engine,
            registry,
            config.hosted_apps_dir(),
            key_store,
            storage_provider,
            blob_provider,
            messaging_broker,
            native_dispatch,
        )
        .await?;
        s.register_native_service(service_id, Arc::new(substrate_service));
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

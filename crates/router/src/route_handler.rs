//! Protocol-specific routing handlers
//!
//! Defines dispatch pipelines for HTTP, JSON-RPC, and raw TCP traffic (wRPC — TODO: not yet implemented).

use std::{fmt, sync::Arc};

use anyhow::Result;
use dashmap::DashMap;
use fmt::{Debug, Formatter};
use iroh::{
    Endpoint,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler as IrohProtocolHandler},
};
use syneroym_app_sandbox::AppSandboxEngine;
use syneroym_control_plane::ControlPlaneService;
use syneroym_core::{
    config::SubstrateConfig, dht_registry::RegistryClient, local_registry::EndpointRegistry,
    storage::MockStorage,
};
use syneroym_identity::Identity;
use syneroym_podman_sandbox::ContainerEngine;
use syneroym_rpc::NativeService;
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

pub struct RouteHandlerInner {
    pub registry: EndpointRegistry,
    pub native_dispatch: DashMap<String, Arc<dyn NativeService>>,
    pub app_sandbox_engine: Option<Arc<AppSandboxEngine>>,
    pub identity: Identity,
    pub iroh_endpoint: Option<Endpoint>,
    pub registry_client: RegistryClient,
    pub _parent_relay_url: Option<String>,
}

impl Debug for RouteHandler {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteHandler")
            .field("registry", &self.inner.registry)
            .field("native_dispatch_len", &self.inner.native_dispatch.len())
            .finish_non_exhaustive()
    }
}

impl RouteHandler {
    pub async fn init(
        service_id: String,
        config: &SubstrateConfig,
        registry: EndpointRegistry,
        secret_key: [u8; 32],
    ) -> Result<Self> {
        let app_sandbox_engine =
            Arc::new(AppSandboxEngine::init(config, registry.get_all_endpoints()).await?);

        let podman_path = config
            .roles
            .podman_sandbox
            .as_ref()
            .map(|cfg| cfg.podman_path.clone())
            .unwrap_or_else(|| "podman".to_string());
        let podman_sandbox_engine =
            Arc::new(ContainerEngine::new(podman_path, &config.app_local_data_dir));

        let identity = Identity::from_bytes(&secret_key);

        let parent_coordinator_url =
            config.parent_coordinator.iroh.as_ref().map(|cfg| cfg.url.clone());

        let registry_client = RegistryClient::new(
            config.substrate.enable_bep0044_dht,
            config.substrate.registry_url.clone(),
        );

        let inner = Arc::new(RouteHandlerInner {
            registry: registry.clone(),
            native_dispatch: DashMap::new(),
            app_sandbox_engine: Some(app_sandbox_engine.clone()),
            identity,
            iroh_endpoint: None,
            registry_client,
            _parent_relay_url: parent_coordinator_url,
        });

        let s = Self { inner };

        let substrate_service = ControlPlaneService::init(
            service_id.clone(),
            app_sandbox_engine,
            podman_sandbox_engine,
            registry,
            config.hosted_apps_dir(),
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
    ) -> Self {
        let inner = Arc::new(RouteHandlerInner {
            registry: EndpointRegistry::new_mock(Arc::new(MockStorage::new())),
            native_dispatch: DashMap::new(),
            app_sandbox_engine: None,
            identity: syneroym_identity::Identity::generate().expect("coordinator identity"),
            iroh_endpoint: Some(iroh_endpoint),
            registry_client,
            _parent_relay_url: parent_relay_url,
        });
        Self { inner }
    }

    pub fn register_native_service(&self, service_id: String, service: Arc<dyn NativeService>) {
        self.inner.native_dispatch.insert(service_id, service);
    }
}

impl IrohProtocolHandler for RouteHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let endpoint_id = connection.remote_id();
        debug!("[Router] Accepted Iroh connection from {endpoint_id}");
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
                            error!(
                                "[Router] Error handling Iroh stream from {}: {}",
                                endpoint_id, e
                            );
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

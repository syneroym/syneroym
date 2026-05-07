use anyhow::Result;
use dashmap::DashMap;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler as IrohProtocolHandler};
use std::fmt;
use std::sync::Arc;
use syneroym_control_plane::ControlPlaneService;
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::EndpointRegistry;
use syneroym_rpc::NativeService;
use tracing::{debug, error};

use crate::net_iroh::IrohStream;
use syneroym_app_sandbox::AppSandboxEngine;

pub mod dispatch;
pub mod http;
pub mod io;

#[derive(Clone)]
pub struct RouteHandler {
    pub(crate) inner: Arc<RouteHandlerInner>,
}

pub(crate) struct RouteHandlerInner {
    pub registry: EndpointRegistry,
    pub native_dispatch: DashMap<String, Arc<dyn NativeService>>,
    pub app_sandbox_engine: Arc<AppSandboxEngine>,
}

impl fmt::Debug for RouteHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    ) -> Result<Self> {
        let app_sandbox_engine =
            Arc::new(AppSandboxEngine::init(config, registry.get_all_endpoints()).await?);

        let inner = Arc::new(RouteHandlerInner {
            registry: registry.clone(),
            native_dispatch: DashMap::new(),
            app_sandbox_engine: app_sandbox_engine.clone(),
        });

        let s = Self { inner };

        let substrate_service =
            ControlPlaneService::init(service_id.clone(), app_sandbox_engine, registry).await?;
        s.register_native_service(service_id, Arc::new(substrate_service));
        Ok(s)
    }

    pub fn register_native_service(&self, service_id: String, service: Arc<dyn NativeService>) {
        self.inner.native_dispatch.insert(service_id, service);
    }
}

impl IrohProtocolHandler for RouteHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let endpoint_id = connection.remote_id();
        debug!("accepted connection from {endpoint_id}");

        loop {
            match connection.accept_bi().await {
                Ok((send, recv)) => {
                    let iroh_stream = IrohStream::new(send, recv);
                    let handler = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handler.handle_stream(iroh_stream).await {
                            error!("Error handling Iroh stream: {}", e);
                        }
                        debug!("handled stream");
                    });
                }
                Err(e) => {
                    debug!("Connection {endpoint_id} closed or error: {e}");
                    break;
                }
            }
        }

        connection.closed().await;
        debug!("connection closed");

        Ok(())
    }
}

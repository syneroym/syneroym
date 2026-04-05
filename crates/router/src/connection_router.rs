use anyhow::Result;
use iroh::endpoint::presets;
use iroh::protocol::Router as IrohRouter;
use iroh::{RelayMap, RelayMode, RelayUrl, SecretKey};
use syneroym_core::config::{IrohRelayConfig, SubstrateConfig};
use syneroym_core::registry::EndpointRegistry;
use tracing::{debug, info};

use crate::route_handler::RouteHandler;

pub const SYNEROYM_ALPN: &[u8] = b"syneroym/0.1";

/// The Connection Router (The Data Plane)
/// Internal traffic cop that uses the Endpoint Registry to look up
/// the destination for an incoming wRPC stream.
#[derive(Debug, Clone)]
pub struct ConnectionRouter {
    iroh_router: Option<IrohRouter>,
}

impl ConnectionRouter {
    pub async fn init(
        registry: EndpointRegistry,
        config: SubstrateConfig,
        iroh_secret_key: [u8; 32],
        service_id: String,
    ) -> Result<Self> {
        let mut router = Self { iroh_router: None };

        for comm in &config.substrate.communication_interfaces {
            match comm.as_str() {
                "iroh" => {
                    if let Some(iroh_config) = config.uplink.iroh.as_ref() {
                        info!("Initializing Iroh interface for Router...");
                        let iroh_router = router
                            .init_iroh(
                                iroh_config,
                                iroh::SecretKey::from_bytes(&iroh_secret_key),
                                RouteHandler::init(service_id.clone(), &config, registry.clone())
                                    .await?,
                            )
                            .await?;
                        router.iroh_router = Some(iroh_router);
                    }
                }
                "webrtc" => {
                    info!("WebRTC interface initialization not yet implemented in Router.");
                }
                _ => {
                    info!("Unknown or unimplemented communication interface: {}", comm);
                }
            }
        }

        Ok(router)
    }

    async fn init_iroh(
        &self,
        config: &IrohRelayConfig,
        secret_key: SecretKey,
        route_handler: RouteHandler,
    ) -> Result<IrohRouter> {
        debug!("Initializing Iroh communication...");

        let mut ep_bldr = iroh::Endpoint::builder(presets::N0);
        if let Ok(relay_url) = config.relay_url.parse::<RelayUrl>() {
            ep_bldr = iroh::Endpoint::empty_builder()
                .relay_mode(RelayMode::Custom(RelayMap::from(relay_url)));
        }

        let ep_bldr = ep_bldr.secret_key(secret_key);
        let ep = ep_bldr.bind().await?;

        let iroh_router: IrohRouter =
            IrohRouter::builder(ep).accept(SYNEROYM_ALPN, route_handler).spawn();

        info!("Iroh listening on ALPN: {:?}", std::str::from_utf8(SYNEROYM_ALPN).unwrap());

        Ok(iroh_router)
    }

    pub async fn run(&self) -> Result<()> {
        info!("running connection router");
        let endpoint = self.iroh_router.as_ref().map(|router| router.endpoint());
        if let Some(endpoint) = endpoint {
            endpoint.closed().await;
        } else {
            std::future::pending::<()>().await;
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        info!("shutting down connection router");
        if let Some(router) = self.iroh_router.as_ref() {
            router.shutdown().await?;
        }
        Ok(())
    }
}

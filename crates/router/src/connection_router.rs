//! Connection Router orchestrator
//!
//! The main connection router that accepts incoming network streams (Iroh,
//! WebRTC), extracts protocol preambles, and forwards traffic to local
//! endpoints or sandbox instances.

use std::{future, str, sync::Arc};

use anyhow::Result;
use iroh::{
    Endpoint, EndpointAddr, SecretKey,
    protocol::{Router, Router as IrohRouter},
};
use syneroym_control_plane::MailboxPullControl;
use syneroym_core::{config::SubstrateConfig, local_registry::EndpointRegistry};
use tracing::{debug, info};

use crate::{
    home_relay::{HOME_RELAY_CONFIRM_TIMEOUT, HomeRelay},
    mailbox_pull::MailboxPuller,
    net_iroh, net_webrtc,
    proxy::ProxyRouter,
    route_handler::{RouteHandler, RouteHandlerDeps},
};

pub const SYNEROYM_ALPN: &[u8] = b"syneroym/0.1";

/// The Connection Router (The Data Plane)
/// Internal traffic cop that uses the Endpoint Registry to look up
/// the destination for an incoming data stream.
#[derive(Debug, Clone)]
pub struct ConnectionRouter {
    iroh_router: Option<IrohRouter>,
    /// Sets the home relay of the Iroh endpoint. `None` when `iroh` is not in
    /// `communication_interfaces`.
    home_relay: Option<HomeRelay>,
    /// The Universal Proxy this router's handler dispatches through.
    /// Published so the substrate's composition root can drive the durable
    /// outbox worker off it -- the worker delivers by making an ordinary
    /// proxy call, so it needs the same router every live call uses, not a
    /// second one with its own caches.
    proxy: Option<Arc<ProxyRouter>>,
    /// The mailbox receiver of this node. Published so the substrate's
    /// composition root can run it. Absent in coordinator mode.
    mailbox_puller: Option<Arc<MailboxPuller>>,
}

impl ConnectionRouter {
    pub async fn init(
        registry: EndpointRegistry,
        config: SubstrateConfig,
        iroh_secret_key: [u8; 32],
        service_id: String,
        route_handler_deps: RouteHandlerDeps,
    ) -> Result<Self> {
        let mut router =
            Self { iroh_router: None, proxy: None, home_relay: None, mailbox_puller: None };

        // Built *before* `RouteHandler::init`: the Universal Proxy's outbound
        // remote hop needs an Iroh endpoint, and `RouteHandler::init` wires it
        // into its `ProxyRouter`. The endpoint starts with no relay. The
        // substrate sets the home relay in the background through
        // `home_relay()`.
        let iroh_endpoint = if config.substrate.communication_interfaces.iter().any(|c| c == "iroh")
        {
            let idle_timeout = config
                .roles
                .coordinator
                .as_ref()
                .and_then(|c| c.iroh.as_ref())
                .and_then(|i| i.idle_timeout_secs);
            Some(
                net_iroh::build_iroh_endpoint(
                    None,
                    Some(SecretKey::from_bytes(&iroh_secret_key)),
                    idle_timeout,
                )
                .await?,
            )
        } else {
            None
        };
        router.home_relay =
            iroh_endpoint.clone().map(|ep| HomeRelay::new(ep, HOME_RELAY_CONFIRM_TIMEOUT));

        let control_plane = route_handler_deps.control_plane.clone();
        let route_handler = RouteHandler::init(
            service_id.clone(),
            &config,
            registry.clone(),
            iroh_secret_key,
            iroh_endpoint.clone(),
            route_handler_deps,
        )
        .await?;
        router.proxy = route_handler.proxy();
        router.mailbox_puller = control_plane.and_then(|control_plane| {
            let puller = route_handler.mailbox_puller(control_plane.clone())?;
            let control: Arc<dyn MailboxPullControl> = puller.clone();
            control_plane.set_mailbox_pull(Arc::downgrade(&control));
            Some(puller)
        });

        for comm in &config.substrate.communication_interfaces {
            match comm.as_str() {
                "iroh" => {
                    if let Some(ep) = iroh_endpoint.clone() {
                        info!("Initializing Iroh interface for Router...");
                        router.iroh_router =
                            Some(Self::spawn_iroh_router(ep, route_handler.clone())?);
                    }
                }
                "webrtc" => {
                    if let Some(webrtc_config) = config.parent_coordinator.webrtc.as_ref() {
                        info!("Initializing WebRTC interface for Router...");
                        net_webrtc::spawn_signaling(
                            service_id.clone(),
                            webrtc_config,
                            route_handler.clone(),
                        );
                    }
                }
                _ => {
                    info!("Unknown or unimplemented communication interface: {}", comm);
                }
            }
        }

        Ok(router)
    }

    /// The Universal Proxy behind this router, when it has one (absent in
    /// coordinator mode).
    #[must_use]
    pub fn proxy(&self) -> Option<Arc<ProxyRouter>> {
        self.proxy.clone()
    }

    /// The mailbox receiver of this node, when it has one (absent in
    /// coordinator mode).
    #[must_use]
    pub fn mailbox_puller(&self) -> Option<Arc<MailboxPuller>> {
        self.mailbox_puller.clone()
    }

    /// The home-relay handle of the Iroh endpoint, when there is one.
    #[must_use]
    pub fn home_relay(&self) -> Option<HomeRelay> {
        self.home_relay.clone()
    }

    /// Spawns the Iroh protocol handler on an already-built, already-bound
    /// endpoint (see `init`'s `iroh_endpoint` construction above -- split
    /// out so that endpoint construction can happen before
    /// `RouteHandler::init`, which needs it for the Universal Proxy's
    /// outbound remote hop).
    fn spawn_iroh_router(ep: Endpoint, route_handler: RouteHandler) -> Result<IrohRouter> {
        debug!("Initializing Iroh communication...");

        let iroh_router: IrohRouter =
            IrohRouter::builder(ep).accept(SYNEROYM_ALPN, route_handler).spawn();

        info!(
            "Iroh listening on ALPN: {:?}",
            str::from_utf8(SYNEROYM_ALPN).unwrap_or("<invalid utf8>")
        );

        Ok(iroh_router)
    }

    pub async fn run(&self) -> Result<()> {
        info!("running connection router");
        let endpoint = self.iroh_router.as_ref().map(Router::endpoint);
        if let Some(endpoint) = endpoint {
            endpoint.closed().await;
        } else {
            future::pending::<()>().await;
        }
        Ok(())
    }

    #[must_use]
    pub fn endpoint_addr(&self) -> Option<EndpointAddr> {
        self.iroh_router.as_ref().map(|router| router.endpoint().addr())
    }

    pub async fn shutdown(&self) -> Result<()> {
        info!("shutting down connection router");
        if let Some(router) = self.iroh_router.clone() {
            let ep = router.endpoint().clone();
            // `Router::shutdown` already closes the endpoint internally on
            // its own success path, so the explicit `ep.close()` below is
            // normally a no-op safety net -- but propagating an `Err` here
            // (its accept-loop task panicked or was aborted before reaching
            // that internal close) used to skip the fallback entirely via
            // `?`, leaving the endpoint to log iroh's own "dropped without
            // calling Endpoint::close" error on drop instead. Same shape
            // `coordinator_iroh::Coordinator::shutdown` already uses.
            //
            // The close can wait for the closing period of a connection with
            // a bad round-trip estimate. Shutdown waits only a bounded time;
            // the close then finishes in the background.
            net_iroh::close_within(
                "connection router",
                net_iroh::ENDPOINT_CLOSE_LIMIT,
                async move {
                    let _ = router.shutdown().await;
                    ep.close().await;
                },
            )
            .await;
        }
        Ok(())
    }
}

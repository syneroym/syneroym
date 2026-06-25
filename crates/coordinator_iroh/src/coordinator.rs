//! Iroh Ecosystem Coordinator
//!
//! Establishes secure peer-to-peer tunnels using the Iroh protocol, hosting
//! the local Iroh endpoint and relay server.

use std::{
    fmt::{Debug, Formatter},
    future,
    net::SocketAddr,
    sync::Arc,
};

use anyhow::{Context, Result};
use axum::{Router, routing::get};
use iroh::{Endpoint, EndpointAddr, SecretKey, protocol::Router as IrohRouter};
use iroh_relay::server::Server;
use reqwest::Client;
use syneroym_core::{
    config::{CoordinatorIrohConfig, CoordinatorRole, RetryPolicy, SubstrateConfig},
    dht_registry::{EndpointInfo, EndpointMechanism, EndpointType, RegistryClient},
    retry::retry_with_backoff,
};
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_router::{RouteHandler, SYNEROYM_ALPN, net_iroh};
use tokio::{net::TcpListener, task::JoinHandle};
use tracing::{debug, info, warn};

use crate::{
    config::build_relay_config,
    info_endpoint::{CoordinatorInfo, InfoState, get_info},
};

pub struct CoordinatorIroh {
    relay_server: Option<Server>,
    iroh_router: Option<IrohRouter>,
    http_info_handle: Option<JoinHandle<()>>,
    registry_registration_handle: Option<JoinHandle<()>>,
    info_addr: Option<SocketAddr>,
}

impl Debug for CoordinatorIroh {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoordinatorIroh")
            .field("relay_server", &self.relay_server.as_ref().map(|_| "Server"))
            .field("iroh_router", &self.iroh_router.as_ref().map(|_| "Router"))
            .field("http_info_handle", &self.http_info_handle)
            .field("registry_registration_handle", &self.registry_registration_handle)
            .field("info_addr", &self.info_addr)
            .finish()
    }
}

impl CoordinatorIroh {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("Initializing Coordinator IROH");
        let config_roles = config.roles.coordinator.clone();

        let mut relay_server = None;
        let mut iroh_router = None;
        let mut http_info_handle = None;
        let mut registry_registration_handle = None;
        let mut info_addr = None;

        if let Some(role) = &config_roles {
            // --- Architectural Note ---
            // Syneroym Coordinators can fulfill two overlapping but distinct IROH roles:
            //
            // A) The standard IROH Relay Server (spawn_relay_server)
            //    This is an infrastructure-level component that assists *other* IROH nodes
            //    in traversing NATs and establishing peer-to-peer UDP punch-throughs.
            //    It does not read or decrypt Syneroym application payloads.
            //
            // B) The Syneroym Coordinator Endpoint (build_iroh_endpoint &
            // spawn_iroh_router)    This is an active participant in the
            // Syneroym network. It establishes    encrypted connections with
            // clients and other coordinators to facilitate    multi-hop E2E
            // routing, identity verification, and service discovery.
            //
            // A coordinator may run A, B, or both simultaneously.
            if role.iroh.as_ref().is_some_and(|i| i.enable_relay) {
                relay_server = Some(Self::spawn_relay_server(role).await?);
            }

            if let Some(iroh_cfg) = &role.iroh {
                // 1. Build Iroh Endpoint
                let (iroh_endpoint, secret_key) =
                    Self::build_iroh_endpoint(config, role, iroh_cfg, &relay_server).await?;
                let node_id = iroh_endpoint.addr().id;

                // 2. Spawn Iroh Router
                let router = Self::spawn_iroh_router(&iroh_endpoint, iroh_cfg, config);
                iroh_router = Some(router);

                // 3. Start Axum /v1/info HTTP Server
                let (handle, local_addr) = Self::spawn_http_info_server(
                    config,
                    role,
                    iroh_cfg,
                    &iroh_endpoint,
                    &relay_server,
                )
                .await?;
                info_addr = Some(local_addr);
                http_info_handle = Some(handle);

                // 4. Register in Global Registry if requested
                if iroh_cfg.share_in_registry
                    && let Some(registry_url) = &iroh_cfg.community_registry_url
                {
                    let actual_http_addr = relay_server.as_ref().and_then(Server::http_addr);
                    let relay_url_payload = if let Some(parent_url) =
                        config.parent_coordinator.iroh.as_ref().map(|u| &u.url)
                    {
                        Some(parent_url.clone())
                    } else if iroh_cfg.enable_relay {
                        if let Some(addr) = actual_http_addr {
                            Some(format!("http://{addr}"))
                        } else {
                            Some(format!("http://{}", iroh_cfg.http_bind_address))
                        }
                    } else {
                        None
                    };

                    let reg_handle = Self::register_in_global_registry(
                        registry_url.clone(),
                        node_id.to_string(),
                        EndpointAddr::new(node_id),
                        relay_url_payload,
                        secret_key,
                        config.retry.clone(),
                    );
                    registry_registration_handle = Some(reg_handle);
                }
            }
        }

        Ok(Self {
            relay_server,
            iroh_router,
            http_info_handle,
            registry_registration_handle,
            info_addr,
        })
    }

    async fn spawn_relay_server(role: &CoordinatorRole) -> Result<Server> {
        let server_config = build_relay_config(role).await?;
        debug!("Iroh Relay Config built: {:?}", server_config.relay.is_some());
        let server =
            Server::spawn(server_config).await.context("failed to spawn iroh relay server")?;

        if let Some(addr) = server.http_addr() {
            wait_for_relay_server(addr).await?;
        }

        Ok(server)
    }

    async fn build_iroh_endpoint(
        config: &SubstrateConfig,
        _role: &CoordinatorRole,
        iroh_cfg: &CoordinatorIrohConfig,
        relay_server: &Option<Server>,
    ) -> Result<(Endpoint, SecretKey)> {
        let secret_key = SecretKey::generate(&mut rand::rng());

        let mut chosen_relay_url_str = None;
        if let Some(parent_url) = config.parent_coordinator.iroh.as_ref().map(|u| u.url.clone()) {
            info!("Registering with parent coordinator at {}", parent_url);
            chosen_relay_url_str = Some(parent_url);
        } else if iroh_cfg.enable_relay {
            let actual_http_addr = relay_server.as_ref().and_then(Server::http_addr);
            if let Some(addr) = actual_http_addr {
                chosen_relay_url_str = Some(format!("http://{addr}"));
            } else {
                chosen_relay_url_str = Some(format!("http://{}", iroh_cfg.http_bind_address));
            }
        }

        let iroh_endpoint = net_iroh::build_iroh_endpoint(
            chosen_relay_url_str,
            Some(secret_key.clone()),
            iroh_cfg.idle_timeout_secs,
        )
        .await?;
        iroh_endpoint.online().await;

        let node_id = iroh_endpoint.addr().id;
        debug!("Coordinator Iroh endpoint bound: {}", node_id);

        Ok((iroh_endpoint, secret_key))
    }

    fn spawn_iroh_router(
        iroh_endpoint: &Endpoint,
        iroh_cfg: &CoordinatorIrohConfig,
        config: &SubstrateConfig,
    ) -> IrohRouter {
        let parent_relay_url = config.parent_coordinator.iroh.as_ref().map(|u| u.url.clone());
        let registry_client = RegistryClient::new(
            config.substrate.enable_bep0044_dht,
            iroh_cfg.community_registry_url.clone(),
        );
        let route_handler = RouteHandler::new_coordinator(
            iroh_endpoint.clone(),
            registry_client,
            parent_relay_url,
            config.retry.clone(),
        );

        IrohRouter::builder(iroh_endpoint.clone()).accept(SYNEROYM_ALPN, route_handler).spawn()
    }

    async fn spawn_http_info_server(
        config: &SubstrateConfig,
        _role: &CoordinatorRole,
        iroh_cfg: &CoordinatorIrohConfig,
        iroh_endpoint: &Endpoint,
        relay_server: &Option<Server>,
    ) -> Result<(JoinHandle<()>, SocketAddr)> {
        let mut http_info_addr: SocketAddr =
            iroh_cfg.http_bind_address.parse().context("invalid http_bind_address")?;
        // Add 10 to the port to avoid conflict, or bind to port 0 for dynamic port in
        // tests
        if http_info_addr.port() != 0 {
            http_info_addr.set_port(http_info_addr.port() + 10);
        }

        let listener = TcpListener::bind(http_info_addr).await?;
        let local_addr = listener.local_addr()?;
        info!("Coordinator /v1/info listening on {}", local_addr);

        let endpoint_addr = iroh_endpoint.addr();
        let node_id = endpoint_addr.id;
        let endpoint_addr_payload = EndpointAddr::new(node_id);
        let endpoint_addr_bytes = serde_json::to_vec(&endpoint_addr_payload)?;
        let parent_relay_url = config.parent_coordinator.iroh.as_ref().map(|u| u.url.clone());
        let actual_http_addr = relay_server.as_ref().and_then(Server::http_addr);
        let relay_url = if let Some(parent_url) = parent_relay_url.as_ref() {
            Some(parent_url.clone())
        } else if iroh_cfg.enable_relay {
            if let Some(addr) = actual_http_addr {
                Some(format!("http://{addr}"))
            } else {
                Some(format!("http://{}", iroh_cfg.http_bind_address))
            }
        } else {
            None
        };

        let info_state = Arc::new(InfoState {
            info: CoordinatorInfo {
                endpoint_addr_bytes,
                substrate_id: node_id.to_string(),
                relay_url,
                parent_coordinator_url: parent_relay_url.clone(),
            },
        });

        let app = Router::new().route("/v1/info", get(get_info)).with_state(info_state);

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                warn!("HTTP info server error: {}", e);
            }
        });

        Ok((handle, local_addr))
    }

    fn register_in_global_registry(
        registry_url: String,
        node_id_str: String,
        endpoint_addr_payload: EndpointAddr,
        relay_url_payload: Option<String>,
        secret_key: SecretKey,
        retry_policy: RetryPolicy,
    ) -> JoinHandle<()> {
        let secret_key_bytes = secret_key.to_bytes();
        tokio::spawn(async move {
            let res = retry_with_backoff(&retry_policy, || {
                let registry_url = registry_url.clone();
                let node_id_str = node_id_str.clone();
                let endpoint_addr_payload = endpoint_addr_payload.clone();
                let relay_url_payload = relay_url_payload.clone();
                let secret_key_bytes = secret_key_bytes.clone();
                async move {
                    register_coordinator_in_registry(
                        &registry_url,
                        &node_id_str,
                        &endpoint_addr_payload,
                        relay_url_payload,
                        &secret_key_bytes,
                    )
                    .await
                }
            })
            .await;

            match res {
                Ok(()) => {
                    info!("Coordinator successfully registered in global registry");
                }
                Err(e) => {
                    warn!(
                        "Failed to register coordinator in global registry after all retries: {}",
                        e
                    );
                }
            }
        })
    }

    #[must_use]
    pub const fn info_addr(&self) -> Option<SocketAddr> {
        self.info_addr
    }

    #[must_use]
    pub fn endpoint_addr(&self) -> Option<EndpointAddr> {
        self.iroh_router.as_ref().map(|r| r.endpoint().addr())
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Running Coordinator IROH");

        let mut relay_fut = std::pin::pin!(async {
            if let Some(server) = &mut self.relay_server {
                server.task_handle().await.context("iroh relay server task panicked")??;
                Ok(())
            } else {
                future::pending::<Result<()>>().await
            }
        });

        let mut router_fut = std::pin::pin!(async {
            if let Some(router) = &self.iroh_router {
                router.endpoint().closed().await;
                Ok(())
            } else {
                future::pending::<Result<()>>().await
            }
        });

        tokio::select! {
            res = &mut relay_fut => {
                res?;
            }
            res = &mut router_fut => {
                res?;
            }
        }

        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down Coordinator IROH");
        if let Some(handle) = self.registry_registration_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.http_info_handle.take() {
            handle.abort();
        }
        if let Some(router) = self.iroh_router.take() {
            let ep = router.endpoint().clone();
            let _ = router.shutdown().await;
            let _ = ep.close().await;
        }
        if let Some(server) = self.relay_server.take() {
            server.shutdown().await.context("failed to cleanly shutdown iroh relay server")?;
        }
        Ok(())
    }
}

async fn register_coordinator_in_registry(
    registry_url: &str,
    node_id_str: &str,
    endpoint_addr: &EndpointAddr,
    relay_url: Option<String>,
    secret_key_bytes: &[u8; 32],
) -> Result<()> {
    let endpoint_addr_bytes = serde_json::to_vec(endpoint_addr)
        .map_err(|e| anyhow::anyhow!("Failed to serialize endpoint addr: {e}"))?;

    let identity = Identity::from_bytes(secret_key_bytes);
    let did = derive_did_key(&identity.public_key());

    let info = EndpointInfo {
        service_id: did.clone(),
        substrate_id: did.clone(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some(format!("coordinator-{}", &node_id_str[..8])),
        mechanisms: vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url }],
        is_private: false,
        ttl: None,
    };

    let signed_info = info.sign(&identity)?;
    let client = Client::new();
    let url = format!("{registry_url}/register");
    let res = client.post(&url).json(&signed_info).send().await?;

    if !res.status().is_success() {
        return Err(anyhow::anyhow!("Registry registration returned status: {}", res.status()));
    }

    Ok(())
}

async fn wait_for_relay_server(addr: std::net::SocketAddr) -> Result<()> {
    let url = format!("http://{}", addr);
    let client = reqwest::Client::new();
    let mut attempts = 0;
    loop {
        // We don't care about the status code (e.g. 404), just that the server accepts
        // the connection and responds.
        if client.get(&url).send().await.is_ok() {
            break;
        }
        attempts += 1;
        if attempts > 30 {
            anyhow::bail!("Relay server failed to start accepting connections at {}", url);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Ok(())
}

//! Iroh Ecosystem Coordinator
//!
//! Establishes secure peer-to-peer tunnels using the Iroh protocol, hosting
//! the local Iroh endpoint and relay server.

use std::{
    fmt::{self, Debug, Formatter},
    future,
    net::SocketAddr,
    pin,
    sync::{Arc, atomic::AtomicUsize},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use axum::{Router, routing::get};
use iroh::{
    Endpoint, EndpointAddr, RelayUrl, SecretKey, TransportAddr, protocol::Router as IrohRouter,
};
use iroh_relay::server::Server;
use reqwest::Client;
use syneroym_core::{
    config::{CoordinatorIrohConfig, CoordinatorRole, RetryPolicy, SubstrateConfig},
    dht_registry::{
        DEFAULT_ENDPOINT_NOT_AFTER_SECS, EndpointInfo, EndpointMechanism, EndpointType,
        RegistryClient,
    },
    retry::retry_with_backoff,
    tls::TlsCertLoader,
};
use syneroym_identity::{Identity, substrate::derive_did_key};
use syneroym_router::{RouteHandler, SYNEROYM_ALPN, net_iroh};
use tokio::{net::TcpListener, task::JoinHandle, time};
use tracing::{debug, info, warn};

use crate::{
    config::{build_relay_config, relay_access},
    info_endpoint::{InfoState, get_info},
};

pub struct CoordinatorIroh {
    relay_server: Option<Server>,
    iroh_router: Option<IrohRouter>,
    http_info_handle: Option<JoinHandle<Result<()>>>,
    registry_registration_handle: Option<JoinHandle<()>>,
    info_addr: Option<SocketAddr>,
}

impl Debug for CoordinatorIroh {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoordinatorIroh")
            .field("relay_server", &self.relay_server.as_ref().map(|_| "Server"))
            .field("iroh_router", &self.iroh_router.as_ref().map(|_| "Router"))
            .field("http_info_handle", &self.http_info_handle)
            .field("registry_registration_handle", &self.registry_registration_handle)
            .field("info_addr", &self.info_addr)
            .finish()
    }
}

/// Resolves the relay URL to advertise/dial for this coordinator: the parent
/// coordinator's relay if configured, otherwise the locally spawned relay
/// server's address (preferring its HTTPS listener when relay TLS is
/// configured, since that is the address that speaks the relay protocol),
/// otherwise `None` if no relay is enabled.
fn resolve_relay_url(
    parent_relay_url: Option<&RelayUrl>,
    relay_server: &Option<Server>,
    iroh_cfg: &CoordinatorIrohConfig,
) -> Result<Option<RelayUrl>> {
    if let Some(parent_url) = parent_relay_url {
        return Ok(Some(parent_url.clone()));
    }
    if !iroh_cfg.enable_relay {
        return Ok(None);
    }
    let addr = relay_server.as_ref().and_then(|server| {
        server
            .https_addr()
            .map(|addr| format!("https://{addr}"))
            .or_else(|| server.http_addr().map(|addr| format!("http://{addr}")))
    });
    let raw = addr.unwrap_or_else(|| format!("http://{}", iroh_cfg.http_bind_address));
    net_iroh::parse_relay_url("roles.coordinator.iroh.http_bind_address", &raw).map(Some)
}

impl CoordinatorIroh {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("Initializing Coordinator IROH");
        let parent_relay_url = net_iroh::parent_relay_url(&config.parent_coordinator.iroh)?;
        // Note: `role.tls` (relay TLS) is intentionally independent of `config.tls`
        // (the public /v1/info HTTPS endpoint's cert). They secure different
        // surfaces and must be configured separately; a relay operator must opt in
        // explicitly by setting `role.tls`.
        let config_roles = config.roles.coordinator.clone();

        let mut relay_server = None;
        let mut iroh_router = None;
        let mut http_info_handle = None;
        let mut registry_registration_handle = None;
        let mut info_addr = None;

        if let Some(role) = &config_roles {
            // Checked even when the relay is off, so a value this build cannot enforce
            // stops the coordinator at startup instead of waiting unused.
            relay_access(&role.access)?;

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
                let relay_url =
                    resolve_relay_url(parent_relay_url.as_ref(), &relay_server, iroh_cfg)?;

                // 1. Build Iroh Endpoint
                let (iroh_endpoint, secret_key) =
                    Self::build_iroh_endpoint(iroh_cfg, relay_url.clone()).await?;
                let node_id = iroh_endpoint.addr().id;

                // 2. Spawn Iroh Router
                let (router, active_conns) = Self::spawn_iroh_router(
                    &iroh_endpoint,
                    iroh_cfg,
                    config,
                    parent_relay_url.as_ref().map(ToString::to_string),
                );
                iroh_router = Some(router);

                // 3. Start Axum /v1/info HTTP Server
                let (handle, local_addr) = Self::spawn_http_info_server(
                    config,
                    iroh_cfg,
                    &iroh_endpoint,
                    relay_url.clone(),
                    parent_relay_url.clone(),
                    active_conns,
                )
                .await?;
                info_addr = Some(local_addr);
                http_info_handle = Some(handle);

                // 4. Register in Global Registry if requested
                if iroh_cfg.share_in_registry
                    && let Some(registry_url) = &iroh_cfg.community_registry_url
                {
                    let relay_url_payload = relay_url.as_ref().map(ToString::to_string);

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
        iroh_cfg: &CoordinatorIrohConfig,
        relay_url: Option<RelayUrl>,
    ) -> Result<(Endpoint, SecretKey)> {
        let secret_key = SecretKey::generate(&mut rand::rng());

        let iroh_endpoint = net_iroh::build_iroh_endpoint(
            relay_url.clone(),
            Some(secret_key.clone()),
            iroh_cfg.idle_timeout_secs,
        )
        .await?;
        if relay_url.is_some() {
            log_when_online(iroh_endpoint.clone());
        } else {
            info!("coordinator Iroh endpoint has no relay; it dials peers through their relays");
        }

        let node_id = iroh_endpoint.addr().id;
        debug!("Coordinator Iroh endpoint bound: {}", node_id);

        Ok((iroh_endpoint, secret_key))
    }

    fn spawn_iroh_router(
        iroh_endpoint: &Endpoint,
        iroh_cfg: &CoordinatorIrohConfig,
        config: &SubstrateConfig,
        parent_relay_url: Option<String>,
    ) -> (IrohRouter, Arc<AtomicUsize>) {
        let registry_client = RegistryClient::new(
            config.substrate.enable_bep0044_dht,
            iroh_cfg.community_registry_url.clone(),
        );
        let route_handler = RouteHandler::new_coordinator(
            iroh_endpoint.clone(),
            registry_client,
            parent_relay_url,
            config.retry.clone(),
            iroh_cfg.max_connections,
        );
        let active_connections = route_handler.active_connections();

        let router =
            IrohRouter::builder(iroh_endpoint.clone()).accept(SYNEROYM_ALPN, route_handler).spawn();
        (router, active_connections)
    }

    async fn spawn_http_info_server(
        config: &SubstrateConfig,
        iroh_cfg: &CoordinatorIrohConfig,
        iroh_endpoint: &Endpoint,
        relay_url: Option<RelayUrl>,
        parent_relay_url: Option<RelayUrl>,
        active_connections: Arc<AtomicUsize>,
    ) -> Result<(JoinHandle<Result<()>>, SocketAddr)> {
        let http_info_addr: SocketAddr = if let Some(addr_str) = &iroh_cfg.info_http_bind_address {
            addr_str.parse().context("invalid info_http_bind_address")?
        } else {
            let mut addr: SocketAddr =
                iroh_cfg.http_bind_address.parse().context("invalid http_bind_address")?;
            if addr.port() != 0 {
                addr.set_port(addr.port() + 10);
            }
            addr
        };

        let listener = TcpListener::bind(http_info_addr).await?;
        let local_addr = listener.local_addr()?;
        info!("Coordinator /v1/info listening on {}", local_addr);

        let node_id = iroh_endpoint.id();
        // Direct addresses are left out because they cannot be dialed between two
        // processes that share only a relay, and link-local addresses lose their
        // scope id. The relay address comes from the chosen URL, because boot does
        // not wait for the endpoint to come online.
        let endpoint_addr_payload =
            EndpointAddr::from_parts(node_id, relay_url.clone().map(TransportAddr::Relay));
        let endpoint_addr_bytes = serde_json::to_vec(&endpoint_addr_payload)?;

        let tls_cert_path = config.tls.as_ref().map(|t| t.cert_path.clone());
        let is_relay_enabled = iroh_cfg.enable_relay;

        let info_state = Arc::new(InfoState {
            endpoint_addr_bytes,
            substrate_id: node_id.to_string(),
            relay_url: relay_url.map(|u| u.to_string()),
            parent_coordinator_url: parent_relay_url.map(|u| u.to_string()),
            active_connections,
            max_connections: iroh_cfg.max_connections,
            tls_cert_path,
            is_relay_enabled,
            registry_client: RegistryClient::new(
                config.substrate.enable_bep0044_dht,
                iroh_cfg.community_registry_url.clone(),
            ),
        });

        let app = Router::new().route("/v1/info", get(get_info)).with_state(info_state);

        let tls_cfg = config.tls.clone();
        let handle = tokio::spawn(async move {
            if let Some(tls_cfg) = tls_cfg {
                info!("Starting HTTPS server with TLS on {}", local_addr);
                let loader =
                    TlsCertLoader::new(tls_cfg.cert_path.clone(), tls_cfg.key_path.clone())
                        .await
                        .context("Failed to initialize TLS config")?;

                if tls_cfg.reload_on_sigusr1 {
                    loader.spawn_watcher(tls_cfg.cert_path.clone(), tls_cfg.key_path.clone());
                }

                let std_listener =
                    listener.into_std().context("Failed to convert TcpListener to std")?;
                let server = axum_server::from_tcp_rustls(std_listener, loader.config())
                    .context("Failed to create TLS server from TcpListener")?;

                server.serve(app.into_make_service()).await.context("HTTPS info server error")?;
            } else {
                axum::serve(listener, app).await.context("HTTP info server error")?;
            }
            Ok(())
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

        let mut relay_fut = pin::pin!(async {
            if let Some(server) = &mut self.relay_server {
                server.task_handle().await.context("iroh relay server task panicked")??;
                Ok(())
            } else {
                future::pending::<Result<()>>().await
            }
        });

        let mut router_fut = pin::pin!(async {
            if let Some(router) = &self.iroh_router {
                router.endpoint().closed().await;
                Ok(())
            } else {
                future::pending::<Result<()>>().await
            }
        });

        let mut http_fut = pin::pin!(async {
            if let Some(handle) = self.http_info_handle.as_mut() {
                match handle.await {
                    Ok(res) => res.context("HTTPS/HTTP info server failed"),
                    Err(e) => Err(anyhow::anyhow!("HTTPS/HTTP info server task panicked: {e}")),
                }
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
            res = &mut http_fut => {
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

    let not_after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_add(DEFAULT_ENDPOINT_NOT_AFTER_SECS);

    let info = EndpointInfo {
        service_id: did.clone(),
        substrate_id: did.clone(),
        endpoint_type: EndpointType::Substrate,
        nickname: Some(format!("coordinator-{}", &node_id_str[..8])),
        mechanisms: vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url }],
        is_private: false,
        ttl: None,
        not_after,
        generation: 0,
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

async fn wait_for_relay_server(addr: SocketAddr) -> Result<()> {
    let url = format!("http://{addr}");
    let client = Client::new();
    let mut attempts = 0;
    loop {
        // We don't care about the status code (e.g. 404), just that the server accepts
        // the connection and responds.
        if client.get(&url).send().await.is_ok() {
            break;
        }
        attempts += 1;
        if attempts > 30 {
            anyhow::bail!("Relay server failed to start accepting connections at {url}");
        }
        time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

const ONLINE_LOG_TIMEOUT: Duration = Duration::from_secs(30);

fn log_when_online(endpoint: Endpoint) {
    tokio::spawn(async move {
        tokio::select! {
            res = time::timeout(ONLINE_LOG_TIMEOUT, endpoint.online()) => match res {
                Ok(()) => debug!("coordinator Iroh endpoint is online"),
                Err(_) => warn!(
                    "coordinator Iroh endpoint did not reach its relay within {ONLINE_LOG_TIMEOUT:?}"
                ),
            },
            () = endpoint.closed() => {}
        }
    });
}

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
    time::Duration,
};

use anyhow::{Context, Result};
use axum::Router;
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey, TransportAddr,
    protocol::Router as IrohRouter,
};
use iroh_relay::server::Server;
use reqwest::Client;
use syneroym_core::{
    config::{CoordinatorIrohConfig, CoordinatorRole, SubstrateConfig},
    dht_registry::{MasterAnchorResolver, RegistryClient},
    host_identity::HostIdentity,
    tls::TlsCertLoader,
};
use syneroym_passport::LeaseStore;
use syneroym_router::{RouteHandler, SYNEROYM_ALPN, net_iroh};
use tokio::{net::TcpListener, sync::watch, task::JoinHandle, time};
use tracing::{debug, info, warn};

use crate::{
    config::{self, build_relay_config},
    info_endpoint::{self, InfoState},
    passport::{self, RelayPassports},
    registration,
};

pub struct CoordinatorIroh {
    relay_server: Option<Server>,
    iroh_router: Option<IrohRouter>,
    http_info_handle: Option<JoinHandle<Result<()>>>,
    /// Keeps the coordinator registered in its community registry.
    registry_registration_handle: Option<JoinHandle<()>>,
    info_addr: Option<SocketAddr>,
    /// The lease store and registrar of a relay with passport access.
    relay_passports: Option<Arc<RelayPassports>>,
}

impl Debug for CoordinatorIroh {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoordinatorIroh")
            .field("relay_server", &self.relay_server.as_ref().map(|_| "Server"))
            .field("iroh_router", &self.iroh_router.as_ref().map(|_| "Router"))
            .field("http_info_handle", &self.http_info_handle)
            .field("registry_registration_handle", &self.registry_registration_handle)
            .field("info_addr", &self.info_addr)
            .field("relay_passports", &self.relay_passports.is_some())
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

/// The address `/v1/info` gives to clients. Direct addresses are left out
/// because they cannot be dialed between two processes that share only a
/// relay, and link-local addresses lose their scope id. The relay address
/// comes from the chosen URL, because boot does not wait for the endpoint to
/// come online.
fn info_addr_bytes(node_id: EndpointId, relay_url: Option<&RelayUrl>) -> Result<Vec<u8>> {
    let addr = EndpointAddr::from_parts(node_id, relay_url.cloned().map(TransportAddr::Relay));
    Ok(serde_json::to_vec(&addr)?)
}

/// What `start_endpoint` starts. `init` stores each part in `CoordinatorIroh`.
struct EndpointParts {
    iroh_router: IrohRouter,
    http_info_handle: JoinHandle<Result<()>>,
    info_addr: SocketAddr,
    registry_registration_handle: Option<JoinHandle<()>>,
}

/// Arguments passed to `start_endpoint`.
struct EndpointArgs<'a> {
    config: &'a SubstrateConfig,
    iroh_cfg: &'a CoordinatorIrohConfig,
    host: &'a HostIdentity,
    secret_key: SecretKey,
    parent_relay_url: Option<&'a RelayUrl>,
    relay_server: &'a Option<Server>,
    relay_passports: Option<Arc<RelayPassports>>,
}

impl CoordinatorIroh {
    pub async fn init(config: &SubstrateConfig, host: &HostIdentity) -> Result<Self> {
        info!("Initializing Coordinator IROH");
        let parent_relay_url = net_iroh::parent_relay_url(&config.parent_coordinator.iroh)?;
        // Note: `role.tls` (relay TLS) is intentionally independent of `config.tls`
        // (the public /v1/info HTTPS endpoint's cert). They secure different
        // surfaces and must be configured separately; a relay operator must opt in
        // explicitly by setting `role.tls`.
        if let Some(role) = &config.roles.coordinator {
            Self::start_role(config, role, host, parent_relay_url.as_ref()).await
        } else {
            Ok(Self {
                relay_server: None,
                iroh_router: None,
                http_info_handle: None,
                registry_registration_handle: None,
                info_addr: None,
                relay_passports: None,
            })
        }
    }

    /// The lease store of a relay with passport access, for the operator's
    /// `deny` and lease list. `None` when the relay does not accept passports.
    #[must_use]
    pub fn relay_leases(&self) -> Option<LeaseStore> {
        self.relay_passports.as_ref().map(|p| p.leases().clone())
    }

    async fn start_role(
        config: &SubstrateConfig,
        role: &CoordinatorRole,
        host: &HostIdentity,
        parent_relay_url: Option<&RelayUrl>,
    ) -> Result<Self> {
        let relay_on = role.iroh.as_ref().is_some_and(|i| i.enable_relay);
        // Checked even when the relay is off, so a value that cannot work stops
        // the coordinator at startup instead of waiting unused.
        config::check_access(&role.access, config.tls.is_some(), relay_on)?;
        if let Some(iroh_cfg) = &role.iroh {
            config::check_relay_cap(iroh_cfg, parent_relay_url.is_some())?;
        }

        // Made before the relay server, so the relay can always admit the
        // coordinator's own endpoint.
        let secret_key = SecretKey::generate(&mut rand::rng());
        let relay_passports = if relay_on && role.access.accepts_passports() {
            let resolver: Arc<dyn MasterAnchorResolver> = Arc::new(RegistryClient::new(
                config.substrate.enable_bep0044_dht,
                config.substrate.registry_url.clone(),
            ));
            Some(Arc::new(RelayPassports::open(&config.storage.db_dir, host, resolver).await?))
        } else {
            None
        };

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
        let relay_server = if relay_on {
            let always = [secret_key.public(), passport::endpoint_of_did(&host.node_did)?];
            let leases = relay_passports.as_ref().map(|p| p.leases().clone());
            Some(Self::spawn_relay_server(role, &always, leases).await?)
        } else {
            None
        };

        let parts = match &role.iroh {
            Some(iroh_cfg) => Some(
                Self::start_endpoint(EndpointArgs {
                    config,
                    iroh_cfg,
                    host,
                    secret_key,
                    parent_relay_url,
                    relay_server: &relay_server,
                    relay_passports: relay_passports.clone(),
                })
                .await?,
            ),
            None => None,
        };

        let (iroh_router, http_info_handle, info_addr, registry_registration_handle) = match parts {
            Some(p) => (
                Some(p.iroh_router),
                Some(p.http_info_handle),
                Some(p.info_addr),
                p.registry_registration_handle,
            ),
            None => (None, None, None, None),
        };

        Ok(Self {
            relay_server,
            iroh_router,
            http_info_handle,
            registry_registration_handle,
            info_addr,
            relay_passports,
        })
    }

    async fn spawn_relay_server(
        role: &CoordinatorRole,
        always: &[EndpointId],
        leases: Option<LeaseStore>,
    ) -> Result<Server> {
        let server_config = build_relay_config(role, always, leases).await?;
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
        secret_key: &SecretKey,
    ) -> Result<Endpoint> {
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

        Ok(iroh_endpoint)
    }

    fn spawn_iroh_router(
        iroh_endpoint: &Endpoint,
        iroh_cfg: &CoordinatorIrohConfig,
        config: &SubstrateConfig,
        parent_relay_url: Option<String>,
    ) -> Result<(IrohRouter, Arc<AtomicUsize>)> {
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
        )?;
        let active_connections = route_handler.active_connections();

        let router =
            IrohRouter::builder(iroh_endpoint.clone()).accept(SYNEROYM_ALPN, route_handler).spawn();
        Ok((router, active_connections))
    }

    async fn start_endpoint(args: EndpointArgs<'_>) -> Result<EndpointParts> {
        let EndpointArgs {
            config,
            iroh_cfg,
            host,
            secret_key,
            parent_relay_url,
            relay_server,
            relay_passports,
        } = args;
        let relay_url = resolve_relay_url(parent_relay_url, relay_server, iroh_cfg)?;
        let endpoint = Self::build_iroh_endpoint(iroh_cfg, relay_url.clone(), &secret_key).await?;
        let (iroh_router, active_connections) = Self::spawn_iroh_router(
            &endpoint,
            iroh_cfg,
            config,
            parent_relay_url.map(ToString::to_string),
        )?;

        let registry_url =
            iroh_cfg.community_registry_url.clone().filter(|_| iroh_cfg.share_in_registry);
        let (registered_tx, registered_rx) = watch::channel(false);

        let coord_did = registration::coordinator_did(&secret_key);
        let state = InfoState {
            endpoint_addr_bytes: info_addr_bytes(endpoint.id(), relay_url.as_ref())?,
            substrate_id: endpoint.id().to_string(),
            relay_url: relay_url.as_ref().map(ToString::to_string),
            parent_coordinator_url: parent_relay_url.map(ToString::to_string),
            active_connections,
            max_connections: iroh_cfg.max_connections,
            tls_cert_path: config.tls.as_ref().map(|t| t.cert_path.clone()),
            relay_metrics: relay_server.as_ref().map(|s| s.metrics().server.clone()),
            max_relay_clients: iroh_cfg.max_relay_clients,
            registry: registry_url.is_some().then(|| (coord_did.clone(), registered_rx)),
            identity: host.identity_info(),
        };
        let app = info_endpoint::info_router(state).merge(passport::routes(relay_passports));
        let (http_info_handle, info_addr) =
            Self::spawn_http_info_server(config, iroh_cfg, app).await?;

        let registry_registration_handle = registry_url.map(|url| {
            registration::spawn(
                url,
                secret_key,
                relay_url.as_ref().map(ToString::to_string),
                config.retry.clone(),
                registered_tx,
            )
        });

        Ok(EndpointParts { iroh_router, http_info_handle, info_addr, registry_registration_handle })
    }

    async fn spawn_http_info_server(
        config: &SubstrateConfig,
        iroh_cfg: &CoordinatorIrohConfig,
        app: Router,
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

                server
                    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                    .await
                    .context("HTTPS info server error")?;
            } else {
                axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
                    .await
                    .context("HTTP info server error")?;
            }
            Ok(())
        });

        Ok((handle, local_addr))
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
            let _ = handle.await;
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

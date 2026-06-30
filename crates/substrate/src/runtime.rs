//! Substrate execution runtime
//!
//! Manages the lifecycle of all substrate components including the App Sandbox,
//! Observability engine, Router, Client Gateway, and Coordinators.

use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    future,
    future::Future,
    path::PathBuf,
    time::Duration,
};

use axum::{Json, Router, routing};
use syneroym_client_gateway::ClientGateway;
use syneroym_community_registry::EcosystemRegistry;
use syneroym_coordinator::EcosystemCoordinator;
use syneroym_core::{
    config::SubstrateConfig,
    dht_registry::{
        EndpointInfo, EndpointMechanism, EndpointType, HEARTBEAT_INTERVAL_SECS, RegistryClient,
        SignedEndpointInfo,
    },
    local_registry::{EndpointRegistry, SubstrateEndpoint},
};
use syneroym_data_layer::registry_store;
use syneroym_identity::Identity;
use syneroym_observability::{MemoryRecorder, MetricsSnapshot, ObservabilityEngine};
use syneroym_router::ConnectionRouter;
use tokio::{fs, net::TcpListener, signal, time};
use tracing::{debug, error, info, warn};

use crate::identity;

/// Runs the substrate given the consolidated configuration, using the default
/// ctrl-c shutdown signal.
pub async fn run(config: SubstrateConfig) -> anyhow::Result<()> {
    init_and_run_with_signal(config, async {
        let _ = signal::ctrl_c().await;
    })
    .await
}

pub struct InitializedRuntime {
    pub observability: ObservabilityEngine,
    pub services: RuntimeServices,
    pub connection_router: ConnectionRouter,
}

impl Debug for InitializedRuntime {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitializedRuntime")
            .field("observability", &"ObservabilityEngine")
            .field("services", &self.services)
            .field("connection_router", &"ConnectionRouter")
            .finish()
    }
}

/// Runs the substrate given the consolidated configuration and a custom
/// shutdown signal.
pub async fn init_and_run_with_signal<F>(
    config: SubstrateConfig,
    shutdown_signal: F,
) -> anyhow::Result<()>
where
    F: Future<Output = ()>,
{
    let runtime = init(config.clone()).await?;
    run_with_signal(config, runtime, shutdown_signal).await
}

/// Runs the substrate given the consolidated configuration and a custom
/// shutdown signal.
pub async fn run_with_signal<F>(
    config: SubstrateConfig,
    mut runtime: InitializedRuntime,
    shutdown_signal: F,
) -> anyhow::Result<()>
where
    F: Future<Output = ()>,
{
    runtime.services.run_until_shutdown(&config, &runtime.connection_router, shutdown_signal).await;

    info!("shutting down substrate components");
    runtime.services.shutdown().await;

    if let Err(error) = runtime.observability.shutdown().await {
        error!(error = %error, "error flushing observability data");
    }

    if let Err(error) = runtime.connection_router.shutdown().await {
        error!(error = %error, "error shutting down connection router");
    }

    info!("shutdown complete");
    Ok(())
}

/// Runs the substrate given the consolidated configuration and a custom
/// shutdown signal.
pub async fn init(config: SubstrateConfig) -> anyhow::Result<InitializedRuntime> {
    info!(profile = %config.profile, "initializing substrate");

    Ok(InitializedRuntime {
        observability: ObservabilityEngine::init(&config)?,
        services: RuntimeServices::init(&config).await?,
        connection_router: setup_connection_router(&config).await?,
    })
}

pub struct RuntimeServices {
    #[cfg(feature = "community_registry")]
    community_registry: Option<EcosystemRegistry>,
    #[cfg(feature = "coordinator")]
    coordinator: Option<EcosystemCoordinator>,
    #[cfg(feature = "client_gateway")]
    client_gateway: Option<ClientGateway>,
}

impl Debug for RuntimeServices {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("RuntimeServices");

        #[cfg(feature = "community_registry")]
        debug_struct.field(
            "community_registry",
            &self.community_registry.as_ref().map(|_| "EcosystemRegistry"),
        );

        #[cfg(feature = "coordinator")]
        debug_struct.field("coordinator", &self.coordinator);

        #[cfg(feature = "client_gateway")]
        debug_struct.field("client_gateway", &self.client_gateway);

        debug_struct.finish()
    }
}

impl RuntimeServices {
    async fn init(config: &SubstrateConfig) -> anyhow::Result<Self> {
        Ok(Self {
            #[cfg(feature = "community_registry")]
            community_registry: if config.roles.community_registry.is_some() {
                Some(EcosystemRegistry::init(config).await?)
            } else {
                None
            },
            #[cfg(feature = "coordinator")]
            coordinator: if config.roles.coordinator.is_some() {
                Some(EcosystemCoordinator::init(config).await?)
            } else {
                None
            },
            #[cfg(feature = "client_gateway")]
            client_gateway: if config.roles.client_gateway.is_some() {
                Some(ClientGateway::init(config).await?)
            } else {
                None
            },
        })
    }

    async fn run_until_shutdown<F>(
        &mut self,
        config: &SubstrateConfig,
        connection_router: &ConnectionRouter,
        shutdown_signal: F,
    ) where
        F: Future<Output = ()>,
    {
        #[cfg(feature = "community_registry")]
        let mut registry_fut = std::pin::pin!(async {
            match self.community_registry.as_mut() {
                Some(service) => service.run().await,
                None => pending_component().await,
            }
        });
        #[cfg(not(feature = "community_registry"))]
        let mut registry_fut = std::pin::pin!(pending_component());

        #[cfg(feature = "coordinator")]
        let mut coordinator_fut = std::pin::pin!(async {
            match self.coordinator.as_mut() {
                Some(service) => service.run().await,
                None => pending_component().await,
            }
        });
        #[cfg(not(feature = "coordinator"))]
        let mut coordinator_fut = std::pin::pin!(pending_component());

        #[cfg(feature = "client_gateway")]
        let mut client_gateway_fut = std::pin::pin!(async {
            match self.client_gateway.as_mut() {
                Some(service) => service.run().await,
                None => pending_component().await,
            }
        });
        #[cfg(not(feature = "client_gateway"))]
        let mut client_gateway_fut = std::pin::pin!(pending_component());

        let mut health_fut = std::pin::pin!(async {
            if let Some(obs) = &config.roles.observability
                && let Some(health) = &obs.health
                && health.enabled
            {
                let app =
                    Router::new().route(&health.endpoint, axum::routing::get(|| async { "OK" }));
                match TcpListener::bind(&health.bind_address).await {
                    Ok(listener) => {
                        if let Ok(addr) = listener.local_addr() {
                            info!("observability health endpoint listening on http://{}", addr);
                        }
                        let _ = axum::serve(listener, app).await;
                    }
                    Err(e) => {
                        error!(
                            "failed to bind health endpoint on {}: {:?}",
                            health.bind_address, e
                        );
                    }
                }
            }
            pending_component().await
        });

        let mut metrics_fut = std::pin::pin!(async {
            if let Some(obs) = &config.roles.observability
                && let Some(metrics_cfg) = &obs.metrics
                && metrics_cfg.enabled
            {
                let app = Router::new().route(
                    &metrics_cfg.endpoint,
                    routing::get(|| async {
                        if let Some(recorder) = MemoryRecorder::global() {
                            let snapshot = recorder.snapshot();
                            Json(snapshot)
                        } else {
                            Json(MetricsSnapshot {
                                counters: HashMap::new(),
                                gauges: HashMap::new(),
                                histograms: HashMap::new(),
                            })
                        }
                    }),
                );
                match TcpListener::bind(&metrics_cfg.bind_address).await {
                    Ok(listener) => {
                        if let Ok(addr) = listener.local_addr() {
                            info!("observability metrics endpoint listening on http://{}", addr);
                        }
                        let _ = axum::serve(listener, app).await;
                    }
                    Err(e) => {
                        error!(
                            "failed to bind metrics endpoint on {}: {:?}",
                            metrics_cfg.bind_address, e
                        );
                    }
                }
            }
            pending_component().await
        });

        let mut connection_router_fut = std::pin::pin!(connection_router.run());
        let mut shutdown_signal = std::pin::pin!(shutdown_signal);

        info!(profile = %config.profile, "starting substrate components");
        tokio::select! {
            res = &mut connection_router_fut => log_component_exit("connection router", res),
            res = &mut registry_fut => log_component_exit("service registry", res),
            res = &mut coordinator_fut => log_component_exit("coordinator", res),
            res = &mut client_gateway_fut => log_component_exit("http proxy", res),
            res = &mut health_fut => log_component_exit("health server", res),
            res = &mut metrics_fut => log_component_exit("metrics server", res),
            () = &mut shutdown_signal => warn!("received shutdown signal"),
        }
    }

    async fn shutdown(&mut self) {
        #[cfg(feature = "client_gateway")]
        if let Some(service) = self.client_gateway.as_mut()
            && let Err(error) = service.shutdown().await
        {
            error!(error = %error, "error shutting down http proxy");
        }

        #[cfg(feature = "coordinator")]
        if let Some(service) = self.coordinator.as_mut()
            && let Err(error) = service.shutdown().await
        {
            error!(error = %error, "error shutting down coordinator");
        }

        #[cfg(feature = "community_registry")]
        if let Some(service) = self.community_registry.as_mut()
            && let Err(error) = service.shutdown().await
        {
            error!(error = %error, "error shutting down service registry");
        }
    }
}

async fn pending_component() -> anyhow::Result<()> {
    future::pending().await
}

fn log_component_exit(component: &str, result: anyhow::Result<()>) {
    match result {
        Ok(()) => info!(component = component, "component finished"),
        Err(error) => {
            error!(component = component, error = %error, "component finished with error");
        }
    }
}

/// Sets up the connection router and its tightly coupled dependencies,
/// including the substrate identity, data store, endpoint registry, and the
/// native service.
async fn setup_connection_router(config: &SubstrateConfig) -> anyhow::Result<ConnectionRouter> {
    let (service_id, secret_key) = setup_identity_and_storage(config).await?;

    let router = setup_router(config, &service_id, secret_key).await?;

    if (config.substrate.enable_bep0044_dht || config.substrate.registry_url.is_some())
        && let Some(endpoint_addr) = router.endpoint_addr()
    {
        let relay_url = config.parent_coordinator.iroh.as_ref().map(|c| c.url.clone());
        publish_to_community_registry(
            config.substrate.registry_url.clone(),
            config.substrate.enable_bep0044_dht,
            service_id,
            endpoint_addr,
            relay_url,
            secret_key,
            config.identity.nickname.clone(),
            config.hosted_apps_dir(),
        );
    }

    Ok(router)
}

async fn setup_identity_and_storage(
    config: &SubstrateConfig,
) -> anyhow::Result<(String, [u8; 32])> {
    let substrate_identity_state =
        identity::setup_substrate_identity(&config.identity, &config.app_data_dir)?;
    let substrate_secret_key = identity::get_secret(&config.identity, &config.app_data_dir)?;
    Ok((substrate_identity_state.did, substrate_secret_key))
}

async fn setup_router(
    config: &SubstrateConfig,
    service_id: &str,
    secret_key: [u8; 32],
) -> anyhow::Result<ConnectionRouter> {
    let data_store = registry_store::init_store(config).await?;
    let endpoint_registry = EndpointRegistry::new(data_store).await?;

    debug!("Registering native SubstrateService at {}", service_id);
    let endpoint = SubstrateEndpoint::NativeHostChannel { service_id: service_id.to_string() };
    endpoint_registry
        .register(service_id.to_string(), "orchestrator".to_string(), endpoint)
        .await?;

    ConnectionRouter::init(endpoint_registry, config.clone(), secret_key, service_id.to_string())
        .await
}

#[allow(clippy::too_many_arguments)]
fn publish_to_community_registry(
    registry_url: Option<String>,
    enable_bep0044_dht: bool,
    service_id: String,
    endpoint_addr: iroh::EndpointAddr,
    relay_url: Option<String>,
    secret_key: [u8; 32],
    nickname: Option<String>,
    hosted_apps_dir: PathBuf,
) {
    tokio::spawn(async move {
        let registry_client = RegistryClient::new(enable_bep0044_dht, registry_url.clone());

        loop {
            // Register native substrate endpoint
            let signed_info = match build_signed_endpoint_info(
                &service_id,
                &endpoint_addr,
                relay_url.clone(),
                &secret_key,
                nickname.clone(),
            ) {
                Ok(info) => info,
                Err(e) => {
                    warn!("Failed to build signed endpoint info: {}", e);
                    time::sleep(Duration::from_secs(60)).await;
                    continue;
                }
            };

            let mut attempts = 0;
            let mut success = false;
            while attempts < 30 {
                if let Err(e) = registry_client.register(&signed_info, false).await {
                    warn!("Failed to register endpoint (attempt {}): {}", attempts + 1, e);
                    time::sleep(Duration::from_millis(500)).await;
                    attempts += 1;
                } else {
                    success = true;
                    break;
                }
            }

            if success {
                info!(
                    service_id = %service_id,
                    "Successfully registered substrate endpoint"
                );
            } else {
                warn!(
                    service_id = %service_id,
                    "Exhausted registration retries. Substrate may be unreachable."
                );
            }

            // Proxy hosted apps
            if hosted_apps_dir.exists()
                && let Ok(mut entries) = fs::read_dir(&hosted_apps_dir).await
            {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    if let Ok(file_type) = entry.file_type().await
                        && file_type.is_file()
                        && let Ok(contents) = fs::read_to_string(entry.path()).await
                        && let Ok(cert) = serde_json::from_str::<SignedEndpointInfo>(&contents)
                    {
                        if let Err(e) = registry_client.register(&cert, false).await {
                            warn!("Failed to register hosted app {}: {}", cert.info.service_id, e);
                        } else {
                            info!("Successfully registered hosted app {}", cert.info.service_id);
                        }
                    }
                }
            }

            // Sleep until the next heartbeat interval
            time::sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
        }
    });
}

fn build_signed_endpoint_info(
    service_id: &str,
    endpoint_addr: &iroh::EndpointAddr,
    relay_url: Option<String>,
    secret_key: &[u8; 32],
    nickname: Option<String>,
) -> anyhow::Result<SignedEndpointInfo> {
    // Prune direct addresses to keep the serialized PKARR record under the
    // 1000-byte DNS limit
    let pruned_addr = iroh::EndpointAddr::new(endpoint_addr.id);
    let endpoint_addr_bytes = serde_json::to_vec(&pruned_addr)
        .map_err(|e| anyhow::anyhow!("Failed to serialize endpoint addr: {e}"))?;

    let info = EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: service_id.to_string(),
        endpoint_type: EndpointType::Substrate,
        nickname,
        mechanisms: vec![EndpointMechanism::Iroh { endpoint_addr_bytes, relay_url }],
        is_private: false,
        ttl: None,
        delegation: None,
    };

    let identity = Identity::from_bytes(secret_key);
    info.sign(&identity)
}

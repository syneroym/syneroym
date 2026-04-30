use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::{EndpointRegistry, SubstrateEndpoint};
use syneroym_observability::ObservabilityEngine;
use syneroym_router::ConnectionRouter;
use tracing::{debug, error, info, warn};

use crate::identity;

/// Runs the substrate given the consolidated configuration, using the default ctrl-c shutdown signal.
pub async fn run(config: SubstrateConfig) -> anyhow::Result<()> {
    init_and_run_with_signal(config, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

pub struct InitializedRuntime {
    pub observability: ObservabilityEngine,
    pub services: RuntimeServices,
    pub connection_router: ConnectionRouter,
}

/// Runs the substrate given the consolidated configuration and a custom shutdown signal.
pub async fn init_and_run_with_signal<F>(
    config: SubstrateConfig,
    shutdown_signal: F,
) -> anyhow::Result<()>
where
    F: std::future::Future<Output = ()>,
{
    let runtime = init(config.clone()).await?;
    run_with_signal(config, runtime, shutdown_signal).await
}

/// Runs the substrate given the consolidated configuration and a custom shutdown signal.
pub async fn run_with_signal<F>(
    config: SubstrateConfig,
    mut runtime: InitializedRuntime,
    shutdown_signal: F,
) -> anyhow::Result<()>
where
    F: std::future::Future<Output = ()>,
{
    runtime
        .services
        .run_until_shutdown(&config.profile, &runtime.connection_router, shutdown_signal)
        .await;

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

/// Runs the substrate given the consolidated configuration and a custom shutdown signal.
pub async fn init(config: SubstrateConfig) -> anyhow::Result<InitializedRuntime> {
    info!(profile = %config.profile, "initializing substrate");

    Ok(InitializedRuntime {
        observability: syneroym_observability::ObservabilityEngine::init(&config)?,
        services: RuntimeServices::init(&config).await?,
        connection_router: setup_connection_router(&config).await?,
    })
}

pub struct RuntimeServices {
    #[cfg(feature = "community_registry")]
    community_registry: Option<syneroym_community_registry::EcosystemRegistry>,
    #[cfg(feature = "coordinator")]
    coordinator: Option<syneroym_coordinator::EcosystemCoordinator>,
    #[cfg(feature = "client_gateway")]
    client_gateway: Option<syneroym_client_gateway::ClientGateway>,
}

impl RuntimeServices {
    async fn init(config: &SubstrateConfig) -> anyhow::Result<Self> {
        Ok(Self {
            #[cfg(feature = "community_registry")]
            community_registry: if config.roles.community_registry.is_some() {
                Some(syneroym_community_registry::EcosystemRegistry::init(config).await?)
            } else {
                None
            },
            #[cfg(feature = "coordinator")]
            coordinator: if config.roles.coordinator.is_some() {
                Some(syneroym_coordinator::EcosystemCoordinator::init(config).await?)
            } else {
                None
            },
            #[cfg(feature = "client_gateway")]
            client_gateway: if config.roles.client_gateway.is_some() {
                Some(syneroym_client_gateway::ClientGateway::init(config).await?)
            } else {
                None
            },
        })
    }

    async fn run_until_shutdown<F>(
        &mut self,
        profile: &str,
        connection_router: &ConnectionRouter,
        shutdown_signal: F,
    ) where
        F: std::future::Future<Output = ()>,
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

        let mut connection_router_fut = std::pin::pin!(connection_router.run());
        let mut shutdown_signal = std::pin::pin!(shutdown_signal);

        info!(profile = %profile, "starting substrate components");
        tokio::select! {
            res = &mut connection_router_fut => log_component_exit("connection router", res),
            res = &mut registry_fut => log_component_exit("service registry", res),
            res = &mut coordinator_fut => log_component_exit("coordinator", res),
            res = &mut client_gateway_fut => log_component_exit("http proxy", res),
            _ = &mut shutdown_signal => warn!("received shutdown signal"),
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
    std::future::pending().await
}

fn log_component_exit(component: &str, result: anyhow::Result<()>) {
    match result {
        Ok(()) => info!(component = component, "component finished"),
        Err(error) => {
            error!(component = component, error = %error, "component finished with error")
        }
    }
}

/// Sets up the connection router and its tightly coupled dependencies, including
/// the substrate identity, data store, endpoint registry, and the native service.
async fn setup_connection_router(config: &SubstrateConfig) -> anyhow::Result<ConnectionRouter> {
    let substrate_identity_state =
        identity::setup_substrate_identity(&config.identity, &config.app_data_dir)?;
    let substrate_secret_key = identity::get_secret(&config.identity, &config.app_data_dir)?;
    let service_id = substrate_identity_state.did.clone();

    let data_store = syneroym_core::storage::init_store(config).await?;
    let endpoint_registry = EndpointRegistry::new(data_store).await?;

    debug!("Registering native SubstrateService at {}", service_id);
    let endpoint = SubstrateEndpoint::NativeHostChannel { channel_details: service_id.clone() };
    endpoint_registry.register(service_id.clone(), "orchestrator".to_string(), endpoint).await?;

    let router = ConnectionRouter::init(
        endpoint_registry,
        config.clone(),
        substrate_secret_key,
        service_id.clone(),
    )
    .await?;

    if let Some(registry_url) = &config.substrate.registry_url
        && let Some(endpoint_addr) = router.endpoint_addr()
    {
        let relay_url = endpoint_addr.relay_urls().next().map(|u| u.to_string());
        let registry_url = registry_url.clone();
        let service_id = service_id.clone();

        tokio::spawn(async move {
            let mut attempts = 0;
            while attempts < 30 {
                if let Err(e) = register_substrate_endpoint(
                    &registry_url,
                    &service_id,
                    &endpoint_addr,
                    relay_url.clone(),
                    &substrate_secret_key,
                )
                .await
                {
                    warn!("Failed to register endpoint (attempt {}): {}", attempts + 1, e);
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    attempts += 1;
                } else {
                    break;
                }
            }
        });
    }

    Ok(router)
}

async fn register_substrate_endpoint<E: serde::Serialize>(
    registry_url: &str,
    service_id: &str,
    endpoint_addr: &E,
    relay_url: Option<String>,
    secret_key: &[u8; 32],
) -> anyhow::Result<()> {
    debug!("Registering substrate endpoint with registry at {}", registry_url);

    let endpoint_addr_bytes = serde_json::to_vec(endpoint_addr)
        .map_err(|e| anyhow::anyhow!("Failed to serialize endpoint addr: {}", e))?;

    let info = syneroym_core::community_registry::EndpointInfo {
        service_id: service_id.to_string(),
        substrate_id: service_id.to_string(),
        endpoint_type: syneroym_core::community_registry::EndpointType::Substrate,
        relay_url,
        endpoint_addr_bytes,
    };

    let info_value = serde_json::to_value(&info)?;
    let canonical_value = syneroym_identity::substrate::canonicalize_json_value(&info_value);
    let canonical_string = serde_json::to_string(&canonical_value)?;

    let signature =
        syneroym_identity::Identity::from_bytes(secret_key).sign(canonical_string.as_bytes());
    let signature_z32 = z32::encode(&signature.to_bytes());

    let signed_info =
        syneroym_core::community_registry::SignedEndpointInfo { info, signature: signature_z32 };

    let client = reqwest::Client::new();
    let url = format!("{}/register", registry_url);
    let res = client.post(&url).json(&signed_info).send().await;
    match res {
        Ok(resp) if resp.status().is_success() => {
            info!("Successfully registered substrate endpoint with registry");
        }
        Ok(resp) => {
            warn!("Failed to register with registry. Status: {}", resp.status());
        }
        Err(e) => {
            warn!("Failed to reach registry: {}", e);
        }
    }
    Ok(())
}

use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::{EndpointRegistry, SubstrateEndpoint};
use syneroym_identity::substrate::resolve_did_z32;
use syneroym_router::ConnectionRouter;
use tracing::{error, info, warn};

use crate::identity;

/// Runs the substrate given the consolidated configuration.
pub async fn run(config: SubstrateConfig) -> anyhow::Result<()> {
    info!(profile = %config.profile, "initializing substrate");

    let observability_engine = syneroym_observability::ObservabilityEngine::init(&config)?;
    let mut services = RuntimeServices::init(&config).await?;
    let connection_router = setup_connection_router(&config).await?;
    services.run_until_shutdown(&config.profile, &connection_router).await;

    info!("shutting down substrate components");
    services.shutdown().await;

    if let Err(e) = observability_engine.shutdown().await {
        error!(error = %e, "error flushing observability data");
    }

    if let Err(e) = connection_router.shutdown().await {
        error!(error = %e, "error shutting down connection router");
    }

    info!("shutdown complete");
    Ok(())
}

struct RuntimeServices {
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

    async fn run_until_shutdown(&mut self, profile: &str, connection_router: &ConnectionRouter) {
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

        info!(profile = %profile, "starting substrate components");
        tokio::select! {
            res = &mut connection_router_fut => log_component_exit("connection router", res),
            res = &mut registry_fut => log_component_exit("service registry", res),
            res = &mut coordinator_fut => log_component_exit("coordinator", res),
            res = &mut client_gateway_fut => log_component_exit("http proxy", res),
            _ = tokio::signal::ctrl_c() => warn!("received ctrl-c signal"),
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
    let service_id = resolve_did_z32(&substrate_identity_state.did)?.to_string();

    let data_store = syneroym_core::storage::init_store(config).await?;
    let endpoint_registry = EndpointRegistry::new(data_store).await?;

    info!("Registering native SubstrateService at {}", service_id);
    let endpoint = SubstrateEndpoint::NativeHostChannel { channel_details: service_id.clone() };
    endpoint_registry.register(service_id.clone(), "orchestrator".to_string(), endpoint).await?;

    ConnectionRouter::init(
        endpoint_registry,
        config.clone(),
        substrate_secret_key,
        service_id.clone(),
    )
    .await
}

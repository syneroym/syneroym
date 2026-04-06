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

    #[cfg(feature = "community_registry")]
    let mut community_registry =
        syneroym_community_registry::EcosystemRegistry::init(&config).await?;

    #[cfg(feature = "coordinator")]
    let mut coordinator = syneroym_coordinator::EcosystemCoordinator::init(&config).await?;

    #[cfg(feature = "client_gateway")]
    let mut client_gateway = syneroym_client_gateway::ClientGateway::init(&config).await?;

    let connection_router = setup_connection_router(&config).await?;

    {
        // Disabled components use pending futures so the select loop only reacts
        // to active services or an external shutdown signal.
        #[cfg(feature = "community_registry")]
        let mut registry_fut = std::pin::pin!(async {
            if config.roles.community_registry.is_some() {
                community_registry.run().await
            } else {
                std::future::pending().await
            }
        });
        #[cfg(not(feature = "community_registry"))]
        let mut registry_fut = std::pin::pin!(std::future::pending::<anyhow::Result<()>>());

        #[cfg(feature = "coordinator")]
        let mut coordinator_bridge_fut = std::pin::pin!(async {
            if config.roles.coordinator.is_some() {
                coordinator.run().await
            } else {
                std::future::pending().await
            }
        });
        #[cfg(not(feature = "coordinator"))]
        let mut coordinator_bridge_fut =
            std::pin::pin!(std::future::pending::<anyhow::Result<()>>());

        #[cfg(feature = "client_gateway")]
        let mut client_gateway_fut = std::pin::pin!(async {
            if config.roles.client_gateway.is_some() {
                client_gateway.run().await
            } else {
                std::future::pending().await
            }
        });
        #[cfg(not(feature = "client_gateway"))]
        let mut client_gateway_fut = std::pin::pin!(std::future::pending::<anyhow::Result<()>>());

        let mut connection_router_fut = std::pin::pin!(connection_router.run());

        info!(profile = %config.profile, "starting substrate components");
        tokio::select! {
            res = &mut connection_router_fut => {
                match res {
                    Ok(()) => info!("connection router component finished"),
                    Err(error) => error!(error = %error, "connection router component finished with error"),
                }
            }
            res = &mut registry_fut => {
                match res {
                    Ok(()) => info!("service registry component finished"),
                    Err(error) => error!(error = %error, "service registry component finished with error"),
                }
            }
            res = &mut coordinator_bridge_fut => {
                match res {
                    Ok(()) => info!("coordinator component finished"),
                    Err(error) => error!(error = %error, "coordinator component finished with error"),
                }
            }
            res = &mut client_gateway_fut => {
                match res {
                    Ok(()) => info!("http proxy component finished"),
                    Err(error) => error!(error = %error, "http proxy component finished with error"),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                warn!("received ctrl-c signal");
            }
        }
    }

    info!("shutting down substrate components");

    #[cfg(feature = "client_gateway")]
    if config.roles.client_gateway.is_some()
        && let Err(e) = client_gateway.shutdown().await
    {
        error!(error = %e, "error shutting down http proxy");
    }

    #[cfg(feature = "coordinator")]
    if config.roles.coordinator.is_some()
        && let Err(e) = coordinator.shutdown().await
    {
        error!(error = %e, "error shutting down coordinator");
    }

    #[cfg(feature = "community_registry")]
    if config.roles.community_registry.is_some()
        && let Err(e) = community_registry.shutdown().await
    {
        error!(error = %e, "error shutting down service registry");
    }

    if let Err(e) = observability_engine.shutdown().await {
        error!(error = %e, "error flushing observability data");
    }

    if let Err(e) = connection_router.shutdown().await {
        error!(error = %e, "error shutting down connection router");
    }

    info!("shutdown complete");

    Ok(())
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

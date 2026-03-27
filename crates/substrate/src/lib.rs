//! Main library entry point for the Syneroym substrate.

pub mod identity;
pub mod substrate_service;

use std::sync::Arc;
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::EndpointRegistry;
use syneroym_core::registry::SubstrateEndpoint;
use syneroym_router::ConnectionRouter;
use tracing::{error, info, warn};

/// Runs the substrate given the consolidated configuration.
pub async fn run(config: SubstrateConfig) -> anyhow::Result<()> {
    // This is the main entry point for the substrate logic within the library.
    info!(profile = %config.profile, "starting syneroym substrate");

    let observability_engine = syneroym_observability::ObservabilityEngine::init(&config)?;

    #[cfg(feature = "ecosystem_registry")]
    let mut ecosystem_registry =
        syneroym_ecosystem_registry::EcosystemRegistry::init(&config).await?;

    #[cfg(feature = "coordinator")]
    let mut coordinator = syneroym_coordinator::EcosystemCoordinator::init(&config).await?;

    #[cfg(feature = "app_sandbox")]
    let _app_sandbox_engine = syneroym_app_sandbox::AppSandboxEngine::new(&config);

    #[cfg(feature = "client_gateway")]
    let mut client_gateway = syneroym_client_gateway::ClientGateway::init(&config).await?;

    // Initialize Substrate Identity
    let substrate_identity_state =
        identity::setup_substrate_identity(&config.identity, &config.app_data_dir)?;
    let iroh_secret_key = identity::get_secret(&config.identity, &config.app_data_dir)?;
    let service_pubkey = substrate_identity_state.did.clone();

    // The Endpoint Registry (Internal Micro-Discovery)
    let db_path = config.app_local_data_dir.join(&config.storage.db_dir);
    if !db_path.exists() {
        std::fs::create_dir_all(&db_path)?;
    }
    let db_url = format!("sqlite://{}/endpoints.db?mode=rwc", db_path.to_string_lossy());
    let endpoint_registry = Arc::new(EndpointRegistry::new(&db_url).await?);

    // The Connection Router (The Data Plane)
    let connection_router =
        ConnectionRouter::init(endpoint_registry.clone(), config.clone(), iroh_secret_key).await?;

    let substrate_service = Arc::new(substrate_service::SubstrateService::new(
        service_pubkey.clone(),
        &config,
        endpoint_registry.clone(),
    ));

    // Register the native SubstrateService at startup
    // The SubstrateService itself no longer handles its own registration.
    // Instead, the main runtime registers it with the EndpointRegistry and
    // registers the service instance with the ConnectionRouter for direct dispatch.
    let service_id = substrate_service.service_id().to_string();
    let endpoint = SubstrateEndpoint::NativeHostChannel { channel_id: service_id.clone() };

    info!("Registering native SubstrateService at {}", service_id);
    endpoint_registry.register(service_id.clone(), endpoint).await?;
    connection_router.register_native_service(service_id, substrate_service);

    {
        let mut connection_router_fut = std::pin::pin!(connection_router.clone().run());

        // We use std::future::pending() for components that are disabled via
        // compile-time features or not configured at runtime. This creates a future
        // that never resolves, ensuring tokio::select! ignores these inactive branches
        // and keeps running until an active component finishes or a shutdown signal is received.
        #[cfg(feature = "ecosystem_registry")]
        let mut registry_fut = std::pin::pin!(async {
            if config.roles.ecosystem_registry.is_some() {
                ecosystem_registry.run().await
            } else {
                std::future::pending().await
            }
        });
        #[cfg(not(feature = "ecosystem_registry"))]
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

        info!("entering main select loop");
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

    info!("shutting down components");

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

    #[cfg(feature = "ecosystem_registry")]
    if config.roles.ecosystem_registry.is_some()
        && let Err(e) = ecosystem_registry.shutdown().await
    {
        error!(error = %e, "error shutting down service registry");
    }

    if let Err(e) = observability_engine.shutdown().await {
        error!(error = %e, "error flushing observability data");
    }

    info!("shutdown complete");

    Ok(())
}

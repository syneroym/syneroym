//! Main library entry point for the Syneroym substrate.

pub mod identity;

use syneroym_core::SubstrateSubsystem;
use syneroym_core::config::SubstrateConfig;

/// Runs the substrate given the consolidated configuration.
pub async fn run(config: SubstrateConfig) -> anyhow::Result<()> {
    // This is the main entry point for the substrate logic within the library.
    println!("Starting Syneroym Substrate with profile '{}'", config.profile);

    let observability_engine = syneroym_observability::ObservabilityEngine::init(&config)?;

    #[cfg(feature = "service_registry")]
    let mut service_registry = syneroym_service_registry::ServiceRegistry::new(&config);

    #[cfg(feature = "coordinator")]
    let mut coordinator_bridge = syneroym_coordinator::CoordinatorSubsystem::new(&config);

    #[cfg(feature = "app_sandbox")]
    let _app_sandbox_engine = syneroym_app_sandbox::AppSandboxEngine::new(&config);

    #[cfg(feature = "http_proxy")]
    let mut http_proxy = syneroym_http_proxy::LocalHttpProxy::new(&config);

    // Initialize Substrate Identity
    let _substrate_identity_state =
        identity::setup_substrate_identity(&config.identity, &config.app_data_dir)?;

    #[cfg(feature = "service_registry")]
    if config.roles.service_registry.is_some() {
        service_registry.init().await?;
    }

    #[cfg(feature = "coordinator")]
    if config.roles.coordinator.is_some() {
        coordinator_bridge.init().await?;
    }

    #[cfg(feature = "http_proxy")]
    if config.roles.http_proxy.is_some() {
        http_proxy.init().await?;
    }

    {
        // We use std::future::pending() for components that are disabled via
        // compile-time features or not configured at runtime. This creates a future
        // that never resolves, ensuring tokio::select! ignores these inactive branches
        // and keeps running until an active component finishes or a shutdown signal is received.
        #[cfg(feature = "service_registry")]
        let mut registry_fut = std::pin::pin!(async {
            if config.roles.service_registry.is_some() {
                service_registry.run().await
            } else {
                std::future::pending().await
            }
        });
        #[cfg(not(feature = "service_registry"))]
        let mut registry_fut = std::pin::pin!(std::future::pending::<anyhow::Result<()>>());

        #[cfg(feature = "coordinator")]
        let mut coordinator_bridge_fut = std::pin::pin!(async {
            if config.roles.coordinator.is_some() {
                coordinator_bridge.run().await
            } else {
                std::future::pending().await
            }
        });
        #[cfg(not(feature = "coordinator"))]
        let mut coordinator_bridge_fut =
            std::pin::pin!(std::future::pending::<anyhow::Result<()>>());

        #[cfg(feature = "http_proxy")]
        let mut http_proxy_fut = std::pin::pin!(async {
            if config.roles.http_proxy.is_some() {
                http_proxy.run().await
            } else {
                std::future::pending().await
            }
        });
        #[cfg(not(feature = "http_proxy"))]
        let mut http_proxy_fut = std::pin::pin!(std::future::pending::<anyhow::Result<()>>());

        println!("entering main select loop");
        tokio::select! {
            res = &mut registry_fut => {
                println!("Service registry component finished: {:?}", res);
            }
            res = &mut coordinator_bridge_fut => {
                println!("Coordinator/Bridge component finished: {:?}", res);
            }
            res = &mut http_proxy_fut => {
                println!("HTTP proxy component finished: {:?}", res);
            }
            _ = tokio::signal::ctrl_c() => {
                println!("Received ctrl-c signal");
            }
        }
    }

    println!("Shutting down components...");

    #[cfg(feature = "http_proxy")]
    if config.roles.http_proxy.is_some()
        && let Err(e) = http_proxy.shutdown().await
    {
        eprintln!("Error shutting down HTTP proxy: {}", e);
    }

    #[cfg(feature = "coordinator")]
    if config.roles.coordinator.is_some()
        && let Err(e) = coordinator_bridge.shutdown().await
    {
        eprintln!("Error shutting down coordinator/bridge: {}", e);
    }

    #[cfg(feature = "service_registry")]
    if config.roles.service_registry.is_some()
        && let Err(e) = service_registry.shutdown().await
    {
        eprintln!("Error shutting down service registry: {}", e);
    }

    if let Err(e) = observability_engine.shutdown().await {
        eprintln!("Error flushing observability data: {}", e);
    }

    println!("Shutdown complete.");

    Ok(())
}

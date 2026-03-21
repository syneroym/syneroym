use syneroym_core::SubstrateComponent;
use syneroym_core::config::SubstrateConfig;

/// Runs the substrate given the consolidated configuration.
pub async fn run(config: SubstrateConfig) -> anyhow::Result<()> {
    // This is the main entry point for the substrate logic within the library.
    println!("Starting Syneroym Substrate with profile '{}'", config.profile);

    let mut observability = syneroym_observability::ObservabilityComponent::new(&config);

    #[cfg(feature = "service_registry")]
    let mut service_registry = syneroym_service_registry::ServiceRegistryComponent::new(&config);

    #[cfg(feature = "coordinator")]
    let mut coordinator_bridge = syneroym_coordinator::CoordinatorBridgeComponent::new(&config);

    #[cfg(feature = "app_sandbox")]
    let mut app_sandbox = syneroym_app_sandbox::AppSandboxComponent::new(&config);

    #[cfg(feature = "http_proxy")]
    let mut http_proxy = syneroym_http_proxy::HttpProxyComponent::new(&config);

    observability.init().await?;

    #[cfg(feature = "service_registry")]
    if config.roles.service_registry.is_some() {
        service_registry.init().await?;
    }

    #[cfg(feature = "coordinator")]
    if config.roles.coordinator.is_some() {
        coordinator_bridge.init().await?;
    }

    #[cfg(feature = "app_sandbox")]
    if config.roles.app_sandbox.is_some() {
        app_sandbox.init().await?;
    }

    #[cfg(feature = "http_proxy")]
    if config.roles.http_proxy.is_some() {
        http_proxy.init().await?;
    }

    {
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

        #[cfg(feature = "app_sandbox")]
        let mut app_sandbox_fut = std::pin::pin!(async {
            if config.roles.app_sandbox.is_some() {
                app_sandbox.run().await
            } else {
                std::future::pending().await
            }
        });
        #[cfg(not(feature = "app_sandbox"))]
        let mut app_sandbox_fut = std::pin::pin!(std::future::pending::<anyhow::Result<()>>());

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

        tokio::select! {
            res = observability.run() => {
                println!("Observability component finished: {:?}", res);
            }
            res = &mut registry_fut => {
                println!("Service registry component finished: {:?}", res);
            }
            res = &mut coordinator_bridge_fut => {
                println!("Coordinator/Bridge component finished: {:?}", res);
            }
            res = &mut app_sandbox_fut => {
                println!("App sandbox component finished: {:?}", res);
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

    #[cfg(feature = "app_sandbox")]
    if config.roles.app_sandbox.is_some()
        && let Err(e) = app_sandbox.shutdown().await
    {
        eprintln!("Error shutting down app sandbox: {}", e);
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

    if let Err(e) = observability.shutdown().await {
        eprintln!("Error shutting down observability: {}", e);
    }

    println!("Shutdown complete.");

    Ok(())
}

//! Substrate execution runtime
//!
//! Manages the lifecycle of all substrate components including the App Sandbox,
//! Observability engine, Router, Client Gateway, and Coordinators.

use std::{
    fmt::{self, Debug, Formatter},
    future::Future,
};

use syneroym_core::{config::SubstrateConfig, local_registry::EndpointRegistry};
use syneroym_observability::ObservabilityEngine;
use syneroym_router::ConnectionRouter;
use tokio::signal;
use tracing::{error, info};

mod auth;
mod dual_build_fixture;
mod handles;
mod publish;
mod router;
mod roym;
mod services;
mod supervisor;

#[cfg(test)]
mod tests;

#[cfg(test)]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use services::RuntimeServices;
#[cfg(test)]
use syneroym_app_orchestration::{
    AppInstanceId, AppRegistry, LogicalServiceName, TopologyEntry, TopologyKey,
};
#[cfg(test)]
use syneroym_identity::Identity;

use self::router::setup_connection_router;
#[cfg(test)]
use self::{publish::warn_on_near_expiry_instance_certs, router::replay_persisted_bindings};

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
    /// The same registry `connection_router` routes through -- kept here too
    /// so `RuntimeServices::run_until_shutdown` can run the instance-
    /// certificate expiry sweep without `ConnectionRouter` growing a getter
    /// for something external to routing.
    pub endpoint_registry: EndpointRegistry,
}

impl Debug for InitializedRuntime {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("InitializedRuntime")
            .field("observability", &"ObservabilityEngine")
            .field("services", &self.services)
            .field("connection_router", &"ConnectionRouter")
            .field("endpoint_registry", &"EndpointRegistry")
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
    runtime
        .services
        .run_until_shutdown(
            &config,
            &runtime.connection_router,
            &runtime.endpoint_registry,
            shutdown_signal,
        )
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

/// Runs the substrate given the consolidated configuration and a custom
/// shutdown signal.
pub async fn init(config: SubstrateConfig) -> anyhow::Result<InitializedRuntime> {
    info!(profile = %config.profile, "initializing substrate");

    let observability = ObservabilityEngine::init(&config)?;
    // `community_registry`/`coordinator`/`client_gateway` construct first,
    // exactly as before this role existed -- swapping this order (tried
    // first, reverted) measurably slowed every substrate's startup, ~6s
    // to 15-30s against the same config, even with `[roles.supervisor]`
    // absent. `RuntimeServices::init` and `setup_connection_router` both
    // do real network setup (iroh endpoint bring-up, DHT bootstrap), and
    // apparently benefit from *this* relative order in ways not worth
    // taking on faith a second time. The supervisor role, constructed
    // inside `setup_connection_router` because it needs handles
    // (`KeyStore`, `StorageProvider`, `native_dispatch`, the node's own
    // identity) that only exist once the connection router's own
    // composition root has built them, is injected into `RuntimeServices`
    // afterward instead of changing when either call runs.
    let mut services = RuntimeServices::init(&config).await?;
    let (connection_router, endpoint_registry, supervisor, conversation, auth_did) =
        setup_connection_router(&config).await?;
    services.set_supervisor(supervisor);
    services.set_conversation(conversation);
    #[cfg(feature = "client_gateway")]
    services.set_gateway_auth_did(auth_did);

    Ok(InitializedRuntime { observability, services, connection_router, endpoint_registry })
}

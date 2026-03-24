//! Service registry component for discovering and managing services within a Syneroym ecosystem.

use anyhow::Result;
use syneroym_core::SubstrateSubsystem;
use syneroym_core::config::SubstrateConfig;
use tracing::info;

pub struct ServiceRegistry {}

impl ServiceRegistry {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}

impl SubstrateSubsystem for ServiceRegistry {
    async fn init(&mut self) -> Result<()> {
        info!("initializing service registry");
        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        info!("running service registry");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        info!("shutting down service registry");
        Ok(())
    }
}

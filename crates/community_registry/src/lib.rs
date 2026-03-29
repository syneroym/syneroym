//! Service registry for discovering and managing services within a Syneroym ecosystem.

use anyhow::Result;
use syneroym_core::config::SubstrateConfig;
use tracing::info;

pub struct EcosystemRegistry {}

impl EcosystemRegistry {
    pub async fn init(_config: &SubstrateConfig) -> Result<Self> {
        info!("initializing service registry");
        Ok(Self {})
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("running service registry");
        std::future::pending::<()>().await;
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("shutting down service registry");
        Ok(())
    }
}

//! HTTP proxy component for routing external requests.

use anyhow::Result;
use syneroym_core::SubstrateSubsystem;
use syneroym_core::config::SubstrateConfig;
use tracing::info;

pub struct LocalHttpProxy {}

impl LocalHttpProxy {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}

impl SubstrateSubsystem for LocalHttpProxy {
    async fn init(&mut self) -> Result<()> {
        info!("initializing http proxy");
        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        info!("running http proxy");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        info!("shutting down http proxy");
        Ok(())
    }
}

//! Service registry component for discovering and managing services within a Syneroym ecosystem.

use anyhow::Result;
use syneroym_core::SubstrateSubsystem;
use syneroym_core::config::SubstrateConfig;

pub struct ServiceRegistry {}

impl ServiceRegistry {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}

impl SubstrateSubsystem for ServiceRegistry {
    async fn init(&mut self) -> Result<()> {
        println!("Initializing Service Registry");
        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        println!("Running Service Registry");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        println!("Shutting down Service Registry");
        Ok(())
    }
}

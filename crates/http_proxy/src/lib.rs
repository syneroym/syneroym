//! HTTP proxy component for routing external requests.

use anyhow::Result;
use syneroym_core::SubstrateSubsystem;
use syneroym_core::config::SubstrateConfig;

pub struct LocalHttpProxy {}

impl LocalHttpProxy {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}

impl SubstrateSubsystem for LocalHttpProxy {
    async fn init(&mut self) -> Result<()> {
        println!("Initializing HTTP Proxy");
        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        println!("Running HTTP Proxy");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        println!("Shutting down HTTP Proxy");
        Ok(())
    }
}

//! HTTP proxy component for routing external requests.

use anyhow::Result;
use syneroym_core::SubstrateComponent;
use syneroym_core::config::SubstrateConfig;

pub struct HttpProxyComponent {}

impl HttpProxyComponent {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}

impl SubstrateComponent for HttpProxyComponent {
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

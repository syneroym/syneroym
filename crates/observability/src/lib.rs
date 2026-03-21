use anyhow::Result;
use syneroym_core::SubstrateComponent;
use syneroym_core::config::SubstrateConfig;

pub struct ObservabilityComponent {
    // config: SubstrateConfig,
}

impl ObservabilityComponent {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self { /* config: config.clone() */ }
    }
}

impl SubstrateComponent for ObservabilityComponent {
    async fn init(&mut self) -> Result<()> {
        println!("Initializing Observability");
        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        println!("Running Observability");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        println!("Shutting down Observability");
        Ok(())
    }
}

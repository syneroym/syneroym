use anyhow::Result;
use syneroym_core::SubstrateComponent;
use syneroym_core::config::SubstrateConfig;

pub struct CoordinatorIroh {}

impl CoordinatorIroh {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}

impl SubstrateComponent for CoordinatorIroh {
    async fn init(&mut self) -> Result<()> {
        println!("Initializing Coordinator IROH");
        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        println!("Running Coordinator IROH");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        println!("Shutting down Coordinator IROH");
        Ok(())
    }
}

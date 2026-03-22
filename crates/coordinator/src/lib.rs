use anyhow::Result;
use syneroym_core::SubstrateComponent;
use syneroym_core::config::SubstrateConfig;

pub struct CoordinatorComponent {}

impl CoordinatorComponent {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}

impl SubstrateComponent for CoordinatorComponent {
    async fn init(&mut self) -> Result<()> {
        println!("Initializing Coordinator and Transport Bridge");
        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        println!("Running Coordinator and Transport Bridge");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        println!("Shutting down Coordinator and Transport Bridge");
        Ok(())
    }
}

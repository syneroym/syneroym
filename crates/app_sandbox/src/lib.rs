use anyhow::Result;
use syneroym_core::SubstrateComponent;
use syneroym_core::config::SubstrateConfig;

pub struct AppSandboxComponent {}

impl AppSandboxComponent {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}

impl SubstrateComponent for AppSandboxComponent {
    async fn init(&mut self) -> Result<()> {
        println!("Initializing App Sandbox");
        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        println!("Running App Sandbox");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        println!("Shutting down App Sandbox");
        Ok(())
    }
}

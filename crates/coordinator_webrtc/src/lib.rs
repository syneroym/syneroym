//! WebRTC transport coordinator component.

use anyhow::Result;
use syneroym_core::SubstrateComponent;
use syneroym_core::config::SubstrateConfig;

pub struct CoordinatorWebRtc {}

impl CoordinatorWebRtc {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}

impl SubstrateComponent for CoordinatorWebRtc {
    async fn init(&mut self) -> Result<()> {
        println!("Initializing Coordinator WebRTC");
        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        println!("Running Coordinator WebRTC");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        println!("Shutting down Coordinator WebRTC");
        Ok(())
    }
}

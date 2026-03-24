//! WebRTC transport coordinator component.

use anyhow::Result;
use syneroym_core::SubstrateSubsystem;
use syneroym_core::config::SubstrateConfig;
use tracing::info;

pub struct CoordinatorWebRtc {}

impl CoordinatorWebRtc {
    pub fn new(_config: &SubstrateConfig) -> Self {
        Self {}
    }
}

impl SubstrateSubsystem for CoordinatorWebRtc {
    async fn init(&mut self) -> Result<()> {
        info!("initializing coordinator webrtc");
        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        info!("running coordinator webrtc");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        info!("shutting down coordinator webrtc");
        Ok(())
    }
}

use anyhow::Result;
use syneroym_core::config::SubstrateConfig;
use tracing::info;

pub struct CoordinatorWebRtc {}

impl CoordinatorWebRtc {
    pub async fn init(_config: &SubstrateConfig) -> Result<Self> {
        info!("initializing coordinator webrtc");
        Ok(Self {})
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("running coordinator webrtc");
        std::future::pending::<()>().await;
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("shutting down coordinator webrtc");
        Ok(())
    }
}

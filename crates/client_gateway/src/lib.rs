//! HTTP proxy component for routing local http client requests to the appropriate substrate within ecosystem

use anyhow::Result;
use syneroym_core::config::SubstrateConfig;
use tracing::info;

/// ClientGateway: Acts as an entry point for local HTTP/WebSocket clients to reach the wider Syneroym network.
/// It accepts HTTP/WebSocket traffic and would ultimately forward it to the target substrate's ConnectionRouter (which contains the ProtocolConverter).
pub struct ClientGateway {}

impl ClientGateway {
    pub async fn init(_config: &SubstrateConfig) -> Result<Self> {
        info!("initializing client gateway");
        Ok(Self {})
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("running client gateway");
        // Here we would bind the HTTP/WebSocket ports and pass incoming JSON-RPC to the ConnectionRouter.
        std::future::pending::<()>().await;
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("shutting down client gateway");
        Ok(())
    }
}

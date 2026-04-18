// Adapted from: https://github.com/n0-computer/iroh/blob/main/iroh-relay/src/main.rs
// Original license: MIT OR Apache-2.0
// Used under MIT OR Apache-2.0 License
use anyhow::{Context, Result};
use iroh_relay::server::Server;
use syneroym_core::config::SubstrateConfig;
use tracing::{debug, info};

use crate::config::build_relay_config;

pub struct CoordinatorIroh {
    server: Option<Server>,
}

impl CoordinatorIroh {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("Initializing Coordinator IROH");
        let config_roles = config.roles.coordinator.clone();
        let mut server = None;
        if let Some(role) = &config_roles {
            let server_config = build_relay_config(role).await?;
            debug!("Iroh Relay Config built: {:?}", server_config.relay.is_some());
            server = Some(
                Server::spawn(server_config).await.context("failed to spawn iroh relay server")?,
            );
        }
        Ok(Self { server })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Running Coordinator IROH");
        if let Some(server) = &mut self.server {
            server.task_handle().await.context("iroh relay server task panicked")??;
        } else {
            std::future::pending::<()>().await;
        }
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down Coordinator IROH");
        if let Some(server) = self.server.take() {
            server.shutdown().await.context("failed to cleanly shutdown iroh relay server")?;
        }
        Ok(())
    }
}

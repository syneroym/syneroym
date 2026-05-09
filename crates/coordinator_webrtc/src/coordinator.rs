use crate::bootstrap::{self, BootstrapState};
use crate::signalling;
use anyhow::Result;
use std::sync::Arc;
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::EndpointRegistry;
use syneroym_core::storage;
use tracing::info;

pub struct CoordinatorWebRtc {
    bootstrap_port: u16,
    signalling_port: u16,
    bootstrap_state: Arc<BootstrapState>,
}

impl CoordinatorWebRtc {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("Initializing coordinator webrtc");

        let webrtc_config = config
            .roles
            .coordinator
            .as_ref()
            .and_then(|c| c.webrtc.as_ref())
            .ok_or_else(|| anyhow::anyhow!("WebRTC coordinator configuration missing"))?;

        let bootstrap_port = webrtc_config
            .bootstrap_page_bind_address
            .split(':')
            .next_back()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(7002);

        let signalling_port = webrtc_config
            .signalling_bind_address
            .split(':')
            .next_back()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(7444);

        let iroh_relay_url = config.uplink.iroh.as_ref().map(|c| c.relay_url.clone());
        let endpoint = common::iroh_utils::bind_endpoint(iroh_relay_url).await?;

        let data_store = storage::init_store(config).await?;
        let registry = EndpointRegistry::new(data_store).await?;

        let bootstrap_state = Arc::new(BootstrapState {
            iroh: endpoint,
            signaling_server_url: format!("ws://localhost:{}", signalling_port), // TODO: Use domain from config
            registry,
        });

        Ok(Self { bootstrap_port, signalling_port, bootstrap_state })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Running coordinator webrtc");

        let bootstrap_fut = bootstrap::start(self.bootstrap_port, self.bootstrap_state.clone());
        let signalling_fut = signalling::start(self.signalling_port);

        tokio::try_join!(bootstrap_fut, signalling_fut)?;

        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("Shutting down coordinator webrtc");
        Ok(())
    }
}

// Internal common module for iroh utils if not available elsewhere
mod common {
    use anyhow::Result;
    use iroh::{Endpoint, SecretKey};
    pub mod iroh_utils {
        use super::*;
        pub async fn bind_endpoint(relay_url: Option<String>) -> Result<Endpoint> {
            let secret_key = SecretKey::from_bytes(&[0u8; 32]);
            let mut builder = Endpoint::builder(iroh::endpoint::presets::N0).secret_key(secret_key);
            if let Some(url) = relay_url
                && let Ok(relay_url) = url.parse::<iroh::RelayUrl>()
            {
                builder =
                    builder.relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::from(relay_url)));
            }
            let endpoint = builder.bind().await?;
            Ok(endpoint)
        }
    }
}

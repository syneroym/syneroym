use crate::bootstrap::{self, BootstrapState};
use crate::signalling;
use anyhow::Result;
use std::sync::Arc;
use syneroym_core::config::SubstrateConfig;
use syneroym_core::registry::EndpointRegistry;
use syneroym_core::storage;
use tracing::info;

pub struct CoordinatorWebRtc {
    bootstrap_listener: Option<tokio::net::TcpListener>,
    signalling_listener: Option<tokio::net::TcpListener>,
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

        let bootstrap_listener =
            tokio::net::TcpListener::bind(&webrtc_config.bootstrap_page_bind_address).await?;
        let signalling_listener =
            tokio::net::TcpListener::bind(&webrtc_config.signalling_bind_address).await?;

        let actual_signalling_port = signalling_listener.local_addr()?.port();

        let iroh_relay_url = config.uplink.iroh.as_ref().map(|c| c.relay_url.clone());
        let endpoint = common::iroh_utils::bind_endpoint(iroh_relay_url).await?;

        let data_store = storage::init_store(config).await?;
        let registry = EndpointRegistry::new(data_store).await?;

        let bootstrap_state = Arc::new(BootstrapState {
            iroh: endpoint,
            signaling_server_url: format!("ws://localhost:{}", actual_signalling_port), // TODO: Use domain from config
            registry,
        });

        Ok(Self {
            bootstrap_listener: Some(bootstrap_listener),
            signalling_listener: Some(signalling_listener),
            bootstrap_state,
        })
    }

    pub fn bootstrap_port(&self) -> u16 {
        self.bootstrap_listener
            .as_ref()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.port())
            .unwrap_or(0)
    }

    pub fn signalling_port(&self) -> u16 {
        self.signalling_listener
            .as_ref()
            .and_then(|l| l.local_addr().ok())
            .map(|a| a.port())
            .unwrap_or(0)
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Running coordinator webrtc");

        let bootstrap_listener = self
            .bootstrap_listener
            .take()
            .ok_or_else(|| anyhow::anyhow!("Bootstrap listener already taken"))?;
        let signalling_listener = self
            .signalling_listener
            .take()
            .ok_or_else(|| anyhow::anyhow!("Signalling listener already taken"))?;

        let bootstrap_fut = bootstrap::start(bootstrap_listener, self.bootstrap_state.clone());
        let signalling_fut = signalling::start(signalling_listener);

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

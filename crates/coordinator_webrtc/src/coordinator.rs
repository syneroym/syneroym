//! WebRTC Ecosystem Coordinator
//!
//! Orchestrates signaling and peer-to-peer transport bridging using WebRTC,
//! handling peer discovery and connection routing.

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

impl std::fmt::Debug for CoordinatorWebRtc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoordinatorWebRtc")
            .field(
                "bootstrap_listener",
                &self.bootstrap_listener.as_ref().map(|l| l.local_addr().ok()),
            )
            .field(
                "signalling_listener",
                &self.signalling_listener.as_ref().map(|l| l.local_addr().ok()),
            )
            .field("bootstrap_state", &self.bootstrap_state)
            .finish()
    }
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

        let iroh_relay_url = config.parent_coordinator.iroh.as_ref().map(|c| c.url.clone());
        let endpoint = common::iroh_utils::bind_endpoint(iroh_relay_url).await?;

        let data_store = storage::init_store(config).await?;
        let registry = EndpointRegistry::new(data_store).await?;

        let bootstrap_state = Arc::new(BootstrapState {
            iroh: endpoint,
            external_host: webrtc_config.external_host.clone(),
            signaling_port: actual_signalling_port,
            registry,
            registry_url: config.substrate.registry_url.clone(),
        });

        Ok(Self {
            bootstrap_listener: Some(bootstrap_listener),
            signalling_listener: Some(signalling_listener),
            bootstrap_state,
        })
    }

    pub fn bootstrap_port(&self) -> u16 {
        self.bootstrap_listener.as_ref().and_then(|l| l.local_addr().ok()).map_or(0, |a| a.port())
    }

    pub fn signalling_port(&self) -> u16 {
        self.signalling_listener.as_ref().and_then(|l| l.local_addr().ok()).map_or(0, |a| a.port())
    }

    pub fn endpoint(&self) -> iroh::Endpoint {
        self.bootstrap_state.iroh.clone()
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
        self.bootstrap_state.iroh.close().await;
        Ok(())
    }
}

// Internal common module for iroh utils if not available elsewhere
mod common {
    use anyhow::Result;
    use iroh::Endpoint;
    pub mod iroh_utils {
        use super::{Endpoint, Result};
        pub async fn bind_endpoint(relay_url: Option<String>) -> Result<Endpoint> {
            let mut builder = Endpoint::builder(iroh::endpoint::presets::N0);
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

//! Coordinator that helps peers within ecosystem discover a channel to communicate and often help relay data.

use anyhow::Result;
use syneroym_core::config::SubstrateConfig;
use tracing::info;

#[cfg(feature = "iroh")]
use syneroym_coordinator_iroh::CoordinatorIroh;
#[cfg(feature = "webrtc")]
use syneroym_coordinator_webrtc::CoordinatorWebRtc;

pub struct EcosystemCoordinator {
    #[cfg(feature = "iroh")]
    iroh_coordinator: Option<CoordinatorIroh>,
    #[cfg(feature = "webrtc")]
    webrtc_coordinator: Option<CoordinatorWebRtc>,
}

impl EcosystemCoordinator {
    pub async fn init(config: &SubstrateConfig) -> Result<Self> {
        info!("initializing coordinator and transport bridge");

        #[cfg(feature = "iroh")]
        let iroh_coordinator = if let Some(role) = &config.roles.coordinator {
            if role.iroh.is_some() { Some(CoordinatorIroh::init(config).await?) } else { None }
        } else {
            None
        };

        #[cfg(feature = "webrtc")]
        let webrtc_coordinator = if let Some(role) = &config.roles.coordinator {
            if role.webrtc.is_some() { Some(CoordinatorWebRtc::init(config).await?) } else { None }
        } else {
            None
        };

        Ok(Self {
            #[cfg(feature = "iroh")]
            iroh_coordinator,
            #[cfg(feature = "webrtc")]
            webrtc_coordinator,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("running coordinator and transport bridge");

        let mut _is_empty = true;
        #[cfg(feature = "iroh")]
        {
            if self.iroh_coordinator.is_some() {
                _is_empty = false;
            }
        }
        #[cfg(feature = "webrtc")]
        {
            if self.webrtc_coordinator.is_some() {
                _is_empty = false;
            }
        }

        if _is_empty {
            std::future::pending::<()>().await;
            return Ok(());
        }

        let EcosystemCoordinator {
            #[cfg(feature = "iroh")]
                iroh_coordinator: iroh_component,
            #[cfg(feature = "webrtc")]
                webrtc_coordinator: webrtc_component,
        } = self;

        #[cfg(feature = "iroh")]
        let iroh_run = async { if let Some(c) = iroh_component { c.run().await } else { Ok(()) } };

        #[cfg(feature = "webrtc")]
        let webrtc_run =
            async { if let Some(c) = webrtc_component { c.run().await } else { Ok(()) } };

        #[cfg(all(feature = "iroh", feature = "webrtc"))]
        {
            tokio::try_join!(iroh_run, webrtc_run)?;
        }
        #[cfg(all(feature = "iroh", not(feature = "webrtc")))]
        {
            iroh_run.await?;
        }
        #[cfg(all(not(feature = "iroh"), feature = "webrtc"))]
        {
            webrtc_run.await?;
        }

        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        info!("shutting down coordinator and transport bridge");

        #[cfg(feature = "iroh")]
        if let Some(c) = &mut self.iroh_coordinator {
            c.shutdown().await?;
        }

        #[cfg(feature = "webrtc")]
        if let Some(c) = &mut self.webrtc_coordinator {
            c.shutdown().await?;
        }

        Ok(())
    }
}

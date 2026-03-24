//! Coordinator component that relays data and also bridges various transport protocols.

use anyhow::Result;
use syneroym_core::SubstrateSubsystem;
use syneroym_core::config::SubstrateConfig;

#[cfg(feature = "iroh")]
use syneroym_coordinator_iroh::CoordinatorIroh;
#[cfg(feature = "webrtc")]
use syneroym_coordinator_webrtc::CoordinatorWebRtc;

pub struct CoordinatorSubsystem {
    #[cfg(feature = "iroh")]
    iroh_coordinator: Option<CoordinatorIroh>,
    #[cfg(feature = "webrtc")]
    webrtc_coordinator: Option<CoordinatorWebRtc>,
}

impl CoordinatorSubsystem {
    pub fn new(config: &SubstrateConfig) -> Self {
        #[cfg(feature = "iroh")]
        let iroh_coordinator = if let Some(role) = &config.roles.coordinator {
            if role.iroh.is_some() { Some(CoordinatorIroh::new(config)) } else { None }
        } else {
            None
        };

        #[cfg(feature = "webrtc")]
        let webrtc_coordinator = if let Some(role) = &config.roles.coordinator {
            if role.webrtc.is_some() { Some(CoordinatorWebRtc::new(config)) } else { None }
        } else {
            None
        };

        Self {
            #[cfg(feature = "iroh")]
            iroh_coordinator,
            #[cfg(feature = "webrtc")]
            webrtc_coordinator,
        }
    }
}

impl SubstrateSubsystem for CoordinatorSubsystem {
    async fn init(&mut self) -> Result<()> {
        println!("Initializing Coordinator and Transport Bridge");

        #[cfg(feature = "iroh")]
        if let Some(c) = &mut self.iroh_coordinator {
            c.init().await?;
        }

        #[cfg(feature = "webrtc")]
        if let Some(c) = &mut self.webrtc_coordinator {
            c.init().await?;
        }

        Ok(())
    }

    async fn run(&mut self) -> Result<()> {
        println!("Running Coordinator and Transport Bridge");

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

        let CoordinatorSubsystem {
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

    async fn shutdown(&mut self) -> Result<()> {
        println!("Shutting down Coordinator and Transport Bridge");

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

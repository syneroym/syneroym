//! Core transport bridge and multi-transport coordination logic
//!
//! Initializes, manages, and orchestrates transport coordinators (like Iroh
//! and WebRTC) based on global Substrate configurations.

use std::future;

use anyhow::Result;
#[cfg(feature = "iroh")]
use syneroym_coordinator_iroh::CoordinatorIroh;
#[cfg(feature = "webrtc")]
use syneroym_coordinator_webrtc::CoordinatorWebRtc;
use syneroym_core::{config::SubstrateConfig, host_identity::HostIdentity};
use syneroym_passport::LeaseStore;
use tracing::{info, warn};

#[derive(Debug)]
pub struct EcosystemCoordinator {
    #[cfg(feature = "iroh")]
    iroh_coordinator: Option<CoordinatorIroh>,
    #[cfg(feature = "webrtc")]
    webrtc_coordinator: Option<CoordinatorWebRtc>,
}

impl EcosystemCoordinator {
    pub async fn init(config: &SubstrateConfig, host: &HostIdentity) -> Result<Self> {
        info!("initializing coordinator and transport bridge");

        // Before any coordinator is built, so a passport form without [tls]
        // stops startup also for a role with no Iroh section (WebRTC only, or
        // nothing): the access value must never wait unused.
        if let Some(role) = &config.roles.coordinator {
            role.access.check_tls(config.tls.is_some())?;
            // With an Iroh section, `CoordinatorIroh::init` warns about a relay
            // that is off; warn here only when there is no Iroh section at all.
            if role.access.accepts_passports() && role.iroh.is_none() {
                warn!(
                    "[roles.coordinator] access accepts passports, but no Iroh relay runs; it has \
                     no effect"
                );
            }
        }

        #[cfg(not(feature = "iroh"))]
        let _ = host;

        #[cfg(feature = "iroh")]
        let iroh_coordinator = if let Some(role) = &config.roles.coordinator {
            if role.iroh.is_some() {
                Some(CoordinatorIroh::init(config, host).await?)
            } else {
                None
            }
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

    /// The lease store of the Iroh relay when it accepts passports.
    #[must_use]
    pub fn relay_leases(&self) -> Option<LeaseStore> {
        #[cfg(feature = "iroh")]
        {
            self.iroh_coordinator.as_ref().and_then(CoordinatorIroh::relay_leases)
        }
        #[cfg(not(feature = "iroh"))]
        {
            None
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("running coordinator and transport bridge");

        let mut is_empty = true;
        #[cfg(feature = "iroh")]
        {
            if self.iroh_coordinator.is_some() {
                is_empty = false;
            }
        }
        #[cfg(feature = "webrtc")]
        {
            if self.webrtc_coordinator.is_some() {
                is_empty = false;
            }
        }

        if is_empty {
            future::pending::<()>().await;
            return Ok(());
        }

        let Self {
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

#[cfg(test)]
mod tests;
